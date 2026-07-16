mod support {
    pub mod conscious_core_harness;
}

use std::collections::BTreeSet;

use support::conscious_core_harness::{baseline, run};

#[tokio::test]
async fn harness_replays_identically() {
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let first = run(first_root.path()).await.unwrap();
    let second = run(second_root.path()).await.unwrap();
    assert_eq!(first.evidence, second.evidence);
    assert_eq!(first.replay_epochs, second.replay_epochs);
    assert_eq!(first.processors, baseline().expected_processors);
    assert_eq!(
        first.external_uses, 0,
        "external provider/process/network use"
    );
    assert_eq!(
        first
            .evidence
            .projection_checksums
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        baseline().expected_projection_names
    );
}

#[tokio::test]
async fn lifecycle_authority_replay() {
    let root = tempfile::tempdir().unwrap();
    let result = run(root.path()).await.unwrap();
    assert_eq!(
        result.replay_epochs.len(),
        baseline().expected_replay_entries
    );
    assert!(result
        .replay_epochs
        .windows(2)
        .all(|pair| pair[0] < pair[1]));
    assert!(result
        .dasein_versions
        .windows(2)
        .all(|pair| pair[0] <= pair[1]));
    assert!(result.memory_candidate_is_private);
    assert!(result.duplicate_delivery_rejected);
    assert_eq!(result.external_uses, 0);
    assert!(result
        .trace
        .events
        .iter()
        .any(|event| matches!(event, fabric::ConsciousTraceEvent::GovernedAction { .. })));
}

#[tokio::test]
async fn memory_isolation() {
    let root = tempfile::tempdir().unwrap();
    let result = run(root.path()).await.unwrap();
    assert!(result.memory_candidate_is_private);
    let memory_log = std::fs::read_to_string(root.path().join("memory.jsonl")).unwrap();
    assert!(memory_log.contains("external_reference"));
    assert!(!serde_json::to_string(&result.trace)
        .unwrap()
        .contains("mutate self"));
}

#[tokio::test]
async fn agent_isolation() {
    let root = tempfile::tempdir().unwrap();
    let result = run(root.path()).await.unwrap();
    assert_eq!(result.agent_tree.len(), 3);
    assert_ne!(result.sibling_roots[0], result.sibling_roots[1]);
    std::fs::write(result.sibling_roots[0].join("private"), "first").unwrap();
    assert!(!result.sibling_roots[1].join("private").exists());
    let labels = result
        .agent_tree
        .iter()
        .map(|(label, _)| label)
        .collect::<BTreeSet<_>>();
    assert_eq!(labels.len(), 3);
}

#[tokio::test]
async fn promotion_preserves_bounded_lineage() {
    let root = tempfile::tempdir().unwrap();
    let result = run(root.path()).await.unwrap();
    assert!(result.trace.events.iter().any(|event| matches!(
        event,
        fabric::ConsciousTraceEvent::Broadcast { recipients, .. }
            if recipients.iter().any(|recipient| recipient == "agent")
    )));
    assert!(result
        .agent_tree
        .iter()
        .skip(1)
        .all(|(_, parent)| parent.as_deref() == Some("root")));
}

#[tokio::test]
async fn bounded_overload_and_cancellation_are_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let result = run(root.path()).await.unwrap();
    // A bounded run completes without an unbounded wait; processor errors are
    // represented as health/detail receipts rather than external retries.
    assert_eq!(result.overload_rejections, 1);
    assert!(result.duplicate_delivery_rejected);
    assert!(result.cancellation_terminal);
    assert_eq!(result.external_uses, 0);
}
