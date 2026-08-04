//! Versioned host-owned memory judgment policy.

use fabric::protocol::memory::MemoryRecordKindV1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryPolicyConfig {
    pub version: String,
    pub semantic_profile: String,
    pub evidence_provenance_weight: i16,
    pub future_utility_weight: i16,
    pub stability_weight: i16,
    pub novelty_dedup_weight: i16,
    pub scope_fit_weight: i16,
    pub verification_weight: i16,
    pub privacy_risk_max_penalty: i16,
    pub contradiction_risk_max_penalty: i16,
    pub candidate_threshold: i16,
    pub promote_local_threshold: i16,
    pub remote_projection_threshold: i16,
    pub lease_duration_ms: u64,
    pub max_items_per_run: usize,
    pub run_deadline_ms: u64,
    pub semantic_retry_delay_ms: u64,
    pub max_provider_rounds: u32,
    pub max_provider_retries: u32,
    pub max_tool_calls: u32,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_projected_writes: usize,
    pub remote_allowed_kinds: Vec<MemoryRecordKindV1>,
}

impl Default for MemoryPolicyConfig {
    fn default() -> Self {
        Self {
            version: "memory-policy-v1".into(),
            semantic_profile: "safe-agent".into(),
            evidence_provenance_weight: 25,
            future_utility_weight: 20,
            stability_weight: 15,
            novelty_dedup_weight: 15,
            scope_fit_weight: 10,
            verification_weight: 15,
            privacy_risk_max_penalty: 25,
            contradiction_risk_max_penalty: 20,
            candidate_threshold: 55,
            promote_local_threshold: 75,
            remote_projection_threshold: 80,
            lease_duration_ms: 30_000,
            max_items_per_run: 20,
            run_deadline_ms: 120_000,
            semantic_retry_delay_ms: 15 * 60 * 1_000,
            max_provider_rounds: 1,
            max_provider_retries: 1,
            max_tool_calls: 0,
            max_input_bytes: 64 * 1024,
            max_output_bytes: 16 * 1024,
            max_projected_writes: 10,
            remote_allowed_kinds: vec![
                MemoryRecordKindV1::GoalOutcome,
                MemoryRecordKindV1::SemanticFact,
                MemoryRecordKindV1::Procedure,
                MemoryRecordKindV1::ArchitectureDecision,
            ],
        }
    }
}

impl MemoryPolicyConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.version.trim().is_empty() && self.version.len() <= 128,
            "memory policy version is empty or exceeds byte limit"
        );
        anyhow::ensure!(
            !self.semantic_profile.trim().is_empty() && self.semantic_profile.len() <= 512,
            "memory semantic profile is empty or exceeds byte limit"
        );
        let positive_weights = [
            self.evidence_provenance_weight,
            self.future_utility_weight,
            self.stability_weight,
            self.novelty_dedup_weight,
            self.scope_fit_weight,
            self.verification_weight,
        ];
        anyhow::ensure!(
            positive_weights.iter().all(|value| *value >= 0)
                && positive_weights.iter().sum::<i16>() == 100,
            "memory policy positive axis weights must be nonnegative and total 100"
        );
        anyhow::ensure!(
            (0..=100).contains(&self.privacy_risk_max_penalty)
                && (0..=100).contains(&self.contradiction_risk_max_penalty),
            "memory policy risk penalties are invalid"
        );
        anyhow::ensure!(
            (0..=100).contains(&self.candidate_threshold)
                && self.candidate_threshold < self.promote_local_threshold
                && self.promote_local_threshold <= self.remote_projection_threshold
                && self.remote_projection_threshold <= 100,
            "memory policy thresholds are invalid"
        );
        anyhow::ensure!(
            (1_000..=300_000).contains(&self.lease_duration_ms)
                && (1..=100).contains(&self.max_items_per_run)
                && (1_000..=600_000).contains(&self.run_deadline_ms)
                && (1_000..=86_400_000).contains(&self.semantic_retry_delay_ms),
            "memory policy scheduling limits are invalid"
        );
        anyhow::ensure!(
            (1..=4).contains(&self.max_provider_rounds)
                && self.max_provider_retries <= 3
                && self.max_tool_calls <= 16
                && (1..=256 * 1024).contains(&self.max_input_bytes)
                && (1..=64 * 1024).contains(&self.max_output_bytes)
                && self.max_projected_writes <= self.max_items_per_run,
            "memory policy run budgets are invalid"
        );
        anyhow::ensure!(
            !self.remote_allowed_kinds.is_empty() && self.remote_allowed_kinds.len() <= 16,
            "memory policy remote kind allow-list is invalid"
        );
        let mut kinds = self.remote_allowed_kinds.clone();
        kinds.sort_by_key(|kind| format!("{kind:?}"));
        kinds.dedup();
        anyhow::ensure!(
            kinds.len() == self.remote_allowed_kinds.len()
                && !kinds.contains(&MemoryRecordKindV1::CoreState),
            "memory policy remote kind allow-list contains duplicates or core state"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_bounded_and_valid() {
        MemoryPolicyConfig::default().validate().unwrap();
    }

    #[test]
    fn rejects_invalid_weights_thresholds_and_run_budgets() {
        let policy = MemoryPolicyConfig {
            future_utility_weight: 19,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
        let mut policy = MemoryPolicyConfig::default();
        policy.candidate_threshold = policy.promote_local_threshold;
        assert!(policy.validate().is_err());
        let policy = MemoryPolicyConfig {
            max_items_per_run: 0,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }
}
