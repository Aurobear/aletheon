use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cognit::config::McpMemoryConfig;
use corpus::tools::mcp::config::{McpConfig, McpServerConfig, McpTransportConfig, McpTrustLevel};
use corpus::tools::mcp::manager::McpManager;
use executive::r#impl::gbrain::build_gbrain_memory_runtime;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use mnemosyne::backends::gbrain::{
    EnqueueOutcome, GbrainBackendError, SupplementalErrorCategory, SupplementalRecall,
    SupplementalRecallHealth,
};
use mnemosyne::{
    CompositeMemoryService, ExperienceEvent, ForgetPolicy, MemoryMetadata, MemoryProvenance,
    MemorySensitivity, MemoryService, RecallItem, RecallRequest, RecallSet,
    SupplementalMemoryService, TemporalState,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

struct LocalMemory {
    records: Mutex<Vec<ExperienceEvent>>,
    recall: Mutex<RecallSet>,
}

#[async_trait]
impl MemoryService for LocalMemory {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()> {
        self.records.lock().unwrap().push(event);
        Ok(())
    }
    async fn recall(&self, _request: RecallRequest) -> anyhow::Result<RecallSet> {
        Ok(self.recall.lock().unwrap().clone())
    }
    async fn consolidate(&self, _scope: mnemosyne::service::MemoryScope) -> anyhow::Result<()> {
        Ok(())
    }
    async fn forget(&self, _policy: ForgetPolicy) -> anyhow::Result<mnemosyne::ForgetReceipt> {
        Ok(mnemosyne::ForgetReceipt::default())
    }
}

impl LocalMemory {
    fn new(items: Vec<RecallItem>) -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            recall: Mutex::new(RecallSet {
                items,
                degraded_sources: vec![],
            }),
        }
    }
}

struct FakeSupplemental {
    records: Mutex<Vec<ExperienceEvent>>,
    recall: SupplementalRecall,
    delay: Duration,
    record_error: bool,
}

#[async_trait]
impl SupplementalMemoryService for FakeSupplemental {
    fn queue_depth(&self) -> usize {
        3
    }
    fn record(
        &self,
        event: &ExperienceEvent,
        _now_ms: i64,
    ) -> Result<EnqueueOutcome, GbrainBackendError> {
        if self.record_error {
            return Err(GbrainBackendError::InvalidRecord);
        }
        self.records.lock().unwrap().push(event.clone());
        Ok(EnqueueOutcome::Inserted)
    }
    async fn recall(
        &self,
        _request: RecallRequest,
        _cancel: &CancellationToken,
    ) -> SupplementalRecall {
        tokio::time::sleep(self.delay).await;
        self.recall.clone()
    }
    fn forget(&self, _policy: ForgetPolicy) -> Result<(), GbrainBackendError> {
        Err(GbrainBackendError::Unsupported)
    }
}

fn metadata(record_id: &str, source_id: &str, observed_seconds: i64) -> MemoryMetadata {
    let observed = DateTime::<Utc>::from_timestamp(observed_seconds, 0).unwrap();
    MemoryMetadata {
        record_id: record_id.into(),
        provenance: MemoryProvenance {
            source: "goal_store".into(),
            source_id: source_id.into(),
            principal: Some("owner".into()),
            source_commit: Some("abc".into()),
        },
        source_time: Some(observed),
        observed_time: observed,
        valid_from: Some(observed),
        valid_until: None,
        supersedes: None,
        superseded_by: None,
        confidence: 0.9,
        sensitivity: MemorySensitivity::Internal,
    }
}

fn item(record_id: &str, source_id: &str, content: &str, observed: i64) -> RecallItem {
    RecallItem {
        content: content.into(),
        metadata: metadata(record_id, source_id, observed),
        temporal_state: TemporalState::Current,
        authority: mnemosyne::MemoryAuthority::AletheonExternal,
        scope: mnemosyne::MemoryScope::Session("s".into()),
    }
}

fn decision(id: &str) -> ExperienceEvent {
    ExperienceEvent::ArchitectureDecision {
        title: "Memory".into(),
        content: "Use MCP".into(),
        metadata: metadata(id, id, 1),
    }
}

fn request(historical: bool) -> RecallRequest {
    let mut request = RecallRequest::bounded("s", "memory");
    request.current_at = DateTime::<Utc>::from_timestamp(10, 0);
    request.include_historical = historical;
    request
}

#[tokio::test]
async fn disabled_and_unavailable_startup_keep_local_memory_operational() {
    let local = Arc::new(LocalMemory::new(vec![item(
        "local",
        "local",
        "local fact",
        1,
    )]));
    let cancel = CancellationToken::new();
    let runtime = build_gbrain_memory_runtime(
        local.clone(),
        None,
        &McpMemoryConfig::default(),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        &cancel,
    );
    assert!(!runtime.health.lock().unwrap().supplemental_enabled);
    runtime.memory_service.record(decision("d1")).await.unwrap();
    assert_eq!(local.records.lock().unwrap().len(), 1);

    let config = McpMemoryConfig {
        enabled: true,
        ..Default::default()
    };
    let runtime = build_gbrain_memory_runtime(
        local,
        None,
        &config,
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        &cancel,
    );
    assert!(runtime.health.lock().unwrap().degraded);
    assert_eq!(
        runtime
            .memory_service
            .recall(request(false))
            .await
            .unwrap()
            .texts(),
        vec!["local fact"]
    );
}

#[tokio::test]
async fn composite_records_local_first_selects_only_durable_types_and_survives_spool_error() {
    let local = Arc::new(LocalMemory::new(Vec::new()));
    let supplemental = Arc::new(FakeSupplemental {
        records: Mutex::new(Vec::new()),
        recall: SupplementalRecall {
            items: Vec::new(),
            health: healthy(),
        },
        delay: Duration::ZERO,
        record_error: false,
    });
    let composite = CompositeMemoryService::new(
        local.clone(),
        Some(supplemental.clone()),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        Duration::from_millis(50),
        Duration::from_millis(50),
    );
    composite.record(decision("d1")).await.unwrap();
    let message = ExperienceEvent::Message {
        session: "s".into(),
        role: "user".into(),
        content: "raw".into(),
        metadata: metadata("m1", "m1", 1),
    };
    composite.record(message).await.unwrap();
    assert_eq!(local.records.lock().unwrap().len(), 2);
    assert_eq!(supplemental.records.lock().unwrap().len(), 1);

    let failing = Arc::new(FakeSupplemental {
        records: Mutex::new(Vec::new()),
        recall: SupplementalRecall {
            items: Vec::new(),
            health: healthy(),
        },
        delay: Duration::ZERO,
        record_error: true,
    });
    let composite = CompositeMemoryService::new(
        local.clone(),
        Some(failing),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        Duration::from_millis(50),
        Duration::from_millis(50),
    );
    composite.record(decision("d2")).await.unwrap();
    assert!(composite.health_handle().lock().unwrap().degraded);
    assert_eq!(local.records.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn merge_prefers_new_valid_remote_and_historical_mode_retains_superseded() {
    let old = item("decision-v1", "adr", "old", 1);
    let mut new = item("decision-v2", "adr", "new", 2);
    new.metadata.supersedes = Some("decision-v1".into());
    let local = Arc::new(LocalMemory::new(vec![old]));
    let supplemental = Arc::new(FakeSupplemental {
        records: Mutex::new(Vec::new()),
        recall: SupplementalRecall {
            items: vec![new],
            health: healthy(),
        },
        delay: Duration::ZERO,
        record_error: false,
    });
    let composite = CompositeMemoryService::new(
        local,
        Some(supplemental),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        Duration::from_millis(50),
        Duration::from_millis(50),
    );
    assert_eq!(
        composite.recall(request(false)).await.unwrap().texts(),
        vec!["new"]
    );
    let historical = composite.recall(request(true)).await.unwrap();
    assert_eq!(historical.items.len(), 2);
    assert!(historical
        .items
        .iter()
        .any(|value| value.temporal_state == TemporalState::Superseded));
}

#[tokio::test]
async fn slow_or_malformed_supplemental_recall_falls_back_to_local_with_health() {
    let local = Arc::new(LocalMemory::new(vec![item("local", "local", "local", 1)]));
    let supplemental = Arc::new(FakeSupplemental {
        records: Mutex::new(Vec::new()),
        recall: SupplementalRecall {
            items: Vec::new(),
            health: SupplementalRecallHealth {
                degraded: true,
                error_category: Some(SupplementalErrorCategory::MalformedResponse),
                queue_depth: 0,
            },
        },
        delay: Duration::from_millis(100),
        record_error: false,
    });
    let composite = CompositeMemoryService::new(
        local,
        Some(supplemental),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        Duration::from_millis(50),
        Duration::from_millis(5),
    );
    assert_eq!(
        composite.recall(request(false)).await.unwrap().texts(),
        vec!["local"]
    );
    let health = composite.health_handle();
    {
        let value = health.lock().unwrap();
        assert!(value.degraded);
        assert_eq!(
            value.error_category,
            Some(SupplementalErrorCategory::Timeout)
        );
    }

    let local = Arc::new(LocalMemory::new(vec![item("local", "local", "local", 1)]));
    let malformed = Arc::new(FakeSupplemental {
        records: Mutex::new(Vec::new()),
        recall: SupplementalRecall {
            items: Vec::new(),
            health: SupplementalRecallHealth {
                degraded: true,
                error_category: Some(SupplementalErrorCategory::MalformedResponse),
                queue_depth: 0,
            },
        },
        delay: Duration::ZERO,
        record_error: false,
    });
    let composite = CompositeMemoryService::new(
        local,
        Some(malformed),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        Duration::from_millis(50),
        Duration::from_millis(50),
    );
    assert_eq!(
        composite.recall(request(false)).await.unwrap().texts(),
        vec!["local"]
    );
    assert_eq!(
        composite.health_handle().lock().unwrap().error_category,
        Some(SupplementalErrorCategory::MalformedResponse)
    );
}

#[tokio::test]
async fn schema_drift_is_local_only_and_marked_degraded() {
    let manager = Arc::new(McpManager::new(McpConfig::default()));
    let config = McpMemoryConfig {
        enabled: true,
        ..Default::default()
    };
    let runtime = build_gbrain_memory_runtime(
        Arc::new(LocalMemory::new(Vec::new())),
        Some(manager),
        &config,
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        &CancellationToken::new(),
    );
    let health = runtime.health.lock().unwrap();
    assert!(health.degraded);
    assert_eq!(
        health.error_category,
        Some(SupplementalErrorCategory::Schema)
    );
}

#[tokio::test]
async fn healthy_http_bootstrap_and_shutdown_leave_committed_queue_durable() {
    let state = HttpState::valid();
    let manager = connected_manager(state).await;
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let config = McpMemoryConfig {
        enabled: true,
        projection_enabled: true,
        spool_path: dir.path().join("spool.db").to_string_lossy().into_owned(),
        legacy_outbox_dir: dir.path().join("legacy").to_string_lossy().into_owned(),
        ..Default::default()
    };
    let runtime = build_gbrain_memory_runtime(
        Arc::new(LocalMemory::new(Vec::new())),
        Some(Arc::new(manager)),
        &config,
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        &cancel,
    );
    assert!(!runtime.health.lock().unwrap().degraded);
    runtime
        .memory_service
        .record(decision("queued-decision"))
        .await
        .unwrap();
    let task = runtime.worker_task.unwrap();
    tokio::time::timeout(Duration::from_millis(100), task)
        .await
        .unwrap()
        .unwrap();
    let spool = mnemosyne::backends::gbrain::GbrainSpool::open(
        &config.spool_path,
        mnemosyne::backends::gbrain::SpoolLimits {
            max_items: config.spool_max_items,
            max_bytes: config.spool_max_bytes,
        },
    )
    .unwrap();
    assert_eq!(spool.queue_depth().unwrap(), 1);
}

#[tokio::test]
async fn legacy_config_and_json_outbox_migrate_before_worker_start() {
    let parsed: McpMemoryConfig = toml::from_str(
        r#"
enabled = true
source = "aletheon"
timeout_ms = 75
max_results = 3
max_chars = 4096
capture_enabled = true
outbox_dir = "/tmp/overridden-below"
"#,
    )
    .unwrap();
    assert_eq!(parsed.write_source, "aletheon");
    assert_eq!(parsed.request_timeout_ms, 75);
    assert!(parsed.projection_enabled);

    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("legacy");
    std::fs::create_dir(&legacy).unwrap();
    std::fs::write(
        legacy.join("one.json"),
        json!({
            "slug":"aletheon/sessions/2026-07-15-one",
            "markdown":"legacy summary",
            "attempts":0,
            "next_attempt_at":0.0,
            "last_error":""
        })
        .to_string(),
    )
    .unwrap();
    let config = McpMemoryConfig {
        spool_path: dir.path().join("spool.db").to_string_lossy().into_owned(),
        legacy_outbox_dir: legacy.to_string_lossy().into_owned(),
        ..parsed
    };
    let cancel = CancellationToken::new();
    cancel.cancel();
    let runtime = build_gbrain_memory_runtime(
        Arc::new(LocalMemory::new(Vec::new())),
        Some(Arc::new(connected_manager(HttpState::valid()).await)),
        &config,
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
        &cancel,
    );
    assert!(legacy.join("one.json.migrated").exists());
    runtime.worker_task.unwrap().await.unwrap();
    let spool = mnemosyne::backends::gbrain::GbrainSpool::open(
        &config.spool_path,
        mnemosyne::backends::gbrain::SpoolLimits {
            max_items: config.spool_max_items,
            max_bytes: config.spool_max_bytes,
        },
    )
    .unwrap();
    assert_eq!(spool.queue_depth().unwrap(), 1);
}

fn healthy() -> SupplementalRecallHealth {
    SupplementalRecallHealth {
        degraded: false,
        error_category: None,
        queue_depth: 0,
    }
}

#[derive(Clone)]
struct HttpState {
    tools: Value,
}
impl HttpState {
    fn valid() -> Self {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../config/gbrain/tools-schema.json")).unwrap();
        Self {
            tools: fixture["result"].clone(),
        }
    }
}

async fn connected_manager(state: HttpState) -> McpManager {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| http_handler(state.clone(), request));
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    let mut manager = McpManager::new(McpConfig {
        servers: vec![McpServerConfig {
            name: "gbrain".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: format!("http://{addr}/mcp"),
            },
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
    manager
}

async fn http_handler(
    state: HttpState,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let body = request.collect().await.unwrap().to_bytes();
    let request: Value = serde_json::from_slice(&body).unwrap();
    let method = request["method"].as_str().unwrap();
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1"}})
        }
        "tools/list" => state.tools,
        "tools/call" => json!({"content":[{"type":"text","text":"{\"ok\":true}"}]}),
        _ => Value::Null,
    };
    let body = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap())
}
