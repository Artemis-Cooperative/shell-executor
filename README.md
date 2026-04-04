# shell-executor

A Rust library and CLI tool for running shell commands with a live spinner, elapsed timer, and configurable success criteria.

## Features

- **Spinner display** — shows a rotating spinner with elapsed time (`HH:MM:SS`) while a command runs
- **Timeouts** — kill long-running commands after a specified duration (exit code `124`)
- **Custom success criteria** — define a closure to determine success based on stdout, stderr, or exit code
- **Quiet mode** — suppress output on success, only show it on failure
- **Log file** — append timestamped execution results to a file after each command
- **Interrupt detection** — commands killed by a signal show `INTERRUPTED` instead of ✓/✘
- **Builder API** — chain `.message()`, `.timeout()`, `.success()`, `.quiet()`, and `.log()` before calling `.run()`

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

# Log results to a file
x "cargo build --release" --msg "Building" --log build.log
```

The `--log` option appends a timestamped entry to the specified file after each execution. The file is created if it doesn't exist. Log entries look like:

```
[2026-04-04 10:16:02] [ ✓ 00:00:09 ] Building
	output line 1
	output line 2
```

When `--quiet` is used, successful commands omit the tabulated output. Failed or interrupted commands always include output.

Exit code is `0` on success, `1` on failure.

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
```

## Running the Demo

```sh
cargo run --example demo
```

## Running Tests

```sh
cargo test
```
