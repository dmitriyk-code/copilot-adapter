//! Human-readable conversation logging for debugging.
//!
//! This module provides a [`ConversationLogger`] that writes structured,
//! readable summaries of request/response cycles to a log file. Unlike
//! trace-level JSON dumps, conversation logs are designed to be skimmed
//! quickly by a developer debugging adapter behaviour.
//!
//! ## File rotation
//!
//! When the log file exceeds `max_size` bytes, the current file is renamed
//! to `<path>.1` (overwriting any previous backup) and a new file is started.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

// ---------------------------------------------------------------------------
// ConversationLogger
// ---------------------------------------------------------------------------

/// Async logger that writes human-readable conversation summaries to a file.
///
/// Cloneable via the inner `Arc`. All public methods take `&self` so that the
/// logger can be shared across request handlers without external locking.
#[derive(Clone)]
pub struct ConversationLogger {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    max_size: u64,
    request_counter: AtomicU64,
}

impl ConversationLogger {
    /// Create a new logger that writes to `path`.
    ///
    /// `max_size` is the approximate threshold (in bytes) at which the log
    /// file is rotated. A value of 0 disables rotation.
    pub fn new(path: impl AsRef<Path>, max_size: u64) -> Self {
        Self {
            inner: Arc::new(Inner {
                path: path.as_ref().to_path_buf(),
                max_size,
                request_counter: AtomicU64::new(0),
            }),
        }
    }

    /// Return the next monotonically-increasing request number.
    pub fn next_request_number(&self) -> u64 {
        self.inner
            .request_counter
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }

    /// Log a complete request/response cycle.
    pub async fn log_cycle(&self, cycle: &ConversationCycle) -> std::io::Result<()> {
        self.maybe_rotate().await?;

        let formatted = cycle.format();

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.inner.path)
            .await?;
        file.write_all(formatted.as_bytes()).await?;
        file.flush().await?;

        Ok(())
    }

    /// Rotate the log file if it exceeds `max_size`.
    async fn maybe_rotate(&self) -> std::io::Result<()> {
        if self.inner.max_size == 0 {
            return Ok(());
        }

        let metadata = match tokio::fs::metadata(&self.inner.path).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };

        if metadata.len() >= self.inner.max_size {
            let backup = format!("{}.1", self.inner.path.display());
            // Best-effort rename; ignore errors (e.g. file in use on Windows).
            let _ = tokio::fs::rename(&self.inner.path, &backup).await;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Conversation cycle data structures
// ---------------------------------------------------------------------------

/// A complete request/response cycle through the adapter.
pub struct ConversationCycle {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub request_number: u64,
    pub request_id: String,

    // From Claude Code
    pub incoming_model: String,
    pub incoming_stream: bool,
    pub incoming_messages: Vec<MessageSummary>,
    pub incoming_system: Option<String>,
    pub incoming_tools: Vec<String>,

    // To Copilot
    pub outgoing_model: String,
    pub outgoing_messages_count: usize,
    pub tools_injected: bool,
    pub xml_injection_size: usize,

    // From Copilot
    pub response_model: String,
    pub response_finish_reason: Option<String>,
    pub response_content_preview: String,
    pub response_has_tool_calls: bool,

    // To Claude Code
    pub final_stop_reason: Option<String>,
    pub final_content_blocks: Vec<ContentBlockSummary>,
    pub parsed_tool_calls: Vec<ToolCallSummary>,
}

/// Summary of a single message in the conversation.
pub struct MessageSummary {
    pub role: String,
    pub content_preview: String,
    pub content_length: usize,
    pub has_tool_use: bool,
    pub has_tool_result: bool,
}

/// Summary of a response content block.
pub struct ContentBlockSummary {
    pub block_type: String,
    pub preview: String,
}

/// Summary of a parsed tool call.
pub struct ToolCallSummary {
    pub id: String,
    pub name: String,
    pub arguments_preview: String,
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Maximum characters shown in content previews.
const PREVIEW_LENGTH: usize = 200;

/// Truncate a string to `max_chars`, appending "…" if truncated.
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

impl ConversationCycle {
    /// Format the cycle as a human-readable multi-section log entry.
    pub fn format(&self) -> String {
        let mut out = String::with_capacity(2048);

        // Header
        out.push_str(&"=".repeat(80));
        out.push_str(&format!(
            "\n[{}] Request #{} ({})\n",
            self.timestamp.format("%Y-%m-%d %H:%M:%S%.3f UTC"),
            self.request_number,
            self.request_id,
        ));
        out.push_str(&"=".repeat(80));
        out.push('\n');

        // --- FROM CLAUDE CODE ---
        out.push_str("\n>>> FROM CLAUDE CODE (Anthropic format)\n");
        out.push_str(&format!("Model: {}\n", self.incoming_model));
        out.push_str(&format!("Stream: {}\n", self.incoming_stream));
        out.push_str(&format!("Messages: {}\n", self.incoming_messages.len()));

        if let Some(ref sys) = self.incoming_system {
            out.push_str(&format!(
                "System prompt: {} chars\n",
                sys.len()
            ));
        }

        if !self.incoming_tools.is_empty() {
            out.push_str(&format!(
                "Tools ({}): {}\n",
                self.incoming_tools.len(),
                self.incoming_tools.join(", "),
            ));
        }

        for (i, msg) in self.incoming_messages.iter().enumerate() {
            let flags = match (msg.has_tool_use, msg.has_tool_result) {
                (true, true) => " [tool_use, tool_result]",
                (true, false) => " [tool_use]",
                (false, true) => " [tool_result]",
                _ => "",
            };
            out.push_str(&format!(
                "  [{}] role={}{}, {} chars: {}\n",
                i,
                msg.role,
                flags,
                msg.content_length,
                truncate(&msg.content_preview, PREVIEW_LENGTH),
            ));
        }

        // --- TO COPILOT ---
        out.push('\n');
        out.push_str(&"-".repeat(80));
        out.push_str("\n>>> TO GITHUB COPILOT API (OpenAI format)\n");
        out.push_str(&format!("Model: {}\n", self.outgoing_model));
        out.push_str(&format!("Messages: {}\n", self.outgoing_messages_count));
        out.push_str(&format!("Tools injected: {}\n", self.tools_injected));
        if self.tools_injected {
            out.push_str(&format!(
                "XML injection size: {} bytes\n",
                self.xml_injection_size
            ));
        }

        // --- FROM COPILOT ---
        out.push('\n');
        out.push_str(&"-".repeat(80));
        out.push_str("\n<<< FROM GITHUB COPILOT API (OpenAI format)\n");
        out.push_str(&format!("Model: {}\n", self.response_model));
        out.push_str(&format!(
            "Finish reason: {}\n",
            self.response_finish_reason.as_deref().unwrap_or("(none)")
        ));
        out.push_str(&format!(
            "Has tool calls: {}\n",
            self.response_has_tool_calls
        ));
        out.push_str(&format!(
            "Content preview: {}\n",
            truncate(&self.response_content_preview, PREVIEW_LENGTH),
        ));

        // --- TO CLAUDE CODE ---
        out.push('\n');
        out.push_str(&"-".repeat(80));
        out.push_str("\n<<< TO CLAUDE CODE (Anthropic format)\n");
        out.push_str(&format!(
            "Stop reason: {}\n",
            self.final_stop_reason.as_deref().unwrap_or("(none)")
        ));
        out.push_str(&format!(
            "Content blocks: {}\n",
            self.final_content_blocks.len()
        ));

        for (i, block) in self.final_content_blocks.iter().enumerate() {
            out.push_str(&format!(
                "  [{}] type={}: {}\n",
                i,
                block.block_type,
                truncate(&block.preview, PREVIEW_LENGTH),
            ));
        }

        if !self.parsed_tool_calls.is_empty() {
            out.push_str(&format!(
                "Parsed tool calls: {}\n",
                self.parsed_tool_calls.len()
            ));
            for tc in &self.parsed_tool_calls {
                out.push_str(&format!(
                    "  - {} (id={}): {}\n",
                    tc.name,
                    tc.id,
                    truncate(&tc.arguments_preview, 100),
                ));
            }
        }

        out.push('\n');
        out.push_str(&"=".repeat(80));
        out.push('\n');

        out
    }
}

// ---------------------------------------------------------------------------
// Builder for incremental construction
// ---------------------------------------------------------------------------

/// Builder that collects data across the request lifecycle and produces a
/// [`ConversationCycle`] at the end.
pub struct ConversationCycleBuilder {
    request_number: u64,
    request_id: String,
    timestamp: chrono::DateTime<chrono::Utc>,

    incoming_model: String,
    incoming_stream: bool,
    incoming_messages: Vec<MessageSummary>,
    incoming_system: Option<String>,
    incoming_tools: Vec<String>,

    outgoing_model: String,
    outgoing_messages_count: usize,
    tools_injected: bool,
    xml_injection_size: usize,

    response_model: String,
    response_finish_reason: Option<String>,
    response_content_preview: String,
    response_has_tool_calls: bool,

    final_stop_reason: Option<String>,
    final_content_blocks: Vec<ContentBlockSummary>,
    parsed_tool_calls: Vec<ToolCallSummary>,
}

impl ConversationCycleBuilder {
    pub fn new(request_number: u64, request_id: String) -> Self {
        Self {
            request_number,
            request_id,
            timestamp: chrono::Utc::now(),

            incoming_model: String::new(),
            incoming_stream: false,
            incoming_messages: Vec::new(),
            incoming_system: None,
            incoming_tools: Vec::new(),

            outgoing_model: String::new(),
            outgoing_messages_count: 0,
            tools_injected: false,
            xml_injection_size: 0,

            response_model: String::new(),
            response_finish_reason: None,
            response_content_preview: String::new(),
            response_has_tool_calls: false,

            final_stop_reason: None,
            final_content_blocks: Vec::new(),
            parsed_tool_calls: Vec::new(),
        }
    }

    /// Capture information from the incoming Anthropic request.
    pub fn set_incoming(&mut self, request: &crate::anthropic::types::AnthropicRequest) {
        self.incoming_model = request.model.clone();
        self.incoming_stream = request.stream.unwrap_or(false);

        if let Some(ref sys) = request.system {
            self.incoming_system = Some(sys.to_text());
        }

        if let Some(ref tools) = request.tools {
            self.incoming_tools = tools.iter().map(|t| t.name.clone()).collect();
        }

        for msg in &request.messages {
            let (preview, length) = match &msg.content {
                crate::anthropic::types::ContentBlockInput::Text(s) => {
                    (truncate(s, PREVIEW_LENGTH), s.len())
                }
                crate::anthropic::types::ContentBlockInput::Blocks(blocks) => {
                    let text: String = blocks
                        .iter()
                        .filter_map(|b| match b {
                            crate::anthropic::types::ContentBlock::Text { text, .. } => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    let len = text.len();
                    (truncate(&text, PREVIEW_LENGTH), len)
                }
            };

            let has_tool_use = matches!(&msg.content,
                crate::anthropic::types::ContentBlockInput::Blocks(blocks)
                if blocks.iter().any(|b| matches!(b, crate::anthropic::types::ContentBlock::ToolUse { .. }))
            );

            let has_tool_result = matches!(&msg.content,
                crate::anthropic::types::ContentBlockInput::Blocks(blocks)
                if blocks.iter().any(|b| matches!(b, crate::anthropic::types::ContentBlock::ToolResult { .. }))
            );

            self.incoming_messages.push(MessageSummary {
                role: msg.role.clone(),
                content_preview: preview,
                content_length: length,
                has_tool_use,
                has_tool_result,
            });
        }
    }

    /// Capture information about the outgoing OpenAI request to Copilot.
    pub fn set_outgoing(
        &mut self,
        openai_request: &crate::copilot::types::ChatCompletionRequest,
        tools_injected: bool,
        xml_injection_size: usize,
    ) {
        self.outgoing_model = openai_request.model.clone();
        self.outgoing_messages_count = openai_request.messages.len();
        self.tools_injected = tools_injected;
        self.xml_injection_size = xml_injection_size;
    }

    /// Capture information from the Copilot API response (non-streaming).
    pub fn set_copilot_response(
        &mut self,
        response: &crate::copilot::types::ChatCompletionResponse,
    ) {
        self.response_model = response.model.clone();

        if let Some(choice) = response.choices.first() {
            self.response_finish_reason = choice.finish_reason.clone();
            let text = choice.message.content.as_text();
            self.response_content_preview = truncate(&text, PREVIEW_LENGTH);
            self.response_has_tool_calls = choice
                .message
                .tool_calls
                .as_ref()
                .map_or(false, |tc| !tc.is_empty());
        }
    }

    /// Capture information from a streaming Copilot API response.
    pub fn set_copilot_streaming_response(
        &mut self,
        model: &str,
        content: &str,
        finish_reason: Option<&str>,
        has_tool_calls: bool,
    ) {
        self.response_model = model.to_string();
        self.response_content_preview = truncate(content, PREVIEW_LENGTH);
        self.response_finish_reason = finish_reason.map(|s| s.to_string());
        self.response_has_tool_calls = has_tool_calls;
    }

    /// Capture the final response sent back to Claude Code.
    pub fn set_final(
        &mut self,
        stop_reason: Option<&str>,
        content_blocks: &[crate::anthropic::types::ResponseContentBlock],
        tool_calls: &[crate::tools::types::ToolCall],
    ) {
        self.final_stop_reason = stop_reason.map(|s| s.to_string());

        self.final_content_blocks = content_blocks
            .iter()
            .map(|block| match block {
                crate::anthropic::types::ResponseContentBlock::Text { text, .. } => {
                    ContentBlockSummary {
                        block_type: "text".to_string(),
                        preview: truncate(text, PREVIEW_LENGTH),
                    }
                }
                crate::anthropic::types::ResponseContentBlock::ToolUse {
                    name, id, ..
                } => ContentBlockSummary {
                    block_type: "tool_use".to_string(),
                    preview: format!("{name} (id={id})"),
                },
            })
            .collect();

        self.parsed_tool_calls = tool_calls
            .iter()
            .map(|tc| ToolCallSummary {
                id: tc.id.clone().unwrap_or_default(),
                name: tc.function.name.clone().unwrap_or_default(),
                arguments_preview: tc.function.arguments.clone().unwrap_or_default(),
            })
            .collect();
    }

    /// Consume the builder and produce a [`ConversationCycle`].
    pub fn build(self) -> ConversationCycle {
        ConversationCycle {
            timestamp: self.timestamp,
            request_number: self.request_number,
            request_id: self.request_id,

            incoming_model: self.incoming_model,
            incoming_stream: self.incoming_stream,
            incoming_messages: self.incoming_messages,
            incoming_system: self.incoming_system,
            incoming_tools: self.incoming_tools,

            outgoing_model: self.outgoing_model,
            outgoing_messages_count: self.outgoing_messages_count,
            tools_injected: self.tools_injected,
            xml_injection_size: self.xml_injection_size,

            response_model: self.response_model,
            response_finish_reason: self.response_finish_reason,
            response_content_preview: self.response_content_preview,
            response_has_tool_calls: self.response_has_tool_calls,

            final_stop_reason: self.final_stop_reason,
            final_content_blocks: self.final_content_blocks,
            parsed_tool_calls: self.parsed_tool_calls,
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cycle() -> ConversationCycle {
        ConversationCycle {
            timestamp: chrono::Utc::now(),
            request_number: 1,
            request_id: "req-abc123".to_string(),

            incoming_model: "claude-sonnet-4-20250514".to_string(),
            incoming_stream: true,
            incoming_messages: vec![
                MessageSummary {
                    role: "user".to_string(),
                    content_preview: "Hello, world!".to_string(),
                    content_length: 13,
                    has_tool_use: false,
                    has_tool_result: false,
                },
                MessageSummary {
                    role: "assistant".to_string(),
                    content_preview: "I'll help you with that.".to_string(),
                    content_length: 24,
                    has_tool_use: true,
                    has_tool_result: false,
                },
            ],
            incoming_system: Some("You are a helpful assistant.".to_string()),
            incoming_tools: vec!["read_file".to_string(), "write_file".to_string()],

            outgoing_model: "claude-sonnet-4".to_string(),
            outgoing_messages_count: 3,
            tools_injected: true,
            xml_injection_size: 1500,

            response_model: "claude-sonnet-4".to_string(),
            response_finish_reason: Some("stop".to_string()),
            response_content_preview: "Here is the result.".to_string(),
            response_has_tool_calls: true,

            final_stop_reason: Some("tool_use".to_string()),
            final_content_blocks: vec![
                ContentBlockSummary {
                    block_type: "text".to_string(),
                    preview: "Here is the result.".to_string(),
                },
                ContentBlockSummary {
                    block_type: "tool_use".to_string(),
                    preview: "read_file (id=call_abc123)".to_string(),
                },
            ],
            parsed_tool_calls: vec![ToolCallSummary {
                id: "call_abc123".to_string(),
                name: "read_file".to_string(),
                arguments_preview: r#"{"path":"/src/main.rs"}"#.to_string(),
            }],
        }
    }

    #[test]
    fn format_produces_readable_output() {
        let cycle = sample_cycle();
        let output = cycle.format();

        // Header section
        assert!(output.contains("Request #1"));
        assert!(output.contains("req-abc123"));

        // From Claude Code section
        assert!(output.contains(">>> FROM CLAUDE CODE (Anthropic format)"));
        assert!(output.contains("Model: claude-sonnet-4-20250514"));
        assert!(output.contains("Stream: true"));
        assert!(output.contains("Messages: 2"));
        assert!(output.contains("Tools (2): read_file, write_file"));

        // To Copilot section
        assert!(output.contains(">>> TO GITHUB COPILOT API (OpenAI format)"));
        assert!(output.contains("Model: claude-sonnet-4"));
        assert!(output.contains("Tools injected: true"));
        assert!(output.contains("XML injection size: 1500 bytes"));

        // From Copilot section
        assert!(output.contains("<<< FROM GITHUB COPILOT API (OpenAI format)"));
        assert!(output.contains("Finish reason: stop"));
        assert!(output.contains("Has tool calls: true"));

        // To Claude Code section
        assert!(output.contains("<<< TO CLAUDE CODE (Anthropic format)"));
        assert!(output.contains("Stop reason: tool_use"));
        assert!(output.contains("Content blocks: 2"));
        assert!(output.contains("Parsed tool calls: 1"));
        assert!(output.contains("read_file"));
    }

    #[test]
    fn format_shows_all_four_sections() {
        let cycle = sample_cycle();
        let output = cycle.format();

        assert!(output.contains(">>> FROM CLAUDE CODE"));
        assert!(output.contains(">>> TO GITHUB COPILOT API"));
        assert!(output.contains("<<< FROM GITHUB COPILOT API"));
        assert!(output.contains("<<< TO CLAUDE CODE"));
    }

    #[test]
    fn handles_empty_messages() {
        let cycle = ConversationCycle {
            timestamp: chrono::Utc::now(),
            request_number: 1,
            request_id: "req-empty".to_string(),

            incoming_model: "claude-sonnet-4".to_string(),
            incoming_stream: false,
            incoming_messages: vec![],
            incoming_system: None,
            incoming_tools: vec![],

            outgoing_model: "claude-sonnet-4".to_string(),
            outgoing_messages_count: 0,
            tools_injected: false,
            xml_injection_size: 0,

            response_model: "claude-sonnet-4".to_string(),
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
        // Should not contain tool call section
        assert!(!output.contains("Parsed tool calls:"));
    }

    #[test]
    fn truncates_long_content() {
        let long_content = "a".repeat(1000);
        let truncated = truncate(&long_content, PREVIEW_LENGTH);
        assert!(truncated.len() < 1000);
        assert!(truncated.ends_with('…'));
        assert_eq!(truncated.chars().count(), PREVIEW_LENGTH + 1); // +1 for the '…'
    }

    #[test]
    fn truncate_preserves_short_content() {
        let short = "hello world";
        assert_eq!(truncate(short, PREVIEW_LENGTH), short);
    }

    #[tokio::test]
    async fn log_rotation_works() {
        let dir = std::env::temp_dir().join(format!(
            "conversation_log_test_{}",
            std::process::id()
        ));
        let _ = tokio::fs::create_dir_all(&dir).await;
        let log_path = dir.join("test.log");
        let backup_path = dir.join("test.log.1");

        // Clean up from previous runs
        let _ = tokio::fs::remove_file(&log_path).await;
        let _ = tokio::fs::remove_file(&backup_path).await;

        // Create a logger with a very small max size to trigger rotation
        let logger = ConversationLogger::new(&log_path, 100);

        // Write a cycle that exceeds the max size
        let cycle = sample_cycle();
        logger.log_cycle(&cycle).await.unwrap();

        // Verify the log file was created
        assert!(tokio::fs::metadata(&log_path).await.is_ok());
        let first_size = tokio::fs::metadata(&log_path).await.unwrap().len();
        assert!(first_size > 100);

        // Write another cycle — should trigger rotation
        logger.log_cycle(&cycle).await.unwrap();

        // The backup file should exist now
        assert!(tokio::fs::metadata(&backup_path).await.is_ok());

        // Clean up
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn async_write_completes() {
        let dir = std::env::temp_dir().join(format!(
            "conversation_log_write_test_{}",
            std::process::id()
        ));
        let _ = tokio::fs::create_dir_all(&dir).await;
        let log_path = dir.join("write_test.log");
        let _ = tokio::fs::remove_file(&log_path).await;

        let logger = ConversationLogger::new(&log_path, 10_485_760);
        let cycle = sample_cycle();

        // Write should succeed
        logger.log_cycle(&cycle).await.unwrap();

        // Read back and verify content
        let content = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert!(content.contains("Request #1"));
        assert!(content.contains("claude-sonnet-4"));

        // Clean up
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[test]
    fn format_shows_tool_use_and_tool_result_flags() {
        let cycle = ConversationCycle {
            timestamp: chrono::Utc::now(),
            request_number: 1,
            request_id: "req-flags".to_string(),

            incoming_model: "claude-sonnet-4".to_string(),
            incoming_stream: false,
            incoming_messages: vec![
                MessageSummary {
                    role: "assistant".to_string(),
                    content_preview: "Using tool".to_string(),
                    content_length: 10,
                    has_tool_use: true,
                    has_tool_result: false,
                },
                MessageSummary {
                    role: "user".to_string(),
                    content_preview: "Tool result".to_string(),
                    content_length: 11,
                    has_tool_use: false,
                    has_tool_result: true,
                },
            ],
            incoming_system: None,
            incoming_tools: vec![],

            outgoing_model: "claude-sonnet-4".to_string(),
            outgoing_messages_count: 2,
            tools_injected: false,
            xml_injection_size: 0,

            response_model: "claude-sonnet-4".to_string(),
            response_finish_reason: Some("stop".to_string()),
            response_content_preview: "Done".to_string(),
            response_has_tool_calls: false,

            final_stop_reason: Some("end_turn".to_string()),
            final_content_blocks: vec![ContentBlockSummary {
                block_type: "text".to_string(),
                preview: "Done".to_string(),
            }],
            parsed_tool_calls: vec![],
        };

        let output = cycle.format();
        assert!(output.contains("[tool_use]"));
        assert!(output.contains("[tool_result]"));
    }
}
