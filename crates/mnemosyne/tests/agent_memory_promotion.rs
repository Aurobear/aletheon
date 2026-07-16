use fabric::{AgentId, AgentTaskId, BroadcastEpoch, ContentId, PrincipalId, ProcessId};
use mnemosyne::{
    AgentMemoryContext, AgentMemoryVault, ChildMemoryDraft, MemoryAuthority, MemoryKind,
    MemoryPromotionRequest, MemoryScope,
};
use tempfile::tempdir;
use uuid::Uuid;

fn fixture(path: &std::path::Path) -> (AgentMemoryVault, AgentMemoryContext, mnemosyne::MemoryRecord) {
    let vault = AgentMemoryVault::open(path).unwrap();
    let context = AgentMemoryContext::verified(
        ProcessId(Uuid::new_v4()),
        AgentId(Uuid::new_v4()),
        AgentTaskId("task-9".into()),
        "sha256:parent-projection",
    )
    .unwrap();
    vault.register(&context).unwrap();
    let record = vault
        .record_child(
            &context,
            ChildMemoryDraft {
                kind: MemoryKind::SemanticFact,
                content: "review me".into(),
                authority: MemoryAuthority::RawExperience,
                source_event_ids: vec!["child-event".into()],
                tags: vec![],
            },
        )
        .unwrap();
    (vault, context, record)
}

fn request(
    context: AgentMemoryContext,
    source: mnemosyne::MemoryRecordId,
) -> MemoryPromotionRequest {
    MemoryPromotionRequest {
        source_record: source,
        child: context,
        root_content: ContentId(Uuid::new_v4()),
        broadcast: BroadcastEpoch(7),
        selected_candidate: ContentId(Uuid::new_v4()),
        selection_receipt: "selection:root:7".into(),
        reviewer: PrincipalId("parent-reviewer".into()),
        review_receipt: "review:approved".into(),
        target_scope: MemoryScope::Session("root-session".into()),
    }
}

#[test]
fn complete_reviewed_promotion_is_restart_idempotent_and_preserves_lineage() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("memory.db");
    let (vault, context, source) = fixture(&path);
    let request = request(context, source.id.clone());
    let first = vault.promote(&request).unwrap();
    drop(vault);

    let reopened = AgentMemoryVault::open(&path).unwrap();
    let repeated = reopened.promote(&request).unwrap();
    assert_eq!(first, repeated);
    let promoted = reopened.get_record(&first.resulting_record).unwrap().unwrap();
    assert_eq!(promoted.scope, MemoryScope::Session("root-session".into()));
    assert_eq!(promoted.metadata.provenance.source_commit.as_deref(), Some(source.id.0.as_str()));
    for expected in ["child-process:", "child-agent:", "child-task:", "root-broadcast:7", "selected-candidate:", "selection-receipt:", "review-receipt:"] {
        assert!(promoted.source_event_ids.iter().any(|item| item.starts_with(expected)), "missing {expected}");
    }
    assert!(reopened.get_record(&source.id).unwrap().is_some(), "child source is immutable");
}

#[test]
fn incomplete_escalating_and_conflicting_promotions_fail_closed() {
    let dir = tempdir().unwrap();
    let (vault, context, source) = fixture(&dir.path().join("memory.db"));
    let mut incomplete = request(context.clone(), source.id.clone());
    incomplete.review_receipt.clear();
    assert!(vault.promote(&incomplete).is_err());
    let mut escaping = request(context.clone(), source.id.clone());
    escaping.target_scope = MemoryScope::Agent("sibling".into());
    assert!(vault.promote(&escaping).is_err());

    let accepted = request(context, source.id);
    vault.promote(&accepted).unwrap();
    let mut conflict = accepted;
    conflict.reviewer = PrincipalId("different-reviewer".into());
    assert!(vault.promote(&conflict).unwrap_err().to_string().contains("conflicting"));
}
