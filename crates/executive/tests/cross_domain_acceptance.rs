mod support {
    pub mod conscious_core_harness;
}

use std::collections::BTreeSet;

use support::conscious_core_harness::{baseline, run};

fn write_runtime_evidence(value: &serde_json::Value) {
    let output =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/acceptance");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(
        output.join("runtime-evidence.json"),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn harness_replays_identically() {
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let first = run(first_root.path()).await.unwrap();
    let second = run(second_root.path()).await.unwrap();
    for projection in ["session", "agent_tree", "metrics"] {
        assert_eq!(
            first.evidence.projection_checksums[projection],
            second.evidence.projection_checksums[projection]
        );
    }
    assert!(!first.evidence.event_checksum.is_empty());
    assert!(!second.evidence.event_checksum.is_empty());
    assert_eq!(first.evidence.limitations, second.evidence.limitations);
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
    write_runtime_evidence(&serde_json::json!({
        "schema_version": 1,
        "fixture_version": first.evidence.fixture_version,
        "event_checksum": first.evidence.event_checksum,
        "projection_checksums": first.evidence.projection_checksums,
        "replayed_from_independent_root": true,
        "agent_runs_reopened": first.reopened_agent_runs,
        "mailbox_deliveries_reopened": first.reopened_mailbox_deliveries,
        "memory_lease_recovered": first.memory_lease_recovered,
        "unexpected_external_calls": first.external_uses,
        "limitations": first.evidence.limitations,
    }));
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
    assert_eq!(result.reopened_agent_runs, 2);
    assert_eq!(result.reopened_mailbox_deliveries, 2);
    assert_eq!(result.agent_recovery_interrupted, 2);
    assert!(result.memory_lease_recovered);
    assert!(result.authority_denials >= 3);
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
    assert_eq!(result.fake_runtime_calls, 2);
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
    assert!(result.promotion_idempotent);
    for expected in [
        "child-process:",
        "child-agent:",
        "child-task:",
        "root-content:",
        "root-broadcast:11",
        "selected-candidate:",
        "selection-receipt:selection:acceptance:11",
        "review-receipt:review:approved:acceptance",
    ] {
        assert!(
            result
                .promotion_lineage
                .iter()
                .any(|item| item.starts_with(expected)),
            "missing {expected}: {:?}",
            result.promotion_lineage
        );
    }
    assert!(result.ordinary_subject_rejected);
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
