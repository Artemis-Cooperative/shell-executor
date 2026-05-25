//! CLI integration tests for the `--parallel` flag on the `x` binary.

use std::process::Command;

fn x_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_x"))
}

/// `x --parallel "true" "true"` → exit 0.
#[test]
fn all_succeed_exits_zero() {
    let status = x_bin()
        .arg("--parallel")
        .arg("true")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert_eq!(status.code(), Some(0));
}

/// Mixed pass/fail → exit 1 (parallel group does not propagate child codes).
#[test]
fn mixed_results_exit_one() {
    let status = x_bin()
        .arg("--parallel")
        .arg("true")
        .arg("exit 42")
        .status()
        .expect("failed to run x");
    assert_eq!(status.code(), Some(1));
}

/// `--parallel` can also be repeated rather than greedy.
#[test]
fn repeated_parallel_flag_accepted() {
    let status = x_bin()
        .arg("--parallel")
        .arg("true")
        .arg("--parallel")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert_eq!(status.code(), Some(0));
}

/// Validator passing rescues a failing parallel group.
#[test]
fn validator_passing_rescues_failed_group() {
    let status = x_bin()
        .arg("--parallel")
        .arg("false")
        .arg("-v")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert_eq!(status.code(), Some(0));
}

/// Validator failing on an all-pass group does NOT poison success.
/// (Per spec: exit 0 if all_succeed OR validator passes.)
#[test]
fn validator_failing_does_not_poison_passing_group() {
    let status = x_bin()
        .arg("--parallel")
        .arg("true")
        .arg("-v")
        .arg("false")
        .status()
        .expect("failed to run x");
    assert_eq!(status.code(), Some(0));
}

/// Both fail → exit 1.
#[test]
fn both_group_and_validator_fail_exits_one() {
    let status = x_bin()
        .arg("--parallel")
        .arg("false")
        .arg("-v")
        .arg("false")
        .status()
        .expect("failed to run x");
    assert_eq!(status.code(), Some(1));
}

// Note: `--parallel` + `--interactive` no longer conflicts — it opens the
// mprocs-style TUI. That path can't be exercised in a non-TTY test runner
// since ratatui requires a terminal device.

/// `--parallel` conflicts with `--timeout`.
#[test]
fn parallel_conflicts_with_timeout() {
    let status = x_bin()
        .arg("--parallel")
        .arg("true")
        .arg("--timeout")
        .arg("5")
        .status()
        .expect("failed to run x");
    assert_eq!(status.code(), Some(2));
}

/// No command and no --parallel → usage error.
#[test]
fn no_command_no_parallel_errors() {
    let status = x_bin().status().expect("failed to run x");
    assert_eq!(status.code(), Some(2));
}

/// Siblings run to completion even when one fails — we should observe the
/// failing-fast child *and* the slow child both finish.
#[test]
fn siblings_run_to_completion_on_failure() {
    let start = std::time::Instant::now();
    let status = x_bin()
        .arg("--parallel")
        .arg("false")
        .arg("sleep 0.3")
        .status()
        .expect("failed to run x");
    let elapsed = start.elapsed();
    assert_eq!(status.code(), Some(1));
    assert!(
        elapsed >= std::time::Duration::from_millis(250),
        "expected the slow sibling to finish (elapsed: {elapsed:?})"
    );
}

/// `--succinct --parallel` produces a return code with no body output.
#[test]
fn succinct_parallel_returns_status_only() {
    let output = x_bin()
        .arg("--succinct")
        .arg("--parallel")
        .arg("true")
        .arg("true")
        .output()
        .expect("failed to run x");
    assert_eq!(output.status.code(), Some(0));
    assert!(
        output.stdout.is_empty(),
        "succinct parallel should emit no stdout, got: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
}
