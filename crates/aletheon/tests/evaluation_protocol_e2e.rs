use std::sync::{Arc, Mutex};

use application::evaluation_projection::{
    EvaluationProjection, EvaluationProjectionContext, EvaluationProjectionMetrics,
    EvaluationProjectionRecord, EvaluationProjectionSink,
};
use async_trait::async_trait;

struct RecordingSink {
    name: &'static str,
    receipt_ids: Mutex<Vec<::contracts::EvaluationReceiptId>>,
    failures_remaining: Mutex<usize>,
}

impl RecordingSink {
    fn new(name: &'static str, failures_remaining: usize) -> Arc<Self> {
        Arc::new(Self {
            name,
            receipt_ids: Mutex::new(Vec::new()),
            failures_remaining: Mutex::new(failures_remaining),
        })
    }

    fn receipt_ids(&self) -> Vec<::contracts::EvaluationReceiptId> {
        self.receipt_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl EvaluationProjectionSink for RecordingSink {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn project(&self, record: &EvaluationProjectionRecord) -> anyhow::Result<()> {
        let mut failures = self
            .failures_remaining
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *failures > 0 {
            *failures -= 1;
            anyhow::bail!("injected projection failure");
        }
        drop(failures);
        self.receipt_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(record.receipt.receipt_id);
        Ok(())
    }
}

fn persisted_receipt() -> EvaluationProjectionRecord {
    EvaluationProjectionRecord {
        receipt: ::contracts::EvaluationReceiptRef {
            schema_version: ::contracts::EVALUATION_SCHEMA_V1,
            receipt_id: ::contracts::EvaluationReceiptId::new(),
            contract_id: ::contracts::EvaluationContractId::new(),
            subject_kind: "turn".into(),
            subject_id: ::contracts::TurnId::new().0.to_string(),
            decision: ::contracts::EvaluationDecision::ObservedFail,
            weighted_total_millis: Some(61_000),
            evidence_coverage_millis: 800,
            confidence_millis: 900,
            failed_gates: vec!["required_verification_passed".into()],
            created_at_ms: 1,
        },
        context: EvaluationProjectionContext {
            session_id: uuid::Uuid::new_v4().to_string(),
            runtime_id: "native-turn".into(),
            profile_id: "code-agent".into(),
            effective_model_id: "leju/deepseek-v4-pro".into(),
            model_display_name: "deepseek-v4-pro".into(),
            workspace_boundary_sha256: "workspace-digest".into(),
            verification_selection_sha256: "verification-digest".into(),
            rubric_id: "coding-v2".into(),
            rubric_version: 2,
            process_id: ::contracts::ProcessId::new(),
            metrics: EvaluationProjectionMetrics {
                elapsed_ms: Some(120),
                inference_rounds: Some(2),
                provider_retries: Some(1),
                tool_calls: Some(4),
                ..Default::default()
            },
        },
    }
}

#[tokio::test]
async fn every_projection_observes_the_same_persisted_receipt_id() {
    let goal = RecordingSink::new("goal", 0);
    let agent = RecordingSink::new("agent_control", 0);
    let memory = RecordingSink::new("mnemosyne", 0);
    // Prove a transient consumer failure is retried without suppressing peers.
    let agora = RecordingSink::new("agora", 1);
    let dasein = RecordingSink::new("dasein", 0);
    let rollup = RecordingSink::new("capability_rollup", 0);
    let projection = EvaluationProjection::new(vec![
        goal.clone(),
        agent.clone(),
        memory.clone(),
        agora.clone(),
        dasein.clone(),
        rollup.clone(),
    ]);
    let receipt = persisted_receipt();
    let receipt_id = receipt.receipt.receipt_id;

    let report = projection.project(receipt).await;

    assert!(report.failures.is_empty());
    assert_eq!(goal.receipt_ids(), vec![receipt_id]);
    assert_eq!(agent.receipt_ids(), vec![receipt_id]);
    assert_eq!(memory.receipt_ids(), vec![receipt_id]);
    assert_eq!(agora.receipt_ids(), vec![receipt_id]);
    assert_eq!(dasein.receipt_ids(), vec![receipt_id]);
    assert_eq!(rollup.receipt_ids(), vec![receipt_id]);
}
