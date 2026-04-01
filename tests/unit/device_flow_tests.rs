use copilot_adapter::auth::device_flow::DeviceCodeResponse;

#[test]
fn device_code_response_deserializes_without_verification_uri_complete() {
    let json = serde_json::json!({
        "device_code": "dc_123",
        "user_code": "ABCD-1234",
        "verification_uri": "https://github.com/login/device",
        "expires_in": 900,
        "interval": 5
    });

    let response: DeviceCodeResponse = serde_json::from_value(json).unwrap();
    assert_eq!(response.device_code, "dc_123");
    assert_eq!(response.user_code, "ABCD-1234");
    assert_eq!(response.verification_uri, "https://github.com/login/device");
    assert!(response.verification_uri_complete.is_none());
    assert_eq!(response.expires_in, 900);
    assert_eq!(response.interval, 5);
}

#[test]
fn device_code_response_deserializes_with_verification_uri_complete() {
    let json = serde_json::json!({
        "device_code": "dc_456",
        "user_code": "EFGH-5678",
        "verification_uri": "https://github.com/login/device",
        "verification_uri_complete": "https://github.com/login/device?user_code=EFGH-5678",
        "expires_in": 900,
        "interval": 5
    });

    let response: DeviceCodeResponse = serde_json::from_value(json).unwrap();
    assert_eq!(response.device_code, "dc_456");
    assert_eq!(
        response.verification_uri_complete.as_deref(),
        Some("https://github.com/login/device?user_code=EFGH-5678")
    );
}

#[test]
fn device_code_response_deserializes_with_null_verification_uri_complete() {
    let json = serde_json::json!({
        "device_code": "dc_789",
        "user_code": "IJKL-9012",
        "verification_uri": "https://github.com/login/device",
        "verification_uri_complete": null,
        "expires_in": 900,
        "interval": 5
    });

    let response: DeviceCodeResponse = serde_json::from_value(json).unwrap();
    assert!(response.verification_uri_complete.is_none());
}
