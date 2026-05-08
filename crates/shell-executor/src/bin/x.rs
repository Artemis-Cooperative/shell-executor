use std::time::Duration;

use clap::Parser;
use shell_executor::{RunStatus, execute};

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

    /// Suppress output on success
    #[arg(long, short)]
    quiet: bool,

    /// Append execution results to a log file
    #[arg(long)]
    log: Option<String>,

    /// Validator shell command run after the main command. Its exit code
    /// determines overall pass/fail (overrides the main command's status).
    /// Not run if the main command timed out or was interrupted by a signal.
    #[arg(long, short = 'v')]
    validator: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    let mut cmd = execute(&cli.command);

    if let Some(msg) = &cli.msg {
        cmd = cmd.message(msg);
    }

    if let Some(secs) = cli.timeout {
        cmd = cmd.timeout(Duration::from_secs(secs));
    }

    if cli.quiet {
        cmd = cmd.quiet();
    }

    if let Some(log_path) = &cli.log {
        cmd = cmd.log(log_path);
    }

    let status = cmd.run_status();

    let overall_success = match (status, &cli.validator) {
        (RunStatus::Timeout, _) | (RunStatus::Interrupted, _) => false,
        (RunStatus::Success, None) => true,
        (RunStatus::Failure, None) => false,
        (RunStatus::Success | RunStatus::Failure, Some(v_cmd)) => {
            let mut v = execute(v_cmd).message("Validator");
            if cli.quiet {
                v = v.quiet();
            }
            if let Some(log_path) = &cli.log {
                v = v.log(log_path);
            }
            v.run()
        }
    };

    std::process::exit(if overall_success { 0 } else { 1 });
}
