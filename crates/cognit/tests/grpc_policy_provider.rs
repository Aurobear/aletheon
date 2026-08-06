//! In-process protocol acceptance for the real gRPC Policy client.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cognit::harness::robot::proposal_validator::validate_proposal;
use cognit::harness::robot::state::{AttemptSummary, ReplanContext};
use cognit::policy::grpc_provider::{GrpcPolicyConfig, GrpcPolicyProvider};
use cognit::policy::wire::v1::policy_gateway_server::{PolicyGateway, PolicyGatewayServer};
use cognit::policy::wire::v1::{
    GetCapabilitiesRequest, GetCapabilitiesResponse, HealthRequest, HealthResponse, ProposeRequest,
    ProposeResponse, SkillProposalWire,
};
use cognit::ports::policy_provider::{PolicyProviderError, PolicyProviderPort};
use fabric::types::embodiment::{DeviceId, RiskClass, SkillDescriptor, SkillId};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::frame::FrameRef;
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::robot_failure::RobotFailureClass;
use fabric::types::world_state::WorldSnapshot;
use fabric::MonoTime;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

#[derive(Clone, Copy)]
enum ResponseMode {
    Valid,
    Empty,
    Slow,
    ProviderMismatch,
}

#[derive(Clone)]
struct FixtureGateway {
    mode: ResponseMode,
    calls: Arc<AtomicUsize>,
    received: Arc<Mutex<Option<ProposeRequest>>>,
}

#[tonic::async_trait]
impl PolicyGateway for FixtureGateway {
    async fn get_capabilities(
        &self,
        request: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<GetCapabilitiesResponse>, Status> {
        let requested = request.into_inner().protocol_version;
        Ok(Response::new(GetCapabilitiesResponse {
            protocol_version: requested,
            provider_id: "fixture-policy".into(),
            max_proposals: 4,
        }))
    }

    async fn propose(
        &self,
        request: Request<ProposeRequest>,
    ) -> Result<Response<ProposeResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let request = request.into_inner();
        *self.received.lock().unwrap() = Some(request.clone());
        if matches!(self.mode, ResponseMode::Slow) {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let proposals = if matches!(
            self.mode,
            ResponseMode::Valid | ResponseMode::Slow | ResponseMode::ProviderMismatch
        ) {
            vec![SkillProposalWire {
                skill_id: "kuavo.stance".into(),
                device_id: "bot".into(),
                parameters: Some(json_struct(&serde_json::json!({}))),
                expected_outcome: Some(json_struct(
                    &serde_json::to_value(ExpectedOutcome {
                        predicate: OutcomePredicate::Equals {
                            path: "mode".into(),
                            value: serde_json::json!("stance"),
                        },
                        freshness_ms: 500,
                        stable_window_ms: 100,
                        timeout_ms: 5_000,
                    })
                    .unwrap(),
                )),
                confidence: 0.9,
                provider: if matches!(self.mode, ResponseMode::ProviderMismatch) {
                    "spoofed-provider".into()
                } else {
                    "fixture-policy".into()
                },
                model: "fixture-vla".into(),
                version: "2026.08".into(),
                digest: "sha256:fixture-model".into(),
                goal_alignment: "direct".into(),
            }]
        } else {
            Vec::new()
        };
        Ok(Response::new(ProposeResponse {
            proposals,
            error: String::new(),
        }))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            status: "ready".into(),
        }))
    }
}

async fn start_gateway(
    mode: ResponseMode,
) -> (
    String,
    Arc<AtomicUsize>,
    Arc<Mutex<Option<ProposeRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(Mutex::new(None));
    let service = FixtureGateway {
        mode,
        calls: calls.clone(),
        received: received.clone(),
    };
    let task = tokio::spawn(async move {
        Server::builder()
            .add_service(PolicyGatewayServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    (format!("http://{address}"), calls, received, task)
}

async fn connect_provider(endpoint: String, request_timeout: Duration) -> GrpcPolicyProvider {
    GrpcPolicyProvider::connect(GrpcPolicyConfig {
        endpoint,
        request_timeout,
        ..GrpcPolicyConfig::default()
    })
    .await
    .unwrap()
}

fn snapshot() -> WorldSnapshot {
    WorldSnapshot {
        device: DeviceId("bot".into()),
        schema: "robot.state".into(),
        schema_version: 1,
        sequence: 7,
        payload: serde_json::json!({"mode": "idle", "balance": {"stable": true}}),
        observed_at: MonoTime(10),
        valid_until: None,
        stale: false,
    }
}

fn visual() -> PerceptionObservation {
    let digest = "a".repeat(64);
    PerceptionObservation {
        device: DeviceId("bot".into()),
        schema: "camera.rgb".into(),
        schema_version: 1,
        frame: FrameRef {
            uri: format!("artifact://sha256/{digest}"),
            sha256: digest,
            mime_type: "image/jpeg".into(),
            width: 640,
            height: 480,
            byte_len: 32_000,
            source_time_ms: 1,
            camera_id: "front".into(),
            frame_id: 4,
        },
        labels: vec!["standing".into()],
        summary: "robot visible".into(),
        confidence: 0.8,
        received_ms: 2,
    }
}

fn allowed_skills() -> Vec<SkillDescriptor> {
    vec![SkillDescriptor {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        summary: "enter a stable stance".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
        risk: RiskClass::Low,
        timeout_ms: 10_000,
        cancellable: true,
        preconditions: vec!["robot ready".into()],
        success_criteria: vec!["stable stance".into()],
    }]
}

#[tokio::test]
async fn real_gateway_roundtrip_carries_typed_inputs_and_host_bound_provenance() {
    let (endpoint, calls, received, task) = start_gateway(ResponseMode::Valid).await;
    let provider = connect_provider(endpoint, Duration::from_secs(1)).await;
    let snapshot = snapshot();
    let visual = visual();
    let allowed = allowed_skills();

    let proposals = provider
        .propose(
            "请让机器人保持稳定站立",
            &DeviceId("bot".into()),
            std::slice::from_ref(&snapshot),
            std::slice::from_ref(&visual),
            &allowed,
        )
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(proposals.len(), 1);
    assert!(validate_proposal(
        &proposals[0],
        &DeviceId("bot".into()),
        &allowed,
        std::slice::from_ref(&snapshot)
    )
    .is_ok());
    assert_eq!(proposals[0].skill.0, "kuavo.stance");
    assert_eq!(proposals[0].frame_refs, vec![visual.frame.clone()]);
    assert_eq!(proposals[0].provenance.provider, "fixture-policy");
    assert_eq!(proposals[0].provenance.protocol_version, "1.0");
    assert_eq!(proposals[0].provenance.model, "fixture-vla");

    let request = received.lock().unwrap().clone().unwrap();
    assert_eq!(request.protocol_version, "1.0");
    assert_eq!(request.goal, "请让机器人保持稳定站立");
    assert_eq!(request.allowed_skill_ids, vec!["kuavo.stance"]);
    assert_eq!(request.frame_uris, vec![visual.frame.uri]);
    assert_eq!(request.snapshots.len(), 1);
    assert_eq!(request.snapshots[0].schema, "robot.state");
    assert_eq!(request.snapshots[0].schema_version, 1);
    assert_eq!(request.snapshots[0].sequence, 7);
    assert!(request.snapshots[0]
        .payload
        .as_ref()
        .unwrap()
        .fields
        .contains_key("mode"));
    assert!(request.replan_context.is_none());
    task.abort();
}

#[tokio::test]
async fn replan_transmits_typed_attempt_context_and_remaining_budgets() {
    let (endpoint, calls, received, task) = start_gateway(ResponseMode::Valid).await;
    let provider = connect_provider(endpoint, Duration::from_secs(1)).await;
    let context = ReplanContext {
        goal: "recover stance".into(),
        device: DeviceId("bot".into()),
        latest_snapshot: snapshot(),
        failure_class: RobotFailureClass::VerificationMismatch,
        completed_attempts: vec![AttemptSummary {
            attempt: 1,
            skill: SkillId("kuavo.stance".into()),
            parameters_digest: "sha256:parameters".into(),
            snapshot_schema: "robot.state".into(),
            snapshot_schema_version: 1,
            snapshot_sequence: 7,
            failure_class: RobotFailureClass::VerificationTimeout,
            operation_id: Some("operation-1".into()),
        }],
        allowed_skills: allowed_skills(),
        retries_remaining: 0,
        replans_remaining: 1,
    };

    let frame = visual();
    let proposals = provider
        .replan(&context, std::slice::from_ref(&frame))
        .await
        .unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let request = received.lock().unwrap().clone().unwrap();
    let replan = request.replan_context.expect("typed replan context");
    assert_eq!(replan.failure_class, "verification_mismatch");
    assert_eq!(replan.retries_remaining, 0);
    assert_eq!(replan.replans_remaining, 1);
    assert_eq!(replan.completed_attempts.len(), 1);
    let attempt = &replan.completed_attempts[0];
    assert_eq!(attempt.attempt, 1);
    assert_eq!(attempt.skill_id, "kuavo.stance");
    assert_eq!(attempt.parameters_digest, "sha256:parameters");
    assert_eq!(attempt.snapshot_schema, "robot.state");
    assert_eq!(attempt.snapshot_schema_version, 1);
    assert_eq!(attempt.snapshot_sequence, 7);
    assert_eq!(attempt.failure_class, "verification_timeout");
    assert_eq!(attempt.operation_id, "operation-1");
    task.abort();
}

#[tokio::test]
async fn empty_and_timeout_are_explicit_and_never_retried() {
    let (endpoint, empty_calls, _, empty_task) = start_gateway(ResponseMode::Empty).await;
    let provider = connect_provider(endpoint, Duration::from_secs(1)).await;
    let error = provider
        .propose(
            "stand",
            &DeviceId("bot".into()),
            &[snapshot()],
            &[],
            &allowed_skills(),
        )
        .await
        .unwrap_err();
    assert_eq!(error, PolicyProviderError::EmptyResponse);
    assert_eq!(empty_calls.load(Ordering::SeqCst), 1);
    empty_task.abort();

    let (endpoint, timeout_calls, _, timeout_task) = start_gateway(ResponseMode::Slow).await;
    let provider = connect_provider(endpoint, Duration::from_millis(25)).await;
    let error = provider
        .propose(
            "stand",
            &DeviceId("bot".into()),
            &[snapshot()],
            &[],
            &allowed_skills(),
        )
        .await
        .unwrap_err();
    assert_eq!(error, PolicyProviderError::Timeout);
    assert_eq!(timeout_calls.load(Ordering::SeqCst), 1);
    timeout_task.abort();
}

#[tokio::test]
async fn stale_snapshot_fails_before_any_gateway_call() {
    let (endpoint, calls, _, task) = start_gateway(ResponseMode::Valid).await;
    let provider = connect_provider(endpoint, Duration::from_secs(1)).await;
    let mut stale = snapshot();
    stale.stale = true;
    let error = provider
        .propose(
            "stand",
            &DeviceId("bot".into()),
            &[stale],
            &[],
            &allowed_skills(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, PolicyProviderError::InvalidRequest(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn response_provider_cannot_spoof_negotiated_connection_identity() {
    let (endpoint, calls, _, task) = start_gateway(ResponseMode::ProviderMismatch).await;
    let provider = connect_provider(endpoint, Duration::from_secs(1)).await;
    let error = provider
        .propose(
            "stand",
            &DeviceId("bot".into()),
            &[snapshot()],
            &[],
            &allowed_skills(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, PolicyProviderError::InvalidResponse(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    task.abort();
}

fn json_struct(value: &serde_json::Value) -> prost_types::Struct {
    let fields = value
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), json_value(value)))
        .collect();
    prost_types::Struct { fields }
}

fn json_value(value: &serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match value {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(value) => Kind::BoolValue(*value),
        serde_json::Value::Number(value) => Kind::NumberValue(value.as_f64().unwrap()),
        serde_json::Value::String(value) => Kind::StringValue(value.clone()),
        serde_json::Value::Array(values) => Kind::ListValue(prost_types::ListValue {
            values: values.iter().map(json_value).collect(),
        }),
        serde_json::Value::Object(_) => Kind::StructValue(json_struct(value)),
    };
    prost_types::Value { kind: Some(kind) }
}
