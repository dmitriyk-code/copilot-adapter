use std::collections::{HashMap, HashSet};

use crate::anthropic::types::{
    build_message_start_response, ContentDelta, InputJsonDelta, MessageDeltaBody,
    MessageDeltaUsage, ResponseContentBlock, StreamEvent, TextDelta,
};
use crate::copilot::types::{ChatCompletionChunk, StreamingToolCall};

/// Tracks the kind of content block currently being generated.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ContentBlockType {
    Text,
    ToolUse,
}

/// Tracks state during incremental translation of OpenAI streaming
/// responses to Anthropic format.
///
/// The state machine processes [`ChatCompletionChunk`]s one at a time via
/// [`process_chunk`](Self::process_chunk) and emits the corresponding
/// Anthropic [`StreamEvent`]s. After the upstream stream ends, call
/// [`finalize`](Self::finalize) to close any open content blocks and emit
/// the terminal `message_stop` event.
///
/// **Tool-use buffering**: Events for tool_use blocks are buffered rather
/// than emitted immediately. When a tool call completes normally (another
/// tool starts, or `finish_reason` is `"tool_calls"`/`"stop"`), the buffer
/// is flushed. If `finish_reason` is `"length"` while a tool call is open,
/// the buffer is discarded — the truncated tool_use block never reaches
/// the consumer. This prevents Claude Code from executing incomplete tool
/// calls when the Copilot API hits its output token limit.
pub struct StreamingState {
    /// Message ID from the first chunk.
    message_id: Option<String>,
    /// Model name from the first chunk.
    model: Option<String>,
    /// Current content block type being generated.
    current_block_type: Option<ContentBlockType>,
    /// Current content block index.
    current_block_index: u32,
    /// Tool call IDs by tool-call index.
    tool_call_ids: HashMap<u32, String>,
    /// Tool call names by tool-call index.
    tool_call_names: HashMap<u32, String>,
    /// Mapping from truncated (OpenAI-safe) tool names back to original
    /// Anthropic tool names. Populated by the translator when names exceed
    /// the 64-character OpenAI limit.
    name_mapping: HashMap<String, String>,
    /// Whether we've emitted the `message_start` event.
    message_started: bool,
    /// Whether the current content block is open.
    block_open: bool,
    /// Whether a `message_delta` (finish) event has already been emitted via
    /// `handle_finish()`. Used by `finalize()` to ensure the required
    /// `message_delta` is always emitted before `message_stop`.
    finish_emitted: bool,
    /// Events buffered during an open tool_use block. Flushed when the
    /// block completes normally; discarded if truncated by `finish_reason`
    /// `"length"`.
    tool_use_buffer: Vec<StreamEvent>,
    /// The OpenAI tool-call index of the tool currently being streamed.
    /// Used to record which tool was truncated.
    current_openai_tool_index: Option<u32>,
    /// OpenAI tool-call indices whose tool_use blocks were discarded due
    /// to output-length truncation. Exposed to the handler so the
    /// conversation logger can exclude them.
    truncated_openai_tool_indices: HashSet<u32>,
}

impl StreamingState {
    /// Create a new streaming state with an optional name mapping for
    /// truncated tool names.
    ///
    /// The `name_mapping` maps *truncated* names (as sent to the OpenAI API)
    /// back to the *original* Anthropic tool names. Pass an empty map when
    /// no truncation occurred.
    pub fn new(name_mapping: HashMap<String, String>) -> Self {
        Self {
            message_id: None,
            model: None,
            current_block_type: None,
            current_block_index: 0,
            tool_call_ids: HashMap::new(),
            tool_call_names: HashMap::new(),
            name_mapping,
            message_started: false,
            block_open: false,
            finish_emitted: false,
            tool_use_buffer: Vec::new(),
            current_openai_tool_index: None,
            truncated_openai_tool_indices: HashSet::new(),
        }
    }

    /// Process an OpenAI streaming chunk and return zero or more Anthropic
    /// events.
    ///
    /// The first chunk triggers a `message_start` event. Subsequent chunks
    /// produce `content_block_start`, `content_block_delta`, and
    /// `content_block_stop` events as content arrives. A chunk with a
    /// `finish_reason` produces a `message_delta` event with the translated
    /// stop reason.
    pub fn process_chunk(&mut self, chunk: &ChatCompletionChunk) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        // Extract message info from the first chunk.
        if self.message_id.is_none() {
            self.message_id = Some(chunk.id.clone());
            self.model = Some(chunk.model.clone());
        }

        // Emit message_start on the very first call.
        if !self.message_started {
            events.push(self.build_message_start());
            self.message_started = true;
        }

        for choice in &chunk.choices {
            // Handle text content deltas.
            if let Some(text) = &choice.delta.content {
                if !text.is_empty() {
                    events.extend(self.handle_text_delta(text));
                }
            }

            // Handle streaming tool call deltas.
            if let Some(tool_calls) = &choice.delta.tool_calls {
                for tc in tool_calls {
                    events.extend(self.handle_tool_call_delta(tc));
                }
            }

            // Handle finish_reason transitions.
            if let Some(reason) = &choice.finish_reason {
                events.extend(self.handle_finish(reason));
            }
        }

        events
    }

    /// Finalize the stream and return closing events.
    ///
    /// Closes any open content block, emits `message_delta` (if not already
    /// emitted by a `finish_reason` chunk), and emits the terminal
    /// `message_stop`. The Anthropic SSE protocol requires a `message_delta`
    /// between the last `content_block_stop` and `message_stop`.
    ///
    /// Must be called exactly once after the upstream chunk stream ends.
    ///
    /// Returns an empty vec if no chunks were ever processed (i.e.,
    /// `message_start` was never emitted), preventing a malformed stream
    /// with a lone `message_stop`.
    pub fn finalize(&mut self) -> Vec<StreamEvent> {
        if !self.message_started {
            return vec![];
        }

        let mut events = Vec::new();

        // Flush any buffered tool_use events (stream ended without explicit
        // finish — treat the tool as complete, preserving existing behavior).
        events.extend(self.flush_tool_use_buffer());

        // Close any open content block.
        if self.block_open {
            events.push(StreamEvent::ContentBlockStop {
                index: self.current_block_index,
            });
            self.block_open = false;
        }

        // Emit message_delta if no finish_reason chunk was received (e.g.,
        // the upstream stream was truncated or dropped). The Anthropic
        // protocol mandates message_delta before message_stop.
        if !self.finish_emitted {
            events.push(StreamEvent::MessageDelta {
                delta: MessageDeltaBody {
                    stop_reason: Some("end_turn".to_string()),
                    stop_sequence: None,
                },
                usage: MessageDeltaUsage { output_tokens: 0 },
            });
        }

        // Terminal event.
        events.push(StreamEvent::MessageStop {});

        events
    }

    /// Returns the set of OpenAI tool-call indices that were truncated
    /// by `finish_reason: "length"`. The handler uses this to exclude
    /// incomplete tool calls from conversation logging.
    pub fn truncated_openai_tool_indices(&self) -> &HashSet<u32> {
        &self.truncated_openai_tool_indices
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Translate a text delta into the appropriate Anthropic events.
    ///
    /// If the current block type is not `Text`, first closes the previous
    /// block (if any) and opens a new text block. When transitioning from
    /// a tool_use block, the tool_use buffer is flushed first (the tool
    /// completed cleanly before text started).
    fn handle_text_delta(&mut self, text: &str) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        // Transition to a text block if we're not already in one.
        if self.current_block_type != Some(ContentBlockType::Text) {
            // Flush buffered tool_use events — the tool completed cleanly
            // because the model moved on to producing text.
            events.extend(self.flush_tool_use_buffer());

            // Close previous block if open.
            if self.block_open {
                events.push(StreamEvent::ContentBlockStop {
                    index: self.current_block_index,
                });
                self.current_block_index += 1;
            }

            // Start new text block.
            events.push(StreamEvent::ContentBlockStart {
                index: self.current_block_index,
                content_block: ResponseContentBlock::text(String::new()),
            });
            self.current_block_type = Some(ContentBlockType::Text);
            self.block_open = true;
        }

        // Emit the text delta.
        events.push(StreamEvent::ContentBlockDelta {
            index: self.current_block_index,
            delta: ContentDelta::Text(TextDelta {
                delta_type: "text_delta".to_string(),
                text: text.to_string(),
            }),
        });

        events
    }

    /// Translate a streaming tool call delta into Anthropic events.
    ///
    /// A new tool call (identified by the presence of a function name)
    /// closes the previous block and opens a new `tool_use` block. Argument
    /// fragments are emitted as `input_json_delta` events.
    ///
    /// All tool_use events are **buffered** rather than returned directly.
    /// When a new tool call starts, the buffer for the previous (now
    /// complete) tool is flushed. The buffer is discarded if the stream
    /// ends with `finish_reason: "length"` (truncated tool call).
    fn handle_tool_call_delta(&mut self, tc: &StreamingToolCall) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        let idx = tc.index;

        // A tool call delta with a function name signals a new call.
        let is_new_call = tc.function.as_ref().and_then(|f| f.name.as_ref()).is_some();

        if is_new_call {
            // A new tool call starting means the *previous* tool call
            // (if any) is complete. Flush the buffer for it.
            events.extend(self.flush_tool_use_buffer());

            // Close previous block if open.
            if self.block_open {
                // This ContentBlockStop goes into the returned events
                // (not the buffer) because it closes the *previous* block.
                events.push(StreamEvent::ContentBlockStop {
                    index: self.current_block_index,
                });
                self.current_block_index += 1;
            }

            // Store tool call metadata.
            if let Some(id) = &tc.id {
                self.tool_call_ids.insert(idx, id.clone());
            }
            if let Some(func) = &tc.function {
                if let Some(name) = &func.name {
                    // Restore original name if it was truncated for the
                    // OpenAI 64-char limit.
                    let original_name = self
                        .name_mapping
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.clone());
                    self.tool_call_names.insert(idx, original_name);
                }
            }

            // Build tool_use content block — buffer it.
            let id = self
                .tool_call_ids
                .get(&idx)
                .cloned()
                .unwrap_or_else(|| format!("call_{}", idx));
            let name = self.tool_call_names.get(&idx).cloned().unwrap_or_default();

            self.tool_use_buffer.push(StreamEvent::ContentBlockStart {
                index: self.current_block_index,
                content_block: ResponseContentBlock::ToolUse {
                    block_type: "tool_use".to_string(),
                    id,
                    name,
                    input: serde_json::Value::Object(serde_json::Map::new()),
                },
            });
            self.current_block_type = Some(ContentBlockType::ToolUse);
            self.block_open = true;
            self.current_openai_tool_index = Some(idx);
        }

        // Buffer input_json_delta for argument fragments.
        if let Some(func) = &tc.function {
            if let Some(args) = &func.arguments {
                if !args.is_empty() {
                    self.tool_use_buffer.push(StreamEvent::ContentBlockDelta {
                        index: self.current_block_index,
                        delta: ContentDelta::InputJson(InputJsonDelta {
                            delta_type: "input_json_delta".to_string(),
                            partial_json: args.clone(),
                        }),
                    });
                }
            }
        }

        events
    }

    /// Handle a finish_reason transition.
    ///
    /// When the finish reason is `"length"` and the current block is a
    /// tool_use, the buffered tool_use events are **discarded** (the tool
    /// call was truncated by the output token limit). The block is not
    /// closed, and `stop_reason` is set to `"max_tokens"` so Claude Code's
    /// escalation/recovery path fires.
    ///
    /// For all other finish reasons (`"tool_calls"`, `"stop"`, etc.), the
    /// buffer is flushed first (tool completed normally), then the block is
    /// closed and the mapped stop reason is emitted.
    fn handle_finish(&mut self, reason: &str) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        if reason == "length" && self.current_block_type == Some(ContentBlockType::ToolUse) {
            // Truncated tool call — discard the buffered events.
            tracing::warn!(
                block_index = self.current_block_index,
                openai_tool_index = ?self.current_openai_tool_index,
                "Dropping truncated tool_use block (finish_reason=\"length\")"
            );
            self.tool_use_buffer.clear();
            if let Some(oi_idx) = self.current_openai_tool_index {
                self.truncated_openai_tool_indices.insert(oi_idx);
            }
            // The block was never emitted to the consumer, so from its
            // perspective nothing to close. Reset block state.
            self.block_open = false;
            self.current_block_type = None;
            // Rewind the block index so the next block (if any) reuses
            // the index that the discarded block would have occupied.
            // (Not strictly necessary since we emit message_delta next,
            // but keeps the state consistent.)
        } else {
            // Normal completion — flush any buffered tool_use events.
            events.extend(self.flush_tool_use_buffer());

            // Close any open content block.
            if self.block_open {
                events.push(StreamEvent::ContentBlockStop {
                    index: self.current_block_index,
                });
                self.block_open = false;
            }
        }

        // Map OpenAI finish_reason → Anthropic stop_reason.
        let stop_reason = match reason {
            "tool_calls" => Some("tool_use".to_string()),
            "stop" => Some("end_turn".to_string()),
            "length" => Some("max_tokens".to_string()),
            other => Some(other.to_string()),
        };

        events.push(StreamEvent::MessageDelta {
            delta: MessageDeltaBody {
                stop_reason,
                stop_sequence: None,
            },
            usage: MessageDeltaUsage {
                // TODO: wire actual token counts once ChatCompletionChunk exposes usage
                output_tokens: 0,
            },
        });

        self.finish_emitted = true;

        events
    }

    /// Drain the tool_use buffer and return its contents.
    fn flush_tool_use_buffer(&mut self) -> Vec<StreamEvent> {
        if self.tool_use_buffer.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut self.tool_use_buffer)
    }

    /// Build the `message_start` event using the stored message ID and model.
    fn build_message_start(&self) -> StreamEvent {
        StreamEvent::MessageStart {
            message: build_message_start_response(
                self.message_id.as_deref().unwrap_or("unknown"),
                self.model.as_deref().unwrap_or("unknown"),
            ),
        }
    }
}
