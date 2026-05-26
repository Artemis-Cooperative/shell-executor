use shell_executor::execute;
use std::time::Duration;

#[test]
fn full_pipeline() {
    let result = execute("echo integration")
        .message("Full pipeline test")
        .timeout(Duration::from_secs(5))
        .success(|output| output.stdout.contains("integration"))
        .run();
    assert!(result);
}

#[test]
fn multiple_sequential_commands() {
    assert!(execute("echo first").run());
    assert!(execute("echo second").run());
    assert!(execute("echo third").run());
}

#[test]
fn success_and_failure_together() {
    let ok = execute("echo works").run();
    assert!(ok);

    let fail = execute("false").run();
    assert!(!fail);

    // Confirm another success after a failure
    let ok_again = execute("echo still works").run();
    assert!(ok_again);
}

#[test]
fn timeout_in_integration() {
    let result = execute("sleep 30")
        .message("Should time out")
        .timeout(Duration::from_millis(200))
        .run();
    assert!(!result);
}
