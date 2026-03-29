use copilot_adapter::anthropic::types::*;

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
        ContentBlock::Image { source, cache_control } => {
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
        ContentBlock::Image { source, cache_control } => {
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
        ContentBlock::Document { source, title, cache_control } => {
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
        ContentBlock::Text { text, cache_control } => {
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
        ContentBlock::Text { text, cache_control } => {
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
    assert_eq!(openai.messages[0].content.as_text(), "What is in this image? [Image]");
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
    assert_eq!(openai.messages[0].content.as_text(), "Summarize: Report.pdf");
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
    assert_eq!(openai.messages[0].content.as_text(), "[Document]");
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
        ContentBlock::Document { source, title, cache_control } => {
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
    assert_eq!(openai.messages[0].content.as_text(), "Look at this: [Image] and this doc: Analysis");
}
