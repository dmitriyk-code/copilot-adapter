use bytes::Bytes;
use futures::stream::{self, StreamExt};

use copilot_adapter::copilot::client::parse_sse_stream;
use copilot_adapter::copilot::types::ChatCompletionChunk;

/// Helper: create a complete SSE frame for a single chunk.
fn make_chunk_json(id: &str, content: &str, finish_reason: Option<&str>) -> String {
    let fr = match finish_reason {
        Some(r) => format!("\"{}\"", r),
        None => "null".to_string(),
    };
    format!(
        r#"{{"id":"{}","object":"chat.completion.chunk","created":1700000000,"model":"gpt-4","choices":[{{"index":0,"delta":{{"content":"{}"}},"finish_reason":{}}}]}}"#,
        id, content, fr
    )
}

/// Helper: wrap a chunk JSON string into SSE `data:` frame format.
fn make_sse_frame(data: &str) -> String {
    format!("data: {}\n\n", data)
}

#[tokio::test]
async fn parse_single_complete_chunk() {
    let chunk_json = make_chunk_json("c1", "Hello", None);
    let sse_data = make_sse_frame(&chunk_json);

    let byte_stream = stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(sse_data))]);
    let mut parsed = std::pin::pin!(parse_sse_stream(byte_stream));

    let item = parsed.next().await.unwrap().unwrap();
    assert_eq!(item.id, "c1");
    assert_eq!(item.choices[0].delta.content, Some("Hello".to_string()));
    assert!(item.choices[0].finish_reason.is_none());

    // Stream should end after single chunk (no [DONE] in this test — stream just ends)
    assert!(parsed.next().await.is_none());
}

#[tokio::test]
async fn parse_multiple_chunks_in_single_bytes() {
    let mut sse_data = String::new();
    sse_data.push_str(&make_sse_frame(&make_chunk_json("c1", "Hello", None)));
    sse_data.push_str(&make_sse_frame(&make_chunk_json("c1", " world", None)));
    sse_data.push_str(&make_sse_frame(&make_chunk_json("c1", "!", Some("stop"))));
    sse_data.push_str("data: [DONE]\n\n");

    let byte_stream = stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(sse_data))]);
    let parsed: Vec<_> = parse_sse_stream(byte_stream)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(parsed.len(), 3);
    assert_eq!(
        parsed[0].choices[0].delta.content,
        Some("Hello".to_string())
    );
    assert_eq!(
        parsed[1].choices[0].delta.content,
        Some(" world".to_string())
    );
    assert_eq!(parsed[2].choices[0].delta.content, Some("!".to_string()));
    assert_eq!(parsed[2].choices[0].finish_reason, Some("stop".to_string()));
}

#[tokio::test]
async fn parse_done_marker_terminates_stream() {
    let mut sse_data = String::new();
    sse_data.push_str(&make_sse_frame(&make_chunk_json("c1", "Hi", None)));
    sse_data.push_str("data: [DONE]\n\n");
    // Anything after [DONE] should be ignored
    sse_data.push_str(&make_sse_frame(&make_chunk_json("c1", "IGNORED", None)));

    let byte_stream = stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(sse_data))]);
    let parsed: Vec<_> = parse_sse_stream(byte_stream)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].choices[0].delta.content, Some("Hi".to_string()));
}

#[tokio::test]
async fn parse_partial_buffers_across_byte_boundaries() {
    // Split a single SSE frame across multiple byte chunks to test buffering.
    let chunk_json = make_chunk_json("c1", "split", None);
    let full = make_sse_frame(&chunk_json);

    let mid = full.len() / 2;
    let part1 = full[..mid].to_string();
    let part2 = full[mid..].to_string();

    let byte_stream = stream::iter(vec![
        Ok::<Bytes, reqwest::Error>(Bytes::from(part1)),
        Ok::<Bytes, reqwest::Error>(Bytes::from(part2)),
    ]);

    let parsed: Vec<_> = parse_sse_stream(byte_stream)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "c1");
    assert_eq!(
        parsed[0].choices[0].delta.content,
        Some("split".to_string())
    );
}

#[tokio::test]
async fn parse_ignores_sse_comments() {
    let mut sse_data = String::new();
    sse_data.push_str(": this is a comment\n\n");
    sse_data.push_str(&make_sse_frame(&make_chunk_json("c1", "data", None)));
    sse_data.push_str("data: [DONE]\n\n");

    let byte_stream = stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(sse_data))]);
    let parsed: Vec<_> = parse_sse_stream(byte_stream)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].choices[0].delta.content, Some("data".to_string()));
}

#[tokio::test]
async fn parse_handles_empty_stream() {
    let byte_stream = stream::iter(Vec::<Result<Bytes, reqwest::Error>>::new());

    let parsed: Vec<_> = parse_sse_stream(byte_stream).collect::<Vec<_>>().await;

    assert!(parsed.is_empty());
}

#[tokio::test]
async fn parse_handles_done_only() {
    let sse_data = "data: [DONE]\n\n".to_string();

    let byte_stream = stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(sse_data))]);
    let parsed: Vec<_> = parse_sse_stream(byte_stream).collect::<Vec<_>>().await;

    assert!(parsed.is_empty());
}

#[tokio::test]
async fn parse_chunk_has_correct_object_field() {
    let chunk_json = make_chunk_json("c1", "test", None);
    let sse_data = make_sse_frame(&chunk_json);

    let byte_stream = stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(sse_data))]);
    let mut parsed = std::pin::pin!(parse_sse_stream(byte_stream));

    let item = parsed.next().await.unwrap().unwrap();
    assert_eq!(item.object, "chat.completion.chunk");
}

#[tokio::test]
async fn parse_role_delta() {
    // First chunk typically carries role in delta
    let json = r#"{"id":"c1","object":"chat.completion.chunk","created":1700000000,"model":"gpt-4","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#;
    let sse_data = make_sse_frame(json);

    let byte_stream = stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(sse_data))]);
    let mut parsed = std::pin::pin!(parse_sse_stream(byte_stream));

    let item = parsed.next().await.unwrap().unwrap();
    assert_eq!(item.choices[0].delta.role, Some("assistant".to_string()));
    assert!(item.choices[0].delta.content.is_none());
}

#[tokio::test]
async fn chunk_types_serialize_roundtrip() {
    use copilot_adapter::copilot::types::{ChunkChoice, ChunkDelta};

    let chunk = ChatCompletionChunk {
        id: "chatcmpl-test".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: Some("assistant".to_string()),
                content: Some("Hello".to_string()),
                tool_calls: None,
            },
            finish_reason: None,
        }],
        usage: None,
    };

    let json_str = serde_json::to_string(&chunk).unwrap();
    let deserialized: ChatCompletionChunk = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.id, "chatcmpl-test");
    assert_eq!(deserialized.object, "chat.completion.chunk");
    assert_eq!(
        deserialized.choices[0].delta.role,
        Some("assistant".to_string())
    );
    assert_eq!(
        deserialized.choices[0].delta.content,
        Some("Hello".to_string())
    );
    assert!(deserialized.choices[0].finish_reason.is_none());
}

#[tokio::test]
async fn chunk_delta_skips_none_fields() {
    use copilot_adapter::copilot::types::ChunkDelta;

    let delta = ChunkDelta {
        role: None,
        content: Some("test".to_string()),
        tool_calls: None,
    };
    let json = serde_json::to_value(&delta).unwrap();
    assert!(json.get("role").is_none());
    assert_eq!(json["content"], "test");
}
