use std::io::{Read, Write, stdout};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

mod interactive;

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
    show_time: bool,
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

/// Combined outcome and exit code from a [`ShellCommand::run_report`] invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunReport {
    pub status: RunStatus,
    /// The propagated exit code:
    /// - command's real exit code on Success/Failure
    /// - `124` on Timeout
    /// - `128 + signal_number` on Interrupted (Unix); `1` if the signal cannot be determined or on non-Unix
    pub exit_code: i32,
}

/// Print a bare success label `[ ✓ ] msg` to stdout.
///
/// Useful for reporting the outcome of in-process work that did not run a
/// shell command (so [`execute`] is not applicable). The label uses the same
/// visual style as the wrapper printed by [`ShellCommand::run`], minus the
/// elapsed-time slot.
pub fn pass(msg: &str) {
    println!("[ \x1b[32m✓\x1b[0m ] {msg}");
}

/// Print a bare failure label `[ ✘ ] msg` to stdout.
///
/// Counterpart to [`pass`]. Useful for surfacing in-process errors with the
/// same visual style as a failed [`ShellCommand::run`].
pub fn fail(msg: &str) {
    println!("[ \x1b[31m✘\x1b[0m ] {msg}");
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
        show_time: false,
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

    /// Show the elapsed `HH:MM:SS` duration in the spinner and final status
    /// line. Off by default — the wrapper renders as `[ ✓ ] message` rather
    /// than `[ ✓ HH:MM:SS ] message`. Log entries always include the duration
    /// regardless of this setting.
    pub fn show_time(mut self) -> Self {
        self.show_time = true;
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
        self.run_report().status
    }

    /// Execute the command and return both its [`RunStatus`] and propagated exit code.
    ///
    /// Behaves identically to [`run_status`](ShellCommand::run_status) in terms
    /// of spinner, printing, and logging, but additionally surfaces the exit
    /// code so callers can propagate it (for example, to the OS).
    pub fn run_report(self) -> RunReport {
        let display_message = derive_display_message(&self.message, &self.command);

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
                return RunReport {
                    status: RunStatus::Failure,
                    exit_code: 1,
                };
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

            if self.show_time {
                let secs = elapsed.as_secs();
                let h = secs / 3600;
                let m = (secs % 3600) / 60;
                let s = secs % 60;
                print!(
                    "\r[ {} {:02}:{:02}:{:02} ] {}",
                    spinner_chars[spinner_idx], h, m, s, display_message
                );
            } else {
                print!("\r[ {} ] {}", spinner_chars[spinner_idx], display_message);
            }
            let _ = stdout().flush();

            spinner_idx = (spinner_idx + 1) % spinner_chars.len();
            std::thread::sleep(Duration::from_millis(100));
        }

        let final_time = format_elapsed(start.elapsed());

        let (output, signal_num) = if timed_out {
            (CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 124,
            }, None)
        } else {
            let stdout_str = child.stdout.take()
                .map(|out| read_bounded(out, self.max_output))
                .unwrap_or_default();
            let stderr_str = child.stderr.take()
                .map(|err| read_bounded(err, self.max_output))
                .unwrap_or_default();

            let status = child.wait().ok();
            let signal_num: Option<i32> = {
                #[cfg(unix)]
                { status.as_ref().and_then(|s| s.signal()) }
                #[cfg(not(unix))]
                { None }
            };
            let exit_code = status
                .and_then(|s| s.code())
                .unwrap_or(1);

            (CommandOutput {
                stdout: stdout_str,
                stderr: stderr_str,
                exit_code,
            }, signal_num)
        };

        let interrupted = signal_num.is_some();

        let success = if interrupted || timed_out {
            false
        } else {
            match &self.success {
                Some(closure) => closure(&output),
                None => output.exit_code == 0,
            }
        };

        let time_slot = if self.show_time {
            format!(" {final_time}")
        } else {
            String::new()
        };
        if interrupted {
            println!("\r[ \x1b[33mINTERRUPTED\x1b[0m{time_slot} ] {display_message}");
        } else if success {
            println!("\r[ \x1b[32m✓\x1b[0m{time_slot} ] {display_message}");
        } else {
            println!("\r[ \x1b[31m✘\x1b[0m{time_slot} ] {display_message}");
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

        let status = if timed_out {
            RunStatus::Timeout
        } else if interrupted {
            RunStatus::Interrupted
        } else if success {
            RunStatus::Success
        } else {
            RunStatus::Failure
        };

        let exit_code = match status {
            RunStatus::Timeout => 124,
            RunStatus::Interrupted => signal_num.map(|n| 128 + n).unwrap_or(1),
            RunStatus::Success | RunStatus::Failure => output.exit_code,
        };

        RunReport { status, exit_code }
    }

    /// Execute the command with inherited stdio (no spinner, no wrapper).
    ///
    /// The child's stdout and stderr are connected directly to the parent's
    /// — output streams live and is not captured. The bracketed status line
    /// is suppressed. Exit code propagation behaves identically to
    /// [`run_report`](ShellCommand::run_report).
    ///
    /// If `.log()` is configured, an entry with timestamp, status icon,
    /// elapsed time, and message is appended, but no output body is included.
    ///
    /// The `success` closure is ignored — it requires captured output that
    /// succinct mode does not produce. Success is determined by exit code only.
    pub fn run_succinct_report(self) -> RunReport {
        let display_message = derive_display_message(&self.message, &self.command);

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                eprintln!("Failed to spawn command: {e}");
                return RunReport {
                    status: RunStatus::Failure,
                    exit_code: 1,
                };
            }
        };

        let start = Instant::now();
        let mut timed_out = false;

        loop {
            if let Some(timeout) = self.timeout {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
            }

            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }

        let final_time = format_elapsed(start.elapsed());

        let (exit_code, signal_num) = if timed_out {
            (124, None)
        } else {
            let status = child.wait().ok();
            let signal_num: Option<i32> = {
                #[cfg(unix)]
                { status.as_ref().and_then(|s| s.signal()) }
                #[cfg(not(unix))]
                { None }
            };
            let code = status.and_then(|s| s.code()).unwrap_or(1);
            (code, signal_num)
        };

        let interrupted = signal_num.is_some();
        let success = !interrupted && !timed_out && exit_code == 0;

        let status = if timed_out {
            RunStatus::Timeout
        } else if interrupted {
            RunStatus::Interrupted
        } else if success {
            RunStatus::Success
        } else {
            RunStatus::Failure
        };

        let final_exit_code = match status {
            RunStatus::Timeout => 124,
            RunStatus::Interrupted => signal_num.map(|n| 128 + n).unwrap_or(1),
            RunStatus::Success | RunStatus::Failure => exit_code,
        };

        if let Some(ref log_path) = self.log {
            let now = chrono::Local::now();
            let timestamp = now.format("%Y-%m-%d %H:%M:%S");
            let icon = if interrupted { "INTERRUPTED" } else if success { "✓" } else { "✘" };

            let entry = format!("[{timestamp}] [ {icon} {final_time} ] {display_message}\n");

            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
            {
                let _ = file.write_all(entry.as_bytes());
            }
        }

        RunReport { status, exit_code: final_exit_code }
    }

    /// Execute the command interactively on a PTY.
    ///
    /// The command runs attached to a pseudo-terminal so TUI programs work
    /// normally. The session uses the terminal's alternate screen for the
    /// duration of the command; on exit, the alt screen is left and the
    /// pass/fail status line is printed on the main screen.
    ///
    /// Interactive mode ignores [`timeout`](ShellCommand::timeout) and the
    /// [`success`](ShellCommand::success) closure (the byte stream is full
    /// of TUI escape codes and isn't usefully inspectable). Success is
    /// determined by exit code only. No spinner is shown.
    pub fn run_interactive_report(self) -> RunReport {
        let display_message = derive_display_message(&self.message, &self.command);
        interactive::run_interactive_report(
            &self.command,
            &display_message,
            self.log.as_ref(),
            self.show_time,
        )
    }
}

fn derive_display_message(message: &Option<String>, command: &str) -> String {
    match message {
        Some(msg) => msg.clone(),
        None => {
            if command.len() > 30 {
                format!("{}...", &command[..30])
            } else {
                command.to_string()
            }
        }
    }
}

pub(crate) fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
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

/// CLI entry point for the `x` binary.
///
/// Exposed so other crates can re-publish `x` as their own `[[bin]]` target
/// with a thin wrapper:
///
/// ```ignore
/// fn main() {
///     std::process::exit(shell_executor::x_main());
/// }
/// ```
///
/// Returns the process exit code rather than calling [`std::process::exit`]
/// itself, so callers stay in control of termination.
pub fn x_main() -> i32 {
    use clap::Parser;

    #[derive(Parser)]
    #[command(name = "x", about = "Execute a shell command with a spinner")]
    struct Cli {
        /// The shell command to execute
        command: String,

        /// Spinner message to display
        #[arg(long, alias = "message")]
        msg: Option<String>,

        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,

        /// Suppress output on success (only shown on failure).
        #[arg(long, short)]
        quiet: bool,

        /// Show output on success (default behavior; inverse of --quiet).
        #[arg(long, conflicts_with = "quiet")]
        verbose: bool,

        /// Drop the [ ✓ … ] wrapper and stream output directly. Overrides --quiet/--verbose.
        #[arg(long)]
        succinct: bool,

        /// Append execution results to a log file
        #[arg(long)]
        log: Option<String>,

        /// Validator shell command run after the main command. Its exit code
        /// determines overall pass/fail (overrides the main command's status).
        /// Not run if the main command timed out or was interrupted by a signal.
        #[arg(long, short = 'v')]
        validator: Option<String>,

        /// Include the elapsed `HH:MM:SS` duration in the status wrapper.
        /// Off by default — the wrapper renders as `[ ✓ ] message`.
        #[arg(long)]
        time: bool,

        /// Run the command interactively on a PTY (alt screen, no spinner).
        /// On exit, the pass/fail status line is printed on the main screen.
        #[arg(long, short = 'i')]
        interactive: bool,
    }

    let cli = Cli::parse();
    let _ = cli.verbose;

    let mut cmd = execute(&cli.command);

    if let Some(msg) = &cli.msg {
        cmd = cmd.message(msg);
    }

    if let Some(secs) = cli.timeout {
        cmd = cmd.timeout(Duration::from_secs(secs));
    }

    if cli.quiet && !cli.succinct {
        cmd = cmd.quiet();
    }

    if let Some(log_path) = &cli.log {
        cmd = cmd.log(log_path);
    }

    if cli.time {
        cmd = cmd.show_time();
    }

    let report = if cli.interactive {
        cmd.run_interactive_report()
    } else if cli.succinct {
        cmd.run_succinct_report()
    } else {
        cmd.run_report()
    };

    match (report.status, &cli.validator) {
        (RunStatus::Timeout, _) | (RunStatus::Interrupted, _) => report.exit_code,
        (RunStatus::Success, None) | (RunStatus::Failure, None) => report.exit_code,
        (RunStatus::Success | RunStatus::Failure, Some(v_cmd)) => {
            let mut v = execute(v_cmd).message("Validator");
            if cli.quiet && !cli.succinct {
                v = v.quiet();
            }
            if let Some(log_path) = &cli.log {
                v = v.log(log_path);
            }
            if cli.time {
                v = v.show_time();
            }
            if cli.succinct {
                v.run_succinct_report().exit_code
            } else {
                v.run_report().exit_code
            }
        }
    }
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
    fn run_report_propagates_exit_42() {
        assert_eq!(execute("exit 42").run_report().exit_code, 42);
    }

    #[test]
    fn run_report_timeout_is_124() {
        let report = execute("sleep 10")
            .timeout(Duration::from_millis(200))
            .run_report();
        assert_eq!(report.exit_code, 124);
        assert_eq!(report.status, RunStatus::Timeout);
    }

    #[cfg(unix)]
    #[test]
    fn run_report_interrupted_is_128_plus_signal() {
        let report = execute("kill -INT $$").run_report();
        assert_eq!(report.exit_code, 130);
        assert_eq!(report.status, RunStatus::Interrupted);
    }

    #[test]
    fn run_report_true_is_zero_false_is_one() {
        assert_eq!(execute("true").run_report().exit_code, 0);
        assert_eq!(execute("false").run_report().exit_code, 1);
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

    #[test]
    fn run_succinct_report_propagates_exit_code() {
        assert_eq!(execute("exit 7").run_succinct_report().exit_code, 7);
    }

    #[test]
    fn run_succinct_report_timeout() {
        let report = execute("sleep 10")
            .timeout(Duration::from_millis(200))
            .run_succinct_report();
        assert_eq!(report.status, RunStatus::Timeout);
        assert_eq!(report.exit_code, 124);
    }

    #[test]
    fn run_succinct_report_log_writes_entry_without_body() {
        let path = temp_log_path("succinct_log_no_body");
        let _ = std::fs::remove_file(&path);

        execute("echo body-line")
            .message("Succinct entry")
            .log(&path)
            .run_succinct_report();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("Succinct entry"));
        assert!(contents.contains("✓"));
        assert!(!contents.contains("body-line"), "succinct log should not include captured output body");
        assert!(!contents.contains('\t'), "succinct log should have no tabulated lines");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_succinct_report_log_no_ansi() {
        let path = temp_log_path("succinct_log_no_ansi");
        let _ = std::fs::remove_file(&path);

        execute("echo hello").log(&path).run_succinct_report();
        execute("false").log(&path).run_succinct_report();

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.contains(&0x1b),
            "succinct log file should not contain ESC (0x1b) ANSI codes"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pass_does_not_panic() {
        pass("test message");
    }

    #[test]
    fn fail_does_not_panic() {
        fail("test message");
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
