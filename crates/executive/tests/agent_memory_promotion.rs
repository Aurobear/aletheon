use std::sync::Arc;

use executive::service::agent_control::{MemoryRecordingAgentEventSink, NoopAgentEventSink};
use fabric::{
    AgentId, AgentTaskId, BroadcastEpoch, ContentId, OperationId, PrincipalId, ProcessId,
};
use mnemosyne::{AgentMemoryContext, AgentMemoryVault, MemoryRecordId, MemoryScope};
use tempfile::tempdir;

#[test]
fn root_selection_and_parent_review_are_required_and_restart_idempotent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("agent-memory.db");
    let vault = Arc::new(AgentMemoryVault::open(&path).unwrap());
    let process = ProcessId::new();
    let agent = AgentId::new();
    let context = AgentMemoryContext::verified(
        process,
        agent,
        AgentTaskId("task:root-selected".into()),
        "sha256:parent-projection",
    )
    .unwrap();
    vault.register(&context).unwrap();
    let source = vault
        .record_child(
            &context,
            mnemosyne::ChildMemoryDraft {
                kind: mnemosyne::MemoryKind::Reflection,
                content: "selected evidence".into(),
                authority: mnemosyne::MemoryAuthority::RawExperience,
                source_event_ids: vec![format!("operation:{}", OperationId::new().0)],
                tags: vec!["explicitly-visible".into()],
            },
        )
        .unwrap();
    let sink = MemoryRecordingAgentEventSink::new(Arc::new(NoopAgentEventSink), vault, context);
    let root_content = ContentId::new();
    let selected = ContentId::new();
    let first = sink
        .promote_reviewed(
            source.id.clone(),
            root_content,
            BroadcastEpoch(11),
            selected,
            "selection:epoch-11".into(),
            PrincipalId("parent".into()),
            "review:approved".into(),
            MemoryScope::Session("root-session".into()),
        )
        .unwrap();
    drop(sink);

    let reopened = Arc::new(AgentMemoryVault::open(&path).unwrap());
    let promoted = reopened
        .get_record(&first.resulting_record)
        .unwrap()
        .unwrap();
    assert!(promoted
        .source_event_ids
        .iter()
        .any(|item| item == "root-broadcast:11"));
    assert!(promoted
        .source_event_ids
        .iter()
        .any(|item| item == &format!("selected-candidate:{}", selected.0)));
    assert!(reopened
        .get_record(&MemoryRecordId(source.id.0))
        .unwrap()
        .is_some());
}
