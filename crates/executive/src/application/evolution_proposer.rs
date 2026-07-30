//! Verify-only evolution proposal adapter. It can create pending approval but
//! owns neither admission nor apply authority.

use crate::application::{
    approval::{ApprovalCreate, ApprovalRepository},
    capability_benchmark::CapabilityRollupProjectionSink,
    goal::ObjectiveStore,
};
use fabric::{ApprovalCategory, ApprovalRisk, ApprovalSubject, GoalBudget, GoalSpec, PrincipalId};
use rusqlite::OptionalExtension;
use sha2::Digest;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
};

pub struct GovernedEvolutionProposer {
    goals: Arc<Mutex<ObjectiveStore>>,
    approvals: Arc<Mutex<ApprovalRepository>>,
    evidence: Arc<CapabilityRollupProjectionSink>,
    clock: Arc<dyn fabric::Clock>,
    state: Mutex<rusqlite::Connection>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::evaluation::{
        EvaluationProjectionContext, EvaluationProjectionMetrics, EvaluationProjectionRecord,
        EvaluationProjectionSink,
    };
    use kernel::chronos::SystemClock;

    fn record(session: &str, profile: &str) -> EvaluationProjectionRecord {
        EvaluationProjectionRecord {
            receipt: fabric::EvaluationReceiptRef {
                schema_version: fabric::EVALUATION_SCHEMA_V1,
                receipt_id: fabric::EvaluationReceiptId::new(),
                contract_id: fabric::EvaluationContractId::new(),
                subject_kind: "turn".into(),
                subject_id: fabric::TurnId::new().0.to_string(),
                decision: fabric::EvaluationDecision::ObservedPass,
                weighted_total_millis: Some(900_000),
                evidence_coverage_millis: 900,
                confidence_millis: 900,
                failed_gates: vec![],
                created_at_ms: 1,
            },
            context: EvaluationProjectionContext {
                session_id: session.into(),
                runtime_id: "native".into(),
                profile_id: profile.into(),
                effective_model_id: "test/model".into(),
                model_display_name: "test".into(),
                workspace_boundary_sha256: "a".repeat(64),
                verification_selection_sha256: "b".repeat(64),
                rubric_id: "coding-v2".into(),
                rubric_version: 2,
                process_id: fabric::ProcessId::new(),
                metrics: EvaluationProjectionMetrics::default(),
            },
        }
    }

    #[tokio::test]
    async fn requires_distinct_durable_profiles_and_only_creates_pending_approval() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("goals.db");
        let evidence =
            Arc::new(CapabilityRollupProjectionSink::open(temp.path().join("eval.db")).unwrap());
        evidence.project(&record("s", "executor")).await.unwrap();
        evidence.project(&record("s", "reviewer")).await.unwrap();
        let proposer = GovernedEvolutionProposer::new(
            Arc::new(Mutex::new(ObjectiveStore::open(&db).unwrap())),
            Arc::new(Mutex::new(ApprovalRepository::open(&db).unwrap())),
            evidence,
            Arc::new(SystemClock::new()),
            temp.path().join("proposals.db"),
        )
        .unwrap();
        let receipt = metacog::VerificationReceipt {
            mutation_id: uuid::Uuid::new_v4(),
            candidate_id: uuid::Uuid::new_v4(),
            base_version: "0.1.0".into(),
            decision: metacog::VerificationDecision::Adopt,
            score: 1.0,
            verification_hash: "c".repeat(64),
            verified_at_ms: 1,
        };
        let approval = proposer
            .propose("s", PrincipalId("owner".into()), &receipt)
            .unwrap()
            .unwrap();
        assert_eq!(approval.status, fabric::ApprovalStatus::Pending);
        assert_eq!(approval.category, ApprovalCategory::DaseinModification);
        assert_eq!(
            approval.subject.attributes["mutation_id"],
            receipt.mutation_id.to_string()
        );
    }
}

impl GovernedEvolutionProposer {
    pub fn new(
        goals: Arc<Mutex<ObjectiveStore>>,
        approvals: Arc<Mutex<ApprovalRepository>>,
        evidence: Arc<CapabilityRollupProjectionSink>,
        clock: Arc<dyn fabric::Clock>,
        state_path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<Self> {
        let state = rusqlite::Connection::open(state_path)?;
        state.execute_batch(
            "PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS evolution_proposals (
              mutation_id TEXT PRIMARY KEY NOT NULL,
              status TEXT NOT NULL, reason TEXT, approval_id TEXT,
              evidence_digest TEXT, updated_at_ms INTEGER NOT NULL
            );",
        )?;
        Ok(Self {
            goals,
            approvals,
            evidence,
            clock,
            state: Mutex::new(state),
        })
    }

    pub fn propose(
        &self,
        session_id: &str,
        owner: PrincipalId,
        receipt: &metacog::VerificationReceipt,
    ) -> anyhow::Result<Option<fabric::ApprovalSnapshot>> {
        if receipt.decision != metacog::VerificationDecision::Adopt {
            return Ok(None);
        }
        let existing = self.state.lock().unwrap().query_row(
            "SELECT approval_id FROM evolution_proposals WHERE mutation_id=?1 AND status='pending_approval'",
            rusqlite::params![receipt.mutation_id.to_string()],
            |row| row.get::<_, Option<String>>(0),
        ).optional()?;
        if let Some(Some(id)) = existing {
            return Ok(self
                .approvals
                .lock()
                .unwrap()
                .get(fabric::ApprovalId(uuid::Uuid::parse_str(&id)?))?);
        }
        let evidence = self
            .evidence
            .evidence_for_session(session_id, "coding-v2")?;
        let profiles = evidence
            .iter()
            .map(|r| r.context.profile_id.as_str())
            .collect::<HashSet<_>>();
        // Require both a coding receipt and a distinct role/profile signal.
        // Thin or single-agent evidence parks the candidate without approval.
        if evidence.len() < 2 || profiles.len() < 2 {
            self.state.lock().unwrap().execute(
                "INSERT INTO evolution_proposals (mutation_id,status,reason,approval_id,evidence_digest,updated_at_ms)
                 VALUES (?1,'parked','insufficient_evidence',NULL,NULL,?2)
                 ON CONFLICT(mutation_id) DO UPDATE SET status='parked',reason='insufficient_evidence',updated_at_ms=excluded.updated_at_ms",
                rusqlite::params![receipt.mutation_id.to_string(), self.clock.wall_now().0],
            )?;
            return Ok(None);
        }
        let evidence_ids = evidence
            .iter()
            .map(|r| r.receipt.receipt_id.0.to_string())
            .collect::<Vec<_>>();
        let evidence_digest = format!(
            "{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&evidence_ids)?)
        );
        let goal = self.goals.lock().unwrap().create_draft_goal(
            &owner,
            session_id,
            "session",
            &GoalSpec {
                original_intent: format!("Review verified genome mutation {}", receipt.mutation_id),
                desired_state: vec!["operator-reviewed governed genome mutation".into()],
                constraints: vec!["genome-only".into(), "human approval required".into()],
                acceptance_criteria: vec!["verification binding remains exact".into()],
                budget: GoalBudget {
                    max_input_tokens: 0,
                    max_output_tokens: 0,
                    max_cost_usd: Some(0.0),
                    max_attempts: 1,
                    deadline_ms: None,
                },
            },
        )?;
        let mut attributes = BTreeMap::new();
        attributes.insert("mutation_id".into(), receipt.mutation_id.to_string());
        attributes.insert("operation".into(), "apply".into());
        attributes.insert(
            "verification_hash".into(),
            receipt.verification_hash.clone(),
        );
        attributes.insert("base_version".into(), receipt.base_version.clone());
        attributes.insert("evidence_receipts_sha256".into(), evidence_digest.clone());
        attributes.insert("evidence_receipt_count".into(), evidence.len().to_string());
        let now = self.clock.wall_now().0;
        let approval = self.approvals.lock().unwrap().create(ApprovalCreate {
            subject: ApprovalSubject {
                category: ApprovalCategory::DaseinModification,
                goal_id: goal.id,
                attempt_id: None,
                job_id: None,
                attributes,
                allowed_scope: Vec::new(),
                apply_target: None,
            },
            risk: ApprovalRisk::Critical,
            summary: format!("Apply verified genome mutation {}", receipt.mutation_id),
            artifacts: Vec::new(),
            created_at_ms: now,
            expires_at_ms: now.saturating_add(24 * 60 * 60 * 1000),
        })?;
        self.state.lock().unwrap().execute(
            "INSERT INTO evolution_proposals (mutation_id,status,reason,approval_id,evidence_digest,updated_at_ms)
             VALUES (?1,'pending_approval',NULL,?2,?3,?4)
             ON CONFLICT(mutation_id) DO UPDATE SET status='pending_approval',reason=NULL,approval_id=excluded.approval_id,evidence_digest=excluded.evidence_digest,updated_at_ms=excluded.updated_at_ms",
            rusqlite::params![receipt.mutation_id.to_string(), approval.id.0.to_string(), evidence_digest, now],
        )?;
        Ok(Some(approval))
    }
}
