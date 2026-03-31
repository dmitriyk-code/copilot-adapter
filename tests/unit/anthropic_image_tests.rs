use copilot_adapter::anthropic::types::*;
use copilot_adapter::copilot::types as openai;

// ---------------------------------------------------------------------------
// E1-T9: Deserialize image block with base64 source
// ---------------------------------------------------------------------------

#[test]
fn deserialize_image_block_base64() {
    let json = serde_json::json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "iVBORw0KGgoAAAANS..."
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Image {
            source,
            cache_control,
        } => {
            assert!(cache_control.is_none());
            match source {
                ImageSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "image/png");
                    assert_eq!(data, "iVBORw0KGgoAAAANS...");
                }
                _ => panic!("Expected Base64 variant"),
            }
        }
        _ => panic!("Expected Image variant"),
    }
}

// ---------------------------------------------------------------------------
// E1-T10: Deserialize image block with URL source
// ---------------------------------------------------------------------------

#[test]
fn deserialize_image_block_url() {
    let json = serde_json::json!({
        "type": "image",
        "source": {
            "type": "url",
            "media_type": "image/jpeg",
            "url": "https://example.com/photo.jpg"
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Image {
            source,
            cache_control,
        } => {
            assert!(cache_control.is_none());
            match source {
                ImageSource::Url { media_type, url } => {
                    assert_eq!(media_type, Some("image/jpeg".to_string()));
                    assert_eq!(url, "https://example.com/photo.jpg");
                }
                _ => panic!("Expected Url variant"),
            }
        }
        _ => panic!("Expected Image variant"),
    }
}

// ---------------------------------------------------------------------------
// E1-T11: Deserialize document block
// ---------------------------------------------------------------------------

#[test]
fn deserialize_document_block_base64() {
    let json = serde_json::json!({
        "type": "document",
        "source": {
            "type": "base64",
            "media_type": "application/pdf",
            "data": "JVBERi0xLjQ..."
        },
        "title": "My Document"
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Document {
            source,
            title,
            cache_control,
        } => {
            assert_eq!(title, Some("My Document".to_string()));
            assert!(cache_control.is_none());
            match source {
                DocumentSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "application/pdf");
                    assert_eq!(data, "JVBERi0xLjQ...");
                }
                _ => panic!("Expected Base64 variant"),
            }
        }
        _ => panic!("Expected Document variant"),
    }
}

#[test]
fn deserialize_document_block_text() {
    let json = serde_json::json!({
        "type": "document",
        "source": {
            "type": "text",
            "media_type": "text/plain",
            "data": "Hello, world!"
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Document { source, title, .. } => {
            assert!(title.is_none());
            match source {
                DocumentSource::Text { media_type, data } => {
                    assert_eq!(media_type, "text/plain");
                    assert_eq!(data, "Hello, world!");
                }
                _ => panic!("Expected Text variant"),
            }
        }
        _ => panic!("Expected Document variant"),
    }
}

#[test]
fn deserialize_document_block_url() {
    let json = serde_json::json!({
        "type": "document",
        "source": {
            "type": "url",
            "media_type": "application/pdf",
            "url": "https://example.com/doc.pdf"
        },
        "title": "Remote Doc"
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Document { source, title, .. } => {
            assert_eq!(title, Some("Remote Doc".to_string()));
            match source {
                DocumentSource::Url { media_type, url } => {
                    assert_eq!(media_type, Some("application/pdf".to_string()));
                    assert_eq!(url, "https://example.com/doc.pdf");
                }
                _ => panic!("Expected Url variant"),
            }
        }
        _ => panic!("Expected Document variant"),
    }
}

// ---------------------------------------------------------------------------
// E1-T12: Deserialize cache_control on text block
// ---------------------------------------------------------------------------

#[test]
fn deserialize_cache_control_on_text_block() {
    let json = serde_json::json!({
        "type": "text",
        "text": "cached content",
        "cache_control": {
            "type": "ephemeral"
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Text {
            text,
            cache_control,
        } => {
            assert_eq!(text, "cached content");
            let cc = cache_control.expect("cache_control should be present");
            assert_eq!(cc.cache_type, "ephemeral");
            assert!(cc.ttl.is_none());
        }
        _ => panic!("Expected Text variant"),
    }
}

#[test]
fn deserialize_cache_control_with_ttl() {
    let json = serde_json::json!({
        "type": "text",
        "text": "ttl content",
        "cache_control": {
            "type": "ephemeral",
            "ttl": 300
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Text { cache_control, .. } => {
            let cc = cache_control.expect("cache_control should be present");
            assert_eq!(cc.cache_type, "ephemeral");
            assert_eq!(cc.ttl, Some(300));
        }
        _ => panic!("Expected Text variant"),
    }
}

#[test]
fn deserialize_text_block_without_cache_control() {
    let json = serde_json::json!({
        "type": "text",
        "text": "plain text"
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Text {
            text,
            cache_control,
        } => {
            assert_eq!(text, "plain text");
            assert!(cache_control.is_none());
        }
        _ => panic!("Expected Text variant"),
    }
}

#[test]
fn deserialize_cache_control_on_image_block() {
    let json = serde_json::json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "abc123"
        },
        "cache_control": {
            "type": "ephemeral"
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Image { cache_control, .. } => {
            let cc = cache_control.expect("cache_control should be present");
            assert_eq!(cc.cache_type, "ephemeral");
        }
        _ => panic!("Expected Image variant"),
    }
}

// ---------------------------------------------------------------------------
// E1-T13: extract_text() handles image/document blocks
// ---------------------------------------------------------------------------

#[test]
fn extract_text_with_image_block() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "What is in this image? "},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}}
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();
    // Multimodal messages are now translated to Blocks; as_text() returns only text blocks
    assert_eq!(
        openai.messages[0].content.as_text(),
        "What is in this image? "
    );
    // Verify it's actually a Blocks variant with an ImageUrl block
    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 2);
            assert!(matches!(&blocks[1], openai::ContentBlock::ImageUrl { .. }));
        }
        _ => panic!("Expected Blocks content"),
    }
}

#[test]
fn extract_text_with_document_block_titled() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Summarize: "},
                {"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": "abc"}, "title": "Report.pdf"}
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();
    // Document blocks are skipped in multimodal translation; only text remains
    assert_eq!(openai.messages[0].content.as_text(), "Summarize: ");
}

#[test]
fn extract_text_with_document_block_untitled() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "document", "source": {"type": "text", "media_type": "text/plain", "data": "some content"}}
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();
    // Document-only messages result in all blocks being skipped, so the message is omitted
    assert_eq!(openai.messages.len(), 0);
}

#[test]
fn image_block_serializes_without_cache_control() {
    let block = ContentBlock::Image {
        source: ImageSource::Base64 {
            media_type: "image/png".to_string(),
            data: "abc123".to_string(),
        },
        cache_control: None,
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "image");
    assert_eq!(json["source"]["type"], "base64");
    assert_eq!(json["source"]["media_type"], "image/png");
    assert!(json.get("cache_control").is_none());
}

#[test]
fn document_block_roundtrip() {
    let block = ContentBlock::Document {
        source: DocumentSource::Url {
            media_type: Some("application/pdf".to_string()),
            url: "https://example.com/doc.pdf".to_string(),
        },
        title: Some("My Doc".to_string()),
        cache_control: Some(CacheControl {
            cache_type: "ephemeral".to_string(),
            ttl: None,
        }),
    };
    let json_str = serde_json::to_string(&block).unwrap();
    let deserialized: ContentBlock = serde_json::from_str(&json_str).unwrap();
    match deserialized {
        ContentBlock::Document {
            source,
            title,
            cache_control,
        } => {
            assert_eq!(title, Some("My Doc".to_string()));
            assert!(cache_control.is_some());
            match source {
                DocumentSource::Url { url, .. } => assert_eq!(url, "https://example.com/doc.pdf"),
                _ => panic!("Expected Url variant"),
            }
        }
        _ => panic!("Expected Document variant"),
    }
}

#[test]
fn mixed_content_blocks_in_message_deserialize() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Look at this: "},
                {"type": "image", "source": {"type": "url", "media_type": "image/jpeg", "url": "https://example.com/img.jpg"}},
                {"type": "text", "text": " and this doc: "},
                {"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": "JVBERi0xLjQ="}, "title": "Analysis"}
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    match &req.messages[0].content {
        ContentBlockInput::Blocks(blocks) => {
            assert_eq!(blocks.len(), 4);
            assert!(matches!(&blocks[0], ContentBlock::Text { .. }));
            assert!(matches!(&blocks[1], ContentBlock::Image { .. }));
            assert!(matches!(&blocks[2], ContentBlock::Text { .. }));
            assert!(matches!(&blocks[3], ContentBlock::Document { .. }));
        }
        _ => panic!("Expected Blocks variant"),
    }
    let openai = req.to_chat_completion_request();
    // Multimodal path: image is translated to ImageUrl, document is skipped → 3 blocks
    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 3);
            assert!(
                matches!(&blocks[0], openai::ContentBlock::Text { text } if text == "Look at this: ")
            );
            assert!(matches!(&blocks[1], openai::ContentBlock::ImageUrl { .. }));
            assert!(
                matches!(&blocks[2], openai::ContentBlock::Text { text } if text == " and this doc: ")
            );
        }
        _ => panic!("Expected Blocks content for multimodal message"),
    }
}

// ---------------------------------------------------------------------------
// cache_control on ToolUse and ToolResult blocks
// ---------------------------------------------------------------------------

#[test]
fn deserialize_cache_control_on_tool_use_block() {
    let json = serde_json::json!({
        "type": "tool_use",
        "id": "toolu_01A",
        "name": "get_weather",
        "input": {"location": "London"},
        "cache_control": {
            "type": "ephemeral"
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::ToolUse {
            id,
            name,
            input,
            cache_control,
        } => {
            assert_eq!(id, "toolu_01A");
            assert_eq!(name, "get_weather");
            assert_eq!(input["location"], "London");
            let cc = cache_control.expect("cache_control should be present");
            assert_eq!(cc.cache_type, "ephemeral");
            assert!(cc.ttl.is_none());
        }
        _ => panic!("Expected ToolUse variant"),
    }
}

#[test]
fn deserialize_cache_control_on_tool_result_block() {
    let json = serde_json::json!({
        "type": "tool_result",
        "tool_use_id": "toolu_01A",
        "content": "Sunny, 22°C",
        "cache_control": {
            "type": "ephemeral",
            "ttl": 600
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            cache_control,
        } => {
            assert_eq!(tool_use_id, "toolu_01A");
            match content {
                ToolResultContent::Text(t) => assert_eq!(t, "Sunny, 22°C"),
                _ => panic!("Expected Text content"),
            }
            let cc = cache_control.expect("cache_control should be present");
            assert_eq!(cc.cache_type, "ephemeral");
            assert_eq!(cc.ttl, Some(600));
        }
        _ => panic!("Expected ToolResult variant"),
    }
}

// ---------------------------------------------------------------------------
// URL sources without media_type (should deserialize successfully)
// ---------------------------------------------------------------------------

#[test]
fn deserialize_image_url_without_media_type() {
    let json = serde_json::json!({
        "type": "image",
        "source": {
            "type": "url",
            "url": "https://example.com/photo.jpg"
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Image { source, .. } => match source {
            ImageSource::Url { media_type, url } => {
                assert!(media_type.is_none());
                assert_eq!(url, "https://example.com/photo.jpg");
            }
            _ => panic!("Expected Url variant"),
        },
        _ => panic!("Expected Image variant"),
    }
}

#[test]
fn deserialize_document_url_without_media_type() {
    let json = serde_json::json!({
        "type": "document",
        "source": {
            "type": "url",
            "url": "https://example.com/report.pdf"
        },
        "title": "Report"
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::Document { source, title, .. } => {
            assert_eq!(title, Some("Report".to_string()));
            match source {
                DocumentSource::Url { media_type, url } => {
                    assert!(media_type.is_none());
                    assert_eq!(url, "https://example.com/report.pdf");
                }
                _ => panic!("Expected Url variant"),
            }
        }
        _ => panic!("Expected Document variant"),
    }
}

// ===========================================================================
// Epic 3: Translation Logic Tests
// ===========================================================================

// ---------------------------------------------------------------------------
// E3-T9: Translate text block
// ---------------------------------------------------------------------------

#[test]
fn translate_text_block_to_openai() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": "Hello, world!"
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();
    // Text-only message should remain as MessageContent::Text (backward compatible)
    match &openai.messages[0].content {
        openai::MessageContent::Text(t) => assert_eq!(t, "Hello, world!"),
        _ => panic!("Expected Text variant for text-only message"),
    }
}

#[test]
fn translate_text_block_in_array_without_images() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "world"}
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();
    // Text-only blocks (no images) should still use Text path
    match &openai.messages[0].content {
        openai::MessageContent::Text(t) => assert_eq!(t, "Hello world"),
        _ => panic!("Expected Text variant for text-only blocks"),
    }
}

// ---------------------------------------------------------------------------
// E3-T10: Translate image with base64 → data URI
// ---------------------------------------------------------------------------

#[test]
fn translate_image_base64_to_data_uri() {
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "What is this?"},
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": "iVBORw0KGgo="
                    }
                }
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 2);
            // First block: text
            match &blocks[0] {
                openai::ContentBlock::Text { text } => {
                    assert_eq!(text, "What is this?");
                }
                _ => panic!("Expected Text block"),
            }
            // Second block: image_url with data URI
            match &blocks[1] {
                openai::ContentBlock::ImageUrl { image_url } => {
                    assert_eq!(image_url.url, "data:image/png;base64,iVBORw0KGgo=");
                    assert!(image_url.detail.is_none());
                }
                _ => panic!("Expected ImageUrl block"),
            }
        }
        _ => panic!("Expected Blocks variant for multimodal message"),
    }
}

#[test]
fn translate_image_base64_jpeg() {
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/jpeg",
                        "data": "/9j/4AAQ..."
                    }
                }
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 1);
            match &blocks[0] {
                openai::ContentBlock::ImageUrl { image_url } => {
                    assert_eq!(image_url.url, "data:image/jpeg;base64,/9j/4AAQ...");
                }
                _ => panic!("Expected ImageUrl block"),
            }
        }
        _ => panic!("Expected Blocks variant"),
    }
}

// ---------------------------------------------------------------------------
// E3-T11: Translate image with URL → URL passthrough
// ---------------------------------------------------------------------------

#[test]
fn translate_image_url_passthrough() {
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Describe this:"},
                {
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": "https://example.com/photo.jpg"
                    }
                }
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 2);
            match &blocks[1] {
                openai::ContentBlock::ImageUrl { image_url } => {
                    assert_eq!(image_url.url, "https://example.com/photo.jpg");
                    assert!(image_url.detail.is_none());
                }
                _ => panic!("Expected ImageUrl block"),
            }
        }
        _ => panic!("Expected Blocks variant"),
    }
}

// ---------------------------------------------------------------------------
// E3-T12: Document block skipped with warning
// ---------------------------------------------------------------------------

#[test]
fn translate_document_block_skipped() {
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Summarize this doc"},
                {
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": "application/pdf",
                        "data": "JVBERi0xLjQ="
                    },
                    "title": "Report.pdf"
                }
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    // Document blocks are skipped; only the text block remains
    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 1);
            match &blocks[0] {
                openai::ContentBlock::Text { text } => {
                    assert_eq!(text, "Summarize this doc");
                }
                _ => panic!("Expected Text block"),
            }
        }
        _ => panic!("Expected Blocks variant (document triggers multimodal path)"),
    }
}

#[test]
fn translate_document_only_message() {
    // A message with only a document block results in no translated blocks,
    // so no message is emitted for it.
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": "application/pdf",
                        "data": "JVBERi0xLjQ="
                    }
                }
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();
    // All blocks were skipped → no message emitted
    assert!(openai.messages.is_empty());
}

// ---------------------------------------------------------------------------
// E3-T13: Mixed content (text + image)
// ---------------------------------------------------------------------------

#[test]
fn translate_mixed_text_and_images() {
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Compare these two images:"},
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": "first_image_data"
                    }
                },
                {"type": "text", "text": "vs"},
                {
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": "https://example.com/second.jpg"
                    }
                }
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 4);
            // Block 0: text
            assert!(
                matches!(&blocks[0], openai::ContentBlock::Text { text } if text == "Compare these two images:")
            );
            // Block 1: base64 image → data URI
            match &blocks[1] {
                openai::ContentBlock::ImageUrl { image_url } => {
                    assert!(image_url.url.starts_with("data:image/png;base64,"));
                }
                _ => panic!("Expected ImageUrl block at index 1"),
            }
            // Block 2: text
            assert!(matches!(&blocks[2], openai::ContentBlock::Text { text } if text == "vs"));
            // Block 3: URL image → passthrough
            match &blocks[3] {
                openai::ContentBlock::ImageUrl { image_url } => {
                    assert_eq!(image_url.url, "https://example.com/second.jpg");
                }
                _ => panic!("Expected ImageUrl block at index 3"),
            }
        }
        _ => panic!("Expected Blocks variant"),
    }
}

#[test]
fn translate_mixed_text_image_document() {
    // Documents are skipped, images and text are preserved
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "See attached:"},
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/webp",
                        "data": "UklGR..."
                    }
                },
                {
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": "application/pdf",
                        "data": "JVBERi0="
                    },
                    "title": "notes.pdf"
                }
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            // Document skipped → 2 blocks remain
            assert_eq!(blocks.len(), 2);
            assert!(
                matches!(&blocks[0], openai::ContentBlock::Text { text } if text == "See attached:")
            );
            match &blocks[1] {
                openai::ContentBlock::ImageUrl { image_url } => {
                    assert_eq!(image_url.url, "data:image/webp;base64,UklGR...");
                }
                _ => panic!("Expected ImageUrl block"),
            }
        }
        _ => panic!("Expected Blocks variant"),
    }
}

// ---------------------------------------------------------------------------
// E3-T14: Full request translation with images
// ---------------------------------------------------------------------------

#[test]
fn full_request_translation_with_images() {
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 2048,
        "system": "You are a helpful vision assistant.",
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "What's in this image?"},
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": "iVBORw0KGgoAAAANSUhEUg=="
                        }
                    }
                ]
            },
            {
                "role": "assistant",
                "content": "It shows a cat sitting on a windowsill."
            },
            {
                "role": "user",
                "content": "What color is the cat?"
            }
        ],
        "temperature": 0.7,
        "stream": true
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    // Verify request-level fields
    assert_eq!(openai.model, "gpt-4o");
    assert_eq!(openai.max_tokens, Some(2048));
    assert_eq!(openai.temperature, Some(0.7));
    assert_eq!(openai.stream, Some(true));

    // 4 messages: system + user(multimodal) + assistant + user(text)
    assert_eq!(openai.messages.len(), 4);

    // Message 0: system
    assert_eq!(openai.messages[0].role, "system");
    assert_eq!(
        openai.messages[0].content.as_text(),
        "You are a helpful vision assistant."
    );

    // Message 1: user with image (multimodal blocks)
    assert_eq!(openai.messages[1].role, "user");
    match &openai.messages[1].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 2);
            match &blocks[0] {
                openai::ContentBlock::Text { text } => {
                    assert_eq!(text, "What's in this image?");
                }
                _ => panic!("Expected Text block"),
            }
            match &blocks[1] {
                openai::ContentBlock::ImageUrl { image_url } => {
                    assert_eq!(
                        image_url.url,
                        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg=="
                    );
                }
                _ => panic!("Expected ImageUrl block"),
            }
        }
        _ => panic!("Expected Blocks variant for multimodal message"),
    }

    // Message 2: assistant (text-only)
    assert_eq!(openai.messages[2].role, "assistant");
    match &openai.messages[2].content {
        openai::MessageContent::Text(t) => {
            assert_eq!(t, "It shows a cat sitting on a windowsill.");
        }
        _ => panic!("Expected Text variant for assistant message"),
    }

    // Message 3: user (text-only, backward compatible)
    assert_eq!(openai.messages[3].role, "user");
    match &openai.messages[3].content {
        openai::MessageContent::Text(t) => {
            assert_eq!(t, "What color is the cat?");
        }
        _ => panic!("Expected Text variant for text-only user message"),
    }
}

#[test]
fn full_request_translation_preserves_tool_result_path() {
    // Ensure that tool_result messages still work alongside multimodal support
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_01ABC",
                        "content": "Result text"
                    }
                ]
            }
        ]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    // Tool result should be translated to tool role message
    assert_eq!(openai.messages.len(), 1);
    assert_eq!(openai.messages[0].role, "tool");
    assert_eq!(openai.messages[0].content.as_text(), "Result text");
    assert_eq!(
        openai.messages[0].tool_call_id,
        Some("toolu_01ABC".to_string())
    );
}

#[test]
fn image_with_cache_control_translates_correctly() {
    // cache_control is accepted but ignored during translation
    let json = serde_json::json!({
        "model": "gpt-4o",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/gif",
                        "data": "R0lGODlh"
                    },
                    "cache_control": {"type": "ephemeral"}
                }
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request();

    match &openai.messages[0].content {
        openai::MessageContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 1);
            match &blocks[0] {
                openai::ContentBlock::ImageUrl { image_url } => {
                    assert_eq!(image_url.url, "data:image/gif;base64,R0lGODlh");
                }
                _ => panic!("Expected ImageUrl block"),
            }
        }
        _ => panic!("Expected Blocks variant"),
    }
}
