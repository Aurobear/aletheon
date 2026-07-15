use agora::{AdmissionOutcome, CandidatePool, CandidatePoolConfig, SelectionPolicy};
use fabric::{
    AgoraSpaceId, ContentId, MonoDeadline, MonoTime, ProcessId, SalienceVector, VisibilityScope,
    WallTime, WorkspaceCandidate, WorkspaceContent, WorkspaceObservation, WorkspaceProvenance,
    WORKSPACE_SCHEMA_V1,
};
use uuid::Uuid;

fn candidate(id: u128, source: u128, score: f32, created: u64) -> WorkspaceCandidate {
    let source = ProcessId(Uuid::from_u128(source));
    WorkspaceCandidate {
        schema_version: WORKSPACE_SCHEMA_V1,
        id: ContentId(Uuid::from_u128(id)),
        space: AgoraSpaceId("space".into()),
        source,
        turn: None,
        content: WorkspaceContent::Observation(WorkspaceObservation {
            what: format!("observation-{id}"),
            source: "fixture".into(),
            data: serde_json::json!({"id": id}),
        }),
        confidence: 1.0,
        salience: SalienceVector {
            urgency: score,
            goal_relevance: 0.0,
            self_relevance: 0.0,
            novelty: 0.0,
            confidence: 0.0,
            prediction_error: 0.0,
            affect_intensity: 0.0,
            social_relevance: 0.0,
        },
        provenance: WorkspaceProvenance {
            producer: source,
            operation: None,
            source_refs: vec![format!("fixture://{id}")],
            observed_at: WallTime(created as i64),
        },
        visibility: VisibilityScope::Session,
        dependencies: Vec::new(),
        created_at: MonoTime(created),
        expires_at: None,
    }
}

fn config(capacity: usize, per_source: usize) -> CandidatePoolConfig {
    CandidatePoolConfig {
        capacity,
        per_source_capacity: per_source,
        max_coalition: capacity.min(4),
        policy: SelectionPolicy {
            aging_per_ms: 0.0,
            unresolved_dependency_boost: 0.1,
            repetition_penalty: 0.2,
            refractory_penalty: 10.0,
            ignition_threshold: 0.1,
            max_consecutive_source_wins: 2,
            ..SelectionPolicy::default()
        },
    }
}

#[test]
fn admission_reports_duplicate_source_quota_capacity_and_wrong_space() {
    let mut pool = CandidatePool::new(AgoraSpaceId("space".into()), config(2, 1)).unwrap();
    let first = candidate(1, 1, 0.9, 1);
    assert_eq!(
        pool.admit(first.clone(), MonoTime(1)),
        AdmissionOutcome::Accepted { id: first.id }
    );
    let mut duplicate = first.clone();
    duplicate.id = ContentId(Uuid::from_u128(2));
    assert_eq!(
        pool.admit(duplicate, MonoTime(1)),
        AdmissionOutcome::Duplicate { existing: first.id }
    );
    assert_eq!(
        pool.admit(candidate(3, 1, 0.8, 1), MonoTime(1)),
        AdmissionOutcome::RejectedSourceQuota {
            source: first.source
        }
    );
    assert!(matches!(
        pool.admit(candidate(4, 2, 0.7, 1), MonoTime(1)),
        AdmissionOutcome::Accepted { .. }
    ));
    assert_eq!(
        pool.admit(candidate(5, 3, 0.6, 1), MonoTime(1)),
        AdmissionOutcome::RejectedCapacity
    );
    let mut wrong = candidate(6, 4, 0.6, 1);
    wrong.space = AgoraSpaceId("other".into());
    assert_eq!(
        pool.admit(wrong, MonoTime(1)),
        AdmissionOutcome::RejectedWrongSpace
    );
    assert_eq!(pool.admission_metrics().duplicates, 1);
    assert_eq!(pool.admission_metrics().capacity_rejections, 1);
}

#[test]
fn admission_expires_stale_candidates_before_capacity_check() {
    let mut pool = CandidatePool::new(AgoraSpaceId("space".into()), config(1, 1)).unwrap();
    let mut expiring = candidate(1, 1, 0.9, 1);
    expiring.expires_at = Some(MonoDeadline(MonoTime(10)));
    assert!(matches!(
        pool.admit(expiring, MonoTime(1)),
        AdmissionOutcome::Accepted { .. }
    ));
    assert!(matches!(
        pool.admit(candidate(2, 2, 0.8, 10), MonoTime(10)),
        AdmissionOutcome::Accepted { .. }
    ));
    assert_eq!(pool.admission_metrics().expired, 1);
}

#[test]
fn selection_fixture_is_byte_deterministic_and_ties_use_creation_then_id() {
    fn run() -> Vec<u8> {
        let mut pool = CandidatePool::new(AgoraSpaceId("space".into()), config(8, 8)).unwrap();
        pool.admit(candidate(3, 3, 0.4, 20), MonoTime(20));
        pool.admit(candidate(2, 2, 0.9, 10), MonoTime(20));
        pool.admit(candidate(1, 1, 0.9, 10), MonoTime(20));
        let result = pool.select(MonoTime(30));
        assert_eq!(result.selected[0].id, ContentId(Uuid::from_u128(1)));
        serde_json::to_vec(&result.explanation).unwrap()
    }
    assert_eq!(run(), run());
}

#[test]
fn selection_exposes_all_score_terms_and_threshold_rejections() {
    let mut cfg = config(8, 8);
    cfg.policy.aging_per_ms = 0.01;
    cfg.policy.ignition_threshold = 99.0;
    let mut pool = CandidatePool::new(AgoraSpaceId("space".into()), cfg).unwrap();
    pool.admit(candidate(1, 1, 0.5, 0), MonoTime(0));
    let result = pool.select(MonoTime(10));
    assert!(result.selected.is_empty());
    let score = &result.explanation.evaluated[0];
    assert_eq!(score.aging_boost, 0.1);
    assert_eq!(score.dependency_boost, 0.0);
    assert_eq!(score.repetition_penalty, 0.0);
    assert_eq!(score.refractory_penalty, 0.0);
    assert_eq!(
        result.explanation.rejected_below_ignition,
        vec![ContentId(Uuid::from_u128(1))]
    );
    pool.record_no_ignition(&result).unwrap();
    assert_eq!(pool.selection_metrics().below_threshold, 1);
}

#[test]
fn selection_repetition_penalty_survives_candidate_reidentification() {
    let mut pool = CandidatePool::new(AgoraSpaceId("space".into()), config(8, 8)).unwrap();
    let original = candidate(1, 1, 1.0, 0);
    pool.admit(original.clone(), MonoTime(0));
    let first = pool.select(MonoTime(0));
    pool.finalize_selection(&first).unwrap();

    let mut repeated = original;
    repeated.id = ContentId(Uuid::from_u128(2));
    repeated.created_at = MonoTime(1);
    pool.admit(repeated, MonoTime(1));
    let second = pool.select(MonoTime(1));
    assert_eq!(second.explanation.evaluated[0].repetition_penalty, 0.2);
}

#[test]
fn fairness_refractory_penalty_prevents_source_monopoly_when_alternative_exists() {
    let mut pool = CandidatePool::new(AgoraSpaceId("space".into()), config(16, 16)).unwrap();
    pool.admit(candidate(100, 2, 0.2, 0), MonoTime(0));
    for id in [1_u128, 2] {
        pool.admit(candidate(id, 1, 1.0, id as u64), MonoTime(id as u64));
        let selected = pool.select(MonoTime(id as u64));
        assert_eq!(selected.selected[0].source, ProcessId(Uuid::from_u128(1)));
        pool.finalize_selection(&selected).unwrap();
    }
    pool.admit(candidate(3, 1, 1.0, 3), MonoTime(3));
    let selected = pool.select(MonoTime(3));
    assert_eq!(selected.selected[0].source, ProcessId(Uuid::from_u128(2)));
    assert!(selected
        .explanation
        .evaluated
        .iter()
        .any(
            |score| score.source == ProcessId(Uuid::from_u128(1)) && score.refractory_penalty > 0.0
        ));
    pool.finalize_selection(&selected).unwrap();
    assert!(pool.selection_metrics().refractory_applications > 0);
}

#[test]
fn coalition_contains_only_declared_available_dependencies_and_is_bounded() {
    let mut cfg = config(16, 16);
    cfg.max_coalition = 2;
    let mut pool = CandidatePool::new(AgoraSpaceId("space".into()), cfg).unwrap();
    let dep1 = candidate(1, 1, 0.1, 0);
    let dep2 = candidate(2, 2, 0.1, 0);
    let unrelated = candidate(3, 3, 0.1, 0);
    let mut winner = candidate(10, 10, 1.0, 0);
    winner.dependencies = vec![dep2.id, ContentId(Uuid::from_u128(999)), dep1.id];
    for item in [dep1.clone(), dep2, unrelated.clone(), winner.clone()] {
        pool.admit(item, MonoTime(0));
    }
    let result = pool.select(MonoTime(0));
    assert_eq!(result.explanation.selected_ids, vec![winner.id, dep1.id]);
    assert!(!result.explanation.selected_ids.contains(&unrelated.id));
}

#[test]
fn projection_and_finalize_leave_unselected_content_private_in_pool() {
    let mut pool = CandidatePool::new(AgoraSpaceId("space".into()), config(8, 8)).unwrap();
    let winner = candidate(1, 1, 1.0, 0);
    let unselected = candidate(2, 2, 0.2, 0);
    pool.admit(winner.clone(), MonoTime(0));
    pool.admit(unselected.clone(), MonoTime(0));
    let result = pool.select(MonoTime(0));
    assert_eq!(
        result
            .selected
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
        vec![winner.id]
    );
    pool.finalize_selection(&result).unwrap();
    assert_eq!(
        pool.pending()
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
        vec![unselected.id]
    );
}
