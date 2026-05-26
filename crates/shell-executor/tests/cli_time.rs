//! CLI integration tests for the `--time` flag on the `x` binary.
//!
//! Default mode renders the status wrapper without an elapsed `HH:MM:SS`
//! segment (`[ ✓ ] message`). Passing `--time` brings the segment back
//! (`[ ✓ 00:00:00 ] message`).

#[allow(dead_code)]
mod common;

use common::x_bin;

/// `x "echo hi"` (no flags) — final wrapper must not contain `HH:MM:SS`.
#[test]
fn default_omits_duration() {
    let output = x_bin().arg("echo hi").output().expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "default run of `echo hi` should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains('✓'),
        "default mode should still emit the ✓ marker; got stdout={stdout:?} stderr={stderr:?}"
    );
    // The duration slot uses `HH:MM:SS`. For a sub-second command, the only
    // possible value is `00:00:00`. Asserting absence of the `00:00:` fragment
    // catches both the spinner and the final line variants without depending
    // on which spinner frame happened to be last.
    assert!(
        !combined.contains("00:00:"),
        "default mode wrapper should not include `HH:MM:SS` duration; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "echo hi" --time` — final wrapper must include the `HH:MM:SS` slot.
#[test]
fn time_flag_includes_duration() {
    let output = x_bin()
        .arg("echo hi")
        .arg("--time")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "`--time` run of `echo hi` should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains('✓'),
        "`--time` should still emit the ✓ marker; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("00:00:"),
        "`--time` should include the `HH:MM:SS` duration in the wrapper; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "false"` (no flags) — failure marker is shown but without `HH:MM:SS`.
#[test]
fn default_omits_duration_on_failure() {
    let output = x_bin().arg("false").output().expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        output.status.code(),
        Some(1),
        "`false` should propagate exit code 1; got {:?}",
        output.status
    );
    assert!(
        combined.contains('✘'),
        "default mode should emit the ✘ marker on failure; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("00:00:"),
        "default failure wrapper should not include `HH:MM:SS`; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "false" --time` — failure marker still includes `HH:MM:SS`.
#[test]
fn time_flag_includes_duration_on_failure() {
    let output = x_bin()
        .arg("false")
        .arg("--time")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        output.status.code(),
        Some(1),
        "`false` should propagate exit code 1; got {:?}",
        output.status
    );
    assert!(
        combined.contains('✘'),
        "`--time` should emit the ✘ marker on failure; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("00:00:"),
        "`--time` failure wrapper should include `HH:MM:SS`; got stdout={stdout:?} stderr={stderr:?}"
    );
}
