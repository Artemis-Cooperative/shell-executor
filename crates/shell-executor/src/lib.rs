use std::io::{Read, Write, stdout};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// The output captured from a completed shell command.
///
/// Contains the standard output, standard error, and exit code of the process.
/// An exit code of `124` indicates the command was killed due to a timeout.
pub struct CommandOutput {
    /// The captured standard output of the command.
    pub stdout: String,
    /// The captured standard error of the command.
    pub stderr: String,
    /// The exit code of the process. `124` means the command timed out.
    pub exit_code: i32,
}

/// A builder for configuring and running a shell command with a spinner display.
///
/// Created via [`execute()`]. Chain optional configuration methods before
/// calling [`run()`](ShellCommand::run) to execute.
///
/// # Example
///
/// ```
/// use std::time::Duration;
/// use shell_executor::execute;
///
/// let ok = execute("echo 'built with the builder'")
///     .message("Builder demo")
///     .timeout(Duration::from_secs(5))
///     .success(|out| out.stdout.contains("built"))
///     .run();
/// assert!(ok);
/// ```
pub struct ShellCommand {
    command: String,
    message: Option<String>,
    timeout: Option<Duration>,
    success: Option<Box<dyn Fn(&CommandOutput) -> bool>>,
    quiet: bool,
    max_output: usize,
    log: Option<PathBuf>,
}

const DEFAULT_MAX_OUTPUT: usize = 10 * 1024 * 1024; // 10 MB

/// The terminal outcome of a [`ShellCommand::run_status`] invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// The command completed and was judged successful.
    Success,
    /// The command completed but was judged a failure.
    Failure,
    /// The command was killed because it exceeded its timeout.
    Timeout,
    /// The command was terminated by a signal (Unix only).
    Interrupted,
}

impl RunStatus {
    /// Returns `true` if the status is [`RunStatus::Success`].
    pub fn is_success(self) -> bool {
        matches!(self, RunStatus::Success)
    }
}

/// Create a new [`ShellCommand`] for the given shell expression.
///
/// This is the primary entry point for the crate. The command string is
/// passed to `sh -c`, so pipes, redirects, and other shell features work.
///
/// # Example
///
/// ```
/// use shell_executor::execute;
///
/// let ok = execute("echo hello").run();
/// assert!(ok);
/// ```
pub fn execute(cmd: &str) -> ShellCommand {
    ShellCommand {
        command: cmd.to_string(),
        message: None,
        timeout: None,
        success: None,
        quiet: false,
        max_output: DEFAULT_MAX_OUTPUT,
        log: None,
    }
}

impl ShellCommand {
    /// Set a custom spinner message displayed while the command runs.
    ///
    /// Without this, the command string itself is shown (truncated to 30 chars).
    ///
    /// # Example
    ///
    /// ```
    /// use shell_executor::execute;
    ///
    /// let ok = execute("echo hi").message("Greeting").run();
    /// assert!(ok);
    /// ```
    pub fn message(mut self, msg: &str) -> Self {
        self.message = Some(msg.to_string());
        self
    }

    /// Set a maximum duration for the command. If the command exceeds this
    /// duration it is killed and the resulting [`CommandOutput`] will have
    /// an exit code of `124`.
    ///
    /// # Example
    ///
    /// ```
    /// use std::time::Duration;
    /// use shell_executor::execute;
    ///
    /// let ok = execute("sleep 10")
    ///     .timeout(Duration::from_millis(200))
    ///     .run();
    /// assert!(!ok); // timed out
    /// ```
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    /// Provide a custom closure to determine success. The closure receives
    /// the [`CommandOutput`] and returns `true` for success, `false` for failure.
    ///
    /// Without this, success is determined by a zero exit code.
    ///
    /// # Example
    ///
    /// ```
    /// use shell_executor::execute;
    ///
    /// let ok = execute("echo hello world")
    ///     .success(|output| output.stdout.contains("world"))
    ///     .run();
    /// assert!(ok);
    /// ```
    pub fn success(mut self, closure: impl Fn(&CommandOutput) -> bool + 'static) -> Self {
        self.success = Some(Box::new(closure));
        self
    }

    /// Suppress command output on success.
    ///
    /// By default, output (stdout and stderr) is printed for both successful
    /// and failed commands. When quiet mode is enabled, output is only printed
    /// on failure.
    ///
    /// # Example
    ///
    /// ```
    /// use shell_executor::execute;
    ///
    /// let ok = execute("echo hello").quiet().run();
    /// assert!(ok);
    /// ```
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// Set the maximum number of bytes to capture from stdout and stderr
    /// (each). If a stream exceeds this limit, the output is truncated and
    /// a note is appended. Defaults to 10 MB.
    pub fn max_output(mut self, bytes: usize) -> Self {
        self.max_output = bytes;
        self
    }

    /// Set a log file path. After each command execution, a timestamped entry
    /// is appended to this file with the result status, elapsed time, message,
    /// and command output.
    pub fn log(mut self, path: impl Into<PathBuf>) -> Self {
        self.log = Some(path.into());
        self
    }

    /// Execute the command silently and return the captured output.
    ///
    /// Unlike [`run()`](ShellCommand::run), this method does not display a
    /// spinner, does not print output, and does not evaluate the success
    /// closure. It simply spawns the command, waits for completion (respecting
    /// any configured [`timeout`](ShellCommand::timeout)), and returns the
    /// raw [`CommandOutput`].
    ///
    /// # Example
    ///
    /// ```
    /// use shell_executor::execute;
    ///
    /// let output = execute("echo hello").run_capture();
    /// assert_eq!(output.stdout.trim(), "hello");
    /// assert_eq!(output.exit_code, 0);
    /// ```
    pub fn run_capture(self) -> CommandOutput {
        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                return CommandOutput {
                    stdout: String::new(),
                    stderr: format!("Failed to spawn command: {e}"),
                    exit_code: 1,
                };
            }
        };

        if let Some(timeout) = self.timeout {
            let start = Instant::now();
            loop {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return CommandOutput {
                        stdout: String::new(),
                        stderr: String::new(),
                        exit_code: 124,
                    };
                }
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => break,
                }
            }
        }

        let stdout_str = child.stdout.take()
            .map(|out| read_bounded(out, self.max_output))
            .unwrap_or_default();
        let stderr_str = child.stderr.take()
            .map(|err| read_bounded(err, self.max_output))
            .unwrap_or_default();

        let exit_code = child
            .wait()
            .map(|s| s.code().unwrap_or(1))
            .unwrap_or(1);

        CommandOutput {
            stdout: stdout_str,
            stderr: stderr_str,
            exit_code,
        }
    }

    /// Execute the command and return whether it succeeded.
    ///
    /// A spinner is displayed on stdout while the command runs. On completion
    /// a check mark (success) or cross (failure) is printed with the elapsed time.
    ///
    /// Returns `true` if the command succeeded according to the success criteria
    /// (zero exit code by default, or the custom [`success`](ShellCommand::success) closure).
    pub fn run(self) -> bool {
        self.run_status().is_success()
    }

    /// Execute the command and return its detailed [`RunStatus`].
    ///
    /// Behaves identically to [`run`](ShellCommand::run) in terms of spinner,
    /// printing, and logging, but distinguishes between failure modes.
    pub fn run_status(self) -> RunStatus {
        let display_message = match &self.message {
            Some(msg) => msg.clone(),
            None => {
                if self.command.len() > 30 {
                    format!("{}...", &self.command[..30])
                } else {
                    self.command.clone()
                }
            }
        };

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                eprintln!("Failed to spawn command: {e}");
                return RunStatus::Failure;
            }
        };

        let spinner_chars = ['-', '\\', '|', '/'];
        let mut spinner_idx = 0;
        let start = Instant::now();
        let mut timed_out = false;

        loop {
            let elapsed = start.elapsed();

            if let Some(timeout) = self.timeout {
                if elapsed >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
            }

            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(_) => break,
            }

            let secs = elapsed.as_secs();
            let h = secs / 3600;
            let m = (secs % 3600) / 60;
            let s = secs % 60;

            print!(
                "\r[ {} {:02}:{:02}:{:02} ] {}",
                spinner_chars[spinner_idx], h, m, s, display_message
            );
            let _ = stdout().flush();

            spinner_idx = (spinner_idx + 1) % spinner_chars.len();
            std::thread::sleep(Duration::from_millis(100));
        }

        let elapsed = start.elapsed();
        let secs = elapsed.as_secs();
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        let final_time = format!("{h:02}:{m:02}:{s:02}");

        let (output, interrupted) = if timed_out {
            (CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 124,
            }, false)
        } else {
            let stdout_str = child.stdout.take()
                .map(|out| read_bounded(out, self.max_output))
                .unwrap_or_default();
            let stderr_str = child.stderr.take()
                .map(|err| read_bounded(err, self.max_output))
                .unwrap_or_default();

            let status = child.wait().ok();
            let was_signaled = {
                #[cfg(unix)]
                { status.as_ref().is_some_and(|s| s.signal().is_some()) }
                #[cfg(not(unix))]
                { false }
            };
            let exit_code = status
                .and_then(|s| s.code())
                .unwrap_or(1);

            (CommandOutput {
                stdout: stdout_str,
                stderr: stderr_str,
                exit_code,
            }, was_signaled)
        };

        let success = if interrupted || timed_out {
            false
        } else {
            match &self.success {
                Some(closure) => closure(&output),
                None => output.exit_code == 0,
            }
        };

        if interrupted {
            println!(
                "\r[ \x1b[33mINTERRUPTED\x1b[0m {final_time} ] {display_message}",
            );
        } else if success {
            println!(
                "\r[ \x1b[32m✓\x1b[0m {final_time} ] {display_message}",
            );
        } else {
            println!(
                "\r[ \x1b[31m✘\x1b[0m {final_time} ] {display_message}",
            );
        }

        if !success || !self.quiet {
            let combined = format!("{}{}", output.stdout, output.stderr);
            let trimmed = combined.trim();
            if !trimmed.is_empty() {
                for line in trimmed.lines() {
                    println!("    {line}");
                }
            }
        }

        if let Some(ref log_path) = self.log {
            let now = chrono::Local::now();
            let timestamp = now.format("%Y-%m-%d %H:%M:%S");
            let icon = if interrupted { "INTERRUPTED" } else if success { "✓" } else { "✘" };

            let mut entry = format!("[{timestamp}] [ {icon} {final_time} ] {display_message}\n");

            let should_include_output = !success || !self.quiet;
            if should_include_output {
                let combined = format!("{}{}", output.stdout, output.stderr);
                let trimmed = combined.trim();
                if !trimmed.is_empty() {
                    for line in trimmed.lines() {
                        entry.push('\t');
                        entry.push_str(line);
                        entry.push('\n');
                    }
                }
            }

            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
            {
                let _ = file.write_all(entry.as_bytes());
            }
        }

        if timed_out {
            RunStatus::Timeout
        } else if interrupted {
            RunStatus::Interrupted
        } else if success {
            RunStatus::Success
        } else {
            RunStatus::Failure
        }
    }
}

fn read_bounded(reader: impl Read, limit: usize) -> String {
    let mut buf = Vec::new();
    let mut limited = reader.take(limit as u64 + 1);
    let _ = limited.read_to_end(&mut buf);
    let truncated = buf.len() > limit;
    if truncated {
        buf.truncate(limit);
    }
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        s.push_str("\n... [output truncated at ");
        s.push_str(&limit.to_string());
        s.push_str(" bytes]");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_status_success_for_zero_exit() {
        assert_eq!(execute("true").run_status(), RunStatus::Success);
    }

    #[test]
    fn run_status_failure_for_nonzero_exit() {
        assert_eq!(execute("false").run_status(), RunStatus::Failure);
    }

    #[test]
    fn run_status_timeout_when_exceeded() {
        let status = execute("sleep 10")
            .timeout(Duration::from_millis(200))
            .run_status();
        assert_eq!(status, RunStatus::Timeout);
    }

    #[cfg(unix)]
    #[test]
    fn run_status_interrupted_on_signal() {
        assert_eq!(execute("kill -INT $$").run_status(), RunStatus::Interrupted);
    }

    #[test]
    fn long_command_truncation() {
        // Commands longer than 30 chars should still run without a custom message
        let result = execute("echo this is a very long command that exceeds thirty characters").run();
        assert!(result);
    }

    #[test]
    fn timeout_overrides_custom_success_closure() {
        let result = execute("sleep 10")
            .timeout(Duration::from_millis(200))
            .success(|output| output.exit_code == 124)
            .run();
        assert!(!result);
    }

    #[test]
    fn custom_closure_rejects_successful_command() {
        // Command exits 0 but the closure says no
        let result = execute("echo fine")
            .success(|_| false)
            .run();
        assert!(!result);
    }

    #[test]
    fn stderr_output_captured() {
        let result = execute("echo err >&2")
            .success(|output| output.stderr.contains("err"))
            .run();
        assert!(result);
    }

    #[test]
    fn empty_command_does_not_panic() {
        // An empty string passed to sh -c should not panic
        let _ = execute("").run();
    }

    #[test]
    fn run_capture_returns_stdout() {
        let output = execute("echo hello").run_capture();
        assert_eq!(output.stdout.trim(), "hello");
        assert_eq!(output.exit_code, 0);
    }

    #[test]
    fn run_capture_returns_stderr() {
        let output = execute("echo err >&2").run_capture();
        assert!(output.stderr.contains("err"));
    }

    #[test]
    fn run_capture_timeout() {
        let output = execute("sleep 10")
            .timeout(Duration::from_millis(200))
            .run_capture();
        assert_eq!(output.exit_code, 124);
    }

    #[test]
    fn max_output_truncates_large_output() {
        // Generate 100 bytes of output but limit to 20
        let output = execute("printf '%0.s.' $(seq 1 100)")
            .max_output(20)
            .run_capture();
        assert!(output.stdout.contains("[output truncated at 20 bytes]"));
        // The actual content before the note should be 20 bytes
        assert!(output.stdout.starts_with("...................."));
    }

    #[test]
    fn chained_builder_all_options() {
        let result = execute("echo chain")
            .message("Chained")
            .timeout(Duration::from_secs(5))
            .success(|output| output.stdout.contains("chain"))
            .run();
        assert!(result);
    }

    fn temp_log_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("shell_executor_test_{name}_{}.log", std::process::id()))
    }

    #[test]
    fn log_creates_file_if_missing() {
        let path = temp_log_path("creates_file");
        let _ = std::fs::remove_file(&path);
        assert!(!path.exists());

        execute("echo hello").log(&path).run();

        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("hello"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_appends_across_runs() {
        let path = temp_log_path("appends");
        let _ = std::fs::remove_file(&path);

        execute("echo first").message("First").log(&path).run();
        execute("echo second").message("Second").log(&path).run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("First"));
        assert!(contents.contains("Second"));
        // Two separate log entries
        let entry_count = contents.matches("] [").count();
        assert!(entry_count >= 2, "expected at least 2 entries, got {entry_count}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_contains_timestamp_format() {
        let path = temp_log_path("timestamp");
        let _ = std::fs::remove_file(&path);

        execute("echo ts").log(&path).run();

        let contents = std::fs::read_to_string(&path).unwrap();
        // Match [YYYY-MM-DD HH:MM:SS] pattern
        let re_like = contents.starts_with('[')
            && contents.contains('-')
            && contents.contains(':');
        assert!(re_like, "log should start with a timestamp, got: {contents}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_success_shows_checkmark() {
        let path = temp_log_path("success_icon");
        let _ = std::fs::remove_file(&path);

        execute("echo ok").message("Success cmd").log(&path).run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("✓"), "success log should contain ✓");
        assert!(contents.contains("Success cmd"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_failure_shows_cross() {
        let path = temp_log_path("failure_icon");
        let _ = std::fs::remove_file(&path);

        execute("false").message("Fail cmd").log(&path).run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("✘"), "failure log should contain ✘");
        assert!(contents.contains("Fail cmd"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_no_ansi_color_codes() {
        let path = temp_log_path("no_ansi");
        let _ = std::fs::remove_file(&path);

        execute("echo colored").log(&path).run();
        execute("false").log(&path).run();

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.contains(&0x1b),
            "log file should not contain ESC (0x1b) ANSI codes"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_success_output_tabulated() {
        let path = temp_log_path("tabulated");
        let _ = std::fs::remove_file(&path);

        execute("printf 'line1\\nline2\\nline3'")
            .message("Multi-line")
            .log(&path)
            .run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\tline1\n"));
        assert!(contents.contains("\tline2\n"));
        assert!(contents.contains("\tline3\n"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_quiet_success_omits_output() {
        let path = temp_log_path("quiet_success");
        let _ = std::fs::remove_file(&path);

        execute("echo suppressed")
            .message("Quiet ok")
            .quiet()
            .log(&path)
            .run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("Quiet ok"));
        assert!(!contents.contains("suppressed"), "quiet success should not log output");
        assert!(!contents.contains('\t'), "quiet success should have no tabulated lines");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_quiet_failure_includes_output() {
        let path = temp_log_path("quiet_failure");
        let _ = std::fs::remove_file(&path);

        execute("echo visible >&2; false")
            .message("Quiet fail")
            .quiet()
            .log(&path)
            .run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("✘"));
        assert!(contents.contains("\tvisible"), "quiet failure should still log output");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_contains_elapsed_time() {
        let path = temp_log_path("elapsed");
        let _ = std::fs::remove_file(&path);

        execute("echo fast").log(&path).run();

        let contents = std::fs::read_to_string(&path).unwrap();
        // Should contain HH:MM:SS format
        assert!(contents.contains("00:00:0"), "log should contain elapsed time");
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn log_interrupted_shows_interrupted() {
        let path = temp_log_path("interrupted");
        let _ = std::fs::remove_file(&path);

        execute("kill -INT $$")
            .message("Signal test")
            .log(&path)
            .run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("INTERRUPTED"), "signal-killed command should log INTERRUPTED");
        assert!(!contents.contains("✓"), "interrupted should not show checkmark");
        assert!(!contents.contains("✘"), "interrupted should not show cross");
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_command_returns_false() {
        let result = execute("kill -INT $$").run();
        assert!(!result);
    }

    #[cfg(unix)]
    #[test]
    fn log_interrupted_no_ansi() {
        let path = temp_log_path("interrupted_no_ansi");
        let _ = std::fs::remove_file(&path);

        execute("kill -INT $$")
            .message("No color")
            .log(&path)
            .run();

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.contains(&0x1b),
            "interrupted log should not contain ANSI codes"
        );
        let _ = std::fs::remove_file(&path);
    }
}
