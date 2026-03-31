use std::collections::HashMap;

use copilot_adapter::anthropic::types::{
    ContentDelta, ResponseContentBlock, StreamEvent,
};
use copilot_adapter::copilot::types::{
    ChatCompletionChunk, ChunkChoice, ChunkDelta, StreamingFunctionCall, StreamingToolCall,
};
use copilot_adapter::streaming::state::StreamingState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a simple text chunk with optional finish_reason.
fn text_chunk(id: &str, model: &str, text: &str, finish_reason: Option<&str>) -> ChatCompletionChunk {
    ChatCompletionChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: model.to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: Some(text.to_string()),
                tool_calls: None,
            },
            finish_reason: finish_reason.map(|s| s.to_string()),
        }],
    }
}

/// Build a chunk that carries only a finish_reason (no content).
fn finish_chunk(id: &str, model: &str, reason: &str) -> ChatCompletionChunk {
    ChatCompletionChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: model.to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: None,
                tool_calls: None,
            },
            finish_reason: Some(reason.to_string()),
        }],
    }
}

/// Build a chunk with a tool call delta (first chunk: has id + name).
fn tool_call_start_chunk(
    id: &str,
    model: &str,
    tc_index: u32,
    call_id: &str,
    name: &str,
    args_fragment: &str,
) -> ChatCompletionChunk {
    ChatCompletionChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: model.to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: None,
                tool_calls: Some(vec![StreamingToolCall {
                    index: tc_index,
                    id: Some(call_id.to_string()),
                    call_type: Some("function".to_string()),
                    function: Some(StreamingFunctionCall {
                        name: Some(name.to_string()),
                        arguments: if args_fragment.is_empty() {
                            None
                        } else {
                            Some(args_fragment.to_string())
                        },
                    }),
                }]),
            },
            finish_reason: None,
        }],
    }
}

/// Build a chunk with tool call argument continuation (no id/name).
fn tool_call_args_chunk(
    id: &str,
    model: &str,
    tc_index: u32,
    args_fragment: &str,
) -> ChatCompletionChunk {
    ChatCompletionChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: model.to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: None,
                tool_calls: Some(vec![StreamingToolCall {
                    index: tc_index,
                    id: None,
                    call_type: None,
                    function: Some(StreamingFunctionCall {
                        name: None,
                        arguments: Some(args_fragment.to_string()),
                    }),
                }]),
            },
            finish_reason: None,
        }],
    }
}

/// Assert a StreamEvent is MessageStart and return its message id.
fn assert_message_start(event: &StreamEvent) -> String {
    match event {
        StreamEvent::MessageStart { message } => {
            assert_eq!(message.response_type, "message");
            assert_eq!(message.role, "assistant");
            message.id.clone()
        }
        other => panic!("Expected MessageStart, got: {:?}", other),
    }
}

/// Assert a StreamEvent is ContentBlockStart with a text block.
fn assert_text_block_start(event: &StreamEvent, expected_index: u32) {
    match event {
        StreamEvent::ContentBlockStart {
            index,
            content_block,
        } => {
            assert_eq!(*index, expected_index);
            assert_eq!(content_block.block_type(), "text");
            assert_eq!(content_block.text_content(), "");
        }
        other => panic!("Expected ContentBlockStart (text), got: {:?}", other),
    }
}

/// Assert a StreamEvent is ContentBlockStart with a tool_use block.
fn assert_tool_use_block_start(
    event: &StreamEvent,
    expected_index: u32,
    expected_id: &str,
    expected_name: &str,
) {
    match event {
        StreamEvent::ContentBlockStart {
            index,
            content_block,
        } => {
            assert_eq!(*index, expected_index);
            match content_block {
                ResponseContentBlock::ToolUse { id, name, .. } => {
                    assert_eq!(id, expected_id);
                    assert_eq!(name, expected_name);
                }
                other => panic!("Expected ToolUse block, got: {:?}", other),
            }
        }
        other => panic!("Expected ContentBlockStart (tool_use), got: {:?}", other),
    }
}

/// Assert a StreamEvent is ContentBlockDelta with text.
fn assert_text_delta(event: &StreamEvent, expected_index: u32, expected_text: &str) {
    match event {
        StreamEvent::ContentBlockDelta { index, delta } => {
            assert_eq!(*index, expected_index);
            match delta {
                ContentDelta::Text(td) => {
                    assert_eq!(td.delta_type, "text_delta");
                    assert_eq!(td.text, expected_text);
                }
                other => panic!("Expected Text delta, got: {:?}", other),
            }
        }
        other => panic!("Expected ContentBlockDelta (text), got: {:?}", other),
    }
}

/// Assert a StreamEvent is ContentBlockDelta with input_json.
fn assert_input_json_delta(event: &StreamEvent, expected_index: u32, expected_json: &str) {
    match event {
        StreamEvent::ContentBlockDelta { index, delta } => {
            assert_eq!(*index, expected_index);
            match delta {
                ContentDelta::InputJson(ijd) => {
                    assert_eq!(ijd.delta_type, "input_json_delta");
                    assert_eq!(ijd.partial_json, expected_json);
                }
                other => panic!("Expected InputJson delta, got: {:?}", other),
            }
        }
        other => panic!("Expected ContentBlockDelta (input_json), got: {:?}", other),
    }
}

/// Assert a StreamEvent is ContentBlockStop.
fn assert_block_stop(event: &StreamEvent, expected_index: u32) {
    match event {
        StreamEvent::ContentBlockStop { index } => {
            assert_eq!(*index, expected_index);
        }
        other => panic!("Expected ContentBlockStop, got: {:?}", other),
    }
}

/// Assert a StreamEvent is MessageDelta with a given stop_reason.
fn assert_message_delta(event: &StreamEvent, expected_stop_reason: &str) {
    match event {
        StreamEvent::MessageDelta { delta, usage } => {
            assert_eq!(delta.stop_reason.as_deref(), Some(expected_stop_reason));
            assert!(delta.stop_sequence.is_none());
            assert_eq!(usage.output_tokens, 0);
        }
        other => panic!("Expected MessageDelta, got: {:?}", other),
    }
}

/// Assert a StreamEvent is MessageStop.
fn assert_message_stop(event: &StreamEvent) {
    match event {
        StreamEvent::MessageStop {} => {}
        other => panic!("Expected MessageStop, got: {:?}", other),
    }
}

// ===========================================================================
// Test: text-only streaming
// ===========================================================================

#[test]
fn text_only_streaming() {
    let mut state = StreamingState::new(HashMap::new());

    // First chunk: triggers message_start + content_block_start + text delta
    let events = state.process_chunk(&text_chunk("chatcmpl-abc", "claude-sonnet-4", "Hello", None));
    assert_eq!(events.len(), 3);
    let msg_id = assert_message_start(&events[0]);
    assert!(msg_id.starts_with("msg_"));
    assert_text_block_start(&events[1], 0);
    assert_text_delta(&events[2], 0, "Hello");

    // Second chunk: only text delta (no new block)
    let events = state.process_chunk(&text_chunk("chatcmpl-abc", "claude-sonnet-4", " world", None));
    assert_eq!(events.len(), 1);
    assert_text_delta(&events[0], 0, " world");

    // Third chunk: text + finish_reason
    let events = state.process_chunk(&text_chunk("chatcmpl-abc", "claude-sonnet-4", "!", Some("stop")));
    assert_eq!(events.len(), 3);
    assert_text_delta(&events[0], 0, "!");
    // finish closes the block and emits message_delta
    assert_block_stop(&events[1], 0);
    assert_message_delta(&events[2], "end_turn");

    // Finalize: message_stop only (block already closed by finish)
    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

#[test]
fn text_streaming_finish_reason_length() {
    let mut state = StreamingState::new(HashMap::new());

    let events = state.process_chunk(&text_chunk("c1", "m1", "Hello", None));
    assert_eq!(events.len(), 3); // message_start, block_start, delta

    // Finish with length
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 2);
    assert_block_stop(&events[0], 0);
    assert_message_delta(&events[1], "max_tokens");
}

#[test]
fn empty_text_deltas_are_skipped() {
    let mut state = StreamingState::new(HashMap::new());

    // First chunk with text
    let events = state.process_chunk(&text_chunk("c1", "m1", "Hi", None));
    assert_eq!(events.len(), 3);

    // Empty text delta — should produce nothing
    let events = state.process_chunk(&text_chunk("c1", "m1", "", None));
    assert_eq!(events.len(), 0);
}

// ===========================================================================
// Test: tool call streaming
// ===========================================================================

#[test]
fn single_tool_call_streaming() {
    let mut state = StreamingState::new(HashMap::new());

    // First chunk: tool call start
    let events = state.process_chunk(&tool_call_start_chunk(
        "chatcmpl-xyz",
        "claude-sonnet-4",
        0,
        "call_abc123",
        "bash",
        "{\"co",
    ));
    assert_eq!(events.len(), 3);
    assert_message_start(&events[0]);
    assert_tool_use_block_start(&events[1], 0, "call_abc123", "bash");
    assert_input_json_delta(&events[2], 0, "{\"co");

    // Argument continuation
    let events = state.process_chunk(&tool_call_args_chunk(
        "chatcmpl-xyz",
        "claude-sonnet-4",
        0,
        "mmand\":\"ls",
    ));
    assert_eq!(events.len(), 1);
    assert_input_json_delta(&events[0], 0, "mmand\":\"ls");

    // More arguments
    let events = state.process_chunk(&tool_call_args_chunk(
        "chatcmpl-xyz",
        "claude-sonnet-4",
        0,
        "\"}",
    ));
    assert_eq!(events.len(), 1);
    assert_input_json_delta(&events[0], 0, "\"}");

    // Finish with tool_calls reason
    let events = state.process_chunk(&finish_chunk("chatcmpl-xyz", "claude-sonnet-4", "tool_calls"));
    assert_eq!(events.len(), 2);
    assert_block_stop(&events[0], 0);
    assert_message_delta(&events[1], "tool_use");

    // Finalize
    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

#[test]
fn tool_call_with_name_restoration() {
    // Simulate a truncated name being restored via name_mapping
    let mut mapping = HashMap::new();
    mapping.insert(
        "very_long_tool_na_abcd1234".to_string(),
        "very_long_tool_name_that_exceeds_64_characters_and_was_truncated".to_string(),
    );

    let mut state = StreamingState::new(mapping);

    let events = state.process_chunk(&tool_call_start_chunk(
        "c1",
        "m1",
        0,
        "call_1",
        "very_long_tool_na_abcd1234",
        "{}",
    ));
    assert_eq!(events.len(), 3);
    assert_message_start(&events[0]);
    // Name should be restored to original
    assert_tool_use_block_start(
        &events[1],
        0,
        "call_1",
        "very_long_tool_name_that_exceeds_64_characters_and_was_truncated",
    );
    assert_input_json_delta(&events[2], 0, "{}");
}

#[test]
fn tool_call_without_id_gets_synthetic_id() {
    let mut state = StreamingState::new(HashMap::new());

    // Tool call without an ID
    let chunk = ChatCompletionChunk {
        id: "c1".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: "m1".to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: None,
                tool_calls: Some(vec![StreamingToolCall {
                    index: 0,
                    id: None,
                    call_type: Some("function".to_string()),
                    function: Some(StreamingFunctionCall {
                        name: Some("test_tool".to_string()),
                        arguments: None,
                    }),
                }]),
            },
            finish_reason: None,
        }],
    };

    let events = state.process_chunk(&chunk);
    assert_eq!(events.len(), 2); // message_start + block_start (no args delta)
    // Should use synthetic "call_0"
    assert_tool_use_block_start(&events[1], 0, "call_0", "test_tool");
}

// ===========================================================================
// Test: mixed content streaming (text then tool)
// ===========================================================================

#[test]
fn mixed_text_then_tool_streaming() {
    let mut state = StreamingState::new(HashMap::new());

    // Start with text
    let events = state.process_chunk(&text_chunk("c1", "m1", "Let me run that", None));
    assert_eq!(events.len(), 3);
    assert_message_start(&events[0]);
    assert_text_block_start(&events[1], 0);
    assert_text_delta(&events[2], 0, "Let me run that");

    // More text
    let events = state.process_chunk(&text_chunk("c1", "m1", " for you.", None));
    assert_eq!(events.len(), 1);
    assert_text_delta(&events[0], 0, " for you.");

    // Transition to tool call — should close text block, increment index, open tool block
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "bash", "{\"command\"",
    ));
    assert_eq!(events.len(), 3);
    assert_block_stop(&events[0], 0);        // close text block at index 0
    assert_tool_use_block_start(&events[1], 1, "call_1", "bash"); // tool at index 1
    assert_input_json_delta(&events[2], 1, "{\"command\"");

    // Continue tool arguments
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, ":\"ls\"}"));
    assert_eq!(events.len(), 1);
    assert_input_json_delta(&events[0], 1, ":\"ls\"}");

    // Finish
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    assert_eq!(events.len(), 2);
    assert_block_stop(&events[0], 1);
    assert_message_delta(&events[1], "tool_use");

    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

// ===========================================================================
// Test: parallel tool calls
// ===========================================================================

#[test]
fn parallel_tool_calls_streaming() {
    let mut state = StreamingState::new(HashMap::new());

    // First tool call
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_a", "bash", "{\"",
    ));
    assert_eq!(events.len(), 3);
    assert_message_start(&events[0]);
    assert_tool_use_block_start(&events[1], 0, "call_a", "bash");
    assert_input_json_delta(&events[2], 0, "{\"");

    // Continue first tool args
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, "command\":\"ls\"}"));
    assert_eq!(events.len(), 1);
    assert_input_json_delta(&events[0], 0, "command\":\"ls\"}");

    // Second tool call starts (parallel) — should close first block, open second
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 1, "call_b", "read_file", "{\"",
    ));
    assert_eq!(events.len(), 3);
    assert_block_stop(&events[0], 0);         // close first tool block
    assert_tool_use_block_start(&events[1], 1, "call_b", "read_file"); // second tool at index 1
    assert_input_json_delta(&events[2], 1, "{\"");

    // Continue second tool args
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 1, "path\":\"README.md\"}"));
    assert_eq!(events.len(), 1);
    assert_input_json_delta(&events[0], 1, "path\":\"README.md\"}");

    // Third tool call
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 2, "call_c", "write_file", "{}",
    ));
    assert_eq!(events.len(), 3);
    assert_block_stop(&events[0], 1);
    assert_tool_use_block_start(&events[1], 2, "call_c", "write_file");
    assert_input_json_delta(&events[2], 2, "{}");

    // Finish
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    assert_eq!(events.len(), 2);
    assert_block_stop(&events[0], 2);
    assert_message_delta(&events[1], "tool_use");

    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

// ===========================================================================
// Test: content block transitions
// ===========================================================================

#[test]
fn block_transitions_emit_stop_then_start() {
    let mut state = StreamingState::new(HashMap::new());

    // Start with text
    let events = state.process_chunk(&text_chunk("c1", "m1", "thinking...", None));
    assert_eq!(events.len(), 3);
    assert_text_block_start(&events[1], 0);

    // Transition to tool — verify stop(0) then start(1)
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "bash", "",
    ));
    // stop(0) + start(1) (no args delta since args is empty)
    assert_eq!(events.len(), 2);
    assert_block_stop(&events[0], 0);
    assert_tool_use_block_start(&events[1], 1, "call_1", "bash");
}

#[test]
fn tool_to_tool_transition() {
    let mut state = StreamingState::new(HashMap::new());

    // First tool
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_a", "tool_a", "{}",
    ));
    assert_eq!(events.len(), 3);
    assert_tool_use_block_start(&events[1], 0, "call_a", "tool_a");

    // Second tool — should close first, open second
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 1, "call_b", "tool_b", "{}",
    ));
    assert_eq!(events.len(), 3);
    assert_block_stop(&events[0], 0);
    assert_tool_use_block_start(&events[1], 1, "call_b", "tool_b");
    assert_input_json_delta(&events[2], 1, "{}");
}

#[test]
fn finalize_closes_open_text_block() {
    let mut state = StreamingState::new(HashMap::new());

    // Send text without finish
    let events = state.process_chunk(&text_chunk("c1", "m1", "Hello", None));
    assert_eq!(events.len(), 3);

    // Finalize should close the open text block
    let events = state.finalize();
    assert_eq!(events.len(), 2);
    assert_block_stop(&events[0], 0);
    assert_message_stop(&events[1]);
}

#[test]
fn finalize_closes_open_tool_block() {
    let mut state = StreamingState::new(HashMap::new());

    // Send tool call without finish
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "bash", "{\"cmd\":\"ls\"}",
    ));
    assert_eq!(events.len(), 3);

    // Finalize should close the open tool block
    let events = state.finalize();
    assert_eq!(events.len(), 2);
    assert_block_stop(&events[0], 0);
    assert_message_stop(&events[1]);
}

#[test]
fn finalize_after_finish_emits_only_message_stop() {
    let mut state = StreamingState::new(HashMap::new());

    let events = state.process_chunk(&text_chunk("c1", "m1", "Hi", Some("stop")));
    assert_eq!(events.len(), 5); // msg_start, block_start, delta, block_stop, msg_delta

    // Block was already closed by finish, so finalize just emits message_stop
    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

// ===========================================================================
// Test: message_start metadata
// ===========================================================================

#[test]
fn message_start_strips_chatcmpl_prefix() {
    let mut state = StreamingState::new(HashMap::new());

    let events = state.process_chunk(&text_chunk("chatcmpl-abc123", "claude-sonnet-4", "Hi", None));
    let msg_id = assert_message_start(&events[0]);
    assert_eq!(msg_id, "msg_abc123");
}

#[test]
fn message_start_preserves_ids_without_prefix() {
    let mut state = StreamingState::new(HashMap::new());

    let events = state.process_chunk(&text_chunk("custom-id", "gpt-4", "Hi", None));
    let msg_id = assert_message_start(&events[0]);
    assert_eq!(msg_id, "msg_custom-id");
}

#[test]
fn message_start_emitted_only_once() {
    let mut state = StreamingState::new(HashMap::new());

    let events1 = state.process_chunk(&text_chunk("c1", "m1", "A", None));
    let events2 = state.process_chunk(&text_chunk("c1", "m1", "B", None));

    // First chunk should have message_start
    assert!(matches!(&events1[0], StreamEvent::MessageStart { .. }));
    // Second chunk should NOT have message_start
    assert!(events2.iter().all(|e| !matches!(e, StreamEvent::MessageStart { .. })));
}

// ===========================================================================
// Test: finish_reason mapping
// ===========================================================================

#[test]
fn finish_reason_stop_maps_to_end_turn() {
    let mut state = StreamingState::new(HashMap::new());
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "stop"));
    assert_message_delta(&events.last().unwrap(), "end_turn");
}

#[test]
fn finish_reason_length_maps_to_max_tokens() {
    let mut state = StreamingState::new(HashMap::new());
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_message_delta(&events.last().unwrap(), "max_tokens");
}

#[test]
fn finish_reason_tool_calls_maps_to_tool_use() {
    let mut state = StreamingState::new(HashMap::new());
    state.process_chunk(&tool_call_start_chunk("c1", "m1", 0, "call_1", "t", "{}"));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    assert_message_delta(&events.last().unwrap(), "tool_use");
}

#[test]
fn finish_reason_unknown_passed_through() {
    let mut state = StreamingState::new(HashMap::new());
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "content_filter"));
    assert_message_delta(&events.last().unwrap(), "content_filter");
}

// ===========================================================================
// Test: edge cases
// ===========================================================================

#[test]
fn finalize_on_unstarted_state_emits_nothing() {
    let mut state = StreamingState::new(HashMap::new());
    // No chunks processed — finalize should return empty vec to avoid
    // a malformed stream with a lone message_stop.
    let events = state.finalize();
    assert!(events.is_empty());
}

#[test]
fn chunk_with_empty_choices_is_noop() {
    let mut state = StreamingState::new(HashMap::new());

    // First chunk to initialise message_start
    let events = state.process_chunk(&text_chunk("c1", "m1", "Hi", None));
    assert_eq!(events.len(), 3);

    // Chunk with no choices — should only be a no-op after message_start
    let empty_choices_chunk = ChatCompletionChunk {
        id: "c1".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: "m1".to_string(),
        choices: vec![],
    };
    let events = state.process_chunk(&empty_choices_chunk);
    assert!(events.is_empty());
}

#[test]
fn role_announcement_chunk_is_noop() {
    let mut state = StreamingState::new(HashMap::new());

    // OpenAI's first chunk often carries only `role: "assistant"` with no
    // content, tool_calls, or finish_reason. After emitting message_start
    // it should produce no further events.
    let role_chunk = ChatCompletionChunk {
        id: "c1".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: "m1".to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: Some("assistant".to_string()),
                content: None,
                tool_calls: None,
            },
            finish_reason: None,
        }],
    };
    let events = state.process_chunk(&role_chunk);
    // Should emit exactly message_start and nothing else
    assert_eq!(events.len(), 1);
    assert_message_start(&events[0]);

    // A subsequent text chunk should NOT re-emit message_start
    let events = state.process_chunk(&text_chunk("c1", "m1", "Hello", None));
    assert_eq!(events.len(), 2); // block_start + text_delta only
    assert_text_block_start(&events[0], 0);
    assert_text_delta(&events[1], 0, "Hello");
}

// ===========================================================================
// Test: serialization round-trip
// ===========================================================================

#[test]
fn events_serialize_to_valid_json() {
    let mut state = StreamingState::new(HashMap::new());

    // Process a mixed stream
    let events1 = state.process_chunk(&text_chunk("c1", "claude-sonnet-4", "Hello", None));
    let events2 = state.process_chunk(&tool_call_start_chunk(
        "c1", "claude-sonnet-4", 0, "call_1", "bash", "{\"cmd\":\"ls\"}",
    ));
    let events3 = state.process_chunk(&finish_chunk("c1", "claude-sonnet-4", "tool_calls"));
    let events4 = state.finalize();

    // All events must serialize to valid JSON
    for events in [&events1, &events2, &events3, &events4] {
        for event in events {
            let json = serde_json::to_string(event)
                .unwrap_or_else(|e| panic!("Failed to serialize event {:?}: {}", event, e));
            assert!(!json.is_empty());

            // Verify it round-trips
            let _: serde_json::Value = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("Failed to parse JSON '{}': {}", json, e));
        }
    }
}
