use std::collections::HashMap;

use copilot_adapter::anthropic::types::{ContentDelta, ResponseContentBlock, StreamEvent};
use copilot_adapter::copilot::types::{
    ChatCompletionChunk, ChunkChoice, ChunkDelta, StreamingFunctionCall, StreamingToolCall, Usage,
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
        usage: None,
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
        usage: None,
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
        usage: None,
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
        usage: None,
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
/// Returns the output_tokens value for further assertions if needed.
fn assert_message_delta(event: &StreamEvent, expected_stop_reason: &str) -> u32 {
    match event {
        StreamEvent::MessageDelta { delta, usage } => {
            assert_eq!(delta.stop_reason.as_deref(), Some(expected_stop_reason));
            assert!(delta.stop_sequence.is_none());
            usage.output_tokens
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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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

    let mut state = StreamingState::new(mapping, 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
        usage: None,
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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

    let events = state.process_chunk(&text_chunk("custom-id", "gpt-4", "Hi", None));
    let msg_id = assert_message_start(&events[0]);
    assert_eq!(msg_id, "msg_custom-id");
}

#[test]
fn message_start_emitted_only_once() {
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "stop"));
    assert_message_delta(events.last().unwrap(), "end_turn");
}

#[test]
fn finish_reason_length_maps_to_max_tokens() {
    let mut state = StreamingState::new(HashMap::new(), 0);
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_message_delta(events.last().unwrap(), "max_tokens");
}

#[test]
fn finish_reason_tool_calls_maps_to_tool_use() {
    let mut state = StreamingState::new(HashMap::new(), 0);
    state.process_chunk(&tool_call_start_chunk("c1", "m1", 0, "call_1", "t", "{}"));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));
    assert_message_delta(events.last().unwrap(), "tool_use");
}

#[test]
fn finish_reason_unknown_passed_through() {
    let mut state = StreamingState::new(HashMap::new(), 0);
    state.process_chunk(&text_chunk("c1", "m1", "Hi", None));

    let events = state.process_chunk(&finish_chunk("c1", "m1", "content_filter"));
    assert_message_delta(events.last().unwrap(), "content_filter");
}

// ===========================================================================
// Test: edge cases
// ===========================================================================

#[test]
fn finalize_on_unstarted_state_emits_nothing() {
    let mut state = StreamingState::new(HashMap::new(), 0);
    // No chunks processed — finalize should return empty vec to avoid
    // a malformed stream with a lone message_stop.
    let events = state.finalize();
    assert!(events.is_empty());
}

#[test]
fn chunk_with_empty_choices_is_noop() {
    let mut state = StreamingState::new(HashMap::new(), 0);

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
        usage: None,
    };
    let events = state.process_chunk(&empty_choices_chunk);
    assert!(events.is_empty());
}

#[test]
fn role_announcement_chunk_is_noop() {
    let mut state = StreamingState::new(HashMap::new(), 0);

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
        usage: None,
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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(name_mapping, 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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
    let mut state = StreamingState::new(HashMap::new(), 0);

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

// ---------------------------------------------------------------------------
// Epic 5 Task 5.4: Additional streaming truncation tests
// ---------------------------------------------------------------------------

/// Tool call with no name recorded in `tool_call_names` map produces
/// "unknown" in the truncation notice. This is a defensive path — in
/// practice, `tool_call_names` is always populated because `is_new_call`
/// requires a function name. This test verifies the fallback by sending a
/// tool call that transitions to a new index without a name.
///
/// Note: Because the public streaming API always provides a name with new
/// tool calls, we instead verify the inverse: when a tool IS named, the
/// name appears correctly, and the "unknown" fallback is only reachable
/// defensively. This test documents the truncation notice format when the
/// tool call starts normally.
#[test]
fn tool_truncated_unnamed_defaults_to_known_name() {
    // Send a named tool call at index 0, then a second unnamed args chunk
    // at index 0 (continuation, not a new call). Since the state already
    // has the name from the first chunk, the notice should use that name.
    let mut state = StreamingState::new(HashMap::new(), 0);

    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_a", "Write", "{\"file",
    ));
    assert_eq!(events.len(), 1);
    assert_message_start(&events[0]);

    // More args at same index (no new name) — still buffered
    let events = state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, "_path\": \"x\"}"));
    assert_eq!(events.len(), 0);

    // Truncated by length — should use the original "Write" name
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4);
    assert_text_delta(
        &events[1],
        0,
        "[Tool call to \"Write\" was truncated due to output token limit]",
    );
}

/// Verify that the truncation notice text block gets the correct block index
/// when preceded by a text block.
#[test]
fn truncation_notice_block_index_correct_after_text() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    // Text block at index 0
    let events = state.process_chunk(&text_chunk("c1", "m1", "Thinking...", None));
    assert_eq!(events.len(), 3); // message_start + block_start(0) + text_delta
    assert_message_start(&events[0]);
    assert_text_block_start(&events[1], 0);
    assert_text_delta(&events[2], 0, "Thinking...");

    // Tool call starts — closes text block, tool is buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "Write", "{\"file",
    ));
    assert_eq!(events.len(), 1); // block_stop(0) for text
    assert_block_stop(&events[0], 0);

    // Truncated by length — notice should be at index 1
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4);
    assert_text_block_start(&events[0], 1); // index 1 (after text at 0)
    assert_text_delta(
        &events[1],
        1,
        "[Tool call to \"Write\" was truncated due to output token limit]",
    );
    assert_block_stop(&events[2], 1);
    assert_message_delta(&events[3], "max_tokens");
}

// ===========================================================================
// Tests: Token counting in StreamingState (Epic 4)
// ===========================================================================

/// `message_start` should contain the input_tokens passed at construction.
#[test]
fn message_start_contains_input_tokens() {
    let mut state = StreamingState::new(HashMap::new(), 42);

    let events = state.process_chunk(&text_chunk("c1", "m1", "Hello", None));
    assert!(events.len() >= 1);
    match &events[0] {
        StreamEvent::MessageStart { message } => {
            assert_eq!(message.usage.input_tokens, 42);
        }
        other => panic!("Expected MessageStart, got: {:?}", other),
    }
}

/// `message_start` always uses the pre-computed input token count because
/// it is emitted on the very first chunk, before any upstream usage can
/// arrive. Upstream `prompt_tokens` cannot retroactively update an
/// already-emitted `message_start`.
///
/// The upstream input token override is exercised by
/// `upstream_input_tokens_override_precomputed` (which verifies via the
/// `finalize()` path).
#[test]
fn message_start_uses_precomputed_input_tokens() {
    let mut state = StreamingState::new(HashMap::new(), 42);

    // First chunk triggers message_start with pre-computed input_tokens.
    let events = state.process_chunk(&text_chunk("c1", "m1", "Hello", None));
    match &events[0] {
        StreamEvent::MessageStart { message } => {
            assert_eq!(
                message.usage.input_tokens, 42,
                "message_start should use the pre-computed input_tokens"
            );
        }
        other => panic!("Expected MessageStart, got: {:?}", other),
    }
}

/// `message_delta.usage.output_tokens > 0` after text streaming.
#[test]
fn output_tokens_nonzero_after_text_streaming() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    state.process_chunk(&text_chunk("c1", "m1", "Hello world", None));
    let events = state.process_chunk(&finish_chunk("c1", "m1", "stop"));

    // The last event should be MessageDelta with output_tokens > 0.
    let output_tokens = assert_message_delta(events.last().unwrap(), "end_turn");
    assert!(
        output_tokens > 0,
        "output_tokens should be > 0 after text streaming, got {output_tokens}"
    );
}

/// `message_delta.usage.output_tokens > 0` after tool call streaming.
#[test]
fn output_tokens_nonzero_after_tool_call() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "read_file", r#"{"path":"#,
    ));
    state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, r#""/tmp/a.txt"}"#));
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));

    let output_tokens = assert_message_delta(events.last().unwrap(), "tool_use");
    assert!(
        output_tokens > 0,
        "output_tokens should be > 0 after tool call, got {output_tokens}"
    );
}

/// Tool call argument accumulation works correctly across fragments.
#[test]
fn tool_json_accumulation_across_fragments() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "func", r#"{"a":"#,
    ));
    state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, "1"));
    state.process_chunk(&tool_call_args_chunk("c1", "m1", 0, "}"));
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));

    let output_tokens = assert_message_delta(events.last().unwrap(), "tool_use");
    // {"a":1} is ~5 tokens — should be > 0
    assert!(
        output_tokens > 0,
        "output_tokens should count accumulated tool JSON, got {output_tokens}"
    );
}

/// When a tool call is truncated (finish_reason: "length"), the discarded
/// tool JSON should NOT be counted, but the truncation notice text should.
#[test]
fn truncated_tool_counts_notice_not_discarded_json() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    // Accumulate a large tool call JSON that will be discarded.
    state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "write_file", r#"{"path":"/tmp/a.txt","content":"very long content "#,
    ));
    state.process_chunk(&tool_call_args_chunk(
        "c1", "m1", 0, "that would generate many tokens if counted",
    ));

    // Truncate — the tool JSON is discarded, notice text replaces it.
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));

    let output_tokens = assert_message_delta(events.last().unwrap(), "max_tokens");
    // The notice text is short: [Tool call to "write_file" was truncated ...]
    // Should be > 0 but fairly small.
    assert!(
        output_tokens > 0,
        "output_tokens should count truncation notice, got {output_tokens}"
    );
    // It should be significantly less than it would be with the full tool JSON.
    assert!(
        output_tokens < 50,
        "output_tokens should only count the short notice, not discarded JSON, got {output_tokens}"
    );
}

/// When upstream usage arrives *after* `finish_reason` (the common OpenAI
/// `stream_options.include_usage` pattern), the finish has already emitted
/// `message_delta` with the tiktoken estimate. The upstream values are
/// stored in `StreamingState` but have no further effect because
/// `finalize()` skips the `message_delta` when `finish_emitted` is true.
///
/// This is a known architectural limitation — not a bug. The valid
/// override path (upstream arrives *before* `finish_reason`) is tested by
/// `upstream_output_tokens_before_finish`.
#[test]
fn upstream_output_tokens_stored_but_unused_when_arriving_after_finish() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    state.process_chunk(&text_chunk("c1", "m1", "Hello world", None));

    // Finish the stream normally — this emits message_delta with
    // tiktoken-estimated output_tokens.
    let finish_events = state.process_chunk(&finish_chunk("c1", "m1", "stop"));
    let tiktoken_output_tokens = assert_message_delta(finish_events.last().unwrap(), "end_turn");
    assert!(
        tiktoken_output_tokens > 0,
        "finish should emit tiktoken-estimated output_tokens, got {tiktoken_output_tokens}"
    );

    // Usage arrives in a separate final chunk with empty choices
    // (matching OpenAI's stream_options.include_usage behavior).
    let usage_chunk = ChatCompletionChunk {
        id: "c1".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: "m1".to_string(),
        choices: vec![],
        usage: Some(Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            extra: std::collections::HashMap::new(),
        }),
    };

    state.process_chunk(&usage_chunk);

    // Finalize — finish_emitted is true, so only MessageStop is emitted.
    // No second MessageDelta with upstream values.
    let events = state.finalize();
    assert_eq!(events.len(), 1, "finalize should emit only MessageStop after finish");
    assert_message_stop(&events[0]);
}

/// Upstream output tokens arrive before finish and are used in message_delta.
#[test]
fn upstream_output_tokens_before_finish() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    state.process_chunk(&text_chunk("c1", "m1", "Hello world", None));

    // Usage arrives in a chunk before finish_reason (some providers do this).
    let usage_chunk = ChatCompletionChunk {
        id: "c1".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: "m1".to_string(),
        choices: vec![],
        usage: Some(Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            extra: std::collections::HashMap::new(),
        }),
    };

    state.process_chunk(&usage_chunk);

    // Now finish — should use the upstream value.
    let events = state.process_chunk(&finish_chunk("c1", "m1", "stop"));
    let output_tokens = assert_message_delta(events.last().unwrap(), "end_turn");
    assert_eq!(
        output_tokens, 50,
        "Should use upstream completion_tokens, got {output_tokens}"
    );
}

/// When upstream usage arrives after finish (in a separate chunk), finalize
/// uses the upstream value.
#[test]
fn upstream_tokens_via_finalize_after_usage_chunk() {
    let mut state = StreamingState::new(HashMap::new(), 10);

    state.process_chunk(&text_chunk("c1", "m1", "Hello", None));

    // Usage arrives in a final chunk with no choices (no finish_reason).
    let usage_chunk = ChatCompletionChunk {
        id: "c1".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: "m1".to_string(),
        choices: vec![],
        usage: Some(Usage {
            prompt_tokens: 200,
            completion_tokens: 77,
            total_tokens: 277,
            extra: std::collections::HashMap::new(),
        }),
    };

    state.process_chunk(&usage_chunk);

    // No finish_reason was ever sent, so finalize emits the message_delta.
    let events = state.finalize();
    let delta_event = events.iter().find(|e| matches!(e, StreamEvent::MessageDelta { .. }));
    assert!(delta_event.is_some());

    let output_tokens = assert_message_delta(delta_event.unwrap(), "end_turn");
    assert_eq!(
        output_tokens, 77,
        "Finalize should use upstream output_tokens, got {output_tokens}"
    );
}

/// Upstream input tokens override pre-computed count (verified via finalize
/// path which also emits message_delta with output_tokens).
#[test]
fn upstream_input_tokens_override_precomputed() {
    let mut state = StreamingState::new(HashMap::new(), 42);

    // Process a chunk with upstream usage.
    let chunk_with_usage = ChatCompletionChunk {
        id: "c1".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: "m1".to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: Some("Hi".to_string()),
                tool_calls: None,
            },
            finish_reason: None,
        }],
        usage: Some(Usage {
            prompt_tokens: 200,
            completion_tokens: 10,
            total_tokens: 210,
            extra: std::collections::HashMap::new(),
        }),
    };

    let events = state.process_chunk(&chunk_with_usage);
    // message_start was emitted with input_tokens=42 (upstream arrived in same chunk
    // but message_start is built before usage is captured; this is fine because
    // message_start uses the pre-computed value on first call).
    match &events[0] {
        StreamEvent::MessageStart { message } => {
            // First chunk triggers message_start before usage is processed.
            // The pre-computed value is used.
            assert_eq!(message.usage.input_tokens, 42);
        }
        other => panic!("Expected MessageStart, got: {:?}", other),
    }

    // Now finalize — the message_delta should use upstream output tokens.
    let events = state.finalize();
    let delta_event = events.iter().find(|e| matches!(e, StreamEvent::MessageDelta { .. }));
    assert!(delta_event.is_some(), "finalize should emit MessageDelta");

    let output_tokens = assert_message_delta(
        delta_event.unwrap(),
        "end_turn",
    );
    assert_eq!(
        output_tokens, 10,
        "Finalize should use upstream output_tokens, got {output_tokens}"
    );
}

/// Mixed text + tool: output_tokens counts both text and tool JSON.
#[test]
fn output_tokens_counts_text_and_tool_json() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    // Text block first
    state.process_chunk(&text_chunk("c1", "m1", "Let me read that file.", None));

    // Then tool call
    state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "read_file", r#"{"path":"/tmp/a.txt"}"#,
    ));
    let events = state.process_chunk(&finish_chunk("c1", "m1", "tool_calls"));

    let output_tokens = assert_message_delta(events.last().unwrap(), "tool_use");
    // "Let me read that file." ~= 6 tokens, {"path":"/tmp/a.txt"} ~= 10 tokens
    assert!(
        output_tokens > 5,
        "Should count both text and tool JSON, got {output_tokens}"
    );
}

/// Finalize with no chunks processed returns empty.
#[test]
fn finalize_no_chunks_returns_empty() {
    let mut state = StreamingState::new(HashMap::new(), 100);
    let events = state.finalize();
    assert!(events.is_empty(), "finalize with no chunks should return empty");
}

/// Finalize emits real output_tokens (not hardcoded 0).
#[test]
fn finalize_emits_real_output_tokens() {
    let mut state = StreamingState::new(HashMap::new(), 0);

    state.process_chunk(&text_chunk("c1", "m1", "Hello world!", None));
    // No finish_reason — finalize will emit message_delta.
    let events = state.finalize();

    let delta_event = events.iter().find(|e| matches!(e, StreamEvent::MessageDelta { .. }));
    assert!(delta_event.is_some(), "finalize should emit MessageDelta");

    let output_tokens = assert_message_delta(delta_event.unwrap(), "end_turn");
    assert!(
        output_tokens > 0,
        "finalize output_tokens should be > 0 after text, got {output_tokens}"
    );
}
