use std::collections::HashMap;

use copilot_adapter::anthropic::types::{ContentDelta, ResponseContentBlock, StreamEvent};
use copilot_adapter::copilot::types::{
    ChatCompletionChunk, ChunkChoice, ChunkDelta, StreamingFunctionCall, StreamingToolCall,
};
use copilot_adapter::streaming::state::StreamingState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a simple text chunk with optional finish_reason.
fn text_chunk(
    id: &str,
    model: &str,
    text: &str,
    finish_reason: Option<&str>,
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
    let events = state.process_chunk(&text_chunk(
        "chatcmpl-abc",
        "claude-sonnet-4",
        "Hello",
        None,
    ));
    assert_eq!(events.len(), 3);
    let msg_id = assert_message_start(&events[0]);
    assert!(msg_id.starts_with("msg_"));
    assert_text_block_start(&events[1], 0);
    assert_text_delta(&events[2], 0, "Hello");

    // Second chunk: only text delta (no new block)
    let events = state.process_chunk(&text_chunk(
        "chatcmpl-abc",
        "claude-sonnet-4",
        " world",
        None,
    ));
    assert_eq!(events.len(), 1);
    assert_text_delta(&events[0], 0, " world");

    // Third chunk: text + finish_reason
    let events = state.process_chunk(&text_chunk(
        "chatcmpl-abc",
        "claude-sonnet-4",
        "!",
        Some("stop"),
    ));
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

    // First chunk: tool call start — tool_use events are buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "chatcmpl-xyz",
        "claude-sonnet-4",
        0,
        "call_abc123",
        "bash",
        "{\"co",
    ));
    assert_eq!(events.len(), 1); // only message_start (tool_use buffered)
    assert_message_start(&events[0]);

    // Argument continuation — buffered
    let events = state.process_chunk(&tool_call_args_chunk(
        "chatcmpl-xyz",
        "claude-sonnet-4",
        0,
        "mmand\":\"ls",
    ));
    assert_eq!(events.len(), 0);

    // More arguments — buffered
    let events = state.process_chunk(&tool_call_args_chunk(
        "chatcmpl-xyz",
        "claude-sonnet-4",
        0,
        "\"}",
    ));
    assert_eq!(events.len(), 0);

    // Finish with tool_calls reason — buffer flushed
    let events = state.process_chunk(&finish_chunk(
        "chatcmpl-xyz",
        "claude-sonnet-4",
        "tool_calls",
    ));
    // Flushed: block_start + 3 deltas, then block_stop + message_delta
    assert_eq!(events.len(), 6);
    assert_tool_use_block_start(&events[0], 0, "call_abc123", "bash");
    assert_input_json_delta(&events[1], 0, "{\"co");
    assert_input_json_delta(&events[2], 0, "mmand\":\"ls");
    assert_input_json_delta(&events[3], 0, "\"}");
    assert_block_stop(&events[4], 0);
    assert_message_delta(&events[5], "tool_use");

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
    assert_eq!(events.len(), 1); // message_start (tool_use buffered)
    assert_message_start(&events[0]);

    // Finish to flush buffer
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    // Flushed: block_start + delta, then block_stop + message_delta
    assert_eq!(events.len(), 4);
    // Name should be restored to original
    assert_tool_use_block_start(
        &events[0],
        0,
        "call_1",
        "very_long_tool_name_that_exceeds_64_characters_and_was_truncated",
    );
    assert_input_json_delta(&events[1], 0, "{}");
    assert_block_stop(&events[2], 0);
    assert_message_delta(&events[3], "tool_use");
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
    assert_eq!(events.len(), 1); // message_start only (tool_use buffered)
    assert_message_start(&events[0]);

    // Finish to flush buffer
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    // Flushed: block_start, then block_stop + message_delta
    assert_eq!(events.len(), 3);
    // Should use synthetic "call_0"
    assert_tool_use_block_start(&events[0], 0, "call_0", "test_tool");
    assert_block_stop(&events[1], 0);
    assert_message_delta(&events[2], "tool_use");
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

    // Transition to tool call — should close text block, tool events buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1",
        "m1",
        0,
        "call_1",
        "bash",
        "{\"command\"",
    ));
    assert_eq!(events.len(), 1);
    assert_block_stop(&events[0], 0); // close text block at index 0
    // tool_use start + delta are buffered

    // Continue tool arguments — buffered
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, ":\"ls\"}"));
    assert_eq!(events.len(), 0);

    // Finish — buffer flushed
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    // Flushed: tool block_start + 2 deltas, then block_stop + message_delta
    assert_eq!(events.len(), 5);
    assert_tool_use_block_start(&events[0], 1, "call_1", "bash"); // tool at index 1
    assert_input_json_delta(&events[1], 1, "{\"command\"");
    assert_input_json_delta(&events[2], 1, ":\"ls\"}");
    assert_block_stop(&events[3], 1);
    assert_message_delta(&events[4], "tool_use");

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

    // First tool call — buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_a", "bash", "{\"",
    ));
    assert_eq!(events.len(), 1); // message_start
    assert_message_start(&events[0]);

    // Continue first tool args — buffered
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, "command\":\"ls\"}"));
    assert_eq!(events.len(), 0);

    // Second tool call starts (parallel) — flushes first tool, closes its block
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1",
        "m1",
        1,
        "call_b",
        "read_file",
        "{\"",
    ));
    // Flushed: first tool's start + 2 deltas, then block_stop(0)
    assert_eq!(events.len(), 4);
    assert_tool_use_block_start(&events[0], 0, "call_a", "bash");
    assert_input_json_delta(&events[1], 0, "{\"");
    assert_input_json_delta(&events[2], 0, "command\":\"ls\"}");
    assert_block_stop(&events[3], 0); // close first tool block

    // Continue second tool args — buffered
    let events = state.process_chunk(&tool_call_args_chunk(
        "c1",
        "m1",
        1,
        "path\":\"README.md\"}",
    ));
    assert_eq!(events.len(), 0);

    // Third tool call — flushes second tool
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1",
        "m1",
        2,
        "call_c",
        "write_file",
        "{}",
    ));
    assert_eq!(events.len(), 4);
    assert_tool_use_block_start(&events[0], 1, "call_b", "read_file");
    assert_input_json_delta(&events[1], 1, "{\"");
    assert_input_json_delta(&events[2], 1, "path\":\"README.md\"}");
    assert_block_stop(&events[3], 1);

    // Finish — flushes third tool
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    assert_eq!(events.len(), 4);
    assert_tool_use_block_start(&events[0], 2, "call_c", "write_file");
    assert_input_json_delta(&events[1], 2, "{}");
    assert_block_stop(&events[2], 2);
    assert_message_delta(&events[3], "tool_use");

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

    // Transition to tool — text block closed, tool events buffered
    let events = state.process_chunk(&tool_call_start_chunk("c1", "m1", 0, "call_1", "bash", ""));
    // stop(0) only (tool_use start is buffered, no args delta since empty)
    assert_eq!(events.len(), 1);
    assert_block_stop(&events[0], 0);
}

#[test]
fn tool_to_tool_transition() {
    let mut state = StreamingState::new(HashMap::new());

    // First tool — buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_a", "tool_a", "{}",
    ));
    assert_eq!(events.len(), 1); // message_start
    assert_message_start(&events[0]);

    // Second tool — flushes first, starts buffering second
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 1, "call_b", "tool_b", "{}",
    ));
    // Flushed: first tool start + delta, then block_stop(0)
    assert_eq!(events.len(), 3);
    assert_tool_use_block_start(&events[0], 0, "call_a", "tool_a");
    assert_input_json_delta(&events[1], 0, "{}");
    assert_block_stop(&events[2], 0);
}

#[test]
fn finalize_closes_open_text_block() {
    let mut state = StreamingState::new(HashMap::new());

    // Send text without finish
    let events = state.process_chunk(&text_chunk("c1", "m1", "Hello", None));
    assert_eq!(events.len(), 3);

    // Finalize should close the open text block, emit message_delta, then message_stop
    let events = state.finalize();
    assert_eq!(events.len(), 3);
    assert_block_stop(&events[0], 0);
    assert_message_delta(&events[1], "end_turn");
    assert_message_stop(&events[2]);
}

#[test]
fn finalize_closes_open_tool_block() {
    let mut state = StreamingState::new(HashMap::new());

    // Send tool call without finish — events buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1",
        "m1",
        0,
        "call_1",
        "bash",
        "{\"cmd\":\"ls\"}",
    ));
    assert_eq!(events.len(), 1); // message_start only

    // Finalize should flush buffer, close the open tool block, emit message_delta, then message_stop
    let events = state.finalize();
    assert_eq!(events.len(), 5);
    assert_tool_use_block_start(&events[0], 0, "call_1", "bash");
    assert_input_json_delta(&events[1], 0, "{\"cmd\":\"ls\"}");
    assert_block_stop(&events[2], 0);
    assert_message_delta(&events[3], "end_turn");
    assert_message_stop(&events[4]);
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

    let events = state.process_chunk(&text_chunk(
        "chatcmpl-abc123",
        "claude-sonnet-4",
        "Hi",
        None,
    ));
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
    assert!(events2
        .iter()
        .all(|e| !matches!(e, StreamEvent::MessageStart { .. })));
}

// ===========================================================================
// Test: finish_reason mapping
// ===========================================================================

#[test]
fn finish_reason_stop_maps_to_end_turn() {
    let mut state = StreamingState::new(HashMap::new());
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "stop"));
    assert_message_delta(events.last().unwrap(), "end_turn");
}

#[test]
fn finish_reason_length_maps_to_max_tokens() {
    let mut state = StreamingState::new(HashMap::new());
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_message_delta(events.last().unwrap(), "max_tokens");
}

#[test]
fn finish_reason_tool_calls_maps_to_tool_use() {
    let mut state = StreamingState::new(HashMap::new());
    state.process_chunk(&tool_call_start_chunk("c1", "m1", 0, "call_1", "t", "{}"));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    assert_message_delta(events.last().unwrap(), "tool_use");
}

#[test]
fn finish_reason_unknown_passed_through() {
    let mut state = StreamingState::new(HashMap::new());
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "content_filter"));
    assert_message_delta(events.last().unwrap(), "content_filter");
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
        "c1",
        "claude-sonnet-4",
        0,
        "call_1",
        "bash",
        "{\"cmd\":\"ls\"}",
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

// ===========================================================================
// Test: truncated tool calls (finish_reason="length" during tool_use)
// ===========================================================================

/// When a single tool call is truncated by finish_reason="length",
/// the tool_use block should be completely dropped and replaced with a
/// text block containing a truncation notice, followed by
/// message_delta("max_tokens").
#[test]
fn tool_call_truncated_by_length() {
    let mut state = StreamingState::new(HashMap::new());

    // Tool call starts — events are buffered, only message_start returned
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_abc", "Write", "{\"file_path",
    ));
    assert_eq!(events.len(), 1); // only message_start (tool_use is buffered)
    assert_message_start(&events[0]);

    // More arguments — still buffered
    let events = state.process_chunk(&tool_call_args_chunk(
        "c1", "m1", 0, "\": \"test.md\"",
    ));
    assert_eq!(events.len(), 0); // all buffered

    // Truncated by length — tool_use block discarded, text notice emitted
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4); // text_start + text_delta + text_stop + message_delta
    assert_text_block_start(&events[0], 0);
    assert_text_delta(
        &events[1],
        0,
        "[Tool call to \"Write\" was truncated due to output token limit]",
    );
    assert_block_stop(&events[2], 0);
    assert_message_delta(&events[3], "max_tokens");

    // Verify truncation was tracked
    assert!(state.truncated_openai_tool_indices().contains(&0));

    // Finalize — just message_stop
    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

/// Text block followed by a tool call that gets truncated by length.
/// The text block should be emitted normally; the tool_use block dropped
/// and replaced with a truncation notice text block.
#[test]
fn text_then_tool_truncated_by_length() {
    let mut state = StreamingState::new(HashMap::new());

    // Text streams normally
    let events = state.process_chunk(&text_chunk("c1", "m1", "Let me write that.", None));
    assert_eq!(events.len(), 3); // message_start + block_start + text_delta
    assert_message_start(&events[0]);
    assert_text_block_start(&events[1], 0);
    assert_text_delta(&events[2], 0, "Let me write that.");

    // Tool call starts — previous text block is closed, tool is buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "Write", "{\"file",
    ));
    // text block stop(0) is emitted (closes completed text);
    // tool_use start + delta are buffered
    assert_eq!(events.len(), 1);
    assert_block_stop(&events[0], 0); // close text block

    // More tool args — buffered
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, "_path\": \"x\"}"));
    assert_eq!(events.len(), 0);

    // Truncated by length — tool_use block dropped, truncation notice emitted
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4); // text_start + text_delta + text_stop + message_delta
    assert_text_block_start(&events[0], 1); // index 1 (after the original text block at 0)
    assert_text_delta(
        &events[1],
        1,
        "[Tool call to \"Write\" was truncated due to output token limit]",
    );
    assert_block_stop(&events[2], 1);
    assert_message_delta(&events[3], "max_tokens");

    // Verify truncation tracked
    assert!(state.truncated_openai_tool_indices().contains(&0));

    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

/// Two parallel tool calls: first completes, second is truncated.
/// First tool_use should be fully emitted; second dropped with truncation notice.
#[test]
fn first_tool_complete_second_truncated() {
    let mut state = StreamingState::new(HashMap::new());

    // First tool call
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_a", "Read", "{\"path\":\"a.rs\"}",
    ));
    assert_eq!(events.len(), 1); // message_start only (tool buffered)
    assert_message_start(&events[0]);

    // Second tool call starts — flushes first tool (complete)
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 1, "call_b", "Write", "{\"file",
    ));
    // Flushed from buffer: first tool's start + delta
    // Then: block_stop(0) for first tool
    // Second tool's start + delta are buffered
    assert_eq!(events.len(), 3);
    assert_tool_use_block_start(&events[0], 0, "call_a", "Read");
    assert_input_json_delta(&events[1], 0, "{\"path\":\"a.rs\"}");
    assert_block_stop(&events[2], 0);

    // More args for second tool — buffered
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 1, "_path\": \"b\"}"));
    assert_eq!(events.len(), 0);

    // Truncated by length — second tool dropped, truncation notice emitted
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4); // text_start + text_delta + text_stop + message_delta
    assert_text_block_start(&events[0], 1); // index 1 (first tool was at 0)
    assert_text_delta(
        &events[1],
        1,
        "[Tool call to \"Write\" was truncated due to output token limit]",
    );
    assert_block_stop(&events[2], 1);
    assert_message_delta(&events[3], "max_tokens");

    // Only second tool was truncated
    assert!(!state.truncated_openai_tool_indices().contains(&0));
    assert!(state.truncated_openai_tool_indices().contains(&1));

    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

/// Even if the tool call has complete-looking JSON, finish_reason="length"
/// always causes the tool_use block to be dropped with a truncation notice.
#[test]
fn tool_call_with_length_finish_but_complete_json() {
    let mut state = StreamingState::new(HashMap::new());

    // Tool call with seemingly complete JSON
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "Bash",
        "{\"command\": \"ls\"}",
    ));
    assert_eq!(events.len(), 1); // message_start
    assert_message_start(&events[0]);

    // Finish with "length" — tool is STILL dropped (always-drop policy) + notice
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4); // text_start + text_delta + text_stop + message_delta
    assert_text_block_start(&events[0], 0);
    assert_text_delta(
        &events[1],
        0,
        "[Tool call to \"Bash\" was truncated due to output token limit]",
    );
    assert_block_stop(&events[2], 0);
    assert_message_delta(&events[3], "max_tokens");

    assert!(state.truncated_openai_tool_indices().contains(&0));

    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

/// Single tool call truncated by length — verifies the truncation notice
/// includes the actual tool name. This is a simpler variant of
/// `tool_call_truncated_by_length` (no intermediate arg chunks).
///
/// Note: the "unknown" fallback path (where `tool_call_names` has no entry
/// for the index) is defensive-only and not reachable via the public API
/// because `is_new_call` requires a function name, which always populates
/// `tool_call_names`.
#[test]
fn tool_call_truncated_single_tool_name_in_notice() {
    let mut state = StreamingState::new(HashMap::new());

    // First, send a tool call start to establish ToolUse mode
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "Write", "{\"file",
    ));
    assert_eq!(events.len(), 1);
    assert_message_start(&events[0]);

    // Truncated by length — notice should include the tool name "Write"
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4);
    assert_text_delta(
        &events[1],
        0,
        "[Tool call to \"Write\" was truncated due to output token limit]",
    );
    assert_message_delta(&events[3], "max_tokens");
}

/// Truncation notice uses the restored (original) tool name, not the
/// truncated OpenAI name, when a name mapping is provided.
#[test]
fn truncated_tool_uses_restored_name_in_notice() {
    let mut name_mapping = HashMap::new();
    name_mapping.insert(
        "mcp__codemogger__code_08a3f".to_string(),
        "mcp__codemogger__codemogger_search".to_string(),
    );
    let mut state = StreamingState::new(name_mapping);

    // Tool call with a truncated name that should be restored
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1",
        "m1",
        0,
        "call_1",
        "mcp__codemogger__code_08a3f",
        "{\"query",
    ));
    assert_eq!(events.len(), 1);
    assert_message_start(&events[0]);

    // Truncated by length — notice should use the RESTORED name
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4);
    assert_text_delta(
        &events[1],
        0,
        "[Tool call to \"mcp__codemogger__codemogger_search\" was truncated due to output token limit]",
    );
    assert_message_delta(&events[3], "max_tokens");
}

/// finish_reason="length" during a text block (not tool_use) should NOT
/// emit a truncation notice — only tool_use blocks get special treatment.
#[test]
fn length_finish_during_text_block_no_notice() {
    let mut state = StreamingState::new(HashMap::new());

    // Text block with finish_reason="length" on the same chunk
    let events = state.process_chunk(&text_chunk("c1", "m1", "Hello world", Some("length")));
    // message_start + text_start + text_delta + text_stop + message_delta
    assert_eq!(events.len(), 5);
    assert_message_start(&events[0]);
    assert_text_block_start(&events[1], 0);
    assert_text_delta(&events[2], 0, "Hello world");
    assert_block_stop(&events[3], 0);
    assert_message_delta(&events[4], "max_tokens");

    // No truncation recorded
    assert!(state.truncated_openai_tool_indices().is_empty());
}

/// Tool calls that finish normally (finish_reason="tool_calls") should
/// still be emitted correctly — the buffering must not break normal flow.
#[test]
fn tool_call_normal_finish_with_buffering() {
    let mut state = StreamingState::new(HashMap::new());

    // Tool call starts — buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "Bash", "{\"co",
    ));
    assert_eq!(events.len(), 1); // message_start
    assert_message_start(&events[0]);

    // More args — buffered
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, "mmand\":\"ls\"}"));
    assert_eq!(events.len(), 0);

    // Normal finish — buffer flushed, block closed
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    // Flushed: tool_use start + 2 deltas
    // Then: block_stop + message_delta
    assert_eq!(events.len(), 5);
    assert_tool_use_block_start(&events[0], 0, "call_1", "Bash");
    assert_input_json_delta(&events[1], 0, "{\"co");
    assert_input_json_delta(&events[2], 0, "mmand\":\"ls\"}");
    assert_block_stop(&events[3], 0);
    assert_message_delta(&events[4], "tool_use");

    assert!(state.truncated_openai_tool_indices().is_empty());

    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}
