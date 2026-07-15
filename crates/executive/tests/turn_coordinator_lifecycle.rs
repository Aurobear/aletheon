use std::sync::Arc;

use aletheon_kernel::service::ServicePorts;
use async_trait::async_trait;
use executive::r#impl::session::canonical_store::CanonicalSessionStore;
use executive::service::harness_factory::CognitiveSessionFactory;
use executive::service::turn_coordinator::{TurnCoordinator, TurnExecution};
use executive::service::turn_policy::*;
use executive::service::{PostTurnPipeline, PreTurnPipeline, TurnService};
use fabric::{
    ItemPayload, OperationState, SessionAppendStore, SessionId, TurnMetrics, TurnRequest,
    TurnResult, TurnStop,
};

fn request(session: &str) -> TurnRequest {
    TurnRequest {
        operation_id: fabric::OperationId::default(),
        process_id: fabric::ProcessId::new(),
        session_id: session.into(),
        input: "hello".into(),
        working_dir: std::env::temp_dir(),
        model_policy: None,
        deadline: None,
    }
}

#[test]
fn policy_contains_all_mode_differences() {
    let daemon = TurnPolicy::daemon();
    let exec = TurnPolicy::exec();
    assert_eq!(daemon.persistence, PersistenceMode::Durable);
    assert_eq!(exec.persistence, PersistenceMode::Durable);
    assert_ne!(daemon.reviewer, exec.reviewer);
    assert_ne!(daemon.memory_eligible, exec.memory_eligible);
    assert_ne!(daemon.agora_available, exec.agora_available);
    assert_ne!(daemon.event_delivery, exec.event_delivery);
    assert_ne!(daemon.environment, exec.environment);
}

#[tokio::test]
async fn coordinator_owns_turn_operation_and_ordered_canonical_items() {
    let ports = ServicePorts::new();
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = TurnCoordinator::new(&ports, store.clone());
    let captured = Arc::new(tokio::sync::Mutex::new(None));
    let capture = captured.clone();
    let result = coordinator
        .submit_with(
            request("success"),
            &TurnPolicy::daemon(),
            move |request, _| async move {
                *capture.lock().await = Some(request.operation_id);
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "answer".into(),
                        stop: TurnStop::Completed,
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![
                        ItemPayload::ToolCall {
                            call_id: "c".into(),
                            name: "tool".into(),
                            input: serde_json::json!({}),
                        },
                        ItemPayload::ToolResult {
                            call_id: "c".into(),
                            content: "ok".into(),
                            is_error: false,
                            permit_id: None,
                            audit_id: None,
                        },
                    ],
                })
            },
        )
        .await
        .unwrap();
    assert_eq!(result.output, "answer");
    let operation = ports
        .operation_table
        .inspect(captured.lock().await.unwrap())
        .await
        .unwrap();
    assert_eq!(operation.kind, fabric::OperationKind::Turn);
    assert_eq!(operation.state, OperationState::Succeeded);
    let items = store
        .load_items(&SessionId("success".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 4);
    assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
    assert!(matches!(items[1].payload, ItemPayload::ToolCall { .. }));
    assert!(matches!(items[2].payload, ItemPayload::ToolResult { .. }));
    assert!(matches!(
        items[3].payload,
        ItemPayload::AssistantMessage { .. }
    ));
}

#[tokio::test]
async fn failure_is_terminal_and_remains_replayable() {
    let ports = ServicePorts::new();
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = TurnCoordinator::new(&ports, store.clone());
    let result = coordinator
        .submit_with(
            request("failed"),
            &TurnPolicy::exec(),
            |_request, _| async { anyhow::bail!("model unavailable") },
        )
        .await;
    assert!(result.is_err());
    let items = store
        .load_items(&SessionId("failed".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
    assert!(matches!(items[1].payload, ItemPayload::SystemNotice { .. }));
}

struct SeedCapturingFactory(Arc<tokio::sync::Mutex<Vec<usize>>>);

#[async_trait]
impl CognitiveSessionFactory for SeedCapturingFactory {
    async fn create(
        &self,
        _session: &fabric::SessionRecord,
        _policy: &TurnPolicy,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        Ok(Box::new(SeedCapturingSession(self.0.clone())))
    }
}

struct SeedCapturingSession(Arc<tokio::sync::Mutex<Vec<usize>>>);

#[async_trait]
impl cognit::harness::CognitiveSession for SeedCapturingSession {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn fabric::TurnServices,
        _events: &dyn fabric::TurnEventSink,
    ) -> anyhow::Result<TurnResult> {
        self.0
            .lock()
            .await
            .push(services.seed_messages(&request).len());
        Ok(TurnResult {
            output: format!("answer: {}", request.input),
            stop: TurnStop::Completed,
            metrics: TurnMetrics {
                completed_normally: true,
                ..Default::default()
            },
        })
    }
}

struct EmptyServices;

#[async_trait]
impl fabric::TurnServices for EmptyServices {
    async fn recall(&self, _request: fabric::RecallRequest) -> anyhow::Result<fabric::RecallSet> {
        Ok(Default::default())
    }
    async fn dasein_view(&self, _process: fabric::ProcessId) -> anyhow::Result<fabric::DaseinView> {
        Ok(Default::default())
    }
    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
        Ok(Default::default())
    }
    async fn invoke(&self, call: fabric::CapabilityCall) -> fabric::CapabilityResult {
        fabric::CapabilityResult {
            call_id: call.call_id,
            output: "unused".into(),
            is_error: true,
            usage: Default::default(),
            audit_id: None,
        }
    }
}

#[tokio::test]
async fn daemon_then_exec_restart_projects_prior_canonical_context() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sessions.db");
    let captures = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    for policy in [TurnPolicy::daemon(), TurnPolicy::exec()] {
        let ports = Arc::new(ServicePorts::new());
        let store: Arc<dyn SessionAppendStore> =
            Arc::new(CanonicalSessionStore::open(&db).unwrap());
        let coordinator = Arc::new(TurnCoordinator::new(ports.as_ref(), store));
        TurnService::new(
            Arc::new(EmptyServices),
            PreTurnPipeline,
            PostTurnPipeline,
            ports,
        )
        .with_coordinator(coordinator)
        .with_policy(policy)
        .with_session_factory(Arc::new(SeedCapturingFactory(captures.clone())))
        .submit(request("restart"), &fabric::NoopTurnEventSink)
        .await
        .unwrap();
    }
    let captures = captures.lock().await.clone();
    assert_eq!(captures[0], 0);
    assert_eq!(
        captures[1], 2,
        "second mode must receive prior user+assistant context"
    );
}
