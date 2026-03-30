use copilot_adapter::conversation_log::{
    ContentBlockSummary, ConversationCycle, ConversationLogger, MessageSummary, ToolCallSummary,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sample_cycle() -> ConversationCycle {
    ConversationCycle {
        timestamp: chrono::Utc::now(),
        request_number: 42,
        request_id: "req-test-123".to_string(),

        incoming_model: "claude-sonnet-4-20250514".to_string(),
        incoming_stream: true,
        incoming_messages: vec![
            MessageSummary {
                role: "user".to_string(),
                content_preview: "Hello world".to_string(),
                content_length: 11,
                has_tool_use: false,
                has_tool_result: false,
            },
            MessageSummary {
                role: "assistant".to_string(),
                content_preview: "I'll help you.".to_string(),
                content_length: 14,
                has_tool_use: true,
                has_tool_result: false,
            },
        ],
        incoming_system: Some("You are helpful.".to_string()),
        incoming_tools: vec!["read_file".to_string(), "write_file".to_string()],

        outgoing_model: "claude-sonnet-4".to_string(),
        outgoing_messages_count: 3,
        tools_injected: true,
        xml_injection_size: 2048,

        response_model: "claude-sonnet-4".to_string(),
        response_finish_reason: Some("stop".to_string()),
        response_content_preview: "Here is the file content.".to_string(),
        response_has_tool_calls: true,

        final_stop_reason: Some("tool_use".to_string()),
        final_content_blocks: vec![
            ContentBlockSummary {
                block_type: "text".to_string(),
                preview: "Here is the file content.".to_string(),
            },
            ContentBlockSummary {
                block_type: "tool_use".to_string(),
                preview: "read_file (id=call_abc)".to_string(),
            },
        ],
        parsed_tool_calls: vec![ToolCallSummary {
            id: "call_abc".to_string(),
            name: "read_file".to_string(),
            arguments_preview: r#"{"path":"/src/main.rs"}"#.to_string(),
        }],
    }
}

// ---------------------------------------------------------------------------
// Format tests
// ---------------------------------------------------------------------------

#[test]
fn format_produces_readable_output() {
    let cycle = sample_cycle();
    let output = cycle.format();

    assert!(output.contains("Request #42"));
    assert!(output.contains("req-test-123"));
    assert!(output.contains("claude-sonnet-4-20250514"));
    assert!(output.contains("Stream: true"));
    assert!(output.contains("Messages: 2"));
    assert!(output.contains("Tools (2): read_file, write_file"));
    assert!(output.contains("XML injection size: 2048 bytes"));
    assert!(output.contains("Parsed tool calls: 1"));
}

#[test]
fn format_shows_all_four_sections() {
    let output = sample_cycle().format();
    assert!(output.contains(">>> FROM CLAUDE CODE (Anthropic format)"));
    assert!(output.contains(">>> TO GITHUB COPILOT API (OpenAI format)"));
    assert!(output.contains("<<< FROM GITHUB COPILOT API (OpenAI format)"));
    assert!(output.contains("<<< TO CLAUDE CODE (Anthropic format)"));
}

#[test]
fn format_handles_empty_messages() {
    let cycle = ConversationCycle {
        timestamp: chrono::Utc::now(),
        request_number: 1,
        request_id: "req-empty".to_string(),
        incoming_model: "gpt-4".to_string(),
        incoming_stream: false,
        incoming_messages: vec![],
        incoming_system: None,
        incoming_tools: vec![],
        outgoing_model: "gpt-4".to_string(),
        outgoing_messages_count: 0,
        tools_injected: false,
        xml_injection_size: 0,
        response_model: "gpt-4".to_string(),
        response_finish_reason: None,
        response_content_preview: String::new(),
        response_has_tool_calls: false,
        final_stop_reason: None,
        final_content_blocks: vec![],
        parsed_tool_calls: vec![],
    };

    let output = cycle.format();
    assert!(output.contains("Messages: 0"));
    assert!(output.contains("Finish reason: (none)"));
    assert!(output.contains("Stop reason: (none)"));
    assert!(output.contains("Content blocks: 0"));
    // No parsed tool calls section when empty
    assert!(!output.contains("Parsed tool calls:"));
}

#[test]
fn format_truncates_long_content() {
    let long_preview = "x".repeat(1000);
    let cycle = ConversationCycle {
        timestamp: chrono::Utc::now(),
        request_number: 1,
        request_id: "req-long".to_string(),
        incoming_model: "model".to_string(),
        incoming_stream: false,
        incoming_messages: vec![MessageSummary {
            role: "user".to_string(),
            content_preview: long_preview,
            content_length: 1000,
            has_tool_use: false,
            has_tool_result: false,
        }],
        incoming_system: None,
        incoming_tools: vec![],
        outgoing_model: "model".to_string(),
        outgoing_messages_count: 1,
        tools_injected: false,
        xml_injection_size: 0,
        response_model: "model".to_string(),
        response_finish_reason: Some("stop".to_string()),
        response_content_preview: "ok".to_string(),
        response_has_tool_calls: false,
        final_stop_reason: Some("end_turn".to_string()),
        final_content_blocks: vec![],
        parsed_tool_calls: vec![],
    };

    let output = cycle.format();
    // The preview should be truncated (200 chars + "…")
    assert!(output.len() < 2000);
    assert!(output.contains('…'));
}

#[test]
fn format_shows_tool_result_flags() {
    let cycle = ConversationCycle {
        timestamp: chrono::Utc::now(),
        request_number: 1,
        request_id: "req-tr".to_string(),
        incoming_model: "m".to_string(),
        incoming_stream: false,
        incoming_messages: vec![MessageSummary {
            role: "user".to_string(),
            content_preview: "result".to_string(),
            content_length: 6,
            has_tool_use: false,
            has_tool_result: true,
        }],
        incoming_system: None,
        incoming_tools: vec![],
        outgoing_model: "m".to_string(),
        outgoing_messages_count: 1,
        tools_injected: false,
        xml_injection_size: 0,
        response_model: "m".to_string(),
        response_finish_reason: None,
        response_content_preview: String::new(),
        response_has_tool_calls: false,
        final_stop_reason: None,
        final_content_blocks: vec![],
        parsed_tool_calls: vec![],
    };

    let output = cycle.format();
    assert!(output.contains("[tool_result]"));
}

// ---------------------------------------------------------------------------
// Logger I/O tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn log_rotation_works() {
    let dir = std::env::temp_dir().join(format!("conv_log_rot_{}", std::process::id()));
    let _ = tokio::fs::create_dir_all(&dir).await;
    let log_path = dir.join("rotation.log");
    let backup_path = dir.join("rotation.log.1");

    let _ = tokio::fs::remove_file(&log_path).await;
    let _ = tokio::fs::remove_file(&backup_path).await;

    let logger = ConversationLogger::new(&log_path, 50);
    let cycle = sample_cycle();

    // First write — creates the file
    logger.log_cycle(&cycle).await.unwrap();
    assert!(tokio::fs::metadata(&log_path).await.is_ok());

    // Second write — triggers rotation (file > 50 bytes)
    logger.log_cycle(&cycle).await.unwrap();
    assert!(tokio::fs::metadata(&backup_path).await.is_ok());

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn async_write_completes() {
    let dir = std::env::temp_dir().join(format!("conv_log_write_{}", std::process::id()));
    let _ = tokio::fs::create_dir_all(&dir).await;
    let log_path = dir.join("write_test.log");
    let _ = tokio::fs::remove_file(&log_path).await;

    let logger = ConversationLogger::new(&log_path, 10_485_760);
    logger.log_cycle(&sample_cycle()).await.unwrap();

    let content = tokio::fs::read_to_string(&log_path).await.unwrap();
    assert!(content.contains("Request #42"));
    assert!(content.contains(">>> FROM CLAUDE CODE"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[test]
fn request_counter_increments() {
    let dir = std::env::temp_dir().join("conv_log_counter");
    let logger = ConversationLogger::new(dir.join("counter.log"), 0);
    assert_eq!(logger.next_request_number(), 1);
    assert_eq!(logger.next_request_number(), 2);
    assert_eq!(logger.next_request_number(), 3);
}
