//! CLI integration tests for the `--log <path>` flag on the `x` binary.
//!
//! These tests drive the `x` process and read the resulting log file from
//! disk. They assert the same invariants as the library-level log tests in
//! `src/lib.rs` `mod tests`, but at the CLI/process boundary so they survive
//! internal refactors of the library API.
//!
//! Exit-code propagation is intentionally NOT asserted here — that surface is
//! owned by `tests/cli_exit_code.rs`. These tests only inspect log contents.

#[allow(dead_code, reason = "shared test helpers; not every item is used in this file")]
mod common;

use common::{fresh_temp_path, x_bin};

// ---------------------------------------------------------------------------
// File creation / append behavior
// ---------------------------------------------------------------------------

#[test]
fn cli_log_creates_file_if_missing() {
    let path = fresh_temp_path("creates_file", "log");
    assert!(!path.exists(), "precondition: log path should not exist");

    let _ = x_bin()
        .arg("echo hello")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    assert!(
        path.exists(),
        "log file should have been created at {}",
        path.display()
    );
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("hello"),
        "log should contain command output `hello`, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cli_log_appends_across_runs() {
    let path = fresh_temp_path("appends", "log");

    let _ = x_bin()
        .arg("echo first")
        .arg("--msg")
        .arg("First")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let _ = x_bin()
        .arg("echo second")
        .arg("--msg")
        .arg("Second")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("First"),
        "log missing `First`: {contents}"
    );
    assert!(
        contents.contains("Second"),
        "log missing `Second`: {contents}"
    );
    let entry_count = contents.matches("] [").count();
    assert!(
        entry_count >= 2,
        "expected at least 2 entry separators (`] [`), got {entry_count}: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// Formatting: timestamp, status icons, elapsed
// ---------------------------------------------------------------------------

#[test]
fn cli_log_contains_timestamp_format() {
    let path = fresh_temp_path("timestamp", "log");

    let _ = x_bin()
        .arg("echo ts")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    let re_like = contents.starts_with('[') && contents.contains('-') && contents.contains(':');
    assert!(
        re_like,
        "log should start with a `[YYYY-MM-DD HH:MM:SS]`-style timestamp, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cli_log_success_shows_checkmark() {
    let path = fresh_temp_path("success_icon", "log");

    let _ = x_bin()
        .arg("echo ok")
        .arg("--msg")
        .arg("Success cmd")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("✓"),
        "success log should contain ✓, got: {contents}"
    );
    assert!(
        contents.contains("Success cmd"),
        "success log should contain the message, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cli_log_failure_shows_cross() {
    let path = fresh_temp_path("failure_icon", "log");

    let _ = x_bin()
        .arg("false")
        .arg("--msg")
        .arg("Fail cmd")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("✘"),
        "failure log should contain ✘, got: {contents}"
    );
    assert!(
        contents.contains("Fail cmd"),
        "failure log should contain the message, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cli_log_contains_elapsed_time() {
    let path = fresh_temp_path("elapsed", "log");

    let _ = x_bin()
        .arg("echo fast")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("00:00:0"),
        "log should contain an elapsed `HH:MM:SS` token like `00:00:0…`, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// ANSI hygiene
// ---------------------------------------------------------------------------

#[test]
fn cli_log_no_ansi_color_codes() {
    let path = fresh_temp_path("no_ansi", "log");

    let _ = x_bin()
        .arg("echo colored")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let _ = x_bin()
        .arg("false")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let bytes = std::fs::read(&path).unwrap();
    assert!(
        !bytes.contains(&0x1b),
        "log file should not contain ESC (0x1b) ANSI codes"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// Output body formatting
// ---------------------------------------------------------------------------

#[test]
fn cli_log_success_output_tabulated() {
    let path = fresh_temp_path("tabulated", "log");

    let _ = x_bin()
        .arg("printf 'line1\\nline2\\nline3'")
        .arg("--msg")
        .arg("Multi-line")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("\tline1\n"),
        "missing `\\tline1\\n` in: {contents}"
    );
    assert!(
        contents.contains("\tline2\n"),
        "missing `\\tline2\\n` in: {contents}"
    );
    assert!(
        contents.contains("\tline3\n"),
        "missing `\\tline3\\n` in: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// --quiet interaction
// ---------------------------------------------------------------------------

#[test]
fn cli_log_quiet_success_omits_output() {
    let path = fresh_temp_path("quiet_success", "log");

    let _ = x_bin()
        .arg("echo suppressed")
        .arg("--msg")
        .arg("Quiet ok")
        .arg("--quiet")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("Quiet ok"),
        "log missing message: {contents}"
    );
    assert!(
        !contents.contains("suppressed"),
        "quiet success should not log captured output, got: {contents}"
    );
    assert!(
        !contents.contains('\t'),
        "quiet success should have no tabulated lines, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cli_log_quiet_failure_includes_output() {
    let path = fresh_temp_path("quiet_failure", "log");

    let _ = x_bin()
        .arg("echo visible >&2; false")
        .arg("--msg")
        .arg("Quiet fail")
        .arg("--quiet")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("✘"),
        "quiet failure log should contain ✘, got: {contents}"
    );
    assert!(
        contents.contains("\tvisible"),
        "quiet failure should still log output as `\\tvisible`, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// Signal / interrupt
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn cli_log_interrupted_shows_interrupted() {
    let path = fresh_temp_path("interrupted", "log");

    let _ = x_bin()
        .arg("kill -INT $$")
        .arg("--msg")
        .arg("Signal test")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("INTERRUPTED"),
        "signal-killed command should log INTERRUPTED, got: {contents}"
    );
    assert!(
        !contents.contains("✓"),
        "interrupted should not show checkmark, got: {contents}"
    );
    assert!(
        !contents.contains("✘"),
        "interrupted should not show cross, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

#[cfg(unix)]
#[test]
fn cli_log_interrupted_no_ansi() {
    let path = fresh_temp_path("interrupted_no_ansi", "log");

    let _ = x_bin()
        .arg("kill -INT $$")
        .arg("--msg")
        .arg("No color")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let bytes = std::fs::read(&path).unwrap();
    assert!(
        !bytes.contains(&0x1b),
        "interrupted log should not contain ESC (0x1b) ANSI codes"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// --succinct interaction
// ---------------------------------------------------------------------------

#[test]
fn cli_log_succinct_writes_entry_without_body() {
    let path = fresh_temp_path("succinct_no_body", "log");

    let _ = x_bin()
        .arg("echo body-line")
        .arg("--msg")
        .arg("Succinct entry")
        .arg("--succinct")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(
        contents.contains("Succinct entry"),
        "succinct log should contain the message, got: {contents}"
    );
    assert!(
        contents.contains("✓"),
        "succinct log should contain ✓ on success, got: {contents}"
    );
    assert!(
        !contents.contains("body-line"),
        "succinct log should not include captured output body, got: {contents}"
    );
    assert!(
        !contents.contains('\t'),
        "succinct log should have no tabulated lines, got: {contents}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cli_log_succinct_no_ansi() {
    let path = fresh_temp_path("succinct_no_ansi", "log");

    let _ = x_bin()
        .arg("echo hello")
        .arg("--succinct")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let _ = x_bin()
        .arg("false")
        .arg("--succinct")
        .arg("--log")
        .arg(&path)
        .status()
        .expect("failed to run x");

    let bytes = std::fs::read(&path).unwrap();
    assert!(
        !bytes.contains(&0x1b),
        "succinct log file should not contain ESC (0x1b) ANSI codes"
    );
    let _ = std::fs::remove_file(&path);
}
