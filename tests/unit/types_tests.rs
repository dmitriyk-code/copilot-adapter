use copilot_adapter::copilot::types::*;

#[test]
fn message_serializes_to_json() {
    let msg = Message {
        role: "user".to_string(),
        content: MessageContent::Text("Hello".to_string()),
        name: None,
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["role"], "user");
    assert_eq!(json["content"], "Hello");
    // name is None and skip_serializing_if means it should be absent
    assert!(json.get("name").is_none());
}

#[test]
fn message_with_name_serializes() {
    let msg = Message {
        role: "user".to_string(),
        content: MessageContent::Text("Hello".to_string()),
        name: Some("Alice".to_string()),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["name"], "Alice");
}

#[test]
fn message_deserializes_from_json() {
    let json = serde_json::json!({
        "role": "assistant",
        "content": "Hi there!"
    });
    let msg: Message = serde_json::from_value(json).unwrap();
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.content.as_text(), "Hi there!");
    assert!(msg.name.is_none());
}

#[test]
fn message_deserializes_from_content_blocks() {
    // Claude models return content as an array of content blocks
    let json = serde_json::json!({
        "role": "assistant",
        "content": [
            {"type": "text", "text": "Hello "},
            {"type": "text", "text": "world!"}
        ]
    });
    let msg: Message = serde_json::from_value(json).unwrap();
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.content.as_text(), "Hello world!");
}

#[test]
fn chat_completion_request_minimal() {
    let json = serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let req: ChatCompletionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.model, "gpt-4");
    assert_eq!(req.messages.len(), 1);
    assert!(req.stream.is_none());
    assert!(req.temperature.is_none());
    assert!(req.max_tokens.is_none());
}

#[test]
fn chat_completion_request_full() {
    let json = serde_json::json!({
        "model": "gpt-4",
        "messages": [
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "Hello"}
        ],
        "stream": false,
        "temperature": 0.7,
        "max_tokens": 4096,
        "top_p": 1.0,
        "n": 1
    });
    let req: ChatCompletionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.model, "gpt-4");
    assert_eq!(req.messages.len(), 2);
    assert_eq!(req.stream, Some(false));
    assert_eq!(req.temperature, Some(0.7));
    assert_eq!(req.max_tokens, Some(4096));
    assert_eq!(req.top_p, Some(1.0));
    assert_eq!(req.n, Some(1));
}

#[test]
fn chat_completion_request_roundtrip() {
    let req = ChatCompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text("Hello".to_string()),
            name: None,
        }],
        stream: Some(false),
        temperature: Some(0.5),
        max_tokens: Some(100),
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
    };
    let json_str = serde_json::to_string(&req).unwrap();
    let deserialized: ChatCompletionRequest = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.model, "gpt-4");
    assert_eq!(deserialized.temperature, Some(0.5));
}

#[test]
fn chat_completion_request_skips_none_fields() {
    let req = ChatCompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text("Hello".to_string()),
            name: None,
        }],
        stream: None,
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    // Optional fields that are None should be absent
    assert!(json.get("stream").is_none());
    assert!(json.get("temperature").is_none());
    assert!(json.get("max_tokens").is_none());
    // Required fields must be present
    assert!(json.get("model").is_some());
    assert!(json.get("messages").is_some());
}

#[test]
fn chat_completion_response_deserializes() {
    let json = serde_json::json!({
        "id": "chatcmpl-abc123",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello! How can I help?"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 7,
            "total_tokens": 17
        }
    });
    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    assert_eq!(resp.id, "chatcmpl-abc123");
    assert_eq!(resp.object, "chat.completion");
    assert_eq!(resp.created, 1700000000);
    assert_eq!(resp.model, "gpt-4");
    assert_eq!(resp.choices.len(), 1);
    assert_eq!(resp.choices[0].index, 0);
    assert_eq!(resp.choices[0].message.role, "assistant");
    assert_eq!(resp.choices[0].finish_reason, Some("stop".to_string()));
    let usage = resp.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(usage.total_tokens, 17);
}

#[test]
fn chat_completion_response_roundtrip() {
    let resp = ChatCompletionResponse {
        id: "chatcmpl-test".to_string(),
        object: "chat.completion".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: MessageContent::Text("Hi".to_string()),
                name: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(Usage {
            prompt_tokens: 5,
            completion_tokens: 1,
            total_tokens: 6,
            ..Default::default()
        }),
    };
    let json_str = serde_json::to_string(&resp).unwrap();
    let deserialized: ChatCompletionResponse = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.id, "chatcmpl-test");
    assert_eq!(deserialized.choices[0].message.content.as_text(), "Hi");
}

#[test]
fn model_serializes_correctly() {
    let model = Model {
        id: "gpt-4".to_string(),
        object: "model".to_string(),
        created: 1686935002,
        owned_by: "github-copilot".to_string(),
    };
    let json = serde_json::to_value(&model).unwrap();
    assert_eq!(json["id"], "gpt-4");
    assert_eq!(json["object"], "model");
    assert_eq!(json["created"], 1686935002);
    assert_eq!(json["owned_by"], "github-copilot");
}

#[test]
fn model_list_serializes_correctly() {
    let list = ModelList {
        object: "list".to_string(),
        data: vec![Model {
            id: "gpt-4".to_string(),
            object: "model".to_string(),
            created: 1686935002,
            owned_by: "github-copilot".to_string(),
        }],
    };
    let json = serde_json::to_value(&list).unwrap();
    assert_eq!(json["object"], "list");
    assert_eq!(json["data"].as_array().unwrap().len(), 1);
    assert_eq!(json["data"][0]["id"], "gpt-4");
}

#[test]
fn usage_roundtrip() {
    let usage = Usage {
        prompt_tokens: 42,
        completion_tokens: 15,
        total_tokens: 57,
        ..Default::default()
    };
    let json_str = serde_json::to_string(&usage).unwrap();
    let deserialized: Usage = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.prompt_tokens, 42);
    assert_eq!(deserialized.completion_tokens, 15);
    assert_eq!(deserialized.total_tokens, 57);
}
