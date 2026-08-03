//! gRPC Policy provider adapter — proposes skills, never executes.
//! No environment reads, no string error classification.

use crate::adapters::policy::wire::v1::policy_gateway_client::PolicyGatewayClient;
use crate::adapters::policy::wire::v1::{HealthRequest, ProposeRequest, SkillProposalWire};
use crate::ports::policy_provider::PolicyProviderPort;
use async_trait::async_trait;
use fabric::types::embodiment::{DeviceId, SkillDescriptor};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};
use fabric::types::world_state::WorldSnapshot;
use std::time::Duration;
use tonic::transport::Channel;

#[derive(Debug, Clone)]
pub struct GrpcPolicyConfig {
    pub endpoint: String,
    pub protocol_version: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_proposals: usize,
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
    if endpoint.is_empty() {
        return Err("endpoint is empty".into());
    }
    // Loopback allowed for development, but must be plaintext http://
    let is_loopback = endpoint.contains("127.0.0.1") || endpoint.contains("localhost");
    let is_tls = endpoint.starts_with("https://") || endpoint.starts_with("grpcs://");

    if !is_loopback && !is_tls {
        return Err(format!(
            "non-loopback endpoint '{endpoint}' must use TLS (https:// or grpcs://)"
        ));
    }
    Ok(())
}

/// Stub policy provider for testing — returns a fixed proposal.
/// In production this connects to an actual gRPC policy service.
pub struct StubPolicyProvider {
    #[allow(dead_code)]
    config: GrpcPolicyConfig,
}

impl StubPolicyProvider {
    pub fn new(config: GrpcPolicyConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl PolicyProviderPort for StubPolicyProvider {
    async fn propose(
        &self,
        _goal: &str,
        device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, String> {
        // For now returns a stub — real implementation connects to external policy
        if allowed_skills.is_empty() {
            return Ok(vec![]);
        }

        // Find a stance skill if available
        let stance = allowed_skills.iter().find(|s| s.skill.0.contains("stance"));
        let skill = stance.unwrap_or(&allowed_skills[0]);

        Ok(vec![SkillProposal {
            skill: skill.skill.clone(),
            device: device.clone(),
            parameters: serde_json::json!({}),
            expected_outcome: ExpectedOutcome {
                predicate: OutcomePredicate::Equals {
                    path: "mode".into(),
                    value: serde_json::json!("stance"),
                },
                freshness_ms: 500,
                stable_window_ms: 200,
                timeout_ms: 5000,
            },
            confidence: 0.9,
            frame_refs: vec![],
            provenance: PolicyProvenance {
                provider: "stub-policy".into(),
                model: "stub-v1".into(),
                version: "1.0".into(),
                digest: "sha256:stub".into(),
            },
        }])
    }

    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
    }
}

// ── wire ↔ domain conversion ─────────────────────────────────────────────────

#[cfg(test)]
fn value_to_struct(value: &serde_json::Value) -> prost_types::Struct {
    let mut fields = std::collections::BTreeMap::new();
    if let serde_json::Value::Object(map) = value {
        for (key, val) in map {
            fields.insert(key.clone(), json_to_prost_value(val));
        }
    }
    prost_types::Struct { fields }
}

#[cfg(test)]
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

fn proposal_wire_to_domain(wire: SkillProposalWire) -> Result<SkillProposal, String> {
    let expected_outcome = struct_to_value(&wire.expected_outcome);
    let expected_outcome = serde_json::from_value::<ExpectedOutcome>(expected_outcome)
        .map_err(|e| format!("invalid expected_outcome from policy: {e}"))?;
    let parameters = struct_to_value(&wire.parameters);
    Ok(SkillProposal {
        skill: fabric::types::embodiment::SkillId(wire.skill_id),
        device: DeviceId(wire.device_id),
        parameters,
        expected_outcome,
        confidence: wire.confidence,
        frame_refs: vec![],
        provenance: PolicyProvenance {
            provider: wire.provider,
            model: wire.model,
            version: wire.version,
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
}

impl GrpcPolicyProvider {
    pub async fn connect(config: GrpcPolicyConfig) -> Result<Self, String> {
        validate_policy_endpoint(&config.endpoint)?;
        let endpoint = tonic::transport::Endpoint::from_shared(config.endpoint.clone())
            .map_err(|e| format!("invalid endpoint: {e}"))?
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout);
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| format!("connect to policy gateway: {e}"))?;
        Ok(Self {
            client: PolicyGatewayClient::new(channel),
            config,
        })
    }

    fn propose_request(
        &self,
        goal: &str,
        device: &DeviceId,
        snapshots: &[WorldSnapshot],
        visual: &[PerceptionObservation],
        allowed_skills: &[SkillDescriptor],
    ) -> ProposeRequest {
        let frame_summary = snapshots
            .last()
            .map(|snapshot| snapshot.payload.to_string())
            .unwrap_or_default();
        let frame_confidence = visual
            .iter()
            .fold(0.0f32, |acc, v| acc + v.confidence)
            / (visual.len().max(1) as f32);
        ProposeRequest {
            protocol_version: self.config.protocol_version.clone(),
            goal: goal.to_string(),
            device_id: device.0.clone(),
            frame_uris: visual.iter().map(|v| v.frame.uri.clone()).collect(),
            frame_labels: visual.iter().flat_map(|v| v.labels.clone()).collect(),
            frame_summary,
            frame_confidence,
            allowed_skill_ids: allowed_skills.iter().map(|s| s.skill.0.clone()).collect(),
        }
    }
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
    ) -> Result<Vec<SkillProposal>, String> {
        let request = self.propose_request(goal, device, snapshots, visual, allowed_skills);
        let response = self
            .client
            .clone()
            .propose(request)
            .await
            .map_err(|e| format!("policy propose rpc: {e}"))?
            .into_inner();
        if !response.error.is_empty() {
            return Err(response.error);
        }
        let mut proposals = Vec::with_capacity(response.proposals.len());
        for wire in response.proposals {
            proposals.push(proposal_wire_to_domain(wire)?);
        }
        Ok(proposals)
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

    #[test]
    fn loopback_without_tls_allowed_for_dev() {
        assert!(validate_policy_endpoint("http://127.0.0.1:50052").is_ok());
        assert!(validate_policy_endpoint("http://localhost:50052").is_ok());
    }

    #[test]
    fn non_loopback_without_tls_rejected() {
        assert!(validate_policy_endpoint("http://policy-server:50052").is_err());
    }

    #[test]
    fn non_loopback_with_tls_allowed() {
        assert!(validate_policy_endpoint("https://policy.example.com:443").is_ok());
        assert!(validate_policy_endpoint("grpcs://policy.internal:50052").is_ok());
    }

    #[test]
    fn empty_endpoint_rejected() {
        assert!(validate_policy_endpoint("").is_err());
    }

    #[tokio::test]
    async fn stub_provider_health_is_ready() {
        let provider = StubPolicyProvider::new(GrpcPolicyConfig::default());
        assert_eq!(provider.health().await.unwrap(), "ready");
    }

    #[tokio::test]
    async fn stub_provider_returns_proposal() {
        use fabric::types::embodiment::{RiskClass, SkillDescriptor, SkillId};
        let provider = StubPolicyProvider::new(GrpcPolicyConfig::default());
        let skills = vec![SkillDescriptor {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            summary: "stance".into(),
            input_schema: serde_json::json!({}),
            risk: RiskClass::Low,
            timeout_ms: 10000,
            cancellable: false,
            preconditions: vec![],
            success_criteria: vec![],
        }];
        let proposals = provider
            .propose("test goal", &DeviceId("bot".into()), &[], &[], &skills)
            .await
            .unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].skill.0, "kuavo.stance");
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
        }
    }

    #[test]
    fn proposal_wire_round_trips_to_domain() {
        let proposal = proposal_wire_to_domain(sample_wire()).unwrap();
        assert_eq!(proposal.skill.0, "kuavo.stance");
        assert_eq!(proposal.device.0, "bot");
        assert_eq!(proposal.confidence, 0.9);
        assert_eq!(proposal.provenance.provider, "openvla-v1");
        assert_eq!(proposal.expected_outcome, stance_expected());
        assert_eq!(proposal.parameters["duration_ms"], 2000);
    }

    #[tokio::test]
    async fn propose_request_maps_frames_and_allowed_skills() {
        use fabric::types::frame::FrameRef;
        use fabric::types::perception_observation::PerceptionObservation;
        let provider = GrpcPolicyProvider {
            client: unreachable_client(),
            config: GrpcPolicyConfig::default(),
        };
        let visual = vec![PerceptionObservation {
            frame: FrameRef {
                uri: "artifact://sha256:frame".into(),
                sha256: "f".into(),
                mime_type: "image/jpeg".into(),
                width: 640,
                height: 480,
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
            skill: fabric::types::embodiment::SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            summary: "stance".into(),
            input_schema: serde_json::json!({}),
            risk: fabric::types::embodiment::RiskClass::Low,
            timeout_ms: 10_000,
            cancellable: false,
            preconditions: vec![],
            success_criteria: vec![],
        }];
        let request = provider.propose_request(
            "stand",
            &DeviceId("bot".into()),
            &[],
            &visual,
            &skills,
        );
        assert_eq!(request.goal, "stand");
        assert_eq!(request.device_id, "bot");
        assert_eq!(request.frame_uris, vec!["artifact://sha256:frame"]);
        assert_eq!(request.frame_labels, vec!["standing"]);
        assert_eq!(request.allowed_skill_ids, vec!["kuavo.stance"]);
        assert_eq!(request.frame_confidence, 0.8);
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
