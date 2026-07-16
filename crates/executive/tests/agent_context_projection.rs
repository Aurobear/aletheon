use executive::service::agent_control::{
    AgentContextItemKind, AgentContextProjection, AgentContextProjectionBuilder,
};
use fabric::{AgentContextFork, AgentControlErrorKind, ContentId};

#[test]
fn none_last_turns_and_selected_projection_are_labelled_and_deterministic() {
    assert_eq!(
        AgentContextProjection::from_fork(&AgentContextFork::None)
            .unwrap()
            .items
            .len(),
        0
    );
    let recent =
        AgentContextProjection::from_fork(&AgentContextFork::LastTurns { count: 3 }).unwrap();
    assert_eq!(recent.items[0].kind, AgentContextItemKind::RecentTurns);
    assert_eq!(recent.items[0].label, "requested_recent_turn_count");

    let first = AgentContextProjectionBuilder::new()
        .constraint("z constraint")
        .unwrap()
        .constraint("a constraint")
        .unwrap()
        .item(AgentContextItemKind::Evidence, "b", "second")
        .unwrap()
        .item(AgentContextItemKind::Memory, "a", "first")
        .unwrap()
        .build()
        .unwrap();
    let second = AgentContextProjectionBuilder::new()
        .item(AgentContextItemKind::Memory, "a", "first")
        .unwrap()
        .item(AgentContextItemKind::Evidence, "b", "second")
        .unwrap()
        .constraint("a constraint")
        .unwrap()
        .constraint("z constraint")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.constraints, vec!["a constraint", "z constraint"]);
}

#[test]
fn projection_rejects_hidden_reasoning_and_raw_tool_output() {
    for kind in [
        AgentContextItemKind::HiddenReasoning,
        AgentContextItemKind::RawToolOutput,
    ] {
        let error = AgentContextProjectionBuilder::new()
            .item(kind, "private", "secret")
            .unwrap_err();
        assert_eq!(error.kind, AgentControlErrorKind::InvalidRequest);
    }
    let error = AgentContextProjection::from_fork(&AgentContextFork::SelectedProjection {
        items: vec!["chain_of_thought: private tokens".into()],
    })
    .unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::InvalidRequest);
}

#[test]
fn large_tool_content_requires_artifact_reference() {
    let error = AgentContextProjectionBuilder::new()
        .item(
            AgentContextItemKind::ArtifactReference,
            "tool output",
            "raw bytes",
        )
        .unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::InvalidRequest);
    let projection = AgentContextProjectionBuilder::new()
        .item(
            AgentContextItemKind::ArtifactReference,
            "tool output",
            format!("sha256:{}", "a".repeat(64)),
        )
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(projection.items.len(), 1);
}

#[test]
fn limits_truncate_at_utf8_boundaries_and_count_omissions() {
    let multi_byte = "界".repeat(4_000);
    let mut builder = AgentContextProjectionBuilder::new();
    for index in 0..80 {
        builder = builder
            .item(
                AgentContextItemKind::Selected,
                format!("item-{index:03}"),
                multi_byte.clone(),
            )
            .unwrap();
    }
    let projection = builder.build().unwrap();
    assert!(projection.total_bytes() <= 64 * 1024);
    assert!(projection.omitted_count > 0);
    assert!(projection
        .items
        .iter()
        .all(|item| item.content.is_char_boundary(item.content.len())));
}

#[test]
fn broadcast_references_are_deduplicated_and_ordered() {
    let first = ContentId::new();
    let second = ContentId::new();
    let projection = AgentContextProjectionBuilder::new()
        .broadcast_ref(second)
        .broadcast_ref(first)
        .broadcast_ref(second)
        .build()
        .unwrap();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(projection.broadcast_refs, expected);
}
