//! Deterministic host judgment for governed memory observations.

use fabric::protocol::memory::{
    MemoryObservationKindV1, MemoryRecordKindV1, MemoryScorecardV1, MemorySensitivityV1,
};
use mnemosyne::GovernedMemoryObservation;

use crate::composition::config::MemoryPolicyConfig;

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
