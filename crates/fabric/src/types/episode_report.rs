//! Episode report — the authoritative, evidence-backed record of one embodied
//! robot task. The natural-language answer returned to the user is only a
//! projection of this structure; it is never a new source of truth.
//!
//! Only a `Matched` + settled episode may promote into long-term memory.

use serde::{Deserialize, Serialize};

use crate::types::embodiment::{DeviceId, EvidenceRef};
use crate::types::expected_outcome::ExpectedOutcome;
use crate::types::outcome_verification::{VerificationDecision, VerificationReport};

/// One recorded attempt within an episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub attempt: u32,
    /// Independent attempt identifier — always present, even when the
    /// underlying operation was never created.
    pub attempt_id: String,
    /// Host/provider-issued typed operation id, if the operation was created.
    /// `None` for pre-execution failures — never a fabricated id.
    pub operation_id: Option<String>,
    /// Expected outcome carried by the proposal for this attempt.
    pub expected: ExpectedOutcome,
    /// Provider terminal outcome (e.g. "succeeded"), if the attempt executed.
    pub result_outcome: Option<String>,
    pub verification_decision: Option<VerificationDecision>,
    pub verification_reasons: Vec<String>,
    /// Why a retry/replan was taken (from VerificationReport reasons), if any.
    pub retry_reason: Option<String>,
}

impl AttemptRecord {
    pub fn from_verification(
        attempt: u32,
        attempt_id: String,
        operation_id: Option<String>,
        expected: ExpectedOutcome,
        result_outcome: Option<String>,
        verification: Option<&VerificationReport>,
        retry_reason: Option<String>,
    ) -> Self {
        Self {
            attempt,
            attempt_id,
            operation_id,
            expected,
            result_outcome,
            verification_decision: verification.map(|v| v.decision.clone()),
            verification_reasons: verification.map(|v| v.reasons.clone()).unwrap_or_default(),
            retry_reason,
        }
    }
}

/// Structured, serializable record of one embodied task episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeReport {
    pub episode_id: String,
    pub goal: String,
    pub device: DeviceId,
    pub sim_scene_version: String,
    pub aletheon_commit: String,
    pub bridge_protocol_digest: String,
    pub before_sequence: Option<u64>,
    pub after_sequence: Option<u64>,
    pub settlement: String,
    pub attempts: Vec<AttemptRecord>,
    /// Large artifacts (rosbag/log/plot) are referenced here, never inlined.
    pub artifacts: Vec<EvidenceRef>,
}

/// Inputs required to assemble an authoritative episode report.
///
/// Keeping this contract typed avoids positional argument drift as report
/// metadata evolves.
pub struct EpisodeReportInput {
    pub episode_id: String,
    pub goal: String,
    pub device: DeviceId,
    pub sim_scene_version: String,
    pub aletheon_commit: String,
    pub bridge_protocol_digest: String,
    pub before_sequence: Option<u64>,
    pub after_sequence: Option<u64>,
    pub settlement: String,
    pub attempts: Vec<AttemptRecord>,
    pub artifacts: Vec<EvidenceRef>,
}

impl EpisodeReport {
    pub fn final_decision(&self) -> Option<VerificationDecision> {
        self.attempts
            .last()
            .and_then(|attempt| attempt.verification_decision.clone())
    }

    /// Promotion gate: only a `Matched` and settled episode may promote into
    /// Mnemosyne. Failed/unknown episodes keep their failure evidence but are
    /// never distilled as "successful experience".
    pub fn can_promote(&self) -> bool {
        self.settlement == "completed"
            && self.final_decision() == Some(VerificationDecision::Matched)
    }
}

/// Assemble an episode report from host metadata and the recorded attempts.
pub fn build_report(input: EpisodeReportInput) -> EpisodeReport {
    EpisodeReport {
        episode_id: input.episode_id,
        goal: input.goal,
        device: input.device,
        sim_scene_version: input.sim_scene_version,
        aletheon_commit: input.aletheon_commit,
        bridge_protocol_digest: input.bridge_protocol_digest,
        before_sequence: input.before_sequence,
        after_sequence: input.after_sequence,
        settlement: input.settlement,
        attempts: input.attempts,
        artifacts: input.artifacts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::expected_outcome::OutcomePredicate;
    use crate::types::outcome_verification::VerificationReport;

    fn expected() -> ExpectedOutcome {
        ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 5_000,
        }
    }

    fn matched_report() -> VerificationReport {
        VerificationReport {
            decision: VerificationDecision::Matched,
            evaluated_sequence: 2,
            observed_paths: vec![],
            reasons: vec![],
            evidence: vec![],
        }
    }

    fn sample_report(settlement: &str) -> EpisodeReport {
        build_report(EpisodeReportInput {
            episode_id: "ep-1".into(),
            goal: "stand".into(),
            device: DeviceId("kuavo-mujoco-01".into()),
            sim_scene_version: "mujoco-v1".into(),
            aletheon_commit: "abc123".into(),
            bridge_protocol_digest: "sha256:proto".into(),
            before_sequence: Some(1),
            after_sequence: Some(2),
            settlement: settlement.into(),
            attempts: vec![AttemptRecord::from_verification(
                1,
                "attempt:ep-1:1".into(),
                Some("00000000-0000-0000-0000-000000000001".into()),
                expected(),
                Some("succeeded".into()),
                Some(&matched_report()),
                None,
            )],
            artifacts: vec![EvidenceRef {
                kind: "rosbag".into(),
                uri: "artifact://sha256/rosbag".into(),
            }],
        })
    }

    #[test]
    fn matched_and_settled_can_promote() {
        assert!(sample_report("completed").can_promote());
    }

    #[test]
    fn failed_or_unknown_never_promote() {
        assert!(!sample_report("failed").can_promote());
        let mut report = sample_report("completed");
        report.attempts[0].verification_decision = Some(VerificationDecision::Unsafe);
        assert!(!report.can_promote());
        report.attempts[0].verification_decision = Some(VerificationDecision::Unknown);
        assert!(!report.can_promote());
        report.attempts[0].verification_decision = None;
        assert!(!report.can_promote());
    }

    #[test]
    fn report_serde_round_trips() {
        let report = sample_report("completed");
        let json = serde_json::to_string(&report).unwrap();
        let decoded: EpisodeReport = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, report);
        assert_eq!(decoded.artifacts[0].kind, "rosbag");
    }
}
