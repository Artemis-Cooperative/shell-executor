use shell_executor::execute;
use std::time::Duration;

fn main() {
    #[allow(clippy::print_stdout, reason = "demo example intentionally prints to stdout")]
    {
        println!("=== Shell Executor Demo ===\n");
    }

    // 1. Simple success
    execute("echo 'Hello, world!'").message("Greeting").run();

    // 2. Simple failure
    execute("ls /nonexistent_path_12345")
        .message("Listing missing directory")
        .run();

    // 3. Timeout
    execute("sleep 10")
        .message("Long running task (will timeout)")
        .timeout(Duration::from_secs(1))
        .run();

    // 4. Custom success closure
    execute("echo 'tests passed: 42'")
        .message("Running tests")
        .success(|output| output.stdout.contains("passed"))
        .run();

    // 5. Custom success closure that fails
    execute("echo 'no match here'")
        .message("Check for keyword")
        .success(|output| output.stdout.contains("keyword"))
        .run();

    // 6. Minimal usage — no message, uses command as label
    execute("echo done").run();

    #[allow(clippy::print_stdout, reason = "demo example intentionally prints to stdout")]
    {
        println!("\n=== Demo Complete ===");
    }
}
