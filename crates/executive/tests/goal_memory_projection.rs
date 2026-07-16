use std::sync::Arc;

use executive::r#impl::events::{
    memory_job_projection::MemoryJobProjection, DefaultEventProjectionSet, EventReadFilter,
    SqliteEventSpine,
};
use executive::r#impl::goal::{
    GoalApprovalOutcomeSummary, GoalCompletionSummary, GoalProjectionEvidence,
};
use executive::r#impl::memory_projection::{
    ApprovedArchitectureDecision, MemoryProjection, ProjectionStatus,
};
use executive::service::event_projection::SqliteProjectionStore;
use fabric::{ApprovalId, EventSpine, EventTreeId, GoalId, SpineEvent, UnsequencedEvent};
use mnemosyne::MemorySensitivity;
use uuid::Uuid;

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

fn harness() -> (
    MemoryProjection,
    Arc<SqliteEventSpine>,
    Arc<DefaultEventProjectionSet>,
) {
    let spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let projections = Arc::new(DefaultEventProjectionSet::in_memory());
    (
        MemoryProjection::new(spine.clone(), projections.clone()),
        spine,
        projections,
    )
}

fn read_source(spine: &SqliteEventSpine, source: &str) -> Vec<SpineEvent> {
    spine
        .read_tree(
            EventTreeId::for_root_session(source),
            EventReadFilter {
                limit: 100,
                ..Default::default()
            },
        )
        .unwrap()
}

#[tokio::test]
async fn terminal_and_rejected_summaries_queue_durable_source_evidence() {
    for (state, approval) in [
        ("completed", "approved"),
        ("failed", "approved"),
        ("awaiting_human", "rejected"),
    ] {
        let (projection, spine, _) = harness();
        assert!(matches!(
            projection
                .project_goal_summary(
                    &summary(state, approval),
                    &evidence(),
                    MemorySensitivity::Internal,
                )
                .await,
            ProjectionStatus::Queued { .. }
        ));
        let events = read_source(&spine, "goal:42");
        assert_eq!(events.len(), 1);
        let fabric::EventPayload::Inline { value } = &events[0].payload else {
            panic!("expected inline candidate source")
        };
        assert_eq!(value["kind"], "goal_outcome");
        assert_eq!(value["content"]["source_commit"], "abc123");
        assert!(value["content"]["attempt_ids"].is_array());
        assert!(value["content"]["artifact_ids"].is_array());
        assert!(value["content"]["verification"].is_array());

        let reducer = SqliteProjectionStore::open(":memory:").unwrap();
        let jobs = reducer.advance(&MemoryJobProjection, &events).unwrap().0;
        assert_eq!(jobs.eligible.len(), 1);
        assert_eq!(jobs.eligible[0].kind, "goal_outcome");
    }
}

#[tokio::test]
async fn replayed_terminal_event_has_one_stable_spine_identity() {
    let (projection, spine, _) = harness();
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
    assert_eq!(read_source(&spine, "goal:42").len(), 1);
}

#[tokio::test]
async fn revised_decision_preserves_supersedes_edge_in_candidate_source() {
    let (projection, spine, _) = harness();
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
    let ProjectionStatus::Queued {
        record_id: old_id, ..
    } = first
    else {
        panic!("decision was not queued")
    };
    let mut v2 = v1.clone();
    v2.decision_id = "memory-boundary-v2".into();
    v2.approval_id = "approval-2".into();
    v2.content = "Use local-first composite memory.".into();
    v2.supersedes = Some(old_id.clone());
    projection.project_architecture_decision(&v2).await;

    let events = read_source(&spine, "decision:memory-boundary-v2");
    let fabric::EventPayload::Inline { value } = &events[0].payload else {
        panic!("expected inline candidate source")
    };
    assert_eq!(value["content"]["supersedes"], old_id);
}

#[tokio::test]
async fn unapproved_and_sensitive_records_are_excluded_before_append() {
    let (projection, spine, _) = harness();
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
    assert_eq!(spine.metrics().accepted, 0);
}

struct FailingSpine;

impl EventSpine for FailingSpine {
    fn append(&self, _: UnsequencedEvent) -> anyhow::Result<SpineEvent> {
        anyhow::bail!("credential=must-not-leak")
    }
}

#[tokio::test]
async fn spine_outage_is_sanitized_and_does_not_change_source_result() {
    let projection = MemoryProjection::new(
        Arc::new(FailingSpine),
        Arc::new(executive::service::event_projection::NoopEventProjectionSink),
    );
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
    assert_eq!(
        health.last_error_category,
        Some("event_spine_append_failed")
    );
}
