#![cfg(feature = "test-support")]
//! Parity harness for the unified TurnEngine contract (Wave 1, W1-04).
//!
//! These tests validate that the `TurnEngine` trait contract is
//! implementable, that a stub implementation behaves correctly, and
//! that the context/profile/snapshot types carry the expected fields.
//! In W1-05 (migration) the harness is extended to compare daemon/
//! CLI/child execution through the real engine.

use ::contracts::{AgentApprovalPolicy, MonoDeadlineMillis};
use application::turn::settings::ResolvedTurnProfile;
use application::turn::{
    TurnEngine, TurnEngineContext, TurnEngineError, TurnEngineParitySnapshot, TurnEngineRequest,
    TurnEngineResult,
};
use std::sync::Arc;

// ── Stub engine for contract validation ────────────────────────────────────

struct StubTurnEngine {
    behaviour: StubBehaviour,
}

enum StubBehaviour {
    Success(::contracts::TurnId),
    Reject,
}

#[async_trait::async_trait]
impl TurnEngine for StubTurnEngine {
    async fn execute(
        &self,
        _request: TurnEngineRequest,
        _context: TurnEngineContext,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        match &self.behaviour {
            StubBehaviour::Success(turn_id) => {
                let result = TurnEngineResult {
                    turn_id: *turn_id,
                    output: "ok".into(),
                    stop: ::contracts::TurnStop::Completed,
                    failure: None,
                    tool_calls: 2,
                    usage: ::contracts::InferenceUsage::reported(
                        500,
                        128,
                        Some(500),
                        Some(0),
                        Some(0),
                    ),
                    elapsed_ms: 1_200,
                    coordinator_execution: None,
                };
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
        delegated_tools: ["file_read".to_owned(), "bash_exec".to_owned()]
            .into_iter()
            .collect(),
        system_prompt: "Parity test agent.".into(),
        model_policy: Some("gpt-5-code".into()),
        max_iterations: 20,
        max_input_tokens: 100_000,
        max_output_tokens: 16_384,
        tool_schema_tokens: 0.into(),
        max_tool_calls: 64,
        max_elapsed_ms: 600_000,
        approval_policy: AgentApprovalPolicy::AutoApprove,
        tool_timeout_ms: 30_000,
    }
}

fn test_context() -> TurnEngineContext {
    TurnEngineContext {
        principal_id: ::contracts::PrincipalId("test:parity".into()),
        operation_id: ::contracts::OperationId::default(),
        process_id: ::contracts::ProcessId::new(),
        workspace: Arc::new(
            ::contracts::WorkspacePolicy::from_resolved_roots(
                std::path::PathBuf::from("/tmp/parity"),
                vec![],
            )
            .unwrap(),
        ),
        profile: test_profile(),
        cancel_token: tokio_util::sync::CancellationToken::new(),
        notification: None,
        principal_context: None,
    }
}

// ── Contract tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn stub_engine_returns_one_typed_outcome_on_success() {
    let turn_id = ::contracts::TurnId::new();
    let engine = StubTurnEngine {
        behaviour: StubBehaviour::Success(turn_id),
    };
    let got = engine
        .execute(
            TurnEngineRequest {
                input: "test".into(),
                execution_target: ::contracts::ExecutionTargetSelection::default(),
                model_policy: None,
                deadline: None,
                requirements: Vec::new(),
                requested_task_kind: None,
            },
            test_context(),
        )
        .await
        .expect("stub engine should return Ok");

    assert_eq!(got.turn_id, turn_id);
    assert_eq!(got.stop, ::contracts::TurnStop::Completed);
}

fn snapshot_of(result: &TurnEngineResult) -> TurnEngineParitySnapshot {
    TurnEngineParitySnapshot {
        turn_id: result.turn_id,
        output_len: result.output.len(),
        tool_calls: result.tool_calls,
        stop: result.stop.clone(),
        failure: result.failure.clone(),
        usage: result.usage.clone(),
    }
}

#[test]
fn daemon_mapping_matches_engine_result_snapshot() {
    let turn_id = ::contracts::TurnId::new();
    let mapped = aletheon::daemon::turn_engine::map_turn_execution(
        turn_id,
        application::turn::coordinator::TurnExecution {
            result: ::contracts::TurnResult {
                output: "ok".into(),
                stop: ::contracts::TurnStop::Completed,
                failure: None,
                usage: Default::default(),
                metrics: ::contracts::TurnMetrics {
                    tool_calls_made: 2,
                    elapsed_ms: 5,
                    completed_normally: true,
                    ..Default::default()
                },
            },
            items: Vec::new(),
            projection: None,
            context_projection: None,
            evaluation_artifacts: Default::default(),
        },
    );
    let stub = TurnEngineResult {
        turn_id,
        output: "ok".into(),
        stop: ::contracts::TurnStop::Completed,
        failure: None,
        tool_calls: 2,
        usage: ::contracts::InferenceUsage::default(),
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
    let result = engine
        .execute(
            TurnEngineRequest {
                input: "test".into(),
                execution_target: ::contracts::ExecutionTargetSelection::default(),
                model_policy: None,
                deadline: None,
                requirements: Vec::new(),
                requested_task_kind: None,
            },
            test_context(),
        )
        .await;

    assert!(result.is_err());
}

#[test]
fn turn_engine_request_round_trips_model_policy() {
    let request = TurnEngineRequest {
        input: "fix the bug".into(),
        execution_target: ::contracts::ExecutionTargetSelection::default(),
        model_policy: Some("claude-opus-review".into()),
        deadline: Some(MonoDeadlineMillis(30_000)),
        requirements: Vec::new(),
        requested_task_kind: Some(::contracts::TaskKind::Coding),
    };
    assert_eq!(request.model_policy.as_deref(), Some("claude-opus-review"));
    assert_eq!(
        request.requested_task_kind,
        Some(::contracts::TaskKind::Coding)
    );
}

#[test]
fn parity_snapshot_fields_exist() {
    let snap = TurnEngineParitySnapshot {
        turn_id: ::contracts::TurnId::new(),
        output_len: 42,
        tool_calls: 2,
        stop: ::contracts::TurnStop::Completed,
        failure: None,
        usage: ::contracts::InferenceUsage::reported(1000, 200, None, None, None),
    };
    assert_eq!(snap.tool_calls, 2);
    assert_eq!(snap.stop, ::contracts::TurnStop::Completed);
    assert_eq!(snap.usage.total_input_tokens, Some(1000));
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

#[test]
fn missing_authenticated_principal_context_fails_closed() {
    let error = test_context()
        .require_principal_context()
        .expect_err("missing authenticated context must be rejected");
    assert!(matches!(error, TurnEngineError::InvalidContext(_)));
    assert_eq!(error.code(), "turn_context_invalid");
    assert!(!error.retryable());
}
