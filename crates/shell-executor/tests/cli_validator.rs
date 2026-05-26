//! CLI integration tests for the `-v` / `--validator` flag on the `x` binary.
//!
//! The validator runs after the main command. Its exit code overrides the
//! main command's exit code when determining overall pass/fail — except when
//! the main command timed out or was interrupted by a signal.

#[allow(dead_code, reason = "shared test helper module; not all helpers are used in every test file")]
mod common;

use common::{fresh_temp_path, x_bin};

/// Main succeeds, validator succeeds → overall success (exit 0).
#[test]
fn validator_short_flag_main_pass_validator_pass() {
    let status = x_bin()
        .arg("echo hello")
        .arg("-v")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert!(
        status.success(),
        "expected exit 0 when both main and validator succeed, got {status:?}"
    );
}

/// Main succeeds, validator fails → overall failure (non-zero exit).
/// Verifies the validator actually ran by checking a side-effect marker file.
#[test]
fn validator_short_flag_main_pass_validator_fail() {
    let marker = fresh_temp_path("pass_then_fail", "marker");
    let validator_cmd = format!("touch {}; exit 1", marker.display());

    let status = x_bin()
        .arg("echo hello")
        .arg("-v")
        .arg(&validator_cmd)
        .status()
        .expect("failed to run x");
    assert!(
        !status.success(),
        "expected non-zero exit when validator fails, got {status:?}"
    );
    assert!(
        marker.exists(),
        "validator should have run and created marker file at {}",
        marker.display()
    );
    let _ = std::fs::remove_file(&marker);
}

/// Main fails, validator succeeds → overall success (validator overrides).
#[test]
fn validator_overrides_failed_main_command() {
    let status = x_bin()
        .arg("false")
        .arg("-v")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert!(
        status.success(),
        "validator success should override failed main command, got {status:?}"
    );
}

/// Main fails, validator fails → overall failure.
/// Verifies validator ran (it must run even though main failed).
#[test]
fn validator_main_fail_validator_fail() {
    let marker = fresh_temp_path("fail_fail", "marker");
    let validator_cmd = format!("touch {}; exit 2", marker.display());

    let status = x_bin()
        .arg("false")
        .arg("-v")
        .arg(&validator_cmd)
        .status()
        .expect("failed to run x");
    assert!(
        !status.success(),
        "expected non-zero exit when both main and validator fail, got {status:?}"
    );
    assert!(
        marker.exists(),
        "validator must run even when main fails; marker should exist at {}",
        marker.display()
    );
    let _ = std::fs::remove_file(&marker);
}

/// `--validator` long form should behave identically to `-v`.
#[test]
fn validator_long_flag_main_pass_validator_pass() {
    let status = x_bin()
        .arg("echo hi")
        .arg("--validator")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert!(
        status.success(),
        "expected exit 0 with --validator long form, got {status:?}"
    );
}

/// `--validator` long form propagates validator failure.
/// Verifies the long-form flag actually parses by checking the validator's side effect.
#[test]
fn validator_long_flag_validator_fail_overrides_main_pass() {
    let marker = fresh_temp_path("long_fail", "marker");
    let validator_cmd = format!("touch {}; exit 1", marker.display());

    let status = x_bin()
        .arg("echo hi")
        .arg("--validator")
        .arg(&validator_cmd)
        .status()
        .expect("failed to run x");
    assert!(
        !status.success(),
        "expected non-zero exit when --validator fails, got {status:?}"
    );
    assert!(
        marker.exists(),
        "--validator long form should have parsed and run the validator; marker missing at {}",
        marker.display()
    );
    let _ = std::fs::remove_file(&marker);
}

/// `--validator` long form: validator success overrides failed main.
#[test]
fn validator_long_flag_overrides_failed_main() {
    let status = x_bin()
        .arg("false")
        .arg("--validator")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert!(
        status.success(),
        "expected --validator success to override failed main, got {status:?}"
    );
}

/// When the main command times out, the validator should NOT run; the timeout
/// status wins, so the overall result must be failure regardless of what the
/// validator would have returned.
#[test]
fn validator_does_not_run_on_main_timeout() {
    // Main sleeps longer than the timeout. Validator would succeed if run,
    // but it must NOT run — overall must be failure.
    let status = x_bin()
        .arg("sleep 30")
        .arg("--timeout")
        .arg("1")
        .arg("-v")
        .arg("true")
        .status()
        .expect("failed to run x");
    assert!(
        !status.success(),
        "timeout should win over validator; expected non-zero exit, got {status:?}"
    );
}

/// Sanity check: a validator with a side effect (writing a marker file)
/// must NOT execute when the main command times out.
#[test]
fn validator_side_effect_not_observed_on_timeout() {
    let marker = std::env::temp_dir().join(format!(
        "shell_executor_validator_marker_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);

    let validator_cmd = format!("touch {}", marker.display());

    let _ = x_bin()
        .arg("sleep 30")
        .arg("--timeout")
        .arg("1")
        .arg("-v")
        .arg(&validator_cmd)
        .status()
        .expect("failed to run x");

    assert!(
        !marker.exists(),
        "validator must not run when the main command times out; marker file should not exist at {}",
        marker.display()
    );
    let _ = std::fs::remove_file(&marker);
}
