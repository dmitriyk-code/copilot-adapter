use copilot_adapter::copilot::types::*;

#[test]
fn image_url_block_serializes_to_openai_format() {
    let block = ContentBlock::ImageUrl {
        image_url: ImageUrl {
            url: "data:image/jpeg;base64,/9j/4AAQ...".to_string(),
            detail: None,
        },
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "image_url");
    assert_eq!(json["image_url"]["url"], "data:image/jpeg;base64,/9j/4AAQ...");
    assert!(json["image_url"].get("detail").is_none());
}

#[test]
fn image_url_block_with_detail_serializes() {
    let block = ContentBlock::ImageUrl {
        image_url: ImageUrl {
            url: "https://example.com/image.png".to_string(),
            detail: Some("high".to_string()),
        },
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "image_url");
    assert_eq!(json["image_url"]["url"], "https://example.com/image.png");
    assert_eq!(json["image_url"]["detail"], "high");
}

#[test]
fn image_url_block_deserializes_from_openai_format() {
    let json = serde_json::json!({
        "type": "image_url",
        "image_url": {
            "url": "data:image/png;base64,iVBOR..."
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match &block {
        ContentBlock::ImageUrl { image_url } => {
            assert_eq!(image_url.url, "data:image/png;base64,iVBOR...");
            assert!(image_url.detail.is_none());
        }
        _ => panic!("Expected ImageUrl variant"),
    }
}

#[test]
fn as_text_skips_image_blocks() {
    let content = MessageContent::Blocks(vec![
        ContentBlock::Text {
            text: "What is in this image?".to_string(),
        },
        ContentBlock::ImageUrl {
            image_url: ImageUrl {
                url: "data:image/jpeg;base64,/9j/4AAQ...".to_string(),
                detail: None,
            },
        },
    ]);
    assert_eq!(content.as_text(), "What is in this image?");
}

#[test]
fn as_text_returns_empty_when_only_images() {
    let content = MessageContent::Blocks(vec![ContentBlock::ImageUrl {
        image_url: ImageUrl {
            url: "data:image/jpeg;base64,/9j/...".to_string(),
            detail: Some("low".to_string()),
        },
    }]);
    assert_eq!(content.as_text(), "");
}

#[test]
fn message_with_multimodal_content_roundtrips() {
    let json = serde_json::json!({
        "role": "user",
        "content": [
            {"type": "text", "text": "What is in this image?"},
            {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,..."}}
        ]
    });
    let msg: Message = serde_json::from_value(json).unwrap();
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content.as_text(), "What is in this image?");

    // Re-serialize and verify structure
    let serialized = serde_json::to_value(&msg).unwrap();
    let blocks = serialized["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[1]["type"], "image_url");
    assert_eq!(blocks[1]["image_url"]["url"], "data:image/jpeg;base64,...");
}
