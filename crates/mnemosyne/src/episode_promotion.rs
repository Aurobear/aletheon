//! Mnemosyne promotion trigger for settled robot episodes.
//!
//! Distills a `Matched` + settled episode into the governed fact store. Both
//! the receipt digest and promotion gate are re-checked at this boundary;
//! callers cannot turn a failed episode into successful experience.

use std::sync::Arc;

use ::contracts::types::episode_report::{EpisodePromotionPort, SettledEpisodeReport};
use async_trait::async_trait;

/// Production promoter backed by Mnemosyne's governed `FactUseCases`.
pub struct MnemosyneEpisodePromoter {
    facts: Arc<dyn crate::FactUseCases>,
}

impl MnemosyneEpisodePromoter {
    pub fn new(facts: Arc<dyn crate::FactUseCases>) -> Self {
        Self { facts }
    }
}

#[async_trait]
impl EpisodePromotionPort for MnemosyneEpisodePromoter {
    async fn promote(&self, settled: &SettledEpisodeReport) -> Result<(), String> {
        let can_promote = settled.can_promote()?;
        let report = settled.report();
        if !can_promote {
            return Err(format!(
                "episode {} is not completed with matched verification",
                report.episode_id
            ));
        }
        let summary = format!(
            "robot episode {} succeeded: '{}' on {} in {} attempt(s), final decision {:?}",
            report.episode_id,
            report.goal,
            report.device.0,
            report.attempts.len(),
            report.final_decision(),
        );
        self.facts
            .add(crate::AddFactRequest {
                content: summary,
                scope: "global".into(),
                subject: report.device.0.clone(),
                tags: format!("robot-episode,{}", report.device.0),
            })
            .await
            .map(|_| ())
            .map_err(|error| format!("fact add: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::types::embodiment::{DeviceId, EvidenceRef};
    use ::contracts::types::episode_report::{
        build_report, AttemptRecord, EpisodeArtifactManifest, EpisodeSettlement,
        SettledEpisodeReport,
    };
    use ::contracts::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
    use ::contracts::types::outcome_verification::{VerificationDecision, VerificationReport};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingFacts {
        requests: Mutex<Vec<crate::AddFactRequest>>,
    }

    #[async_trait::async_trait]
    impl crate::FactUseCases for RecordingFacts {
        async fn add(
            &self,
            request: crate::AddFactRequest,
        ) -> Result<i64, crate::FactServiceError> {
            self.requests.lock().unwrap().push(request);
            Ok(1)
        }
        async fn list(
            &self,
            _request: crate::ListFactsRequest,
        ) -> Result<Vec<crate::FactView>, crate::FactServiceError> {
            Ok(vec![])
        }
        async fn search(
            &self,
            _request: crate::SearchFactsRequest,
        ) -> Result<Vec<crate::FactView>, crate::FactServiceError> {
            Ok(vec![])
        }
        async fn show(&self, _fact_id: i64) -> Result<crate::FactView, crate::FactServiceError> {
            Err(crate::FactServiceError::Store("not in test".into()))
        }
        async fn forget(
            &self,
            _fact_id: i64,
            _hard: bool,
        ) -> Result<bool, crate::FactServiceError> {
            Ok(false)
        }
        async fn set_pinned(
            &self,
            _fact_id: i64,
            _pinned: bool,
        ) -> Result<bool, crate::FactServiceError> {
            Ok(false)
        }
    }

    fn report_with_settlement(settlement: EpisodeSettlement) -> SettledEpisodeReport {
        let expected = ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 5_000,
        };
        let verification = VerificationReport {
            decision: VerificationDecision::Matched,
            evaluated_sequence: 2,
            observed_paths: vec![],
            reasons: vec![],
            evidence: vec![EvidenceRef {
                kind: "rosbag".into(),
                uri: "artifact://sha256/rosbag".into(),
            }],
        };
        let report = build_report(::contracts::types::episode_report::EpisodeReportInput {
            episode_id: "ep-1".into(),
            goal: "stand".into(),
            device: DeviceId("kuavo-mujoco-01".into()),
            sim_scene_version: "mujoco-v1".into(),
            aletheon_commit: "abc123".into(),
            bridge_protocol_digest: "sha256:proto".into(),
            skill_descriptor_digest: "sha256:skills".into(),
            policy_provenance: Some(::contracts::types::skill_proposal::PolicyProvenance {
                provider: "local-policy-gateway".into(),
                model: "openvla-7b".into(),
                version: "2026-08".into(),
                protocol_version: "1.0".into(),
                digest: "sha256:model".into(),
            }),
            failures: vec![],
            safe_stop: (settlement
                != ::contracts::types::episode_report::EpisodeSettlement::Completed)
                .then_some(::contracts::types::episode_report::SafeStopReceipt {
                    attempted_after_attempt: 1,
                    trigger: None,
                    outcome: ::contracts::types::episode_report::SafeStopOutcome::Succeeded,
                }),
            selected_frames: vec![],
            settlement,
            attempts: vec![AttemptRecord::from_verification(
                1,
                "attempt:ep-1:1".into(),
                None,
                Some(::contracts::types::embodiment::SkillRequest {
                    skill: ::contracts::types::embodiment::SkillId("kuavo.stance".into()),
                    device: ::contracts::types::embodiment::DeviceId("kuavo-mujoco-01".into()),
                    parameters: serde_json::json!({}),
                }),
                expected,
                Some("succeeded".into()),
                Some(&verification),
                None,
                Some(1),
                Some(2),
            )],
            artifacts: verification
                .evidence
                .iter()
                .map(EpisodeArtifactManifest::from_legacy_evidence)
                .collect(),
        });
        SettledEpisodeReport::new(report, 1_000).unwrap()
    }

    fn promotable_report() -> SettledEpisodeReport {
        report_with_settlement(EpisodeSettlement::Completed)
    }

    #[tokio::test]
    async fn matched_settled_episode_distills_into_fact_store() {
        let facts = Arc::new(RecordingFacts::default());
        let promoter = MnemosyneEpisodePromoter::new(facts.clone());

        let report = promotable_report();
        assert!(report.report().can_promote(), "fixture must be promotable");
        promoter.promote(&report).await.expect("promote succeeds");

        let requests = facts.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].content.contains("stand"),
            "summary carries the goal"
        );
        assert!(requests[0].content.contains("kuavo-mujoco-01"));
        assert_eq!(requests[0].scope, "global");
        assert!(requests[0].tags.contains("robot-episode"));
    }

    #[tokio::test]
    async fn failed_episode_is_rejected_without_writing_a_fact() {
        let facts = Arc::new(RecordingFacts::default());
        let promoter = MnemosyneEpisodePromoter::new(facts.clone());
        let failed = report_with_settlement(EpisodeSettlement::Failed);

        assert!(!failed.can_promote().unwrap());
        assert_eq!(
            failed.report().artifacts.len(),
            1,
            "failed evidence is retained"
        );
        assert!(promoter.promote(&failed).await.is_err());
        assert!(facts.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tampered_receipt_is_rejected_before_fact_write() {
        let facts = Arc::new(RecordingFacts::default());
        let promoter = MnemosyneEpisodePromoter::new(facts.clone());
        let mut value = serde_json::to_value(promotable_report()).unwrap();
        value["report_sha256"] = serde_json::json!("invalid");
        let tampered: SettledEpisodeReport = serde_json::from_value(value).unwrap();

        assert!(promoter.promote(&tampered).await.is_err());
        assert!(facts.requests.lock().unwrap().is_empty());
    }
}
