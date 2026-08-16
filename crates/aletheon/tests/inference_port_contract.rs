use std::collections::BTreeSet;
use std::sync::Arc;

use ::contracts::{LlmResponse, LlmStream, Message, StopReason, StreamChunk, ToolDefinition};
use cognit::ports::inference::{
    CoreInferenceRequest, InferenceError, InferencePort, LocalInferencePort,
};
use cognit::testing::mock_llm::MockLlmProvider;

struct NoAuthorityPort;

#[async_trait::async_trait]
impl InferencePort for NoAuthorityPort {
    async fn complete(
        &self,
        _request: CoreInferenceRequest,
    ) -> Result<LlmResponse, InferenceError> {
        Err(anyhow::anyhow!("fixture completion unavailable").into())
    }

    async fn stream(&self, _request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        Err(anyhow::anyhow!("fixture stream unavailable").into())
    }
}

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
    let provider = Arc::new(MockLlmProvider::new("fake").with_max_context(1_000_000));
    provider.push_text_response("ok", StopReason::EndTurn);
    provider.push_text_response("streamed", StopReason::EndTurn);
    let port = LocalInferencePort::new(provider);

    let capabilities = port.capabilities("fast").await.unwrap();
    assert_eq!(capabilities.model_spec, "fast");
    assert_eq!(capabilities.display_name, "fake");
    assert_eq!(capabilities.max_context_tokens, 1_000_000);

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

#[tokio::test]
async fn unimplemented_provider_authority_fails_closed_instead_of_using_local_state() {
    let port = NoAuthorityPort;
    let error = match port.acquire_provider_permit("provider::fixture").await {
        Ok(_) => panic!("a port without machine authority must not create a local permit"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("provider machine authority unavailable"));
    let error = port
        .provider_backpressure_metrics()
        .await
        .expect_err("metrics must not silently report an unrelated local registry");
    assert!(error
        .to_string()
        .contains("provider machine authority unavailable"));
}
