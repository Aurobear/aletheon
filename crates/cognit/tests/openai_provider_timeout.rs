use std::time::{Duration, Instant};

use cognit::config::{CognitConfig, ProviderTimeoutConfig};
use fabric::{LlmProvider, Message};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn server(response: Option<&'static [u8]>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await;
        if let Some(response) = response {
            socket.write_all(response).await.unwrap();
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    format!("http://{address}")
}

fn timeouts() -> ProviderTimeoutConfig {
    ProviderTimeoutConfig {
        connect_timeout_ms: 50,
        request_timeout_ms: 80,
        stream_idle_timeout_ms: 50,
    }
}

fn provider(base_url: String) -> std::sync::Arc<dyn LlmProvider> {
    let config = cognit::config::ProviderConfig {
        name: "test-adapter".into(), base_url, api_key: "secret-api-key".into(),
        transport: cognit::config::Transport::Openai, models: vec!["test-model".into()],
        max_context_length: None, pricing: None,
    };
    cognit::composition::inference_factory::create_provider(
        &config, "test-model",
        cognit::composition::inference_factory::ProviderBuildOptions { max_tokens: 100_000, timeouts: timeouts() },
    ).unwrap()
}

#[test]
fn provider_timeout_config_has_safe_defaults_and_rejects_invalid_bounds() {
    let parsed: CognitConfig = toml::from_str(
        r#"
[agent.provider_timeouts]
connect_timeout_ms = 25
request_timeout_ms = 75
stream_idle_timeout_ms = 30
"#,
    )
    .unwrap();
    assert_eq!(parsed.agent.provider_timeouts.request_timeout_ms, 75);
    parsed.validate().unwrap();

    let defaults = ProviderTimeoutConfig::default();
    assert!(defaults.connect_timeout_ms > 0);
    assert!(defaults.stream_idle_timeout_ms < defaults.request_timeout_ms);

    let mut invalid = parsed;
    invalid.agent.provider_timeouts.request_timeout_ms = 0;
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn non_stream_request_is_bounded_and_classified_without_body_leakage() {
    let base_url = server(None).await;
    let provider = provider(base_url);
    let started = Instant::now();
    let error = provider
        .complete(&[Message::user("hello")], &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(error, "provider_timeout");
    assert!(!error.contains("secret-api-key"));
}

#[tokio::test]
async fn stream_header_and_chunk_idle_are_bounded() {
    let header_provider = provider(server(None).await);
    let header_error = match header_provider
        .complete_stream(&[Message::user("hello")], &[])
        .await
    {
        Ok(_) => panic!("stream header unexpectedly completed"),
        Err(error) => error.to_string(),
    };
    assert_eq!(header_error, "provider_timeout");

    let response =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
    let idle_provider = provider(server(Some(response)).await);
    let mut stream = idle_provider
        .complete_stream(&[Message::user("hello")], &[])
        .await
        .unwrap();
    let error = stream.next().await.unwrap().unwrap_err().to_string();
    assert_eq!(error, "provider_timeout");
}

#[tokio::test]
async fn provider_http_error_never_includes_response_body() {
    let secret = "provider-body-secret";
    let response = Box::leak(
        format!(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            secret.len(), secret
        )
        .into_bytes()
        .into_boxed_slice(),
    );
    let provider = provider(server(Some(response)).await);
    let error = provider
        .complete(&[Message::user("hello")], &[])
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, "provider_unavailable");
    assert!(!error.contains(secret));
}
