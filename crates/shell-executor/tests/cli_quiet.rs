//! CLI integration tests for the `--quiet` / `-q` flag on the `x` binary.
//!
//! Behavior under test (observed at the CLI boundary):
//!
//! - On success, `--quiet` suppresses the indented output body but STILL prints
//!   the `[ ✓ ] <message>` wrapper line.
//! - On failure, `--quiet` is overridden: the output body is shown so the user
//!   can diagnose the failure, and the `[ ✘ ] <message>` wrapper appears.
//! - `-q` is the short alias for `--quiet` and must behave identically.
//!
//! These assertions check only externally observable output (stdout+stderr
//! combined) and exit codes, so they survive refactors of the internal
//! printing pipeline.
//!
//! Implementation note: when no `--msg` is supplied, the wrapper echoes the
//! command string verbatim (if ≤30 chars). To distinguish "wrapper contamination"
//! from "body leak", these tests choose a body token (e.g. `hiddenbody`) that
//! does NOT appear in the command-string portion shown by the wrapper.

#[allow(dead_code, reason = "shared test-helper module; not every item is used in every test file")]
mod common;

use common::x_bin;

/// `x "printf '%s%s\n' hid denbody" --quiet` — successful command's body must
/// be suppressed, but the wrapper line (and ✓ marker) must still appear.
///
/// We split the body token across two `printf` arguments so the produced output
/// is `hiddenbody` (a single token), while the command source string echoed in
/// the wrapper contains only `hid` and `denbody` separately — never the joined
/// token. This lets us assert on the joined token to detect body leakage.
#[test]
fn quiet_success_omits_output() {
    let output = x_bin()
        .arg("printf '%s%s\\n' hid denbody")
        .arg("--quiet")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "quiet run should exit 0; got {:?}",
        output.status
    );
    assert!(
        !combined.contains("hiddenbody"),
        "quiet success should NOT show the command's stdout body; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains('✓'),
        "quiet success should still print the ✓ wrapper; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `--quiet` success must specifically omit the indented body form (four
/// leading spaces + the body text).
#[test]
fn quiet_success_omits_indented_body() {
    let output = x_bin()
        .arg("printf '%s%s\\n' hid denbody")
        .arg("--quiet")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains("    hiddenbody"),
        "quiet success should not emit `    hiddenbody` (4-space indented body); got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("\thiddenbody"),
        "quiet success should not emit `\\thiddenbody` (tab-indented body); got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "echo visible >&2; false" --quiet` — failure overrides quiet:
/// output is shown and the ✘ marker appears.
#[test]
fn quiet_failure_still_shows_output() {
    let output = x_bin()
        .arg("echo visiblebody >&2; false")
        .arg("--quiet")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        output.status.code(),
        Some(1),
        "quiet run of `false` should propagate exit code 1; got {:?}",
        output.status
    );
    assert!(
        combined.contains("visiblebody"),
        "quiet failure must still show stderr body; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains('✘'),
        "quiet failure must show the ✘ marker; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "echo on_stdout; false" --quiet` — stdout body is also shown on failure.
#[test]
fn quiet_failure_shows_stdout_too() {
    let output = x_bin()
        .arg("echo onstdoutbody; false")
        .arg("--quiet")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        output.status.code(),
        Some(1),
        "quiet run of `... ; false` should exit 1; got {:?}",
        output.status
    );
    assert!(
        combined.contains("onstdoutbody"),
        "quiet failure must show stdout body too; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `-q` is the short alias for `--quiet` and must behave identically on success
/// (suppress body, exit 0).
#[test]
fn quiet_short_flag_equivalent() {
    let output = x_bin()
        .arg("printf '%s%s\\n' sho rtqbody")
        .arg("-q")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "`-q` run should exit 0; got {:?}",
        output.status
    );
    assert!(
        !combined.contains("shortqbody"),
        "`-q` success should not emit the joined body token; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// Even with `--quiet`, the bracketed `[ ✓ ] ...` wrapper line is still
/// printed on success.
#[test]
fn quiet_wrapper_still_renders() {
    let output = x_bin()
        .arg("echo h")
        .arg("--quiet")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "quiet `echo h` should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains("[ "),
        "quiet mode should still emit the `[ ` wrapper opener; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains(" ] "),
        "quiet mode should still emit the ` ] ` wrapper closer; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains('✓'),
        "quiet mode should still emit the ✓ marker; got stdout={stdout:?} stderr={stderr:?}"
    );
}
