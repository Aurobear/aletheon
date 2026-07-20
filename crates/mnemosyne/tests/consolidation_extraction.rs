use mnemosyne::consolidation::*;
#[test]
fn extraction_is_bounded_structured_and_redacted_twice() {
    let events = (0..200)
        .map(|i| CanonicalMemoryEvent {
            event_id: format!("e{i}"),
            kind: "assistant_message".into(),
            content: format!("learned api_key=secret-{i} preference"),
        })
        .collect();
    let result = CandidateExtractor::default()
        .extract(&ExtractionBatch {
            scope: mnemosyne::MemoryScope::Session("s".into()),
            events,
        })
        .unwrap();
    let ExtractionCompletion::Succeeded { candidates } = result else {
        panic!()
    };
    assert_eq!(candidates.len(), 128);
    assert!(candidates.iter().all(|c| c.claim.contains("[REDACTED]")
        && !c.claim.contains("secret-")
        && c.source_event_ids.len() == 1
        && c.content_hash.len() == 64))
}
#[test]
fn irrelevant_or_empty_history_succeeds_without_output() {
    let result = CandidateExtractor::default()
        .extract(&ExtractionBatch {
            scope: mnemosyne::MemoryScope::Session("s".into()),
            events: vec![CanonicalMemoryEvent {
                event_id: "e".into(),
                kind: "user_message".into(),
                content: "hello".into(),
            }],
        })
        .unwrap();
    assert!(matches!(result, ExtractionCompletion::SucceededNoOutput))
}
