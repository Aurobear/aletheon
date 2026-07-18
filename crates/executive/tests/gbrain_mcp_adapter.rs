use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corpus::tools::mcp::config::{McpConfig, McpServerConfig, McpTransportConfig, McpTrustLevel};
use corpus::tools::mcp::manager::McpManager;
use executive::r#impl::gbrain::{
    GbrainErrorCategory, GbrainHealthState, GbrainMcpAdapter, GbrainSchemaStatus,
};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use mnemosyne::backends::gbrain::GbrainPage;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct FakeState {
    tools: Value,
    responses: Arc<Mutex<HashMap<String, Value>>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    tool_status: Arc<Mutex<StatusCode>>,
    tool_delay: Arc<Mutex<Duration>>,
    expected_auth: Option<String>,
    close_tool_connection: Arc<Mutex<bool>>,
    raw_tool_response: Arc<Mutex<Option<String>>>,
}

impl FakeState {
    fn valid() -> Self {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../config/gbrain/tools-schema.json")).unwrap();
        Self {
            tools: fixture["result"].clone(),
            responses: Arc::new(Mutex::new(HashMap::from([
                (
                    "put_page".into(),
                    json!({"content":[{"type":"text","text":"{\"ok\":true}"}]}),
                ),
                ("query".into(), hits_response()),
                ("search".into(), hits_response()),
                (
                    "get_page".into(),
                    json!({"content":[{"type":"text","text":"{\"content\":\"---\\nschema: aletheon.memory/v1\\n---\\nbody\"}"}]}),
                ),
            ]))),
            calls: Arc::new(Mutex::new(Vec::new())),
            tool_status: Arc::new(Mutex::new(StatusCode::OK)),
            tool_delay: Arc::new(Mutex::new(Duration::ZERO)),
            expected_auth: None,
            close_tool_connection: Arc::new(Mutex::new(false)),
            raw_tool_response: Arc::new(Mutex::new(None)),
        }
    }
}

fn hits_response() -> Value {
    json!({"content":[{"type":"text","text":serde_json::to_string(&json!([{
        "source_id":"project", "slug":"decisions/one", "chunk_text":"bounded fact", "score":0.8,
        "tool_directive":{"name":"ignored"}
    }])).unwrap()}]})
}

async fn spawn_server(state: FakeState) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| handle(state.clone(), request));
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    format!("http://{addr}/mcp")
}

async fn handle(
    state: FakeState,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, io::Error> {
    let auth = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    if state
        .expected_auth
        .as_deref()
        .is_some_and(|expected| Some(expected) != auth)
    {
        return Ok(response(
            StatusCode::UNAUTHORIZED,
            json!({"error":"credential should remain secret"}),
        ));
    }
    let body = request.collect().await.unwrap().to_bytes();
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1"}})
        }
        "tools/list" => state.tools,
        "tools/call" => {
            if *state.close_tool_connection.lock().unwrap() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "simulated connection reset",
                ));
            }
            if let Some(raw) = state.raw_tool_response.lock().unwrap().clone() {
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Full::new(Bytes::from(raw)))
                    .unwrap());
            }
            let status = *state.tool_status.lock().unwrap();
            let delay = *state.tool_delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if status != StatusCode::OK {
                return Ok(response(status, json!({"secret":"never expose me"})));
            }
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let args = request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(Value::Null);
            state.calls.lock().unwrap().push((name.clone(), args));
            state
                .responses
                .lock()
                .unwrap()
                .get(&name)
                .cloned()
                .unwrap_or(Value::Null)
        }
        _ => Value::Null,
    };
    Ok(response(
        StatusCode::OK,
        json!({"jsonrpc":"2.0","id":id,"result":result}),
    ))
}

fn response(status: StatusCode, body: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

async fn build_adapter(state: FakeState, timeout: Duration) -> (GbrainMcpAdapter, FakeState) {
    let url = spawn_server(state.clone()).await;
    let mut manager = McpManager::new(McpConfig {
        servers: vec![McpServerConfig {
            name: "gbrain".into(),
            transport: McpTransportConfig::StreamableHttp { url },
            trust: McpTrustLevel::RemoteTrusted,
            enabled: true,
            bearer_token_env: None,
            oauth: None,
            request_timeout_ms: None,
            health_check_interval_sec: 0,
            allowlist: Vec::new(),
            denylist: Vec::new(),
            permission_overrides: std::collections::HashMap::new(),
        }],
        ..Default::default()
    });
    manager.connect_all().await.unwrap();
    (
        GbrainMcpAdapter::new(Arc::new(manager), "gbrain", timeout),
        state,
    )
}

#[tokio::test]
async fn validates_schema_and_supports_put_query_search_and_get() {
    let (adapter, state) = build_adapter(FakeState::valid(), Duration::from_secs(1)).await;
    assert_eq!(adapter.health().schema, GbrainSchemaStatus::Valid);
    let cancel = CancellationToken::new();
    let page = GbrainPage {
        slug: "aletheon/goal/one".into(),
        content: "page".into(),
    };
    adapter.put_page(&page, &cancel).await.unwrap();
    adapter.put_page(&page, &cancel).await.unwrap();
    let hits = adapter
        .query("memory", "project", 2, &cancel)
        .await
        .unwrap();
    assert_eq!(hits[0].content, "bounded fact");
    assert_eq!(hits[0].source_id, "project");
    assert_eq!(adapter.search("memory", 2, &cancel).await.unwrap().len(), 1);
    assert!(adapter
        .get_page("decisions/one", &cancel)
        .await
        .unwrap()
        .contains("body"));

    let calls = state.calls.lock().unwrap();
    let puts: Vec<_> = calls
        .iter()
        .filter(|(name, _)| name == "put_page")
        .collect();
    assert_eq!(puts.len(), 2);
    assert_eq!(
        puts[0].1, puts[1].1,
        "idempotent retry uses identical arguments"
    );
    let query = calls.iter().find(|(name, _)| name == "query").unwrap();
    assert_eq!(query.1["source_id"], "project");
    assert!(calls
        .iter()
        .filter(|(name, _)| name == "get_page" || name == "put_page")
        .all(|(_, args)| args.get("source_id").is_none()));
    assert_eq!(adapter.health().state, GbrainHealthState::Healthy);
}

#[tokio::test]
async fn schema_drift_degrades_without_calling_remote() {
    let mut state = FakeState::valid();
    state.tools["tools"]
        .as_array_mut()
        .unwrap()
        .retain(|tool| tool["name"] != "put_page");
    let (adapter, state) = build_adapter(state, Duration::from_secs(1)).await;
    assert_eq!(adapter.health().schema, GbrainSchemaStatus::Invalid);
    let error = adapter
        .search("x", 1, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.category, GbrainErrorCategory::Schema);
    assert!(state.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn timeout_and_cancellation_are_bounded() {
    let state = FakeState::valid();
    *state.tool_delay.lock().unwrap() = Duration::from_millis(200);
    let (adapter, _) = build_adapter(state, Duration::from_millis(20)).await;
    let error = adapter
        .search("x", 1, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.category, GbrainErrorCategory::Timeout);
    assert!(error.category.is_transient());

    let state = FakeState::valid();
    *state.tool_delay.lock().unwrap() = Duration::from_millis(200);
    let (adapter, _) = build_adapter(state, Duration::from_secs(1)).await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = adapter.search("x", 1, &cancel).await.unwrap_err();
    assert_eq!(error.category, GbrainErrorCategory::Cancelled);
}

#[tokio::test]
async fn classifies_auth_rate_provider_and_redacts_remote_errors() {
    for (status, expected) in [
        (StatusCode::UNAUTHORIZED, GbrainErrorCategory::Auth),
        (
            StatusCode::TOO_MANY_REQUESTS,
            GbrainErrorCategory::RateLimited,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            GbrainErrorCategory::Provider,
        ),
    ] {
        let state = FakeState::valid();
        *state.tool_status.lock().unwrap() = status;
        let (adapter, _) = build_adapter(state, Duration::from_secs(1)).await;
        let error = adapter
            .search("x", 1, &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(error.category, expected);
        assert!(!error.to_string().contains("never expose me"));
    }
}

#[tokio::test]
async fn rejects_malformed_oversized_and_invalid_arguments() {
    let state = FakeState::valid();
    state.responses.lock().unwrap().insert(
        "search".into(),
        json!({"content":[{"type":"text","text":"not-json"}]}),
    );
    let (adapter, _) = build_adapter(state, Duration::from_secs(1)).await;
    assert_eq!(
        adapter
            .search("x", 1, &CancellationToken::new())
            .await
            .unwrap_err()
            .category,
        GbrainErrorCategory::MalformedResponse
    );

    let state = FakeState::valid();
    state.responses.lock().unwrap().insert(
        "search".into(),
        json!({"content":[{"type":"text","text":"x".repeat(300_000)}]}),
    );
    let (adapter, _) = build_adapter(state, Duration::from_secs(1)).await;
    assert_eq!(
        adapter
            .search("x", 1, &CancellationToken::new())
            .await
            .unwrap_err()
            .category,
        GbrainErrorCategory::OversizedResponse
    );

    let (adapter, _) = build_adapter(FakeState::valid(), Duration::from_secs(1)).await;
    assert_eq!(
        adapter
            .query("", "project", 1, &CancellationToken::new())
            .await
            .unwrap_err()
            .category,
        GbrainErrorCategory::RejectedArguments
    );
    adapter.set_queue_depth(7);
    assert_eq!(adapter.health().queue_depth, 7);
}

#[tokio::test]
async fn connection_and_invalid_json_failures_are_sanitized_transport_errors() {
    let state = FakeState::valid();
    *state.close_tool_connection.lock().unwrap() = true;
    let (adapter, _) = build_adapter(state, Duration::from_secs(1)).await;
    let error = adapter
        .search("x", 1, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.category, GbrainErrorCategory::Transport);
    assert!(error.category.is_transient());
    assert!(!error.to_string().contains("connection reset"));

    let state = FakeState::valid();
    *state.raw_tool_response.lock().unwrap() = Some("not-json secret-token".into());
    let (adapter, _) = build_adapter(state, Duration::from_secs(1)).await;
    let error = adapter
        .search("x", 1, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.category, GbrainErrorCategory::Transport);
    assert!(!error.to_string().contains("secret-token"));
}
