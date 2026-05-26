//! CLI integration tests for exit-code propagation on the `x` binary.
//!
//! The `x` CLI should preserve the actual exit code of the underlying shell
//! command instead of clamping to 0/1. When a validator is supplied, the
//! validator's exit code is what propagates (validator overrides main).
//!
//! Conventional exit codes that remain fixed:
//! - Timeout: 124 (matches Unix `timeout` command convention)
//! - Signal-killed main: non-zero (specific value left to the implementer;
//!   reasonable choices include 1, or 128 + signum a la POSIX shells)

#[allow(dead_code)]
mod common;

use common::{fresh_temp_path, x_bin};

/// `x true` → exit 0.
#[test]
fn exit_code_true_is_zero() {
    let status = x_bin().arg("true").status().expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(0),
        "expected exit 0 for `x true`, got {status:?}"
    );
}

/// `x false` → exit 1 (the conventional non-zero from `false`).
#[test]
fn exit_code_false_is_one() {
    let status = x_bin().arg("false").status().expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(1),
        "expected exit 1 for `x false`, got {status:?}"
    );
}

/// `x "exit 0"` → exit 0.
#[test]
fn exit_code_explicit_zero() {
    let status = x_bin().arg("exit 0").status().expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(0),
        "expected exit 0 for `x \"exit 0\"`, got {status:?}"
    );
}

/// `x "exit 42"` → exit 42 (preserve the actual exit code, don't clamp to 1).
#[test]
fn exit_code_42_propagates() {
    let status = x_bin().arg("exit 42").status().expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(42),
        "expected exit 42 for `x \"exit 42\"`, got {status:?}"
    );
}

/// `x "exit 7"` → exit 7.
#[test]
fn exit_code_7_propagates() {
    let status = x_bin().arg("exit 7").status().expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(7),
        "expected exit 7 for `x \"exit 7\"`, got {status:?}"
    );
}

/// `x "exit 42" -v "true"` → exit 0. The validator overrides the main
/// command's exit code, and the validator succeeded.
#[test]
fn validator_success_overrides_main_nonzero() {
    let status = x_bin()
        .arg("exit 42")
        .arg("-v")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(0),
        "validator success should override main's exit 42; expected 0, got {status:?}"
    );
}

/// `x "exit 0" -v "exit 7"` → exit 7. The validator's exit code propagates
/// even when the main command succeeded.
#[test]
fn validator_nonzero_overrides_main_success() {
    let status = x_bin()
        .arg("exit 0")
        .arg("-v")
        .arg("exit 7")
        .status()
        .expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(7),
        "validator's exit 7 should propagate over main's success; got {status:?}"
    );
}

/// `x "exit 42" -v "exit 5"` → exit 5. The validator's code wins; main's 42
/// is discarded.
#[test]
fn validator_nonzero_overrides_main_nonzero() {
    let status = x_bin()
        .arg("exit 42")
        .arg("-v")
        .arg("exit 5")
        .status()
        .expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(5),
        "validator's exit 5 should override main's exit 42; got {status:?}"
    );
}

/// `x "true" -v "false"` → exit 1. Validator's standard non-zero propagates.
#[test]
fn validator_false_yields_one() {
    let status = x_bin()
        .arg("true")
        .arg("-v")
        .arg("false")
        .status()
        .expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(1),
        "expected exit 1 from validator `false`, got {status:?}"
    );
}

/// `x "sleep 30" --timeout 1` → exit 124 (conventional Unix `timeout` code).
/// The library already emits `exit_code` 124 internally on timeout; the CLI
/// should surface that to the OS.
#[test]
fn timeout_exits_124() {
    let status = x_bin()
        .arg("sleep 30")
        .arg("--timeout")
        .arg("1")
        .status()
        .expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(124),
        "expected exit 124 on timeout (matching Unix `timeout` convention), got {status:?}"
    );
}

/// Signal-killed main command → non-zero exit.
///
/// We deliberately do NOT pin a specific value here. Reasonable choices the
/// implementer/planner may pick include:
///   * `1` (current clamp behavior)
///   * `128 + signum` (POSIX shell convention; SIGINT → 130)
///   * some other sentinel
///
/// This test only locks in the invariant that an interrupted main command
/// exits non-zero; the precise value is left for the implementer to decide.
#[cfg(unix)]
#[test]
fn signal_killed_main_exits_nonzero() {
    let status = x_bin()
        .arg("kill -INT $$")
        .status()
        .expect("failed to run x");
    // Intentionally loose: only assert non-zero. See comment above.
    assert_ne!(
        status.code(),
        Some(0),
        "expected non-zero exit when main command is killed by a signal, got {status:?}"
    );
}

/// `x "sleep 30" --timeout 1 -v "exit 0"` → exit 124, validator must NOT run.
/// The timeout exit code propagates and the validator is skipped entirely.
/// We prove the skip by giving the validator a side-effect (touching a marker
/// file) that we then assert is absent.
#[test]
fn validator_skipped_on_timeout_exits_124() {
    let marker = fresh_temp_path("timeout_skip", "marker");
    let validator_cmd = format!("touch {}; exit 0", marker.display());

    let status = x_bin()
        .arg("sleep 30")
        .arg("--timeout")
        .arg("1")
        .arg("-v")
        .arg(&validator_cmd)
        .status()
        .expect("failed to run x");

    assert_eq!(
        status.code(),
        Some(124),
        "expected exit 124 on timeout (validator must be skipped), got {status:?}"
    );
    assert!(
        !marker.exists(),
        "validator must not run when the main command times out; marker should not exist at {}",
        marker.display()
    );
    let _ = std::fs::remove_file(&marker);
}

/// `x "kill -INT $$" -v "exit 0"` → exit 130 (128 + SIGINT(2)), validator must NOT run.
/// When the main command is killed by a signal, the signal exit code propagates
/// and the validator is skipped. We prove the skip with a marker-file side effect.
#[cfg(unix)]
#[test]
fn signal_killed_main_with_validator_propagates_signal_code() {
    let marker = fresh_temp_path("signal_skip", "marker");
    let validator_cmd = format!("touch {}; exit 0", marker.display());

    let status = x_bin()
        .arg("kill -INT $$")
        .arg("-v")
        .arg(&validator_cmd)
        .status()
        .expect("failed to run x");

    assert_eq!(
        status.code(),
        Some(130),
        "expected exit 130 (128 + SIGINT) when main is signal-killed, got {status:?}"
    );
    assert!(
        !marker.exists(),
        "validator must not run when the main command is signal-killed; marker should not exist at {}",
        marker.display()
    );
    let _ = std::fs::remove_file(&marker);
}

/// Tight version of `signal_killed_main_exits_nonzero`: pin the exact
/// `128 + signum` POSIX-shell convention so silent regressions get caught.
/// SIGINT is signal 2, so the expected exit code is exactly 130.
#[cfg(unix)]
#[test]
fn signal_int_exits_exactly_130() {
    let status = x_bin()
        .arg("kill -INT $$")
        .status()
        .expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(130),
        "expected exit exactly 130 (128 + SIGINT(2)) per POSIX shell convention, got {status:?}"
    );
}
