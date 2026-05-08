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
}

fn main() {
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

    let report = if cli.succinct {
        cmd.run_succinct_report()
    } else {
        cmd.run_report()
    };

    let final_code: i32 = match (report.status, &cli.validator) {
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
            if cli.succinct {
                v.run_succinct_report().exit_code
            } else {
                v.run_report().exit_code
            }
        }
    };

    std::process::exit(final_code);
}
