use mnemosyne::consolidation::*;
fn enqueue(
    r: &ConsolidationRepository,
    key: &str,
    scope: mnemosyne::MemoryScope,
    kind: mnemosyne::MemoryKind,
    claim: &str,
) {
    r.enqueue_extraction(&ExtractionJob {
        idempotency_key: key.into(),
        session_id: key.into(),
        goal_id: None,
        ephemeral: false,
        memory_worker: false,
        completed_at_ms: Some(10),
        watermark: format!("wm-{key}"),
        created_at_ms: 1,
    })
    .unwrap();
    let l = r.claim_extraction(key, 20, 10, 100).unwrap().unwrap();
    r.complete(
        &l,
        ExtractionCompletion::Succeeded {
            candidates: vec![MemoryCandidate::new(
                kind,
                claim.into(),
                vec![format!("event-{key}")],
                0.8,
                scope,
                None,
                None,
                1,
            )
            .unwrap()],
        },
        21,
    )
    .unwrap()
}
#[test]
fn scoped_decisions_are_leased_deterministic_and_replay_safe() {
    let r = ConsolidationRepository::open(":memory:").unwrap();
    let scope = mnemosyne::MemoryScope::Session("s".into());
    enqueue(
        &r,
        "a",
        scope.clone(),
        mnemosyne::MemoryKind::SemanticFact,
        "stable fact",
    );
    enqueue(
        &r,
        "b",
        scope.clone(),
        mnemosyne::MemoryKind::SemanticFact,
        " stable   fact ",
    );
    let out = ScopedConsolidator::new(&r)
        .run(&scope, "worker", 30, None)
        .unwrap();
    assert_eq!(out.consumed, 2);
    assert_eq!(out.decisions[0].1, ConsolidationDecision::Insert);
    assert_eq!(out.decisions[1].1, ConsolidationDecision::Merge);
    assert_eq!(
        ScopedConsolidator::new(&r)
            .run(&scope, "worker", 31, None)
            .unwrap()
            .consumed,
        0
    )
}
#[test]
fn core_adjacent_global_candidate_requires_approval_evidence() {
    let r = ConsolidationRepository::open(":memory:").unwrap();
    let scope = mnemosyne::MemoryScope::Global;
    enqueue(
        &r,
        "core",
        scope.clone(),
        mnemosyne::MemoryKind::CoreState,
        "change identity",
    );
    let out = ScopedConsolidator::new(&r)
        .run(&scope, "worker", 30, None)
        .unwrap();
    assert_eq!(out.decisions[0].1, ConsolidationDecision::Reject)
}
