use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use executive::r#impl::goal::{
    GoalApprovalOutcomeSummary, GoalCompletionSummary, GoalProjectionEvidence,
};
use executive::r#impl::memory_projection::{
    ApprovedArchitectureDecision, MemoryProjection, ProjectionStatus,
};
use fabric::{ApprovalId, GoalId};
use mnemosyne::service::MemoryScope;
use mnemosyne::{
    ExperienceEvent, ForgetPolicy, MemorySensitivity, MemoryService, RecallRequest, RecallSet,
};
use uuid::Uuid;

#[derive(Default)]
struct CapturingMemory {
    events: Mutex<Vec<ExperienceEvent>>,
    fail: bool,
}

#[async_trait]
impl MemoryService for CapturingMemory {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()> {
        if self.fail {
            anyhow::bail!("credential=must-not-leak");
        }
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    async fn recall(&self, _: RecallRequest) -> anyhow::Result<RecallSet> {
        Ok(RecallSet { items: vec![] })
    }

    async fn consolidate(&self, _: MemoryScope) -> anyhow::Result<()> {
        Ok(())
    }

    async fn forget(&self, _: ForgetPolicy) -> anyhow::Result<()> {
        Ok(())
    }
}

fn summary(final_state: &str, approval_status: &str) -> GoalCompletionSummary {
    GoalCompletionSummary {
        goal_id: GoalId(42),
        approval_id: ApprovalId(Uuid::from_u128(7)),
        intent: "ship verified memory".into(),
        attempts: vec![],
        changed_files: vec!["src/lib.rs".into()],
        checks: vec![],
        approval: GoalApprovalOutcomeSummary {
            status: approval_status.into(),
            principal_id: Some("owner".into()),
            channel: Some("telegram".into()),
            reason: None,
            resolved_at_ms: Some(1_000),
        },
        apply: None,
        risks: vec!["bounded".into()],
        final_state: final_state.into(),
        generated_at_ms: 2_000,
    }
}

fn evidence() -> GoalProjectionEvidence {
    GoalProjectionEvidence {
        attempt_ids: vec![Uuid::from_u128(8).to_string()],
        artifact_ids: vec![Uuid::from_u128(9).to_string()],
        source_commit: Some("abc123".into()),
        verification: vec!["cargo test:true:passed".into()],
    }
}

#[tokio::test]
async fn terminal_and_rejected_summaries_include_durable_evidence() {
    for (state, approval) in [
        ("completed", "approved"),
        ("failed", "approved"),
        ("awaiting_human", "rejected"),
    ] {
        let memory = Arc::new(CapturingMemory::default());
        let projection = MemoryProjection::new(memory.clone());
        assert!(matches!(
            projection
                .project_goal_summary(
                    &summary(state, approval),
                    &evidence(),
                    MemorySensitivity::Internal,
                )
                .await,
            ProjectionStatus::Recorded { .. }
        ));
        let events = memory.events.lock().unwrap();
        let ExperienceEvent::GoalOutcome {
            content, metadata, ..
        } = &events[0]
        else {
            panic!("expected Goal outcome")
        };
        assert!(content.contains("attempt_ids"));
        assert!(content.contains("artifact_ids"));
        assert!(content.contains("verification"));
        assert_eq!(metadata.provenance.source_commit.as_deref(), Some("abc123"));
        assert_eq!(metadata.provenance.principal.as_deref(), Some("owner"));
    }
}

#[tokio::test]
async fn replayed_terminal_event_uses_the_same_record_id() {
    let memory = Arc::new(CapturingMemory::default());
    let projection = MemoryProjection::new(memory.clone());
    let first = projection
        .project_goal_summary(
            &summary("completed", "approved"),
            &evidence(),
            MemorySensitivity::Internal,
        )
        .await;
    let second = projection
        .project_goal_summary(
            &summary("completed", "approved"),
            &evidence(),
            MemorySensitivity::Internal,
        )
        .await;
    assert_eq!(first, second);
    let events = memory.events.lock().unwrap();
    let ids: Vec<_> = events
        .iter()
        .map(|event| match event {
            ExperienceEvent::GoalOutcome { metadata, .. } => metadata.record_id.clone(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(ids[0], ids[1]);
}

#[tokio::test]
async fn revised_decision_preserves_supersedes_edge_and_stable_identity() {
    let memory = Arc::new(CapturingMemory::default());
    let projection = MemoryProjection::new(memory.clone());
    let v1 = ApprovedArchitectureDecision {
        decision_id: "memory-boundary-v1".into(),
        approval_id: "approval-1".into(),
        title: "Memory boundary".into(),
        content: "Use local memory first.".into(),
        principal_id: "owner".into(),
        source_commit: "abc".into(),
        approved_at_ms: 1_000,
        supersedes: None,
        sensitivity: MemorySensitivity::Internal,
        approved: true,
    };
    let first = projection.project_architecture_decision(&v1).await;
    let ProjectionStatus::Recorded { record_id: old_id } = first else {
        panic!("decision was not recorded")
    };
    let mut v2 = v1.clone();
    v2.decision_id = "memory-boundary-v2".into();
    v2.approval_id = "approval-2".into();
    v2.content = "Use local-first composite memory.".into();
    v2.supersedes = Some(old_id.clone());
    projection.project_architecture_decision(&v2).await;

    let events = memory.events.lock().unwrap();
    let ExperienceEvent::ArchitectureDecision { metadata, .. } = &events[1] else {
        panic!("expected decision")
    };
    assert_eq!(metadata.supersedes.as_deref(), Some(old_id.as_str()));
    assert_ne!(metadata.record_id, old_id);
}

#[tokio::test]
async fn unapproved_and_sensitive_records_are_excluded() {
    let memory = Arc::new(CapturingMemory::default());
    let projection = MemoryProjection::new(memory.clone());
    assert!(matches!(
        projection
            .project_goal_summary(
                &summary("completed", "approved"),
                &evidence(),
                MemorySensitivity::Restricted,
            )
            .await,
        ProjectionStatus::Excluded { .. }
    ));
    let decision = ApprovedArchitectureDecision {
        decision_id: "draft".into(),
        approval_id: "none".into(),
        title: "Draft".into(),
        content: "not approved".into(),
        principal_id: "owner".into(),
        source_commit: "abc".into(),
        approved_at_ms: 1,
        supersedes: None,
        sensitivity: MemorySensitivity::Internal,
        approved: false,
    };
    assert!(matches!(
        projection.project_architecture_decision(&decision).await,
        ProjectionStatus::Excluded { .. }
    ));
    assert!(memory.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn memory_outage_is_sanitized_and_does_not_change_source_result() {
    let memory = Arc::new(CapturingMemory {
        events: Mutex::new(vec![]),
        fail: true,
    });
    let projection = MemoryProjection::new(memory);
    assert_eq!(
        projection
            .project_goal_summary(
                &summary("completed", "approved"),
                &evidence(),
                MemorySensitivity::Internal,
            )
            .await,
        ProjectionStatus::Degraded
    );
    let health = projection.health();
    let health = health.lock().unwrap();
    assert!(health.degraded);
    assert_eq!(health.last_error_category, Some("memory_record_failed"));
}
