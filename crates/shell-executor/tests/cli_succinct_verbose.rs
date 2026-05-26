//! CLI integration tests for the `--succinct` and `--verbose` flags on the `x` binary.
//!
//! - `--succinct` suppresses the spinner and the `[ ✓ HH:MM:SS ] message` wrapper.
//!   The command's stdout/stderr passes through directly without leading-tab
//!   indentation. Exit code still propagates per existing rules.
//!
//! - `--verbose` shows stdout/stderr on success (the inverse of `--quiet`).
//!   Note: `-v` is NOT a short alias for `--verbose` because `-v` is taken by
//!   `--validator`. `--verbose` is long-only.

#[allow(dead_code)]
mod common;

use common::x_bin;

// ---------------------------------------------------------------------------
// Baseline: confirm the wrapper IS present in default mode so the negative
// assertions in the --succinct tests below are meaningful.
// ---------------------------------------------------------------------------

/// Default mode (no flags) shows the bracketed wrapper, the success marker,
/// and indents the command's output with a leading tab/spaces.
#[test]
fn default_mode_shows_wrapper() {
    let output = x_bin().arg("echo hello").output().expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("[ "),
        "default mode should contain the `[ ` wrapper opener; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("✓"),
        "default mode should contain the ✓ success marker; got stdout={stdout:?} stderr={stderr:?}"
    );
    // The library indents output with four leading spaces. Either four spaces
    // or a literal tab counts as the indented-output indicator.
    assert!(
        combined.contains("    hello") || combined.contains("\thello"),
        "default mode should indent the command's stdout line; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// --succinct
// ---------------------------------------------------------------------------

/// `x "echo hello" --succinct` — stdout passes through, no wrapper.
#[test]
fn succinct_passes_stdout_through() {
    let output = x_bin()
        .arg("echo hello")
        .arg("--succinct")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("hello"),
        "succinct should pass stdout through; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("[ "),
        "succinct should not emit the `[ ` wrapper opener; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains('✓'),
        "succinct should not emit the ✓ marker; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains('✘'),
        "succinct should not emit the ✘ marker; got stdout={stdout:?} stderr={stderr:?}"
    );
    // The default mode prefixes command output lines with four spaces (or a
    // tab). Succinct should not — `hello` should appear at column zero.
    assert!(
        !combined.contains("    hello"),
        "succinct should not indent command output with leading spaces; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("\thello"),
        "succinct should not indent command output with a leading tab; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "echo err >&2" --succinct` — stderr passes through, no wrapper, no indent.
#[test]
fn succinct_passes_stderr_through() {
    let output = x_bin()
        .arg("echo err >&2")
        .arg("--succinct")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "succinct run of `echo err >&2` should exit 0; got {:?} stderr={stderr:?}",
        output.status
    );
    // The marker substring `err` must appear AND must not be the clap-error
    // text (`error: unexpected argument`). Asserting `err\n` (newline-terminated)
    // makes this strict: clap's error message has no bare `err\n` token.
    assert!(
        combined.contains("err\n") && !combined.contains("error: unexpected argument"),
        "succinct should pass stderr through and not emit a clap error; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("[ "),
        "succinct should not emit the `[ ` wrapper opener around stderr; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains('✓'),
        "succinct should not emit the ✓ marker; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("    err"),
        "succinct should not indent stderr lines with leading spaces; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("\terr"),
        "succinct should not indent stderr lines with a leading tab; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// Spinner animation characters (`-`, `\`, `|`, `/`) wrapped by the bracket
/// pair `[ ... ] ` must not appear in succinct mode. Asserting absence of the
/// closing ` ] ` substring (which appears in every wrapper line including the
/// final success/failure line) is the sharpest single check.
#[test]
fn succinct_no_spinner_animation_chars() {
    let output = x_bin()
        .arg("echo hi")
        .arg("--succinct")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "succinct run of `echo hi` should exit 0 (not a clap error); got {:?} stderr={stderr:?}",
        output.status
    );
    assert!(
        combined.contains("hi"),
        "succinct should still emit the command's actual stdout; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains(" ] "),
        "succinct should not produce the ` ] ` wrapper closer (spinner / final line); got stdout={stdout:?} stderr={stderr:?}"
    );
    // Also verify none of the spinner-frame fragments leak through.
    for frag in [" - 00:", " \\ 00:", " | 00:", " / 00:"] {
        assert!(
            !combined.contains(frag),
            "succinct should not emit spinner fragment {frag:?}; got stdout={stdout:?} stderr={stderr:?}"
        );
    }
}

/// `x "false" --succinct` — even on failure, no `[ ✘ ... ]` line. Exit code is
/// still 1 (false's conventional non-zero).
#[test]
fn succinct_failed_command_no_wrapper() {
    let output = x_bin()
        .arg("false")
        .arg("--succinct")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        output.status.code(),
        Some(1),
        "succinct should still propagate exit code 1 from `false`; got {:?}",
        output.status
    );
    assert!(
        !combined.contains('✘'),
        "succinct must not emit the ✘ failure marker; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("[ "),
        "succinct must not emit the `[ ` wrapper opener even on failure; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains(" ] "),
        "succinct must not emit the ` ] ` wrapper closer even on failure; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "exit 42" --succinct` — succinct must not change exit-code propagation.
#[test]
fn succinct_preserves_exit_code() {
    let status = x_bin()
        .arg("exit 42")
        .arg("--succinct")
        .status()
        .expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(42),
        "succinct should preserve `exit 42` propagation; got {status:?}"
    );
}

// ---------------------------------------------------------------------------
// --verbose
// ---------------------------------------------------------------------------

/// `x "echo hi" --verbose` — output is shown on success.
#[test]
fn verbose_shows_output_on_success() {
    let output = x_bin()
        .arg("echo hi")
        .arg("--verbose")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "verbose run of `echo hi` should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains("hi"),
        "verbose should show stdout on success; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "echo bye; false" --verbose` — output is shown even on failure.
#[test]
fn verbose_shows_output_on_failure() {
    let output = x_bin()
        .arg("echo bye; false")
        .arg("--verbose")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !output.status.success(),
        "verbose run of `echo bye; false` should exit non-zero; got {:?}",
        output.status
    );
    assert!(
        combined.contains("bye"),
        "verbose should show stdout on failure; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `-v` must remain bound to `--validator`, not `--verbose`.
///
/// Running `x "echo x" -v "echo y"` should parse `echo y` as the validator
/// command, which exits 0, so the overall status is success.
#[test]
fn verbose_no_short_flag_v() {
    let status = x_bin()
        .arg("echo x")
        .arg("-v")
        .arg("echo y")
        .status()
        .expect("failed to run x");
    assert_eq!(
        status.code(),
        Some(0),
        "`-v` must continue to mean --validator (validator `echo y` exits 0); got {status:?}"
    );
}

/// `--verbose` long form parses and runs successfully.
#[test]
fn verbose_long_only_flag() {
    let output = x_bin()
        .arg("echo x")
        .arg("--verbose")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "`--verbose` long form should be accepted by clap and run successfully; got {:?} stdout={stdout:?} stderr={stderr:?}",
        output.status
    );
    assert!(
        combined.contains('x'),
        "`--verbose` long form should display the command's stdout; got stdout={stdout:?} stderr={stderr:?}"
    );
}
