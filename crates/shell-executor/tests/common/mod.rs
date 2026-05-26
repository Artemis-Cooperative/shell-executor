//! Shared helpers for integration tests.
//!
//! Each `tests/*.rs` file is its own crate, so this module is `mod common;`
//! into each test file rather than imported.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Returns a `Command` invoking the workspace's compiled `x` binary.
pub fn x_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_x"))
}

static PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns a unique temp path under `std::env::temp_dir()` for a given test label.
/// Combines process id and an atomic counter so parallel tests do not collide.
/// The caller is responsible for deleting the path.
pub fn fresh_temp_path(label: &str, ext: &str) -> PathBuf {
    let n = PATH_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "shell_executor_test_{label}_{}_{n}.{ext}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

/// Strip ANSI escape sequences from a string. Handles CSI (`ESC [ ... letter`),
/// OSC (`ESC ] ... BEL`), and standalone single-char ESC sequences. Used by
/// assertions that compare text content without color/cursor noise.
pub fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // Skip ESC + one or two intro chars + parameters until a final byte.
            i += 1;
            if i < bytes.len() {
                let intro = bytes[i];
                i += 1;
                if intro == b'[' || intro == b']' {
                    while i < bytes.len() {
                        let c = bytes[i];
                        i += 1;
                        // CSI final: 0x40..=0x7e; OSC terminator: BEL (0x07) or ESC \
                        if (intro == b'[' && (0x40..=0x7e).contains(&c))
                            || (intro == b']' && c == 0x07)
                        {
                            break;
                        }
                    }
                }
            }
        } else {
            out.push(char::from(bytes[i]));
            i += 1;
        }
    }
    out
}

/// Capture stdout+stderr of a finished `Command`, return them as `String`s plus
/// the exit code. Convenience wrapper around `.output()` + `from_utf8_lossy`.
pub fn run_and_capture(mut cmd: Command) -> Result<CapturedOutput, std::io::Error> {
    let output = cmd.output()?;
    Ok(CapturedOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
    })
}

pub struct CapturedOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

impl CapturedOutput {
    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    pub fn combined_stripped(&self) -> String {
        strip_ansi(&self.combined())
    }
}
