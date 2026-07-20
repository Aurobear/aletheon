use mnemosyne::consolidation::*;
use tempfile::TempDir;
fn job(
    key: &str,
    session: &str,
    completed: Option<u64>,
    ephemeral: bool,
    memory_worker: bool,
) -> ExtractionJob {
    ExtractionJob {
        idempotency_key: key.into(),
        session_id: session.into(),
        goal_id: None,
        ephemeral,
        memory_worker,
        completed_at_ms: completed,
        watermark: "event:7".into(),
        created_at_ms: 10,
    }
}
#[test]
fn leased_transitions_are_durable_idempotent_and_restart_safe() {
    let d = TempDir::new().unwrap();
    let p = d.path().join("jobs.db");
    let r = ConsolidationRepository::open(&p).unwrap();
    r.enqueue_extraction(&job("a", "s", Some(100), false, false))
        .unwrap();
    let l = r.claim_extraction("one", 200, 50, 1_000).unwrap().unwrap();
    assert!(r.claim_extraction("two", 210, 50, 1_000).unwrap().is_none());
    drop(r);
    let r = ConsolidationRepository::open(&p).unwrap();
    let recovered = r.claim_extraction("two", 251, 50, 1_000).unwrap().unwrap();
    assert_eq!(recovered.id, l.id);
    r.complete(&recovered, ExtractionCompletion::SucceededNoOutput, 260)
        .unwrap();
    r.complete(&recovered, ExtractionCompletion::SucceededNoOutput, 261)
        .unwrap();
    assert_eq!(r.status("a").unwrap(), ExtractionStatus::SucceededNoOutput)
}
#[test]
fn failure_and_success_states_are_explicit() {
    let r = ConsolidationRepository::open(":memory:").unwrap();
    for key in ["retry", "permanent", "success"] {
        r.enqueue_extraction(&job(key, key, Some(100), false, false))
            .unwrap();
        let l = r.claim_extraction(key, 200, 10, 1_000).unwrap().unwrap();
        let completion = match key {
            "retry" => ExtractionCompletion::RetryableFailure {
                error: "temporary".into(),
                retry_at_ms: 300,
            },
            "permanent" => ExtractionCompletion::PermanentFailure {
                error: "invalid".into(),
            },
            _ => ExtractionCompletion::Succeeded {
                candidates: vec![MemoryCandidate::new(
                    mnemosyne::MemoryKind::SemanticFact,
                    "fact".into(),
                    vec!["event-1".into()],
                    0.8,
                    mnemosyne::MemoryScope::Session(key.into()),
                    None,
                    None,
                    1,
                )
                .unwrap()],
            },
        };
        r.complete(&l, completion, 210).unwrap()
    }
    assert_eq!(
        r.status("retry").unwrap(),
        ExtractionStatus::RetryableFailure
    );
    assert_eq!(
        r.status("permanent").unwrap(),
        ExtractionStatus::PermanentFailure
    );
    assert_eq!(r.status("success").unwrap(), ExtractionStatus::Succeeded)
}
#[test]
fn active_ephemeral_memory_worker_and_stale_sessions_are_ineligible() {
    let r = ConsolidationRepository::open(":memory:").unwrap();
    r.enqueue_extraction(&job("active", "a", None, false, false))
        .unwrap();
    r.enqueue_extraction(&job("ephemeral", "e", Some(100), true, false))
        .unwrap();
    r.enqueue_extraction(&job("worker", "w", Some(100), false, true))
        .unwrap();
    r.enqueue_extraction(&job("stale", "s", Some(1), false, false))
        .unwrap();
    assert!(r
        .claim_extraction("owner", 1_000, 10, 100)
        .unwrap()
        .is_none())
}

#[test]
fn explicit_scope_completion_makes_only_that_session_eligible() {
    let r = ConsolidationRepository::open(":memory:").unwrap();
    let session_a = mnemosyne::MemoryScope::Session("a".into());
    r.enqueue_extraction(&job("a", "a", None, false, false))
        .unwrap();
    r.enqueue_extraction(&job("b", "b", None, false, false))
        .unwrap();

    assert_eq!(r.complete_scope(&session_a, 100).unwrap(), 1);
    let claimed = r
        .claim_extraction("owner", 100, 10, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.scope, session_a);
    r.complete(&claimed, ExtractionCompletion::SucceededNoOutput, 101)
        .unwrap();
    assert!(r
        .claim_extraction("owner", 102, 10, 1_000)
        .unwrap()
        .is_none());
}
