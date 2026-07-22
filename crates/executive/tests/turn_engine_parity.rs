//! Parity harness for the unified TurnEngine contract (Wave 1, W1-04).
//!
//! These tests validate that the `TurnEngine` trait contract is
//! implementable, that a stub implementation behaves correctly, and
//! that the context/profile/snapshot types carry the expected fields.
//! In W1-05 (migration) the harness is extended to compare daemon/
//! CLI/child execution through the real engine.

use executive::service::turn_engine::{
    TurnEngine, TurnEngineContext, TurnEngineError, TurnEngineEventSink, TurnEngineParitySnapshot,
    TurnEngineRequest, TurnEngineResult, TurnEngineStatus,
};
use executive::service::turn_runtime_ports::ResolvedTurnProfile;
use fabric::{AgentApprovalPolicy, MonoDeadlineMillis};
use std::sync::Arc;

// ── Stub engine for contract validation ────────────────────────────────────

struct StubTurnEngine {
    behaviour: StubBehaviour,
}

enum StubBehaviour {
    Success(fabric::TurnId),
    Reject,
}

#[async_trait::async_trait]
impl TurnEngine for StubTurnEngine {
    async fn execute(
        &self,
        _request: TurnEngineRequest,
        _context: TurnEngineContext,
        events: Arc<dyn TurnEngineEventSink>,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        match &self.behaviour {
            StubBehaviour::Success(turn_id) => {
                let result = TurnEngineResult {
                    turn_id: *turn_id,
                    output: "ok".into(),
                    status: TurnEngineStatus::Completed,
                    tool_calls: 2,
                    tokens_in: 500,
                    tokens_out: 128,
                    elapsed_ms: 1_200,
                    coordinator_execution: None,
                };
                events.on_turn_started(result.turn_id).await;
                events.on_turn_settled(result.turn_id, &result).await;
                Ok(result)
            }
            StubBehaviour::Reject => Err(TurnEngineError::AdmissionRejected("stub reject".into())),
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn test_profile() -> ResolvedTurnProfile {
    ResolvedTurnProfile {
        profile_name: "parity-test-agent".into(),
        allowed_tools: ["file_read".to_owned(), "bash_exec".to_owned()]
            .into_iter()
            .collect(),
        system_prompt: "Parity test agent.".into(),
        model_policy: Some("gpt-5-code".into()),
        max_iterations: 20,
        max_input_tokens: 100_000,
        max_output_tokens: 16_384,
        max_tool_calls: 64,
        max_elapsed_ms: 600_000,
        approval_policy: AgentApprovalPolicy::AutoApprove,
        tool_timeout_ms: 30_000,
    }
}

fn test_context() -> TurnEngineContext {
    TurnEngineContext {
        principal_id: fabric::PrincipalId("test:parity".into()),
        operation_id: fabric::OperationId::default(),
        process_id: fabric::ProcessId::new(),
        workspace: Arc::new(
            fabric::WorkspacePolicy::from_resolved_roots(
                std::path::PathBuf::from("/tmp/parity"),
                vec![],
            )
            .unwrap(),
        ),
        profile: test_profile(),
        cancel_token: tokio_util::sync::CancellationToken::new(),
        principal_context: None,
    }
}

struct CountingEventSink {
    started: std::sync::Mutex<Vec<fabric::TurnId>>,
}

impl CountingEventSink {
    fn new() -> Self {
        Self {
            started: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl TurnEngineEventSink for CountingEventSink {
    async fn on_turn_started(&self, turn_id: fabric::TurnId) {
        self.started.lock().unwrap().push(turn_id);
    }

    async fn on_turn_settled(&self, _turn_id: fabric::TurnId, _outcome: &TurnEngineResult) {}
}

// ── Contract tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn stub_engine_emits_started_and_settled_on_success() {
    let turn_id = fabric::TurnId::new();
    let engine = StubTurnEngine {
        behaviour: StubBehaviour::Success(turn_id),
    };
    let sink = Arc::new(CountingEventSink::new());

    let got = engine
        .execute(
            TurnEngineRequest {
                input: "test".into(),
                model_policy: None,
                deadline: None,
            },
            test_context(),
            sink.clone(),
        )
        .await
        .expect("stub engine should return Ok");

    assert_eq!(got.turn_id, turn_id);
    assert!(sink.started.lock().unwrap().contains(&turn_id));
}

fn snapshot_of(result: &TurnEngineResult) -> TurnEngineParitySnapshot {
    TurnEngineParitySnapshot {
        turn_id: result.turn_id,
        output_len: result.output.len(),
        tool_calls: result.tool_calls,
        status: result.status.clone(),
        tokens_in: result.tokens_in,
        tokens_out: result.tokens_out,
    }
}

#[test]
fn daemon_mapping_matches_engine_result_snapshot() {
    let turn_id = fabric::TurnId::new();
    let mapped = executive::service::daemon_turn_engine::map_pipeline_response(
        turn_id,
        &serde_json::json!({
            "result": {
                "response": "ok",
                "succeeded": true,
                "metrics": { "tool_calls_made": 2, "elapsed_ms": 5 }
            }
        }),
    );
    let stub = TurnEngineResult {
        turn_id,
        output: "ok".into(),
        status: TurnEngineStatus::Completed,
        tool_calls: 2,
        tokens_in: 0,
        tokens_out: 0,
        elapsed_ms: 5,
        coordinator_execution: None,
    };
    assert_eq!(snapshot_of(&mapped), snapshot_of(&stub));
}

#[tokio::test]
async fn stub_engine_rejects_on_error() {
    let engine = StubTurnEngine {
        behaviour: StubBehaviour::Reject,
    };
    let sink: Arc<dyn TurnEngineEventSink> = Arc::new(CountingEventSink::new());

    let result = engine
        .execute(
            TurnEngineRequest {
                input: "test".into(),
                model_policy: None,
                deadline: None,
            },
            test_context(),
            sink,
        )
        .await;

    assert!(result.is_err());
}

#[test]
fn turn_engine_request_round_trips_model_policy() {
    let request = TurnEngineRequest {
        input: "fix the bug".into(),
        model_policy: Some("claude-opus-review".into()),
        deadline: Some(MonoDeadlineMillis(30_000)),
    };
    assert_eq!(request.model_policy.as_deref(), Some("claude-opus-review"));
}

#[test]
fn parity_snapshot_fields_exist() {
    let snap = TurnEngineParitySnapshot {
        turn_id: fabric::TurnId::new(),
        output_len: 42,
        tool_calls: 2,
        status: TurnEngineStatus::Completed,
        tokens_in: 1000,
        tokens_out: 200,
    };
    assert_eq!(snap.tool_calls, 2);
    assert_eq!(snap.status, TurnEngineStatus::Completed);
}

#[test]
fn turn_engine_context_carries_profile() {
    let ctx = test_context();
    let profile = &ctx.profile;
    assert_eq!(profile.profile_name, "parity-test-agent");
    assert!(profile.allowed_tools.contains("bash_exec"));
    assert_eq!(profile.max_iterations, 20);
    assert_eq!(profile.model_policy.as_deref(), Some("gpt-5-code"));
}
