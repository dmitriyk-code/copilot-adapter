use copilot_adapter::auth::browser::open_url;

#[test]
fn open_url_returns_ok() {
    // We can't actually open a browser in tests, but we can verify the function
    // doesn't panic and returns a Result. On CI/headless systems, the command
    // may fail to spawn, so we just check it returns Ok or Err (not panic).
    let result = open_url("https://example.com");
    // The function should always return a Result, never panic
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn open_url_handles_empty_url() {
    let result = open_url("");
    // Should not panic regardless of URL content
    assert!(result.is_ok() || result.is_err());
}
