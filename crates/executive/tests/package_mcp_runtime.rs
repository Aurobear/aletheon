use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use corpus::tools::tools::skill_tools::SharedSkills;
use executive::application::extension_coordinator::ExtensionCoordinator;
use executive::application::extension_snapshot::{ExtensionRuntimeView, ExtensionSnapshotCompiler};
use executive::host::daemon::bootstrap::extension_publisher::DaemonExtensionRuntimePublisher;
use flate2::{write::GzEncoder, Compression};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

async fn handle_mcp(request: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    if request.method() != Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Full::new(Bytes::new()))
            .unwrap());
    }
    let authorized = request
        .headers()
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        == Some("Bearer package-token");
    if !authorized {
        return Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Full::new(Bytes::from("unauthorized")))
            .unwrap());
    }
    let bytes = request
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    let request: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let result = match request.get("method").and_then(Value::as_str) {
        Some("initialize") => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "package-test", "version": "1.0.0"}
        }),
        Some("tools/list") => json!({
            "tools": [{
                "name": "search",
                "description": "Search package data",
                "inputSchema": {
                    "type": "object",
                    "properties": {"q": {"type": "string"}},
                    "required": ["q"]
                }
            }]
        }),
        Some("tools/call") => json!({
            "content": [{"type": "text", "text": "package search result"}]
        }),
        Some("resources/list") => json!({"resources": []}),
        _ => json!({}),
    };
    let response = json!({"jsonrpc":"2.0", "id":id, "result":result});
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(response.to_string())))
        .unwrap())
}

async fn start_mcp_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service_fn(handle_mcp))
                    .await;
            });
        }
    });
    address
}

fn package(root: &Path, endpoint: &str) -> PathBuf {
    let source = root.join("source");
    let connector = source.join("assets/connectors/search.json");
    fs::create_dir_all(connector.parent().unwrap()).unwrap();
    let manifest = r#"schema_version = 1
[package]
id = "aurb.core"
version = "1.0.0"
description = "MCP Connector runtime fixture"
compatibility = { min_aletheon = "0.1.0" }
[[assets]]
kind = "connector"
id = "connector.search"
path = "assets/connectors/search.json"
"#;
    let connector_body = json!({
        "schema_version": 1,
        "id": "aurb",
        "transport": {"kind":"streamable_http", "url":endpoint},
        "bearer_token_env": "ALETHEON_TEST_PACKAGE_MCP_TOKEN",
        "request_timeout_ms": 5000,
        "allowed_tools": ["search"],
        "allowed_resources": []
    })
    .to_string();
    fs::write(source.join("extension.toml"), manifest).unwrap();
    fs::write(&connector, &connector_body).unwrap();
    fs::write(
        source.join("checksums.sha256"),
        format!(
            "{:x}  extension.toml\n{:x}  assets/connectors/search.json\n",
            Sha256::digest(manifest.as_bytes()),
            Sha256::digest(connector_body.as_bytes())
        ),
    )
    .unwrap();
    let archive = root.join("aurb-core-mcp.tar.gz");
    let encoder = GzEncoder::new(fs::File::create(&archive).unwrap(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.append_dir_all(".", source).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    archive
}

fn tool_context() -> fabric::tool::ToolContext {
    fabric::tool::ToolContext {
        approval_authority: None,
        agent: None,
        working_dir: PathBuf::from("/tmp"),
        session_id: "package-mcp-test".into(),
        clock: Arc::new(kernel::chronos::TestClock::default()),
        turn_event_sender: None,
    }
}

#[tokio::test]
async fn packaged_mcp_connects_and_is_removed_on_disable() {
    let server = start_mcp_server().await;
    std::env::set_var("ALETHEON_TEST_PACKAGE_MCP_TOKEN", "package-token");
    let temp = TempDir::new().unwrap();
    let archive = package(temp.path(), &format!("http://{server}/mcp"));
    let tools = Arc::new(Mutex::new(corpus::ToolRegistry::new()));
    let hooks = Arc::new(Mutex::new(corpus::HookRegistry::new(Arc::new(
        kernel::chronos::TestClock::default(),
    ))));
    let publisher = Arc::new(DaemonExtensionRuntimePublisher::new(
        tools.clone(),
        hooks,
        SharedSkills::new(Arc::new(Vec::new())),
    ));
    let coordinator = ExtensionCoordinator::new(
        &temp.path().join("store"),
        ExtensionSnapshotCompiler::default(),
        publisher,
        ExtensionRuntimeView::default(),
        Arc::new(kernel::chronos::TestClock::default()),
    )
    .unwrap();
    coordinator
        .install("operator:test", &archive, false)
        .await
        .unwrap();
    coordinator
        .enable("operator:test", "aurb.core", true)
        .await
        .unwrap();

    let tool = tools.lock().await.get("aurb__search").cloned().unwrap();
    let result = tool.execute(json!({"q":"x"}), &tool_context()).await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("package search result"));

    coordinator
        .disable("operator:test", "aurb.core")
        .await
        .unwrap();
    assert!(tools.lock().await.get("aurb__search").is_none());
    std::env::remove_var("ALETHEON_TEST_PACKAGE_MCP_TOKEN");
}
