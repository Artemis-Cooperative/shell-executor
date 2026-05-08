# shell-executor

A Rust library and CLI tool for running shell commands with a live spinner, elapsed timer, and configurable success criteria.

## Features

- **Spinner display** — shows a rotating spinner with elapsed time (`HH:MM:SS`) while a command runs
- **Timeouts** — kill long-running commands after a specified duration (exit code `124`)
- **Custom success criteria** — define a closure to determine success based on stdout, stderr, or exit code
- **Quiet mode** — suppress output on success, only show it on failure
- **Verbose mode** — `--verbose` makes the default "show output on success" behavior explicit (inverse of `--quiet`); the two are mutually exclusive
- **Succinct mode** — `--succinct` drops the `[ ✓ … ]` wrapper and streams the child's stdout/stderr directly to the terminal without capture or indentation
- **Log file** — append timestamped execution results to a file after each command
- **Interrupt detection** — commands killed by a signal show `INTERRUPTED` instead of ✓/✘
- **Validator command** — a `-v / --validator <cmd>` flag on the CLI runs a second shell command after the main one; its exit code determines overall pass/fail (overrides the main command's status, unless the main command timed out or was interrupted)
- **Exit-code propagation** — the `x` CLI exits with the underlying command's actual exit code (e.g. `exit 42` → `x` exits 42), `124` on timeout, and `128 + signal` on signal-kill (Unix)
- **Builder API** — chain `.message()`, `.timeout()`, `.success()`, `.quiet()`, and `.log()` before calling a terminal method: `.run()` (bool), `.run_status()` (`RunStatus`), `.run_report()` (`RunReport` — status + exit code), `.run_succinct_report()` (inherited stdio, no wrapper), or `.run_capture()` (silent, returns `CommandOutput`)
- **Bare-label printers** — `pass(msg)` and `fail(msg)` emit `[ ✓ ] msg` / `[ ✘ ] msg` for reporting the outcome of in-process work where there is no shell command to wrap

## CLI Usage

The `x` binary wraps the library for command-line use:

```sh
# Basic usage
x "echo hello"

# Custom spinner message
x "cargo build --release" --msg "Building project"

# With a timeout (in seconds)
x "sleep 60" --timeout 5

# Quiet mode — only print output on failure
x "cargo test" --msg "Running tests" --quiet

# Verbose mode — explicitly show output on success (default; inverse of --quiet)
x "cargo test" --verbose

# Succinct mode — drop the [ ✓ … ] wrapper and stream output directly
x "cargo build" --succinct

# Log results to a file
x "cargo build --release" --msg "Building" --log build.log

# Run a validator after the main command — its exit code decides overall pass/fail
x "cargo build" -v "cargo test"
x "deploy.sh" --validator "curl -fsS https://example.com/health"
```

The `--log` option appends a timestamped entry to the specified file after each execution. The file is created if it doesn't exist. Log entries look like:

```
[2026-04-04 10:16:02] [ ✓ 00:00:09 ] Building
	output line 1
	output line 2
```

When `--quiet` is used, successful commands omit the tabulated output. Failed or interrupted commands always include output.

When `--succinct` is used, the bracketed wrapper line and the spinner are suppressed entirely; the child's stdout and stderr stream live to the terminal (no capture, no leading-tab indentation). Since no `[ ✓ ]`/`[ ✘ ]` marker is shown, callers should rely on `x`'s exit code to detect failure. With `--log`, an entry is still written for the run with the timestamp, status icon, elapsed time, and message — but no tabulated output body, since succinct mode does not capture output. `--quiet` and `--verbose` have no effect in succinct mode.

`x` exits with the underlying command's actual exit code, so e.g. `x "exit 42"` exits 42. On timeout `x` exits `124` (matching the Unix `timeout` convention); on signal-kill (Unix) `x` exits `128 + signal_number` (POSIX shell convention; SIGINT → 130, SIGTERM → 143). When `-v / --validator` is supplied, the validator's exit code propagates instead of the main command's.

## Library Usage

Add the dependency to your `Cargo.toml`:

```toml
[dependencies]
shell-executor = { path = "." }
```

Then use the builder API:

```rust
use std::time::Duration;
use shell_executor::execute;

// Simple command
execute("echo hello").run();

// With all options
let ok = execute("cargo test")
    .message("Running tests")
    .timeout(Duration::from_secs(60))
    .success(|output| output.stdout.contains("passed"))
    .quiet()
    .run();

if !ok {
    eprintln!("Tests failed!");
}

// Log results to a file
execute("cargo build --release")
    .message("Building")
    .quiet()
    .log("build.log")
    .run();

// Get the actual exit code (not just success/failure)
use shell_executor::{RunStatus, execute};
let report = execute("exit 42").run_report();
assert_eq!(report.exit_code, 42);
assert_eq!(report.status, RunStatus::Failure);

// Stream output directly with no spinner / wrapper
execute("cargo build").run_succinct_report();

// Print a bare-label outcome for in-process work (no command run)
use shell_executor::{pass, fail};
pass("working tree clean");
fail("could not open repository");
```

## Running the Demo

```sh
cargo run --example demo
```

## Running Tests

```sh
cargo test
```

