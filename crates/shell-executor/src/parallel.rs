#![expect(
    clippy::print_stdout,
    reason = "CLI tool intentionally writes to stdout for user-facing output"
)]
#![expect(
    clippy::unwrap_used,
    reason = "Mutex poisoning is treated as a fatal logic error in this single-process tool"
)]

use std::fmt::Write as _;
use std::io::{stdout, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::{
    derive_display_message, format_elapsed, read_bounded, CommandOutput, RunReport, RunStatus,
};

const DEFAULT_MAX_OUTPUT: usize = 10 * 1024 * 1024;

/// A builder for running multiple shell commands in parallel.
///
/// Created via [`parallel()`]. The group renders a parent spinner line plus one
/// indented child line per command, refreshed in place. Children run
/// concurrently; if one fails, the others run to completion. Overall success
/// requires every child to succeed.
///
/// Per-child timeouts, messages, and validators are not supported — nest `x`
/// invocations inside the parallel command strings if you need them.
pub struct ParallelGroup {
    commands: Vec<String>,
    message: Option<String>,
    quiet: bool,
    max_output: usize,
    log: Option<PathBuf>,
    show_time: bool,
}

/// Create a [`ParallelGroup`] from an iterable of shell command strings.
///
/// # Example
///
/// ```
/// use shell_executor::parallel;
///
/// let ok = parallel(["true", "true"])
///     .message("two truths")
///     .run();
/// assert!(ok);
/// ```
pub fn parallel<I, S>(commands: I) -> ParallelGroup
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    ParallelGroup {
        commands: commands.into_iter().map(Into::into).collect(),
        message: None,
        quiet: false,
        max_output: DEFAULT_MAX_OUTPUT,
        log: None,
        show_time: false,
    }
}

impl ParallelGroup {
    /// Set the parent spinner message. Defaults to "N parallel commands".
    pub fn message(mut self, msg: &str) -> Self {
        self.message = Some(msg.to_string());
        self
    }

    /// Suppress per-child output on success. Failed children still print output.
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// Maximum bytes to capture per child stream (stdout and stderr each).
    pub fn max_output(mut self, bytes: usize) -> Self {
        self.max_output = bytes;
        self
    }

    /// Append a log entry for the group after completion.
    pub fn log(mut self, path: impl Into<PathBuf>) -> Self {
        self.log = Some(path.into());
        self
    }

    /// Include elapsed `HH:MM:SS` in the spinner / final status lines.
    pub fn show_time(mut self) -> Self {
        self.show_time = true;
        self
    }

    /// Run the group and return `true` if every child succeeded.
    pub fn run(self) -> bool {
        self.run_status().is_success()
    }

    /// Run the group and return its [`RunStatus`].
    pub fn run_status(self) -> RunStatus {
        self.run_report().status
    }

    /// Run the group with spinner + indented child lines, then print per-child
    /// captured output below the locked-in status block.
    ///
    /// The returned exit code is `0` if every child succeeded, otherwise `1`.
    pub fn run_report(self) -> RunReport {
        let n = self.commands.len();
        let parent_label = self.parent_label(n);

        if n == 0 {
            println!("[ \x1b[32m✓\x1b[0m ] {parent_label}");
            return RunReport {
                status: RunStatus::Success,
                exit_code: 0,
            };
        }

        let child_labels: Vec<String> = self
            .commands
            .iter()
            .map(|c| derive_display_message(None, c))
            .collect();

        let states: Vec<Arc<Mutex<ChildState>>> = (0..n)
            .map(|_| Arc::new(Mutex::new(ChildState::Running)))
            .collect();

        let start = Instant::now();
        let handles = self.spawn_workers(&states);

        // Reserve N+1 lines for in-place rendering, position cursor at parent line.
        for _ in 0..=n {
            println!();
        }
        print!("\x1b[{}F", n + 1);
        let _ = stdout().flush();

        let spinner_chars = ['-', '\\', '|', '/'];
        let mut spinner_idx = 0;

        loop {
            let all_done = states
                .iter()
                .all(|s| matches!(&*s.lock().unwrap(), ChildState::Done { .. }));
            if all_done {
                break;
            }

            render_in_place(
                &states,
                &parent_label,
                &child_labels,
                spinner_chars[spinner_idx],
                start.elapsed(),
                self.show_time,
            );

            // Move cursor back to parent line for the next tick.
            print!("\x1b[{n}F");
            let _ = stdout().flush();

            spinner_idx = (spinner_idx + 1) % spinner_chars.len();
            thread::sleep(Duration::from_millis(100));
        }

        for h in handles {
            let _ = h.join();
        }

        let group_elapsed = start.elapsed();
        let parent_status = compute_parent_status(&states);

        // Cursor is at start of parent line. Erase the in-place region and
        // re-render as a static block with per-child output below each child.
        print!("\x1b[J");
        let _ = stdout().flush();

        print_final_block(
            &states,
            &parent_label,
            &child_labels,
            parent_status,
            group_elapsed,
            self.show_time,
            self.quiet,
        );

        if let Some(ref log_path) = self.log {
            write_log_entry(
                log_path,
                &states,
                &parent_label,
                &child_labels,
                parent_status,
                group_elapsed,
                self.quiet,
                /* include_bodies = */ true,
            );
        }

        RunReport {
            status: parent_status,
            exit_code: status_to_exit_code(parent_status),
        }
    }

    /// Run the group with no console output (matches the single-command
    /// `--succinct` behavior: no spinner, no body, return code only).
    pub fn run_succinct_report(self) -> RunReport {
        let n = self.commands.len();
        let parent_label = self.parent_label(n);

        if n == 0 {
            return RunReport {
                status: RunStatus::Success,
                exit_code: 0,
            };
        }

        let child_labels: Vec<String> = self
            .commands
            .iter()
            .map(|c| derive_display_message(None, c))
            .collect();

        let states: Vec<Arc<Mutex<ChildState>>> = (0..n)
            .map(|_| Arc::new(Mutex::new(ChildState::Running)))
            .collect();

        let start = Instant::now();
        let handles = self.spawn_workers(&states);
        for h in handles {
            let _ = h.join();
        }

        let group_elapsed = start.elapsed();
        let parent_status = compute_parent_status(&states);

        if let Some(ref log_path) = self.log {
            write_log_entry(
                log_path,
                &states,
                &parent_label,
                &child_labels,
                parent_status,
                group_elapsed,
                self.quiet,
                /* include_bodies = */ false,
            );
        }

        RunReport {
            status: parent_status,
            exit_code: status_to_exit_code(parent_status),
        }
    }

    fn parent_label(&self, n: usize) -> String {
        match &self.message {
            Some(m) => m.clone(),
            None => format!("{n} parallel commands"),
        }
    }

    fn spawn_workers(&self, states: &[Arc<Mutex<ChildState>>]) -> Vec<thread::JoinHandle<()>> {
        self.commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let cmd = cmd.clone();
                let state = states[i].clone();
                let max_output = self.max_output;
                thread::spawn(move || run_child(cmd, state, max_output))
            })
            .collect()
    }
}

pub(crate) enum ChildState {
    Running,
    Done {
        status: RunStatus,
        output: CommandOutput,
        elapsed: Duration,
    },
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "ownership crosses the spawned-thread boundary"
)]
fn run_child(cmd: String, state: Arc<Mutex<ChildState>>, max_output: usize) {
    let start = Instant::now();

    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            *state.lock().unwrap() = ChildState::Done {
                status: RunStatus::Failure,
                output: CommandOutput {
                    stdout: String::new(),
                    stderr: format!("Failed to spawn command: {e}"),
                    exit_code: 1,
                },
                elapsed: start.elapsed(),
            };
            return;
        }
    };

    // Drain pipes concurrently so the child doesn't block on a full pipe.
    let stdout_reader = child
        .stdout
        .take()
        .map(|out| thread::spawn(move || read_bounded(out, max_output)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|err| thread::spawn(move || read_bounded(err, max_output)));

    let wait_result = child.wait();
    let stdout_str = stdout_reader
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let stderr_str = stderr_reader
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let elapsed = start.elapsed();

    let signal_num: Option<i32> = {
        #[cfg(unix)]
        {
            wait_result
                .as_ref()
                .ok()
                .and_then(std::os::unix::process::ExitStatusExt::signal)
        }
        #[cfg(not(unix))]
        {
            None
        }
    };
    let raw_exit = wait_result
        .as_ref()
        .ok()
        .and_then(std::process::ExitStatus::code)
        .unwrap_or(1);

    let interrupted = signal_num.is_some();
    let status = if interrupted {
        RunStatus::Interrupted
    } else if raw_exit == 0 {
        RunStatus::Success
    } else {
        RunStatus::Failure
    };
    let exit_code = if interrupted {
        signal_num.map_or(1, |n| 128 + n)
    } else {
        raw_exit
    };

    *state.lock().unwrap() = ChildState::Done {
        status,
        output: CommandOutput {
            stdout: stdout_str,
            stderr: stderr_str,
            exit_code,
        },
        elapsed,
    };
}

fn render_in_place(
    states: &[Arc<Mutex<ChildState>>],
    parent_label: &str,
    child_labels: &[String],
    spinner_char: char,
    group_elapsed: Duration,
    show_time: bool,
) {
    let time_slot = if show_time {
        format!(" {}", format_elapsed(group_elapsed))
    } else {
        String::new()
    };

    print!("\x1b[2K\r[ {spinner_char}{time_slot} ] {parent_label}");

    for (i, label) in child_labels.iter().enumerate() {
        print!("\x1b[1E\x1b[2K");
        let (icon, ts) = {
            let state = states[i].lock().unwrap();
            match &*state {
                ChildState::Running => {
                    let ts = if show_time {
                        format!(" {}", format_elapsed(group_elapsed))
                    } else {
                        String::new()
                    };
                    (spinner_char.to_string(), ts)
                }
                ChildState::Done {
                    status, elapsed, ..
                } => {
                    let icon = colored_icon(*status);
                    let ts = if show_time {
                        format!(" {}", format_elapsed(*elapsed))
                    } else {
                        String::new()
                    };
                    (icon, ts)
                }
            }
        };
        print!("    [ {icon}{ts} ] {label}");
    }

    let _ = stdout().flush();
}

pub(crate) fn print_final_block(
    states: &[Arc<Mutex<ChildState>>],
    parent_label: &str,
    child_labels: &[String],
    parent_status: RunStatus,
    group_elapsed: Duration,
    show_time: bool,
    quiet: bool,
) {
    let parent_icon = colored_icon(parent_status);
    let parent_ts = if show_time {
        format!(" {}", format_elapsed(group_elapsed))
    } else {
        String::new()
    };
    println!("[ {parent_icon}{parent_ts} ] {parent_label}");

    for (i, label) in child_labels.iter().enumerate() {
        let state = states[i].lock().unwrap();
        let ChildState::Done {
            status,
            output,
            elapsed,
        } = &*state
        else {
            continue;
        };
        let icon = colored_icon(*status);
        let ts = if show_time {
            format!(" {}", format_elapsed(*elapsed))
        } else {
            String::new()
        };
        println!("    [ {icon}{ts} ] {label}");

        let show_output = !matches!(status, RunStatus::Success) || !quiet;
        if show_output {
            let combined = format!("{}{}", output.stdout, output.stderr);
            let trimmed = combined.trim();
            if !trimmed.is_empty() {
                for line in trimmed.lines() {
                    println!("        {line}");
                }
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "write_log_entry aggregates many independent log fields"
)]
pub(crate) fn write_log_entry(
    log_path: &PathBuf,
    states: &[Arc<Mutex<ChildState>>],
    parent_label: &str,
    child_labels: &[String],
    parent_status: RunStatus,
    group_elapsed: Duration,
    quiet: bool,
    include_bodies: bool,
) {
    let now = chrono::Local::now();
    let timestamp = now.format("%Y-%m-%d %H:%M:%S");
    let parent_icon = plain_icon(parent_status);
    let mut entry = format!(
        "[{timestamp}] [ {parent_icon} {} ] {parent_label}\n",
        format_elapsed(group_elapsed)
    );

    for (i, label) in child_labels.iter().enumerate() {
        let state = states[i].lock().unwrap();
        let ChildState::Done {
            status,
            output,
            elapsed,
        } = &*state
        else {
            continue;
        };
        let icon = plain_icon(*status);
        entry.push('\t');
        let _ = writeln!(entry, "[ {icon} {} ] {label}", format_elapsed(*elapsed));

        if include_bodies {
            let show_output = !matches!(status, RunStatus::Success) || !quiet;
            if show_output {
                let combined = format!("{}{}", output.stdout, output.stderr);
                let trimmed = combined.trim();
                if !trimmed.is_empty() {
                    for line in trimmed.lines() {
                        entry.push_str("\t\t");
                        entry.push_str(line);
                        entry.push('\n');
                    }
                }
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

fn colored_icon(status: RunStatus) -> String {
    match status {
        RunStatus::Success => "\x1b[32m✓\x1b[0m".to_string(),
        RunStatus::Failure | RunStatus::Timeout => "\x1b[31m✘\x1b[0m".to_string(),
        RunStatus::Interrupted => "\x1b[33mINTERRUPTED\x1b[0m".to_string(),
    }
}

fn plain_icon(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Success => "✓",
        RunStatus::Failure | RunStatus::Timeout => "✘",
        RunStatus::Interrupted => "INTERRUPTED",
    }
}

pub(crate) fn compute_parent_status(states: &[Arc<Mutex<ChildState>>]) -> RunStatus {
    let mut any_interrupted = false;
    let mut all_success = true;
    for s in states {
        match &*s.lock().unwrap() {
            ChildState::Done { status, .. } => match status {
                RunStatus::Interrupted => any_interrupted = true,
                RunStatus::Success => {}
                _ => all_success = false,
            },
            ChildState::Running => all_success = false,
        }
    }
    if any_interrupted {
        RunStatus::Interrupted
    } else if all_success {
        RunStatus::Success
    } else {
        RunStatus::Failure
    }
}

pub(crate) fn status_to_exit_code(status: RunStatus) -> i32 {
    i32::from(!matches!(status, RunStatus::Success))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_true_succeeds() {
        assert!(parallel(["true", "true", "true"]).run());
    }

    #[test]
    fn one_false_fails_group() {
        assert!(!parallel(["true", "false", "true"]).run());
    }

    #[test]
    fn empty_group_succeeds() {
        let cmds: Vec<&str> = vec![];
        assert!(parallel(cmds).run());
    }

    #[test]
    fn report_exit_code_zero_on_all_success() {
        assert_eq!(parallel(["true", "true"]).run_report().exit_code, 0);
    }

    #[test]
    fn report_exit_code_one_on_any_failure() {
        assert_eq!(parallel(["true", "exit 42"]).run_report().exit_code, 1);
    }

    #[test]
    fn one_failure_does_not_cancel_siblings() {
        // All children should complete; the group reports failure but the
        // "true" sibling still ran (verified by overall elapsed being at
        // least ~the sleeping sibling's duration).
        let start = Instant::now();
        let report = parallel(["false", "sleep 0.3"]).run_report();
        let elapsed = start.elapsed();
        assert_eq!(report.status, RunStatus::Failure);
        assert!(
            elapsed >= Duration::from_millis(250),
            "sibling appears to have been cancelled (elapsed: {elapsed:?})"
        );
    }

    #[test]
    fn succinct_report_no_console_output_but_returns_status() {
        let report = parallel(["true", "true"]).run_succinct_report();
        assert_eq!(report.status, RunStatus::Success);
        assert_eq!(report.exit_code, 0);
    }

    #[test]
    fn succinct_report_failure_returns_one() {
        let report = parallel(["true", "false"]).run_succinct_report();
        assert_eq!(report.status, RunStatus::Failure);
        assert_eq!(report.exit_code, 1);
    }

    fn temp_log_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "shell_executor_parallel_test_{name}_{}.log",
            std::process::id()
        ))
    }

    #[test]
    fn log_entry_lists_children() {
        let path = temp_log_path("children");
        let _ = std::fs::remove_file(&path);

        parallel(["echo first", "echo second"])
            .message("two echoes")
            .log(&path)
            .run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("two echoes"));
        assert!(contents.contains("echo first"));
        assert!(contents.contains("echo second"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_no_ansi_codes() {
        let path = temp_log_path("no_ansi");
        let _ = std::fs::remove_file(&path);

        parallel(["echo ok", "false"]).log(&path).run();

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.contains(&0x1b),
            "parallel log file should not contain ANSI codes"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn quiet_failure_still_shown_in_log() {
        let path = temp_log_path("quiet_fail");
        let _ = std::fs::remove_file(&path);

        parallel(["echo silent", "echo loud >&2; false"])
            .quiet()
            .log(&path)
            .run();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            !contents.contains("\t\tsilent"),
            "quiet success body should be omitted"
        );
        assert!(
            contents.contains("\t\tloud"),
            "failure body should still be present"
        );
        let _ = std::fs::remove_file(&path);
    }
}
