use copilot_adapter::auth::input::wait_for_enter_or_timeout;
use std::time::{Duration, Instant};

#[test]
fn timeout_returns_false_when_no_input() {
    // No one presses Enter during the test, so the function should return
    // false after the timeout (or immediately if stdin is not a terminal).
    let start = Instant::now();
    let result = wait_for_enter_or_timeout(Duration::from_millis(200));
    let elapsed = start.elapsed();

    assert!(!result, "should return false when no input is provided");
    // Should complete within a reasonable time (timeout + overhead)
    assert!(
        elapsed < Duration::from_secs(2),
        "should complete within a reasonable time (took {:?})",
        elapsed
    );
}

#[test]
fn zero_timeout_returns_false() {
    let result = wait_for_enter_or_timeout(Duration::from_secs(0));
    assert!(!result, "zero timeout should return false");
}
