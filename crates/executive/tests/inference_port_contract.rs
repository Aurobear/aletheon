use std::collections::BTreeSet;
use std::sync::Arc;

use cognit::testing::mock_llm::MockLlmProvider;
use executive::application::inference_port::{CoreInferenceRequest, InferencePort, LocalInferencePort};
use fabric::{LlmStream, Message, StopReason, StreamChunk, ToolDefinition};

fn request() -> CoreInferenceRequest {
    CoreInferenceRequest {
        messages: vec![Message::user("hello")],
        tools: vec![ToolDefinition {
            name: "lookup".into(),
            description: "look up a value".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }],
        model_spec: "fast".into(),
    }
}

async fn next_chunk(stream: &mut LlmStream) -> Option<anyhow::Result<StreamChunk>> {
    std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
}

#[tokio::test]
async fn local_inference_port_preserves_response_and_stream_frames() {
    let provider = Arc::new(MockLlmProvider::new("fake"));
    provider.push_text_response("ok", StopReason::EndTurn);
    provider.push_text_response("streamed", StopReason::EndTurn);
    let port = LocalInferencePort::new(provider);

    let response = port.complete(request()).await.unwrap();
    assert_eq!(response.stop_reason, StopReason::EndTurn);

    let mut stream = port.stream(request()).await.unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = next_chunk(&mut stream).await {
        chunks.push(chunk.unwrap());
    }
    assert!(matches!(
        chunks.last(),
        Some(StreamChunk::Done {
            stop_reason: StopReason::EndTurn
        })
    ));
}

#[test]
fn core_inference_request_contains_no_identity_or_workspace_authority() {
    let value = serde_json::to_value(request()).unwrap();
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        BTreeSet::from([
            "messages".to_string(),
            "model_spec".to_string(),
            "tools".to_string(),
        ])
    );
    let wire = value.to_string();
    for forbidden in ["uid", "gid", "workspace", "working_dir"] {
        assert!(!wire.contains(forbidden), "request leaked {forbidden}");
    }
}
