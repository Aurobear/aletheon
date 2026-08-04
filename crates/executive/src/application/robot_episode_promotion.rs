//! Mnemosyne promotion trigger for settled robot episodes.
//!
//! Distills a `Matched` + settled episode into the governed fact store. The
//! promotion gate (`EpisodeReport::can_promote`) lives on the report itself;
//! this port is only ever invoked with a promotable report.

use std::sync::Arc;

use async_trait::async_trait;
use cognit::harness::robot::EpisodePromotionPort;
use fabric::types::episode_report::EpisodeReport;

/// Production promoter backed by Mnemosyne's governed `FactUseCases`.
pub struct MnemosyneEpisodePromoter {
    facts: Arc<dyn mnemosyne::FactUseCases>,
}

impl MnemosyneEpisodePromoter {
    pub fn new(facts: Arc<dyn mnemosyne::FactUseCases>) -> Self {
        Self { facts }
    }
}

#[async_trait]
impl EpisodePromotionPort for MnemosyneEpisodePromoter {
    async fn promote(&self, report: &EpisodeReport) -> Result<(), String> {
        let summary = format!(
            "robot episode {} succeeded: '{}' on {} in {} attempt(s), final decision {:?}",
            report.episode_id,
            report.goal,
            report.device.0,
            report.attempts.len(),
            report.final_decision(),
        );
        self.facts
            .add(mnemosyne::AddFactRequest {
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
    use fabric::types::embodiment::{DeviceId, EvidenceRef};
    use fabric::types::episode_report::{build_report, AttemptRecord};
    use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
    use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingFacts {
        requests: Mutex<Vec<mnemosyne::AddFactRequest>>,
    }

    #[async_trait::async_trait]
    impl mnemosyne::FactUseCases for RecordingFacts {
        async fn add(
            &self,
            request: mnemosyne::AddFactRequest,
        ) -> Result<i64, mnemosyne::FactServiceError> {
            self.requests.lock().unwrap().push(request);
            Ok(1)
        }
        async fn list(
            &self,
            _request: mnemosyne::ListFactsRequest,
        ) -> Result<Vec<mnemosyne::FactView>, mnemosyne::FactServiceError> {
            Ok(vec![])
        }
        async fn search(
            &self,
            _request: mnemosyne::SearchFactsRequest,
        ) -> Result<Vec<mnemosyne::FactView>, mnemosyne::FactServiceError> {
            Ok(vec![])
        }
        async fn show(
            &self,
            _fact_id: i64,
        ) -> Result<mnemosyne::FactView, mnemosyne::FactServiceError> {
            Err(mnemosyne::FactServiceError::Store("not in test".into()))
        }
        async fn forget(
            &self,
            _fact_id: i64,
            _hard: bool,
        ) -> Result<bool, mnemosyne::FactServiceError> {
            Ok(false)
        }
        async fn set_pinned(
            &self,
            _fact_id: i64,
            _pinned: bool,
        ) -> Result<bool, mnemosyne::FactServiceError> {
            Ok(false)
        }
    }

    fn promotable_report() -> EpisodeReport {
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
        build_report(
            "ep-1",
            "stand",
            DeviceId("kuavo-mujoco-01".into()),
            "mujoco-v1",
            "abc123",
            "sha256:proto",
            Some(1),
            Some(2),
            "completed",
            vec![AttemptRecord::from_verification(
                1,
                "attempt:ep-1:1".into(),
                None,
                expected,
                Some("succeeded".into()),
                Some(&verification),
                None,
            )],
            vec![],
        )
    }

    #[tokio::test]
    async fn matched_settled_episode_distills_into_fact_store() {
        let facts = Arc::new(RecordingFacts::default());
        let promoter = MnemosyneEpisodePromoter::new(facts.clone());

        let report = promotable_report();
        assert!(report.can_promote(), "fixture must be promotable");
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
}
