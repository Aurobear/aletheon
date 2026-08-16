//! Deterministic host judgment for governed memory observations.

//! Versioned host-owned memory judgment policy.

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

use crate::GovernedMemoryObservation;
use ::contracts::protocol::memory::{
    MemoryObservationKindV1, MemoryRecordKindV1, MemoryScorecardV1, MemorySensitivityV1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryNovelty {
    New,
    SemanticRelated,
    ExactDuplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAxisEvidence {
    pub evidence_provenance_millis: u16,
    pub future_utility_millis: u16,
    pub stability_millis: u16,
    pub novelty_dedup_millis: u16,
    pub scope_fit_millis: u16,
    pub verification_millis: u16,
    pub privacy_risk_millis: u16,
    pub contradiction_risk_millis: u16,
}

impl MemoryAxisEvidence {
    fn validate(self) -> anyhow::Result<Self> {
        anyhow::ensure!(
            [
                self.evidence_provenance_millis,
                self.future_utility_millis,
                self.stability_millis,
                self.novelty_dedup_millis,
                self.scope_fit_millis,
                self.verification_millis,
                self.privacy_risk_millis,
                self.contradiction_risk_millis,
            ]
            .into_iter()
            .all(|value| value <= 1_000),
            "memory axis evidence exceeds normalized range"
        );
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPolicyFacts {
    pub provenance_complete: bool,
    pub scope_verified: bool,
    pub scrub_passed: bool,
    pub control_instruction_detected: bool,
    pub model_only_claim: bool,
    pub approved_core_conflict: bool,
    pub binding_verified: bool,
    pub verification_receipts: u16,
    pub novelty: MemoryNovelty,
    pub contradiction_unresolved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPolicyDecisionKind {
    Reject,
    Candidate,
    PromoteLocal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPolicyDecision {
    pub kind: MemoryPolicyDecisionKind,
    pub record_kind: MemoryRecordKindV1,
    pub scorecard: MemoryScorecardV1,
    pub hard_gate_reasons: Vec<String>,
    pub remote_eligible: bool,
    pub remote_block_reasons: Vec<String>,
}

pub struct MemoryPolicyEvaluator {
    config: MemoryPolicyConfig,
}

impl MemoryPolicyEvaluator {
    pub fn new(config: MemoryPolicyConfig) -> anyhow::Result<Self> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &MemoryPolicyConfig {
        &self.config
    }

    pub fn derive_axes(
        &self,
        observation: &GovernedMemoryObservation,
        facts: MemoryPolicyFacts,
    ) -> MemoryAxisEvidence {
        let refs = u16::try_from(observation.source_refs.len()).unwrap_or(u16::MAX);
        let evidence_provenance_millis = 200u16
            .saturating_add(refs.saturating_mul(250))
            .saturating_add(facts.verification_receipts.saturating_mul(100))
            .min(1_000);
        let mut future_utility_millis: u16 = match observation.kind {
            MemoryObservationKindV1::UserMessage | MemoryObservationKindV1::AssistantMessage => 250,
            MemoryObservationKindV1::ToolOutcome => 600,
            MemoryObservationKindV1::TaskOutcome => 900,
            MemoryObservationKindV1::ExplicitNote | MemoryObservationKindV1::Correction => 900,
            MemoryObservationKindV1::Feedback => 700,
        };
        if observation.explicit_user_action {
            future_utility_millis = future_utility_millis.saturating_add(100).min(1_000);
        }
        let mut stability_millis: u16 = match observation.kind {
            MemoryObservationKindV1::UserMessage | MemoryObservationKindV1::AssistantMessage => 250,
            MemoryObservationKindV1::ToolOutcome => 400,
            MemoryObservationKindV1::TaskOutcome => 800,
            MemoryObservationKindV1::ExplicitNote => 900,
            MemoryObservationKindV1::Correction => 850,
            MemoryObservationKindV1::Feedback => 600,
        };
        if observation.occurred_at.is_some() {
            stability_millis = stability_millis.saturating_add(100).min(1_000);
        }
        let novelty_dedup_millis = match facts.novelty {
            MemoryNovelty::New => 1_000,
            MemoryNovelty::SemanticRelated => 500,
            MemoryNovelty::ExactDuplicate => 0,
        };
        let verification_millis = refs
            .saturating_mul(300)
            .saturating_add(facts.verification_receipts.saturating_mul(300))
            .min(1_000);
        let privacy_risk_millis = if observation.scrub_redactions > 0
            || observation.sensitivity == MemorySensitivityV1::Restricted
        {
            1_000
        } else if observation.sensitivity == MemorySensitivityV1::Confidential {
            600
        } else {
            0
        };
        MemoryAxisEvidence {
            evidence_provenance_millis,
            future_utility_millis,
            stability_millis,
            novelty_dedup_millis,
            scope_fit_millis: if facts.scope_verified { 1_000 } else { 0 },
            verification_millis,
            privacy_risk_millis,
            contradiction_risk_millis: if facts.contradiction_unresolved {
                1_000
            } else {
                0
            },
        }
    }

    pub fn evaluate(
        &self,
        observation: &GovernedMemoryObservation,
        facts: MemoryPolicyFacts,
        axes: MemoryAxisEvidence,
    ) -> anyhow::Result<MemoryPolicyDecision> {
        observation.validate()?;
        let axes = axes.validate()?;
        let mut hard_gate_reasons = Vec::new();
        if !facts.scrub_passed {
            hard_gate_reasons.push("scrub_failed".into());
        }
        if !facts.provenance_complete {
            hard_gate_reasons.push("provenance_incomplete".into());
        }
        if !facts.scope_verified {
            hard_gate_reasons.push("scope_unverified".into());
        }
        if facts.control_instruction_detected {
            hard_gate_reasons.push("control_instruction_detected".into());
        }
        if facts.model_only_claim {
            hard_gate_reasons.push("model_only_claim".into());
        }
        if facts.approved_core_conflict {
            hard_gate_reasons.push("approved_core_conflict".into());
        }
        let scorecard = score(&self.config, axes);
        let kind =
            if !hard_gate_reasons.is_empty() || scorecard.total < self.config.candidate_threshold {
                MemoryPolicyDecisionKind::Reject
            } else if scorecard.total < self.config.promote_local_threshold {
                MemoryPolicyDecisionKind::Candidate
            } else {
                MemoryPolicyDecisionKind::PromoteLocal
            };
        let record_kind = record_kind(observation.kind);
        let mut remote_block_reasons = Vec::new();
        if kind != MemoryPolicyDecisionKind::PromoteLocal {
            remote_block_reasons.push("not_promoted_local".into());
        }
        if scorecard.total < self.config.remote_projection_threshold {
            remote_block_reasons.push("remote_score_below_threshold".into());
        }
        if !facts.binding_verified {
            remote_block_reasons.push("binding_unverified".into());
        }
        if !matches!(
            observation.sensitivity,
            MemorySensitivityV1::Public | MemorySensitivityV1::Internal
        ) {
            remote_block_reasons.push("sensitivity_not_projectable".into());
        }
        if !self.config.remote_allowed_kinds.contains(&record_kind) {
            remote_block_reasons.push("kind_not_projectable".into());
        }
        if !hard_gate_reasons.is_empty() {
            remote_block_reasons.push("hard_gate_failed".into());
        }
        remote_block_reasons.sort();
        remote_block_reasons.dedup();
        Ok(MemoryPolicyDecision {
            kind,
            record_kind,
            scorecard,
            hard_gate_reasons,
            remote_eligible: remote_block_reasons.is_empty(),
            remote_block_reasons,
        })
    }
}

fn weighted(maximum: i16, millis: u16) -> i16 {
    ((i32::from(maximum) * i32::from(millis) + 500) / 1_000) as i16
}

fn score(config: &MemoryPolicyConfig, axes: MemoryAxisEvidence) -> MemoryScorecardV1 {
    let evidence_provenance = weighted(
        config.evidence_provenance_weight,
        axes.evidence_provenance_millis,
    );
    let future_utility = weighted(config.future_utility_weight, axes.future_utility_millis);
    let stability = weighted(config.stability_weight, axes.stability_millis);
    let novelty_dedup = weighted(config.novelty_dedup_weight, axes.novelty_dedup_millis);
    let scope_fit = weighted(config.scope_fit_weight, axes.scope_fit_millis);
    let verification = weighted(config.verification_weight, axes.verification_millis);
    let privacy_risk = -weighted(config.privacy_risk_max_penalty, axes.privacy_risk_millis);
    let contradiction_risk = -weighted(
        config.contradiction_risk_max_penalty,
        axes.contradiction_risk_millis,
    );
    // Round the aggregate once. Rounding each displayed axis independently can
    // otherwise move a threshold boundary by one point even though the policy
    // weights sum exactly to 100.
    let total_millis = i32::from(config.evidence_provenance_weight)
        * i32::from(axes.evidence_provenance_millis)
        + i32::from(config.future_utility_weight) * i32::from(axes.future_utility_millis)
        + i32::from(config.stability_weight) * i32::from(axes.stability_millis)
        + i32::from(config.novelty_dedup_weight) * i32::from(axes.novelty_dedup_millis)
        + i32::from(config.scope_fit_weight) * i32::from(axes.scope_fit_millis)
        + i32::from(config.verification_weight) * i32::from(axes.verification_millis)
        - i32::from(config.privacy_risk_max_penalty) * i32::from(axes.privacy_risk_millis)
        - i32::from(config.contradiction_risk_max_penalty)
            * i32::from(axes.contradiction_risk_millis);
    let total = if total_millis <= 0 {
        0
    } else {
        ((total_millis + 500) / 1_000).clamp(0, 100) as i16
    };
    MemoryScorecardV1 {
        policy_version: config.version.clone(),
        evidence_provenance,
        future_utility,
        stability,
        novelty_dedup,
        scope_fit,
        verification,
        privacy_risk,
        contradiction_risk,
        total,
    }
}

fn record_kind(kind: MemoryObservationKindV1) -> MemoryRecordKindV1 {
    match kind {
        MemoryObservationKindV1::UserMessage | MemoryObservationKindV1::AssistantMessage => {
            MemoryRecordKindV1::Message
        }
        MemoryObservationKindV1::ToolOutcome => MemoryRecordKindV1::ToolOutcome,
        MemoryObservationKindV1::TaskOutcome => MemoryRecordKindV1::GoalOutcome,
        MemoryObservationKindV1::ExplicitNote | MemoryObservationKindV1::Correction => {
            MemoryRecordKindV1::SemanticFact
        }
        MemoryObservationKindV1::Feedback => MemoryRecordKindV1::ExternalReference,
    }
}
