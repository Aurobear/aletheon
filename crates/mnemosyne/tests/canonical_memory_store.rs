use chrono::{DateTime, Utc};
use mnemosyne::runtime::FactStore;
use mnemosyne::{
    MemoryAuthority, MemoryKind, MemoryMetadata, MemoryRecord, MemoryRecordId, MemoryScope,
    MemorySensitivity, MemoryStatus, RecallPreFilter, ScopeAncestry,
};

fn record(id: &str, workspace: &str, content: &str) -> MemoryRecord {
    let mut metadata =
        MemoryMetadata::local(id, format!("intake:{id}"), DateTime::<Utc>::UNIX_EPOCH);
    metadata.provenance.principal = Some("principal-a".into());
    MemoryRecord {
        id: MemoryRecordId(id.into()),
        kind: MemoryKind::SemanticFact,
        scope: MemoryScope::Workspace(workspace.into()),
        content: content.into(),
        metadata,
        status: MemoryStatus::Current,
        authority: MemoryAuthority::VerifiedLocalSemantic,
        source_event_ids: vec![format!("intake:{id}")],
        tags: vec!["maintenance-promoted".into()],
    }
}

fn predicate(workspace: &str) -> mnemosyne::ScopePredicate {
    RecallPreFilter {
        ancestry: ScopeAncestry {
            principal_id: Some("principal-a".into()),
            workspace_id: Some(workspace.into()),
            ..Default::default()
        },
        max_sensitivity: MemorySensitivity::Internal,
        allowed_authorities: vec![MemoryAuthority::VerifiedLocalSemantic],
    }
    .to_scope_predicate()
}

#[test]
fn canonical_records_preserve_workspace_identity_and_equal_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = FactStore::open(&dir.path().join("facts.db")).unwrap();
    let first = record("record-a", "workspace-a", "bounded canonical claim");
    let second = record("record-b", "workspace-b", "bounded canonical claim");
    store.store_canonical_record(&first, 1).unwrap();
    store.store_canonical_record(&second, 2).unwrap();
    store.store_canonical_record(&first, 3).unwrap();

    assert_eq!(
        store
            .search_canonical_records_prefiltered("canonical", 10, &predicate("workspace-a"))
            .unwrap(),
        vec![first]
    );
    assert_eq!(
        store
            .search_canonical_records_prefiltered("canonical", 10, &predicate("workspace-b"))
            .unwrap(),
        vec![second]
    );
    assert!(store
        .search_canonical_records_prefiltered("canonical", 10, &predicate("workspace-c"))
        .unwrap()
        .is_empty());
}

#[test]
fn canonical_record_id_cannot_be_rebound() {
    let dir = tempfile::tempdir().unwrap();
    let store = FactStore::open(&dir.path().join("facts.db")).unwrap();
    store
        .store_canonical_record(&record("record-a", "workspace-a", "original claim"), 1)
        .unwrap();
    let error = store
        .store_canonical_record(&record("record-a", "workspace-a", "changed claim"), 2)
        .unwrap_err();
    assert!(error.to_string().contains("reused"));
}
