//! gRPC Policy provider adapter — proposes skills, never executes.
//! No environment reads, no string error classification.

use crate::ports::policy_provider::PolicyProviderPort;
use async_trait::async_trait;
use fabric::types::embodiment::{DeviceId, SkillDescriptor};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};
use fabric::types::world_state::WorldSnapshot;
use std::time::Duration;

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
}
