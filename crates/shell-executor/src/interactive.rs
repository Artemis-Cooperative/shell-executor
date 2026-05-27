//! Interactive PTY-backed execution.
//!
//! The wrapped command runs on a pseudo-terminal so TUI programs work
//! normally. The session uses the terminal's alternate screen for the
//! duration of the command; on exit, the alt screen is left and the
//! pass/fail status line is printed on the main screen.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI tool intentionally writes to stdout/stderr for user-facing output"
)]

use std::io::{self, Read, Write};
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::outcome::{Outcome, OutputCapture};
use crate::{format_elapsed, RunReport, RunStatus};

struct RawModeGuard;
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

/// Owns the "alternate screen" terminal state. Constructing it writes the
/// enter sequence (DECSET 1049); dropping it writes the exit sequence so the
/// terminal returns to the main screen and the interactive session's output
/// disappears from scrollback.
struct AltScreenGuard;
impl AltScreenGuard {
    fn enter() -> Self {
        let mut out = io::stdout().lock();
        let _ = out.write_all(b"\x1b[?1049h");
        let _ = out.flush();
        AltScreenGuard
    }
}
impl Drop for AltScreenGuard {
    fn drop(&mut self) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(b"\x1b[?1049l");
        let _ = out.flush();
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "single coherent PTY lifecycle; splitting would obscure control flow"
)]
pub(crate) fn run_interactive_report(
    command: &str,
    display_message: &str,
    log: Option<&PathBuf>,
    show_time: bool,
) -> RunReport {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to open PTY: {e}");
            return RunReport {
                status: RunStatus::Failure,
                exit_code: 1,
            };
        }
    };

    let mut cmd_builder = CommandBuilder::new("sh");
    cmd_builder.arg("-c");
    cmd_builder.arg(command);
    if let Ok(cwd) = std::env::current_dir() {
        cmd_builder.cwd(cwd);
    }
    for (k, v) in std::env::vars_os() {
        cmd_builder.env(k, v);
    }

    let mut child = match pair.slave.spawn_command(cmd_builder) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to spawn command: {e}");
            return RunReport {
                status: RunStatus::Failure,
                exit_code: 1,
            };
        }
    };
    drop(pair.slave);

    let reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to clone PTY reader: {e}");
            let _ = child.kill();
            return RunReport {
                status: RunStatus::Failure,
                exit_code: 1,
            };
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Failed to take PTY writer: {e}");
            let _ = child.kill();
            return RunReport {
                status: RunStatus::Failure,
                exit_code: 1,
            };
        }
    };

    // Held to keep the PTY master alive for the duration of the session.
    // On Unix it's also shared with the SIGWINCH thread to forward resizes.
    #[cfg(unix)]
    let master = Arc::new(Mutex::new(pair.master));
    #[cfg(not(unix))]
    let _master = pair.master;

    if let Err(e) = enable_raw_mode() {
        eprintln!("Failed to enable raw mode: {e}");
        let _ = child.kill();
        return RunReport {
            status: RunStatus::Failure,
            exit_code: 1,
        };
    }
    let raw_guard = RawModeGuard;
    let alt_guard = AltScreenGuard::enter();

    #[cfg(unix)]
    {
        use signal_hook::consts::SIGWINCH;
        let master_for_sig = Arc::clone(&master);
        if let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGWINCH]) {
            thread::spawn(move || {
                for _ in signals.forever() {
                    if let Ok((cols, rows)) = crossterm::terminal::size() {
                        if let Ok(m) = master_for_sig.lock() {
                            let _ = m.resize(PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            });
                        }
                    }
                }
            });
        }
    }

    // PTY output -> stdout
    thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut out = io::stdout().lock();
            if out.write_all(&buf[..n]).is_err() {
                break;
            }
            let _ = out.flush();
        }
    });

    // Parent stdin -> PTY
    thread::spawn(move || {
        let mut writer = writer;
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        let mut buf = [0u8; 256];
        loop {
            let n = match handle.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if writer.write_all(&buf[..n]).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });

    let start = Instant::now();
    let exit_code: i32 = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                #[expect(
                    clippy::cast_possible_wrap,
                    clippy::as_conversions,
                    reason = "exit codes are at most 255 in practice; wide cast preserves the value"
                )]
                let code = status.exit_code() as i32;
                break code;
            }
            Ok(None) => {}
            Err(_) => break 1,
        }
        thread::sleep(Duration::from_millis(50));
    };

    let elapsed = start.elapsed();
    let final_time = format_elapsed(elapsed);
    let success = exit_code == 0;
    let status = if success {
        RunStatus::Success
    } else {
        RunStatus::Failure
    };

    let outcome = Outcome {
        status,
        output: OutputCapture::Inherited,
        elapsed,
        label: display_message.to_string(),
        signal_num: None,
    };

    // Exit alt screen first (so the status line lands on the main screen
    // where the user typed the command), then disable raw mode so println
    // line endings get translated normally.
    drop(alt_guard);
    drop(raw_guard);

    let time_slot = if show_time {
        format!(" {final_time}")
    } else {
        String::new()
    };
    if matches!(outcome.status, RunStatus::Success) {
        println!("[ \x1b[32m✓\x1b[0m{time_slot} ] {display_message}");
    } else {
        println!("[ \x1b[31m✘\x1b[0m{time_slot} ] {display_message}");
    }

    if let Some(log_path) = log {
        let now = chrono::Local::now();
        let timestamp = now.format("%Y-%m-%d %H:%M:%S");
        let icon = if matches!(outcome.status, RunStatus::Success) {
            "✓"
        } else {
            "✘"
        };
        let entry = format!("[{timestamp}] [ {icon} {final_time} ] {display_message}\n");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            let _ = file.write_all(entry.as_bytes());
        }
    }

    RunReport {
        status: outcome.status,
        exit_code,
    }
}
