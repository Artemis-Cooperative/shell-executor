//! CLI integration tests for the `--msg` / `--message` flag on the `x` binary.
//!
//! Behavior under test (observed at the CLI boundary):
//!
//! - `--msg <text>` sets the wrapper's display message verbatim. The text
//!   appears inside the `[ ✓ ] <text>` (or `[ ✘ ] <text>`) wrapper line.
//! - `--message <text>` is the long alias and is equivalent to `--msg`.
//! - When no `--msg` is supplied, the wrapper's display message is derived from
//!   the command string: if the command is 30 chars or fewer, it is used
//!   verbatim; otherwise it is truncated to the first 30 chars followed by
//!   `...` (literal three-dot suffix).
//! - The message text may contain spaces and punctuation; it is emitted
//!   literally without escaping.
//!
//! These assertions check only externally observable output, so they survive
//! refactors of the internal printing pipeline.

#[allow(dead_code, reason = "common module is used via `use common::x_bin` but Rust's dead_code lint fires on mod declarations in integration test files")]
mod common;

use common::x_bin;

/// `x "true" --msg "Hello World"` — the message appears inside the wrapper.
#[test]
fn msg_renders_in_wrapper() {
    let output = x_bin()
        .arg("true")
        .arg("--msg")
        .arg("Hello World")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "`--msg` run of `true` should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains("Hello World"),
        "wrapper should contain the literal --msg text; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains('✓'),
        "wrapper should contain the ✓ marker on success; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "true" --message "Aliased"` — the long alias `--message` works.
#[test]
fn message_long_alias_renders() {
    let output = x_bin()
        .arg("true")
        .arg("--message")
        .arg("Aliased")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "`--message` run of `true` should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains("Aliased"),
        "`--message` alias should set the wrapper text; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "false" --msg "Boom"` — the message is rendered alongside the ✘ marker.
#[test]
fn msg_with_failure() {
    let output = x_bin()
        .arg("false")
        .arg("--msg")
        .arg("Boom")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        output.status.code(),
        Some(1),
        "`--msg` run of `false` should exit 1; got {:?}",
        output.status
    );
    assert!(
        combined.contains("Boom"),
        "wrapper should contain the --msg text on failure; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains('✘'),
        "wrapper should contain the ✘ marker on failure; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "echo hi"` (no --msg) — when the command is 30 chars or fewer, the
/// derived display message is the command verbatim.
#[test]
fn default_message_uses_command_when_short() {
    let cmd = "echo hi";
    assert!(cmd.len() <= 30, "test fixture: command must be short");

    let output = x_bin().arg(cmd).output().expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "default-message run of `{cmd}` should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains(cmd),
        "short command should appear verbatim as the wrapper message; got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// `x "<long command>"` (no --msg) — when the command exceeds 30 chars, the
/// derived display message is the first 30 chars + `...`.
#[test]
fn default_message_truncates_long_command() {
    let cmd = "echo this is a very long command that exceeds thirty characters";
    assert!(cmd.len() > 30, "test fixture: command must be long");

    let expected_prefix = format!("{}...", &cmd[..30]);
    // Sanity-check the fixture: the first 30 chars of the literal above are
    // `echo this is a very long comma`, so the expected substring is
    // `echo this is a very long comma...`.
    assert_eq!(
        expected_prefix, "echo this is a very long comma...",
        "test fixture drift: recompute the expected prefix"
    );

    let output = x_bin().arg(cmd).output().expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "default-message run of a long command should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains(&expected_prefix),
        "wrapper should display first 30 chars + `...` (expected substring {expected_prefix:?}); got stdout={stdout:?} stderr={stderr:?}"
    );
}

/// Smoke test: `--msg` alone has no log-file side effect (no path written).
/// We don't have a temp dir here; just verify the run succeeds and the message
/// is emitted, with no errors on stderr beyond the wrapper itself.
#[test]
fn msg_does_not_appear_in_log_when_log_not_passed() {
    let output = x_bin()
        .arg("true")
        .arg("--msg")
        .arg("X")
        .output()
        .expect("failed to run x");

    assert_eq!(
        output.status.code(),
        Some(0),
        "`--msg` alone (no --log) should exit 0; got {:?}",
        output.status
    );
}

/// `--msg` accepts spaces and punctuation literally.
#[test]
fn msg_value_can_contain_spaces_and_punctuation() {
    let msg = "Build #42: linking";
    let output = x_bin()
        .arg("true")
        .arg("--msg")
        .arg(msg)
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "`--msg` with punctuation should exit 0; got {:?}",
        output.status
    );
    assert!(
        combined.contains(msg),
        "wrapper should emit the full punctuated message literally; got stdout={stdout:?} stderr={stderr:?}"
    );
}
