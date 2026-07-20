use chrono::Utc;
use fabric::{AgentId, AgentTaskId, ProcessId};
use mnemosyne::{
    AgentMemoryContext, AgentMemoryVault, ChildMemoryDraft, DefaultMemoryWorkspaceProjector,
    MemoryAuthority, MemoryKind, MemoryMetadata, MemoryProjection, MemoryProjectionLimits,
    MemoryScope, MemoryWorkspaceProjector, RecallItem, RecallSet, TemporalState,
};
use uuid::Uuid;

fn process() -> ProcessId {
    ProcessId(Uuid::new_v4())
}

fn context(process: ProcessId, agent: AgentId, task: &str, receipt: &str) -> AgentMemoryContext {
    AgentMemoryContext::verified(process, agent, AgentTaskId(task.into()), receipt).unwrap()
}

fn draft(content: &str) -> ChildMemoryDraft {
    ChildMemoryDraft {
        kind: MemoryKind::Reflection,
        content: content.into(),
        authority: MemoryAuthority::RawExperience,
        source_event_ids: vec![format!("event:{content}")],
        tags: vec![],
    }
}

#[test]
fn child_writes_are_bound_to_process_agent_and_task_without_sibling_leakage() {
    let vault = AgentMemoryVault::in_memory().unwrap();
    let first = context(
        process(),
        AgentId(Uuid::new_v4()),
        "task-a",
        "sha256:none-a",
    );
    let sibling = context(
        process(),
        AgentId(Uuid::new_v4()),
        "task-b",
        "sha256:none-b",
    );
    vault.register(&first).unwrap();
    vault.register(&sibling).unwrap();
    let record = vault
        .record_child(&first, draft("private child fact"))
        .unwrap();

    assert_eq!(record.scope, MemoryScope::Task("task-a".into()));
    assert!(record
        .tags
        .iter()
        .any(|tag| tag == &format!("process:{}", first.process_id.0)));
    assert!(record
        .tags
        .iter()
        .any(|tag| tag == &format!("agent:{}", first.agent_id.0)));
    assert_eq!(vault.recall(&first).unwrap().len(), 1);
    assert!(vault.recall(&sibling).unwrap().is_empty());

    let forged = context(
        first.process_id,
        sibling.agent_id,
        "task-a",
        "sha256:none-a",
    );
    assert!(vault
        .recall(&forged)
        .unwrap_err()
        .to_string()
        .contains("binding"));
    let unbound = context(
        process(),
        AgentId(Uuid::new_v4()),
        "task-x",
        "sha256:none-x",
    );
    assert!(vault.record_child(&unbound, draft("forged")).is_err());
}

#[test]
fn parent_projection_is_bounded_receipted_and_read_only() {
    let now = Utc::now();
    let recall = RecallSet {
        items: (0..4)
            .map(|index| RecallItem {
                content: format!("parent selected fact {index}"),
                metadata: MemoryMetadata::local(format!("parent-{index}"), "parent", now),
                temporal_state: TemporalState::Current,
                authority: MemoryAuthority::VerifiedLocalSemantic,
                scope: MemoryScope::Session("root-session".into()),
                score: 0.0,
                evidence: None,
            })
            .collect(),
        degraded_sources: vec![],
    };
    let projection = DefaultMemoryWorkspaceProjector
        .project(
            &recall,
            MemoryProjectionLimits {
                max_items: 2,
                max_total_bytes: 4096,
                max_item_bytes: 1024,
            },
        )
        .unwrap();
    let receipt = AgentMemoryVault::projection_receipt(&projection).unwrap();
    let child = context(process(), AgentId(Uuid::new_v4()), "task", &receipt);
    let vault = AgentMemoryVault::in_memory().unwrap();
    vault.register(&child).unwrap();
    vault.attach_parent_projection(&child, &projection).unwrap();
    let projected = vault.projected_for_child(&child).unwrap();
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].record_id, projection.records[0].record_id);
    assert_eq!(projected[0].metadata, projection.records[0].metadata);
    assert!(
        vault.recall(&child).unwrap().is_empty(),
        "projection is not a child write"
    );

    let mut changed = MemoryProjection { ..projection };
    changed.records.pop();
    assert!(vault.attach_parent_projection(&child, &changed).is_err());
}
