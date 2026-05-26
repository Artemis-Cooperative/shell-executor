//! CLI integration tests that drive the PTY-backed code paths of `x`
//! (`--interactive` and `-i --parallel`) by spawning the binary inside a
//! PTY we open ourselves. Works under `cargo test` without a real TTY.
//!
//! Unix-gated: PTY work is unix-specific in practice and these tests have
//! no Windows analogue.

#![cfg(unix)]

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

fn x_bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_x")
}

/// Open a 24x80 PTY, spawn `x` with `args` on the slave, drop the slave,
/// and start a reader thread that pushes every chunk read from the master
/// onto a channel. Returns (child, master, rx).
///
/// The master is returned so the caller can `take_writer()` if it needs
/// to send keystrokes (e.g. `q` to a TUI that didn't auto-exit). For
/// these smoke tests every command finishes on its own so writing is not
/// needed.
type PtySpawn = (
    Box<dyn Child + Send + Sync>,
    Box<dyn MasterPty + Send>,
    Receiver<Vec<u8>>,
);

#[allow(clippy::expect_used, reason = "test helper — panic on PTY setup failure is intentional")]
fn spawn_in_pty(args: &[&str]) -> PtySpawn {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty failed");

    let mut cmd = CommandBuilder::new(x_bin_path());
    for a in args {
        cmd.arg(a);
    }
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }
    // Inherit a reasonable env (PATH etc.). Force a sane TERM so any
    // ratatui/crossterm probing succeeds.
    for (k, v) in std::env::vars_os() {
        cmd.env(k, v);
    }
    cmd.env("TERM", "xterm-256color");

    let child = pair.slave.spawn_command(cmd).expect("spawn failed");
    // Drop slave so the PTY closes when the child exits.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    (child, pair.master, rx)
}

/// Drain any remaining bytes for up to `linger` after the child exits, so
/// the final summary block has time to reach the channel.
fn drain_remaining(rx: &Receiver<Vec<u8>>, linger: Duration) -> Vec<u8> {
    let end = Instant::now() + linger;
    let mut out = Vec::new();
    loop {
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(chunk) => out.extend_from_slice(&chunk),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
    out
}

/// Poll `child.try_wait()` until it returns Some(status) or `deadline`
/// elapses. Returns `Some(exit_code)` on clean exit, None on timeout.
fn wait_for_exit(child: &mut (dyn Child + Send + Sync), deadline: Duration) -> Option<i32> {
    let end = Instant::now() + deadline;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(i32::try_from(status.exit_code()).unwrap_or(-1)),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= end {
            return None;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// ---------- single-command --interactive tests ----------

#[test]
fn interactive_single_command_exits_zero_for_true() {
    let (mut child, _master, _rx) = spawn_in_pty(&["--interactive", "true"]);
    let code =
        wait_for_exit(&mut *child, Duration::from_secs(10)).expect("child did not exit within 10s");
    assert_eq!(code, 0);
}

#[test]
fn interactive_single_command_exits_one_for_false() {
    let (mut child, _master, _rx) = spawn_in_pty(&["--interactive", "false"]);
    let code =
        wait_for_exit(&mut *child, Duration::from_secs(10)).expect("child did not exit within 10s");
    assert_eq!(code, 1);
}

#[test]
fn interactive_propagates_exit_code_42() {
    let (mut child, _master, _rx) = spawn_in_pty(&["-i", "exit 42"]);
    let code =
        wait_for_exit(&mut *child, Duration::from_secs(10)).expect("child did not exit within 10s");
    assert_eq!(code, 42);
}

#[test]
fn interactive_enters_and_exits_alt_screen() {
    let (mut child, _master, rx) = spawn_in_pty(&["-i", "true"]);
    let code =
        wait_for_exit(&mut *child, Duration::from_secs(10)).expect("child did not exit within 10s");
    assert_eq!(code, 0);
    let bytes = drain_remaining(&rx, Duration::from_millis(500));
    assert!(
        contains(&bytes, b"\x1b[?1049h"),
        "expected alt-screen ENTER sequence in PTY stream"
    );
    assert!(
        contains(&bytes, b"\x1b[?1049l"),
        "expected alt-screen LEAVE sequence in PTY stream"
    );
}

#[test]
fn interactive_prints_success_line_after_alt_screen() {
    let (mut child, _master, rx) = spawn_in_pty(&["-i", "true", "--msg", "AltDone"]);
    let code =
        wait_for_exit(&mut *child, Duration::from_secs(10)).expect("child did not exit within 10s");
    assert_eq!(code, 0);
    let bytes = drain_remaining(&rx, Duration::from_millis(750));
    let leave = find(&bytes, b"\x1b[?1049l").expect("alt-screen leave not found");
    let msg = find(&bytes, b"AltDone").expect("display message not found");
    assert!(
        leave < msg,
        "success line should be printed AFTER leaving the alt screen \
         (leave={leave} msg={msg})"
    );
    assert!(
        contains(&bytes, "✓".as_bytes()),
        "expected ✓ glyph in success output"
    );
}

#[test]
fn interactive_failure_prints_cross_after_alt_screen() {
    let (mut child, _master, rx) = spawn_in_pty(&["-i", "false", "--msg", "Boom"]);
    let code =
        wait_for_exit(&mut *child, Duration::from_secs(10)).expect("child did not exit within 10s");
    assert_eq!(code, 1);
    let bytes = drain_remaining(&rx, Duration::from_millis(750));
    let leave = find(&bytes, b"\x1b[?1049l").expect("alt-screen leave not found");
    let msg = find(&bytes, b"Boom").expect("display message not found");
    assert!(
        leave < msg,
        "failure line should be printed AFTER leaving the alt screen \
         (leave={leave} msg={msg})"
    );
    assert!(
        contains(&bytes, "✘".as_bytes()),
        "expected ✘ glyph in failure output"
    );
}

// ---------- -i --parallel TUI tests ----------

#[test]
fn tui_parallel_two_true_exits_zero() {
    let (mut child, _master, _rx) = spawn_in_pty(&["-i", "--parallel", "true", "true"]);
    let code = wait_for_exit(&mut *child, Duration::from_secs(15))
        .expect("TUI did not auto-exit within 15s");
    assert_eq!(code, 0);
}

#[test]
fn tui_parallel_with_failure_exits_one() {
    let (mut child, _master, _rx) = spawn_in_pty(&["-i", "--parallel", "true", "false"]);
    let code = wait_for_exit(&mut *child, Duration::from_secs(15))
        .expect("TUI did not auto-exit within 15s");
    assert_eq!(code, 1);
}

#[test]
fn tui_parallel_prints_summary_block_on_main_screen() {
    let (mut child, _master, rx) =
        spawn_in_pty(&["-i", "--parallel", "echo seen_in_summary", "true"]);
    let code = wait_for_exit(&mut *child, Duration::from_secs(15))
        .expect("TUI did not auto-exit within 15s");
    assert_eq!(code, 0);
    let bytes = drain_remaining(&rx, Duration::from_secs(1));
    let leave = find(&bytes, b"\x1b[?1049l").expect("expected alt-screen leave from TUI exit");
    // After leaving alt screen, the standard parallel summary block lists
    // each child label (the raw command string by default).
    let after = &bytes[leave..];
    assert!(
        contains(after, b"echo seen_in_summary"),
        "expected child command label in post-TUI summary block; \
         got {} bytes after leave",
        after.len()
    );
}
