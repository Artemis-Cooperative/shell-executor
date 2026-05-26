//! CLI integration tests for the `--parallel` flag on the `x` binary.

#[allow(dead_code, reason = "shared test helper module; not every item is used in this file")]
mod common;

use common::{fresh_temp_path, x_bin};

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
/// (Per spec: exit 0 if `all_succeed` OR validator passes.)
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

/// `--msg` becomes the parent label of the parallel block.
#[test]
fn parallel_msg_renders_parent_label() {
    let output = x_bin()
        .arg("--msg")
        .arg("build pipeline")
        .arg("--parallel")
        .arg("true")
        .arg("true")
        .output()
        .expect("failed to run x");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(
        combined.contains("build pipeline"),
        "expected parent label `build pipeline` in output, got: {combined}"
    );
    assert!(
        combined.contains('✓'),
        "expected ✓ in parent line, got: {combined}"
    );
}

/// Without `--msg`, the parent label is the count, e.g. `2 parallel commands`.
#[test]
fn parallel_default_parent_label_uses_count() {
    let output = x_bin()
        .arg("--parallel")
        .arg("true")
        .arg("true")
        .output()
        .expect("failed to run x");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(
        combined.contains("2 parallel commands"),
        "expected default count label `2 parallel commands`, got: {combined}"
    );
}

/// `--parallel --log` writes the parent label, every child label, and at
/// least one tab-indented child line per `write_log_entry`.
#[test]
fn parallel_log_records_group_entry_and_children() {
    let path = fresh_temp_path("group_and_children", "log");

    let _ = x_bin()
        .arg("--msg")
        .arg("duo")
        .arg("--parallel")
        .arg("echo first")
        .arg("echo second")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("duo"),
        "log missing parent label `duo`: {contents}"
    );
    assert!(
        contents.contains("echo first"),
        "log missing child label `echo first`: {contents}"
    );
    assert!(
        contents.contains("echo second"),
        "log missing child label `echo second`: {contents}"
    );
    assert!(
        contents.contains("\t["),
        "log missing tab-indented child line (`\\t[`): {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

/// Parallel logs must be ANSI-free even when mixing success and failure.
#[test]
fn parallel_log_no_ansi() {
    let path = fresh_temp_path("no_ansi", "log");

    let _ = x_bin()
        .arg("--parallel")
        .arg("echo ok")
        .arg("false")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let bytes = std::fs::read(&path).unwrap();
    assert!(
        !bytes.contains(&0x1b),
        "parallel log file should not contain ESC (0x1b) ANSI codes"
    );
    let _ = std::fs::remove_file(&path);
}

/// In default (non-quiet) mode the log includes captured output bodies for
/// both successful and failed children.
#[test]
fn parallel_log_failed_child_body_present() {
    let path = fresh_temp_path("failed_body", "log");

    let _ = x_bin()
        .arg("--parallel")
        .arg("echo silent_ok")
        .arg("echo loud_fail >&2; false")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("loud_fail"),
        "log should contain failed-child body `loud_fail`, got: {contents}"
    );
    assert!(
        contents.contains("silent_ok"),
        "default (non-quiet) mode should include successful-child body \
         `silent_ok` too, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

/// `--parallel --quiet` suppresses bodies of successful children on the
/// terminal while still rendering failed-child bodies. Exit code 1 because
/// one child failed.
///
/// Body lines on the terminal are indented 8 spaces; child labels are not.
/// We probe for the body-form (`        SUCCESS_BODY`) so the label
/// `echo SUCCESS_BODY` doesn't trigger a false positive.
#[test]
fn parallel_quiet_omits_successful_child_bodies() {
    let output = x_bin()
        .arg("--parallel")
        .arg("echo SUCCESS_BODY")
        .arg("echo FAIL_BODY >&2; false")
        .arg("--quiet")
        .output()
        .expect("failed to run x");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        combined.contains("FAIL_BODY"),
        "quiet+parallel should still show failed-child body, got: {combined}"
    );
    assert!(
        !combined.contains("        SUCCESS_BODY"),
        "quiet+parallel should NOT show successful-child body line, got: {combined}"
    );
}

/// Same as the terminal case (#6), but asserted against the log file.
///
/// Body lines in the log are doubly-tab-indented (`\t\t…`); child labels are
/// singly-tab-indented. We probe for the body-form so the label
/// `echo SUCCESS_BODY` doesn't false-positive.
#[test]
fn parallel_quiet_log_omits_successful_child_body() {
    let path = fresh_temp_path("quiet_log", "log");

    let _ = x_bin()
        .arg("--parallel")
        .arg("echo SUCCESS_BODY")
        .arg("echo FAIL_BODY >&2; false")
        .arg("--quiet")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("FAIL_BODY"),
        "quiet+parallel log should still include failed-child body, got: {contents}"
    );
    assert!(
        !contents.contains("\t\tSUCCESS_BODY"),
        "quiet+parallel log should NOT include successful-child body line, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

/// `--time` causes the parent line to include an `HH:MM:SS` token.
#[test]
fn parallel_time_includes_duration_in_parent() {
    let output = x_bin()
        .arg("--parallel")
        .arg("true")
        .arg("true")
        .arg("--time")
        .output()
        .expect("failed to run x");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(combined.contains('✓'), "expected ✓ in output: {combined}");
    assert!(
        combined.contains("00:00:0"),
        "expected sub-second `00:00:0…` duration token, got: {combined}"
    );
}

/// Without `--time`, the parent line must NOT include an `HH:MM:SS` token.
#[test]
fn parallel_default_omits_duration_in_parent() {
    let output = x_bin()
        .arg("--parallel")
        .arg("true")
        .arg("true")
        .output()
        .expect("failed to run x");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(combined.contains('✓'), "expected ✓ in output: {combined}");
    assert!(
        !combined.contains("00:00:"),
        "default rendering should omit duration, got: {combined}"
    );
}

/// README-documented pattern: per-child timeouts are achieved by nesting `x`
/// calls inside `--parallel` command strings. The slow inner call times out
/// (exit non-zero), the fast inner call passes, the group reports failure.
///
/// Also asserts that the slow inner child was actually killed by its inner
/// `--timeout 1` rather than allowed to run for 30s.
#[test]
fn parallel_nested_x_timeout_pattern() {
    let inner = std::env::var("CARGO_BIN_EXE_x")
        .expect("CARGO_BIN_EXE_x must be set by cargo for integration tests");

    let start = std::time::Instant::now();
    let status = x_bin()
        .arg("--parallel")
        .arg(format!("{inner} 'sleep 30' --timeout 1"))
        .arg(format!("{inner} 'true'"))
        .status()
        .expect("failed to run x");
    let elapsed = start.elapsed();

    assert_eq!(
        status.code(),
        Some(1),
        "nested-x group should report failure when an inner child times out"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "inner --timeout 1 should have killed the sleep within ~1s; \
         actual elapsed: {elapsed:?}"
    );
}

/// `--succinct --parallel --log` writes the entry header + child labels but
/// omits captured bodies (per `run_succinct_report` calling `write_log_entry`
/// with `include_bodies = false`).
#[test]
fn parallel_succinct_log_writes_entry_without_bodies() {
    let path = fresh_temp_path("succinct_no_body", "log");

    let _ = x_bin()
        .arg("--succinct")
        .arg("--parallel")
        .arg("echo body")
        .arg("true")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("echo body"),
        "succinct parallel log should still include child labels, got: {contents}"
    );
    assert!(
        contents.contains("true"),
        "succinct parallel log should include the second child label, got: {contents}"
    );
    assert!(
        !contents.contains("\t\tbody"),
        "succinct parallel log should NOT include body lines (`\\t\\tbody`), \
         got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}
