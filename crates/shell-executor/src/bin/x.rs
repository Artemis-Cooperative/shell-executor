use std::time::Duration;

use clap::Parser;
use shell_executor::execute;

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
}

fn main() {
    let cli = Cli::parse();

    let mut cmd = execute(&cli.command);

    if let Some(msg) = cli.msg {
        cmd = cmd.message(&msg);
    }

    if let Some(secs) = cli.timeout {
        cmd = cmd.timeout(Duration::from_secs(secs));
    }

    if cli.quiet {
        cmd = cmd.quiet();
    }

    if let Some(log_path) = cli.log {
        cmd = cmd.log(log_path);
    }

    let success = cmd.run();

    std::process::exit(if success { 0 } else { 1 });
}
