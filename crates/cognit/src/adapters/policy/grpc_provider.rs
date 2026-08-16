//! gRPC Policy provider adapter — proposes skills, never executes.
//! No environment reads, no string error classification.

use crate::adapters::policy::wire::v1::policy_gateway_client::PolicyGatewayClient;
use crate::adapters::policy::wire::v1::{
    AttemptSummaryWire, GetCapabilitiesRequest, GetCapabilitiesResponse, HealthRequest,
    HealthResponse, ProposeRequest, ReplanContextWire, SkillProposalWire,
};
use crate::harness::robot::state::ReplanContext;
use crate::harness::robot::PerceptionObservation;
use crate::ports::policy_provider::{PolicyProviderError, PolicyProviderPort};
use ::contracts::types::embodiment::{DeviceId, SkillDescriptor};
use ::contracts::types::expected_outcome::ExpectedOutcome;
use ::contracts::types::skill_proposal::{GoalAlignment, PolicyProvenance, SkillProposal};
use ::contracts::types::world_state::WorldSnapshot;
use async_trait::async_trait;
use std::net::IpAddr;
use std::time::Duration;
use tonic::transport::Channel;

const MAX_POLICY_SNAPSHOTS: usize = 16;
const MAX_POLICY_SNAPSHOT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GrpcPolicyConfig {
    pub endpoint: String,
    pub protocol_version: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_proposals: usize,
}

/// Authoritative Policy capability facts negotiated during startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyCapabilitySnapshot {
    pub protocol_version: String,
    pub provider_id: String,
    pub server_max_proposals: usize,
    pub negotiated_max_proposals: usize,
}

/// Typed Policy bootstrap failure. Runtime callers never have to classify
/// display strings to distinguish transport, health, and compatibility faults.
#[derive(Debug, thiserror::Error)]
pub enum PolicyStartupError {
    #[error("invalid Policy endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("Policy gateway connection failed: {0}")]
    Connection(String),
    #[error("Policy Health RPC failed: {0}")]
    HealthRpc(String),
    #[error("Policy Health RPC timed out")]
    HealthTimeout,
    #[error("Policy gateway is not ready: {status}")]
    Unhealthy { status: String },
    #[error("Policy GetCapabilities RPC failed: {0}")]
    CapabilitiesRpc(String),
    #[error("Policy GetCapabilities RPC timed out")]
    CapabilitiesTimeout,
    #[error("Policy protocol mismatch: client={client}, server={server}")]
    ProtocolMismatch { client: String, server: String },
    #[error("invalid Policy capability snapshot: {0}")]
    InvalidCapabilities(String),
}

impl Default for GrpcPolicyConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:50052".into(),
            protocol_version: "1.0".into(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            max_proposals: 4,
        }
    }
}

/// Validate policy endpoint — production must use TLS (non-loopback).
pub fn validate_policy_endpoint(endpoint: &str) -> Result<(), String> {
    if endpoint.trim() != endpoint || endpoint.is_empty() {
        return Err("endpoint must be non-empty without surrounding whitespace".into());
    }
    if endpoint.contains('@') || endpoint.contains('?') || endpoint.contains('#') {
        return Err("endpoint must not contain userinfo, query parameters, or fragments".into());
    }
    let (scheme, remainder) = endpoint
        .split_once("://")
        .ok_or_else(|| "endpoint must include an explicit URL scheme".to_string())?;
    if !matches!(scheme, "http" | "https" | "grpcs") {
        return Err("endpoint must use http, https, or grpcs".into());
    }
    let authority = remainder.split('/').next().unwrap_or_default();
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed
            .split_once(']')
            .map(|(host, _)| host)
            .ok_or_else(|| "endpoint contains an invalid IPv6 host".to_string())?
    } else {
        authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host)
    };
    if host.is_empty() {
        return Err("endpoint must include a host".into());
    }
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if scheme == "http" && !is_loopback {
        return Err("non-loopback endpoint must use TLS (https:// or grpcs://)".into());
    }
    Ok(())
}

// ── wire ↔ domain conversion ─────────────────────────────────────────────────

fn value_to_struct(value: &serde_json::Value) -> prost_types::Struct {
    let mut fields = std::collections::BTreeMap::new();
    if let serde_json::Value::Object(map) = value {
        for (key, val) in map {
            fields.insert(key.clone(), json_to_prost_value(val));
        }
    }
    prost_types::Struct { fields }
}

fn json_to_prost_value(v: &serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => {
            let values: Vec<prost_types::Value> = arr.iter().map(json_to_prost_value).collect();
            Kind::ListValue(prost_types::ListValue { values })
        }
        serde_json::Value::Object(_) => Kind::StructValue(value_to_struct(v)),
    };
    prost_types::Value { kind: Some(kind) }
}

fn struct_to_value(s: &Option<prost_types::Struct>) -> serde_json::Value {
    let Some(s) = s else {
        return serde_json::Value::Object(Default::default());
    };
    let mut map = serde_json::Map::new();
    for (key, value) in &s.fields {
        map.insert(key.clone(), prost_value_to_json(value));
    }
    serde_json::Value::Object(map)
}

fn prost_value_to_json(v: &prost_types::Value) -> serde_json::Value {
    use prost_types::value::Kind;
    match &v.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::NumberValue(n)) => {
            // Protobuf Struct stores numbers as f64; restore integral values as
            // integers so u64 timestamp fields round-trip through ExpectedOutcome.
            if n.fract() == 0.0 && *n >= 0.0 && *n <= u64::MAX as f64 {
                serde_json::Value::Number(serde_json::Number::from(*n as u64))
            } else {
                serde_json::Number::from_f64(*n)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            }
        }
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::ListValue(list)) => {
            serde_json::Value::Array(list.values.iter().map(prost_value_to_json).collect())
        }
        Some(Kind::StructValue(st)) => serde_json::Value::Object(
            st.fields
                .iter()
                .map(|(k, v)| (k.clone(), prost_value_to_json(v)))
                .collect(),
        ),
        None => serde_json::Value::Null,
    }
}

fn proposal_wire_to_domain(
    wire: SkillProposalWire,
    selected_frames: &[::contracts::types::frame::FrameRef],
    capabilities: &PolicyCapabilitySnapshot,
) -> Result<SkillProposal, String> {
    if wire.provider != capabilities.provider_id {
        return Err(format!(
            "response provider '{}' does not match negotiated provider '{}'",
            wire.provider, capabilities.provider_id
        ));
    }
    let expected_outcome = struct_to_value(&wire.expected_outcome);
    let expected_outcome = serde_json::from_value::<ExpectedOutcome>(expected_outcome)
        .map_err(|e| format!("invalid expected_outcome from policy: {e}"))?;
    let parameters = struct_to_value(&wire.parameters);
    let goal_alignment = match wire.goal_alignment.as_str() {
        "direct" => GoalAlignment::Direct,
        "safety_fallback" => GoalAlignment::SafetyFallback,
        other => {
            return Err(format!(
                "invalid goal_alignment from policy: expected direct or safety_fallback, got {other:?}"
            ));
        }
    };
    Ok(SkillProposal {
        skill: ::contracts::types::embodiment::SkillId(wire.skill_id),
        device: DeviceId(wire.device_id),
        parameters,
        expected_outcome,
        goal_alignment,
        confidence: wire.confidence,
        frame_refs: selected_frames.to_vec(),
        provenance: PolicyProvenance {
            provider: capabilities.provider_id.clone(),
            model: wire.model,
            version: wire.version,
            protocol_version: capabilities.protocol_version.clone(),
            digest: wire.digest,
        },
    })
}

// ── real gRPC policy client ──────────────────────────────────────────────────

/// gRPC `PolicyProviderPort` client. Connects to the `PolicyGateway` service
/// (GetCapabilities/Propose/Health). Proposes semantic skills only — never
/// executes anything directly.
pub struct GrpcPolicyProvider {
    client: PolicyGatewayClient<Channel>,
    config: GrpcPolicyConfig,
    capabilities: PolicyCapabilitySnapshot,
}

impl GrpcPolicyProvider {
    pub async fn connect(config: GrpcPolicyConfig) -> Result<Self, PolicyStartupError> {
        validate_policy_endpoint(&config.endpoint).map_err(PolicyStartupError::InvalidEndpoint)?;
        let endpoint = tonic::transport::Endpoint::from_shared(config.endpoint.clone())
            .map_err(|e| PolicyStartupError::InvalidEndpoint(e.to_string()))?
            .connect_timeout(config.connect_timeout);
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| PolicyStartupError::Connection(e.to_string()))?;
        let mut client = PolicyGatewayClient::new(channel);
        // Startup compatibility is evidence-only: Health and GetCapabilities do
        // not carry any actuation request and run before the provider is exposed.
        let health = tokio::time::timeout(config.request_timeout, client.health(HealthRequest {}))
            .await
            .map_err(|_| PolicyStartupError::HealthTimeout)?
            .map_err(|error| PolicyStartupError::HealthRpc(error.to_string()))?
            .into_inner();
        let capabilities = tokio::time::timeout(
            config.request_timeout,
            client.get_capabilities(GetCapabilitiesRequest {
                protocol_version: config.protocol_version.clone(),
            }),
        )
        .await
        .map_err(|_| PolicyStartupError::CapabilitiesTimeout)?
        .map_err(|error| PolicyStartupError::CapabilitiesRpc(error.to_string()))?
        .into_inner();
        let capabilities = validate_policy_startup(&config, health, capabilities)?;
        Ok(Self {
            client,
            config,
            capabilities,
        })
    }

    pub fn capability_snapshot(&self) -> &PolicyCapabilitySnapshot {
        &self.capabilities
    }

    fn propose_request(
        &self,
        goal: &str,
        device: &DeviceId,
        snapshots: &[WorldSnapshot],
        visual: &[PerceptionObservation],
        allowed_skills: &[SkillDescriptor],
    ) -> Result<ProposeRequest, PolicyProviderError> {
        if snapshots.is_empty() {
            return Err(PolicyProviderError::InvalidRequest(
                "at least one fresh world snapshot is required".into(),
            ));
        }
        if snapshots.len() > MAX_POLICY_SNAPSHOTS {
            return Err(PolicyProviderError::InvalidRequest(format!(
                "snapshot count {} exceeds {MAX_POLICY_SNAPSHOTS}",
                snapshots.len()
            )));
        }
        let mut encoded_snapshot_bytes = 0usize;
        let mut snapshot_wires = Vec::with_capacity(snapshots.len());
        for (index, snapshot) in snapshots.iter().enumerate() {
            if snapshot.device != *device {
                return Err(PolicyProviderError::InvalidRequest(format!(
                    "snapshot[{index}] device '{}' does not match '{}'",
                    snapshot.device.0, device.0
                )));
            }
            if snapshot.stale {
                return Err(PolicyProviderError::InvalidRequest(format!(
                    "snapshot[{index}] is stale"
                )));
            }
            if snapshot.schema.trim().is_empty() || snapshot.schema_version == 0 {
                return Err(PolicyProviderError::InvalidRequest(format!(
                    "snapshot[{index}] has an invalid schema contract"
                )));
            }
            let payload_bytes = serde_json::to_vec(&snapshot.payload).map_err(|error| {
                PolicyProviderError::InvalidRequest(format!(
                    "snapshot[{index}] payload is not serializable: {error}"
                ))
            })?;
            encoded_snapshot_bytes = encoded_snapshot_bytes
                .checked_add(payload_bytes.len())
                .ok_or_else(|| {
                    PolicyProviderError::InvalidRequest(
                        "snapshot payload byte count overflow".into(),
                    )
                })?;
            if encoded_snapshot_bytes > MAX_POLICY_SNAPSHOT_BYTES {
                return Err(PolicyProviderError::InvalidRequest(format!(
                    "snapshot payloads exceed {MAX_POLICY_SNAPSHOT_BYTES} bytes"
                )));
            }
            let payload = snapshot.payload.as_object().ok_or_else(|| {
                PolicyProviderError::InvalidRequest(format!(
                    "snapshot[{index}] payload must be an object"
                ))
            })?;
            snapshot_wires.push(crate::adapters::policy::wire::v1::WorldSnapshotWire {
                device_id: snapshot.device.0.clone(),
                schema: snapshot.schema.clone(),
                schema_version: u32::from(snapshot.schema_version),
                sequence: snapshot.sequence,
                payload: Some(value_to_struct(&serde_json::Value::Object(payload.clone()))),
            });
        }
        let frame_summary = if visual.is_empty() {
            snapshots
                .last()
                .map(|snapshot| {
                    format!(
                        "schema={} sequence={} stale={}",
                        snapshot.schema, snapshot.sequence, snapshot.stale
                    )
                })
                .unwrap_or_default()
        } else {
            visual
                .iter()
                .map(|observation| observation.summary.as_str())
                .filter(|summary| !summary.is_empty())
                .collect::<Vec<_>>()
                .join("; ")
        };
        let frame_confidence =
            visual.iter().fold(0.0f32, |acc, v| acc + v.confidence) / (visual.len().max(1) as f32);
        Ok(ProposeRequest {
            protocol_version: self.config.protocol_version.clone(),
            goal: goal.to_string(),
            device_id: device.0.clone(),
            frame_uris: visual.iter().map(|v| v.frame.uri.clone()).collect(),
            frame_labels: visual.iter().flat_map(|v| v.labels.clone()).collect(),
            frame_summary,
            frame_confidence,
            allowed_skill_ids: allowed_skills.iter().map(|s| s.skill.0.clone()).collect(),
            snapshots: snapshot_wires,
            replan_context: None,
        })
    }

    fn replan_request(
        &self,
        context: &ReplanContext,
        visual: &[PerceptionObservation],
    ) -> Result<ProposeRequest, PolicyProviderError> {
        let mut request = self.propose_request(
            &context.goal,
            &context.device,
            std::slice::from_ref(&context.latest_snapshot),
            visual,
            &context.allowed_skills,
        )?;
        request.replan_context = Some(ReplanContextWire {
            failure_class: failure_class_wire(context.failure_class),
            completed_attempts: context
                .completed_attempts
                .iter()
                .map(|attempt| AttemptSummaryWire {
                    attempt: attempt.attempt,
                    skill_id: attempt.skill.0.clone(),
                    parameters_digest: attempt.parameters_digest.clone(),
                    snapshot_schema: attempt.snapshot_schema.clone(),
                    snapshot_schema_version: u32::from(attempt.snapshot_schema_version),
                    snapshot_sequence: attempt.snapshot_sequence,
                    failure_class: failure_class_wire(attempt.failure_class),
                    operation_id: attempt.operation_id.clone().unwrap_or_default(),
                })
                .collect(),
            retries_remaining: context.retries_remaining,
            replans_remaining: context.replans_remaining,
        });
        Ok(request)
    }

    async fn send_request(
        &self,
        request: ProposeRequest,
        visual: &[PerceptionObservation],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        let mut client = self.client.clone();
        let call = client.propose(request);
        let response = tokio::time::timeout(self.config.request_timeout, call)
            .await
            .map_err(|_| PolicyProviderError::Timeout)?
            .map_err(|error| {
                if error.code() == tonic::Code::DeadlineExceeded {
                    PolicyProviderError::Timeout
                } else {
                    PolicyProviderError::Rpc(error.to_string())
                }
            })?
            .into_inner();
        if !response.error.is_empty() {
            return Err(PolicyProviderError::Gateway(response.error));
        }
        if response.proposals.is_empty() {
            return Err(PolicyProviderError::EmptyResponse);
        }
        if response.proposals.len() > self.capabilities.negotiated_max_proposals {
            return Err(PolicyProviderError::ResponseLimit {
                actual: response.proposals.len(),
                limit: self.capabilities.negotiated_max_proposals,
            });
        }
        let selected_frames = visual
            .iter()
            .map(|observation| observation.frame.clone())
            .collect::<Vec<_>>();
        let mut proposals = Vec::with_capacity(response.proposals.len());
        for wire in response.proposals {
            proposals.push(
                proposal_wire_to_domain(wire, &selected_frames, &self.capabilities)
                    .map_err(PolicyProviderError::InvalidResponse)?,
            );
        }
        Ok(proposals)
    }
}

fn failure_class_wire(class: ::contracts::types::robot_failure::RobotFailureClass) -> String {
    class.as_str().to_owned()
}

fn validate_policy_startup(
    config: &GrpcPolicyConfig,
    health: HealthResponse,
    capabilities: GetCapabilitiesResponse,
) -> Result<PolicyCapabilitySnapshot, PolicyStartupError> {
    let health_status = health.status.trim();
    if !health_status.eq_ignore_ascii_case("ready") {
        return Err(PolicyStartupError::Unhealthy {
            status: if health_status.is_empty() {
                "empty".into()
            } else {
                health_status.into()
            },
        });
    }
    if capabilities.protocol_version != config.protocol_version {
        return Err(PolicyStartupError::ProtocolMismatch {
            client: config.protocol_version.clone(),
            server: capabilities.protocol_version,
        });
    }
    let provider_id = capabilities.provider_id.trim();
    if provider_id.is_empty() {
        return Err(PolicyStartupError::InvalidCapabilities(
            "provider_id is empty".into(),
        ));
    }
    let server_max_proposals = usize::try_from(capabilities.max_proposals).map_err(|_| {
        PolicyStartupError::InvalidCapabilities("max_proposals does not fit usize".into())
    })?;
    if server_max_proposals == 0 {
        return Err(PolicyStartupError::InvalidCapabilities(
            "max_proposals must be nonzero".into(),
        ));
    }
    if config.max_proposals == 0 {
        return Err(PolicyStartupError::InvalidCapabilities(
            "configured max_proposals must be nonzero".into(),
        ));
    }
    Ok(PolicyCapabilitySnapshot {
        protocol_version: config.protocol_version.clone(),
        provider_id: provider_id.to_owned(),
        server_max_proposals,
        negotiated_max_proposals: server_max_proposals.min(config.max_proposals),
    })
}

#[async_trait]
impl PolicyProviderPort for GrpcPolicyProvider {
    async fn propose(
        &self,
        goal: &str,
        device: &DeviceId,
        snapshots: &[WorldSnapshot],
        visual: &[PerceptionObservation],
        allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        let request = self.propose_request(goal, device, snapshots, visual, allowed_skills)?;
        self.send_request(request, visual).await
    }

    async fn replan(
        &self,
        context: &ReplanContext,
        visual: &[PerceptionObservation],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        let request = self.replan_request(context, visual)?;
        self.send_request(request, visual).await
    }

    async fn health(&self) -> Result<String, String> {
        let response = self
            .client
            .clone()
            .health(HealthRequest {})
            .await
            .map_err(|e| format!("policy health rpc: {e}"))?
            .into_inner();
        Ok(response.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::types::expected_outcome::OutcomePredicate;

    #[test]
    fn loopback_without_tls_allowed_for_dev() {
        assert!(validate_policy_endpoint("http://127.0.0.1:50052").is_ok());
        assert!(validate_policy_endpoint("http://localhost:50052").is_ok());
        assert!(validate_policy_endpoint("http://[::1]:50052").is_ok());
    }

    #[test]
    fn non_loopback_without_tls_rejected() {
        assert!(validate_policy_endpoint("http://policy-server:50052").is_err());
        assert!(validate_policy_endpoint("http://localhost.example:50052").is_err());
        assert!(validate_policy_endpoint("http://127.0.0.1.example:50052").is_err());
    }

    #[test]
    fn non_loopback_with_tls_allowed() {
        assert!(validate_policy_endpoint("https://policy.example.com:443").is_ok());
        assert!(validate_policy_endpoint("grpcs://policy.internal:50052").is_ok());
    }

    #[test]
    fn empty_endpoint_rejected() {
        assert!(validate_policy_endpoint("").is_err());
        let error = validate_policy_endpoint("https://user:secret@policy.example:443")
            .expect_err("userinfo must be rejected");
        assert!(!error.contains("secret"));
    }

    fn ready_health() -> HealthResponse {
        HealthResponse {
            status: "ready".into(),
        }
    }

    fn compatible_capabilities() -> GetCapabilitiesResponse {
        GetCapabilitiesResponse {
            protocol_version: "1.0".into(),
            provider_id: "openvla-local".into(),
            max_proposals: 8,
        }
    }

    #[test]
    fn startup_capabilities_negotiate_a_bounded_snapshot() {
        let config = GrpcPolicyConfig {
            max_proposals: 4,
            ..Default::default()
        };
        let snapshot =
            validate_policy_startup(&config, ready_health(), compatible_capabilities()).unwrap();
        assert_eq!(snapshot.provider_id, "openvla-local");
        assert_eq!(snapshot.protocol_version, "1.0");
        assert_eq!(snapshot.server_max_proposals, 8);
        assert_eq!(snapshot.negotiated_max_proposals, 4);
    }

    #[test]
    fn startup_rejects_unhealthy_or_incompatible_policy() {
        let config = GrpcPolicyConfig::default();
        let unhealthy = validate_policy_startup(
            &config,
            HealthResponse {
                status: "degraded".into(),
            },
            compatible_capabilities(),
        )
        .unwrap_err();
        assert!(matches!(unhealthy, PolicyStartupError::Unhealthy { .. }));

        let mut incompatible = compatible_capabilities();
        incompatible.protocol_version = "2.0".into();
        let error = validate_policy_startup(&config, ready_health(), incompatible).unwrap_err();
        assert!(matches!(error, PolicyStartupError::ProtocolMismatch { .. }));
    }

    #[test]
    fn startup_rejects_incomplete_capability_facts() {
        let config = GrpcPolicyConfig::default();
        let mut missing_provider = compatible_capabilities();
        missing_provider.provider_id.clear();
        assert!(matches!(
            validate_policy_startup(&config, ready_health(), missing_provider),
            Err(PolicyStartupError::InvalidCapabilities(_))
        ));

        let mut zero_limit = compatible_capabilities();
        zero_limit.max_proposals = 0;
        assert!(matches!(
            validate_policy_startup(&config, ready_health(), zero_limit),
            Err(PolicyStartupError::InvalidCapabilities(_))
        ));
    }

    fn stance_expected() -> ExpectedOutcome {
        ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5_000,
        }
    }

    fn sample_wire() -> SkillProposalWire {
        SkillProposalWire {
            skill_id: "kuavo.stance".into(),
            device_id: "bot".into(),
            parameters: Some(value_to_struct(&serde_json::json!({"duration_ms": 2000}))),
            expected_outcome: Some(value_to_struct(
                &serde_json::to_value(stance_expected()).unwrap(),
            )),
            confidence: 0.9,
            provider: "openvla-v1".into(),
            model: "m".into(),
            version: "1.0".into(),
            digest: "sha256:abc".into(),
            goal_alignment: "direct".into(),
        }
    }

    fn test_capabilities() -> PolicyCapabilitySnapshot {
        PolicyCapabilitySnapshot {
            protocol_version: "1.0".into(),
            provider_id: "openvla-v1".into(),
            server_max_proposals: 4,
            negotiated_max_proposals: 4,
        }
    }

    #[test]
    fn proposal_wire_round_trips_to_domain() {
        let proposal = proposal_wire_to_domain(sample_wire(), &[], &test_capabilities()).unwrap();
        assert_eq!(proposal.skill.0, "kuavo.stance");
        assert_eq!(proposal.device.0, "bot");
        assert_eq!(proposal.confidence, 0.9);
        assert_eq!(proposal.provenance.provider, "openvla-v1");
        assert_eq!(proposal.provenance.protocol_version, "1.0");
        assert_eq!(proposal.expected_outcome, stance_expected());
        assert_eq!(proposal.parameters["duration_ms"], 2000);
        assert_eq!(proposal.goal_alignment, GoalAlignment::Direct);
    }

    #[test]
    fn proposal_wire_requires_typed_goal_alignment() {
        let mut missing = sample_wire();
        missing.goal_alignment.clear();
        let error = proposal_wire_to_domain(missing, &[], &test_capabilities()).unwrap_err();
        assert!(error.contains("invalid goal_alignment"));

        let mut fallback = sample_wire();
        fallback.goal_alignment = "safety_fallback".into();
        let proposal = proposal_wire_to_domain(fallback, &[], &test_capabilities()).unwrap();
        assert_eq!(proposal.goal_alignment, GoalAlignment::SafetyFallback);
    }

    #[tokio::test]
    async fn propose_request_maps_frames_and_allowed_skills() {
        use crate::harness::robot::PerceptionObservation;
        use ::contracts::types::frame::FrameRef;
        let provider = GrpcPolicyProvider {
            client: unreachable_client(),
            config: GrpcPolicyConfig::default(),
            capabilities: test_capabilities(),
        };
        let visual = vec![PerceptionObservation {
            device: DeviceId("bot".into()),
            schema: "camera.rgb".into(),
            schema_version: 1,
            frame: FrameRef {
                uri: "artifact://sha256/ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
                sha256: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
                mime_type: "image/jpeg".into(),
                width: 640,
                height: 480,
                byte_len: 32_000,
                source_time_ms: 1,
                camera_id: "cam-0".into(),
                frame_id: 1,
            },
            labels: vec!["standing".into()],
            summary: "robot standing".into(),
            confidence: 0.8,
            received_ms: 2,
        }];
        let skills = vec![SkillDescriptor {
            skill: ::contracts::types::embodiment::SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            summary: "stance".into(),
            input_schema: serde_json::json!({}),
            risk: ::contracts::types::embodiment::RiskClass::Low,
            timeout_ms: 10_000,
            cancellable: false,
            preconditions: vec![],
            success_criteria: vec![],
        }];
        let snapshots = vec![WorldSnapshot {
            device: DeviceId("bot".into()),
            schema: "robot.state".into(),
            schema_version: 1,
            sequence: 9,
            payload: serde_json::json!({"mode": "stance"}),
            observed_at: ::contracts::MonoTime(1),
            valid_until: None,
            stale: false,
        }];
        let request = provider
            .propose_request(
                "stand",
                &DeviceId("bot".into()),
                &snapshots,
                &visual,
                &skills,
            )
            .unwrap();
        assert_eq!(request.goal, "stand");
        assert_eq!(request.device_id, "bot");
        assert_eq!(request.frame_uris, vec!["artifact://sha256/ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"]);
        assert_eq!(request.frame_labels, vec!["standing"]);
        assert_eq!(request.allowed_skill_ids, vec!["kuavo.stance"]);
        assert_eq!(request.frame_confidence, 0.8);
        assert_eq!(request.snapshots.len(), 1);
        assert_eq!(request.snapshots[0].schema_version, 1);
        assert_eq!(request.snapshots[0].sequence, 9);
    }

    #[tokio::test]
    async fn connect_to_unreachable_endpoint_fails() {
        let result = GrpcPolicyProvider::connect(GrpcPolicyConfig {
            endpoint: "http://127.0.0.1:1".into(),
            ..Default::default()
        })
        .await;
        assert!(result.is_err());
    }

    fn unreachable_client() -> PolicyGatewayClient<Channel> {
        // Never used by propose_request (it only builds the request).
        let channel = tonic::transport::Endpoint::from_shared("http://127.0.0.1:1".to_string())
            .unwrap()
            .connect_lazy();
        PolicyGatewayClient::new(channel)
    }
}
