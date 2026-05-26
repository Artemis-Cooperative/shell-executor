//! Positive-shape integration tests for default-mode output of the `x` CLI.
//!
//! The existing test suite (see `cli_succinct_verbose.rs`, `cli_quiet.rs`)
//! asserts what default mode does NOT do under `--succinct` / `--quiet`. Only
//! one weak baseline (`default_mode_shows_wrapper`) covers the positive shape,
//! and it only checks a single `echo hello` line. This file pins the full
//! positive shape of default mode so the renderer block in
//! `crates/shell-executor/src/lib.rs` (around lines 440–461) is protected
//! during refactors.
//!
//! Pinned facts:
//! - Body lines are prefixed with exactly four ASCII spaces (not a tab).
//! - Multi-line output emits each line on its own row, each indented.
//! - stderr-only output is captured and indented identically to stdout.
//! - Empty output emits the wrapper line only — no indented body lines.
//! - Trailing newlines in command output are trimmed; no trailing `    `
//!   blank line appears.
//! - The wrapper line is emitted before the body block.
//!
//! NOTE: The wrapper line starts with `\r` and contains ANSI color escapes,
//! so all assertions use `contains` against ANSI-free substrings. We never
//! index lines positionally — the carriage-return prefix would defeat that.

#[allow(dead_code)]
mod common;

use common::x_bin;

// ---------------------------------------------------------------------------
// 1. Single-line stdout — four-space indent (not tab)
// ---------------------------------------------------------------------------

/// `x "echo hi"` produces a body line of exactly four spaces + `hi`.
/// Pins the spaces-vs-tab choice: a tab-indented form must NOT appear.
#[test]
fn stdout_single_line_indented_with_four_spaces() {
    let output = x_bin()
        .arg("echo hi")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("    hi"),
        "default mode should indent stdout with exactly four spaces; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("\thi"),
        "default mode must not indent with a tab; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Multi-line stdout — each line indented independently
// ---------------------------------------------------------------------------

/// Three printf'd lines must each receive their own four-space prefix, in
/// order, joined by single newlines.
#[test]
fn stdout_multi_line_each_indented_separately() {
    let output = x_bin()
        .arg("printf 'line1\nline2\nline3'")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("    line1"),
        "line1 should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("    line2"),
        "line2 should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("    line3"),
        "line3 should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("    line1\n    line2\n    line3"),
        "all three lines should appear in order, each indented, joined by single newlines; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. stderr-only command — indented identically to stdout
// ---------------------------------------------------------------------------

/// A command whose only output goes to stderr is still captured and indented
/// with the same four-space prefix as stdout.
#[test]
fn stderr_only_command_indented_same_as_stdout() {
    let output = x_bin()
        .arg("echo errline >&2")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("    errline"),
        "stderr-only output should be captured and indented; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. Mixed stdout + stderr — both appear, both indented
// ---------------------------------------------------------------------------

/// When a command emits to both stdout and stderr, both lines must appear,
/// each with the four-space indent. Ordering is not pinned because the two
/// pipes are captured independently.
#[test]
fn stdout_and_stderr_both_shown_indented() {
    let output = x_bin()
        .arg("echo out_a; echo err_b >&2")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("    out_a"),
        "stdout line should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("    err_b"),
        "stderr line should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// 5. Empty-output command — wrapper only, no body lines
// ---------------------------------------------------------------------------

/// `x "true"` produces only the wrapper line: no indented body line follows.
/// A "body line" here means a line beginning with `"    "` followed by a
/// non-space character.
#[test]
fn empty_output_command_shows_wrapper_only() {
    let output = x_bin()
        .arg("true")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("[ "),
        "wrapper marker should still appear; got stdout={stdout:?} stderr={stderr:?}"
    );

    let has_body_line = combined.lines().any(|line| {
        line.starts_with("    ")
            && line
                .chars()
                .nth(4)
                .map(|c| !c.is_whitespace())
                .unwrap_or(false)
    });
    assert!(
        !has_body_line,
        "empty-output command should emit no indented body line; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// 6. Trailing newlines trimmed — no trailing blank indented line
// ---------------------------------------------------------------------------

/// `printf 'hello\n\n\n'` has trailing newlines. The `.trim()` call on the
/// combined buffer strips them, so no `"    "` blank line follows `hello`.
/// We assert this by ruling out two consecutive blank indented lines.
#[test]
fn output_trimmed_no_trailing_blank_indented_line() {
    let output = x_bin()
        .arg("printf 'hello\n\n\n'")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("    hello"),
        "hello line should still be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !combined.contains("    \n    \n"),
        "trailing newlines must be trimmed — no two consecutive blank indented lines; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// 7. Failure path — body still emitted and indented
// ---------------------------------------------------------------------------

/// On failure, default mode still shows the body block (the `!success ||
/// !self.quiet` guard short-circuits on `!success`). The wrapper uses ✘.
#[test]
fn failure_command_output_still_shown_indented() {
    let output = x_bin()
        .arg("echo dying; false")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        output.status.code(),
        Some(1),
        "`echo dying; false` should exit 1; got {:?}",
        output.status
    );
    assert!(
        combined.contains("    dying"),
        "failure body should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains('✘'),
        "failure wrapper should contain ✘; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// 8. Larger output — loop-generated lines all indented
// ---------------------------------------------------------------------------

/// Spot-check that loop-generated output stays indented (no chunking artifact
/// drops the prefix on later lines).
#[test]
fn large_output_first_lines_appear_indented() {
    let output = x_bin()
        .arg("for i in $(seq 1 5); do echo line_$i; done")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("    line_1"),
        "line_1 should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("    line_3"),
        "line_3 should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        combined.contains("    line_5"),
        "line_5 should be indented; got stdout={stdout:?} stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// 9. Special chars preserved — only the prefix is added
// ---------------------------------------------------------------------------

/// Internal whitespace in a line is preserved verbatim. Only the *prefix*
/// (four spaces) is added; embedded double-spaces and tabs survive.
#[test]
fn output_with_special_chars_preserved() {
    let output = x_bin()
        .arg("printf 'spaces  and\\ttab\\n'")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("spaces  and"),
        "double space inside line should be preserved; got stdout={stdout:?} stderr={stderr:?}"
    );

    // Find the indented body line containing our marker (the wrapper line
    // also contains the literal command string `spaces  and\\ttab`, so we
    // must skip it by requiring the four-space body prefix).
    let line = combined
        .lines()
        .find(|l| l.starts_with("    ") && l.contains("spaces  and"))
        .unwrap_or_else(|| panic!("no indented body line contained `spaces  and`; got stdout={stdout:?} stderr={stderr:?}"));
    let after_and = line
        .split_once("and")
        .map(|(_, rest)| rest)
        .unwrap_or("");
    assert!(
        after_and.contains('\t'),
        "embedded tab after `and` should survive; line={line:?} stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        line.starts_with("    "),
        "line should still carry the four-space prefix; line={line:?}"
    );
}

// ---------------------------------------------------------------------------
// 10. Order — wrapper line precedes body block
// ---------------------------------------------------------------------------

/// The wrapper `println!` runs before the body loop, so the `[ ` opener must
/// appear earlier in the combined output than any indented body line.
#[test]
fn wrapper_appears_after_body_block_renders() {
    let output = x_bin()
        .arg("echo line1")
        .output()
        .expect("failed to run x");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    let wrapper_pos = combined
        .find("[ ")
        .unwrap_or_else(|| panic!("no `[ ` wrapper opener found; got stdout={stdout:?} stderr={stderr:?}"));
    let body_pos = combined
        .find("    line1")
        .unwrap_or_else(|| panic!("no `    line1` body line found; got stdout={stdout:?} stderr={stderr:?}"));

    assert!(
        wrapper_pos < body_pos,
        "wrapper should be emitted before body block; wrapper_pos={wrapper_pos} body_pos={body_pos} stdout={stdout:?} stderr={stderr:?}"
    );
}
