use mnemosyne::{
    SupplementalCapabilityGrant, WorkspaceMemoryBindingError, WorkspaceMemoryBindingProposal,
    WorkspaceMemoryBindingRegistry, WorkspaceMemoryBindingState, WorkspaceMemoryKey,
};

fn key(value: &str) -> WorkspaceMemoryKey {
    WorkspaceMemoryKey::from_verified(value).unwrap()
}

fn proposal() -> WorkspaceMemoryBindingProposal {
    WorkspaceMemoryBindingProposal {
        backend_id: "supplemental/gbrain".into(),
        write_destination_handle: "mcp/workspace".into(),
        read_destination_handles: vec!["mcp/workspace".into()],
        expected_write_source: "workspace-1".into(),
        expected_read_sources: vec!["workspace-1".into(), "personal".into()],
        credential_ref: "systemd:gbrain-workspace".into(),
    }
}

fn grant() -> SupplementalCapabilityGrant {
    SupplementalCapabilityGrant {
        backend_id: "supplemental/gbrain".into(),
        write_source: Some("workspace-1".into()),
        read_sources: vec!["personal".into(), "workspace-1".into()],
        can_read: true,
        can_write: true,
    }
}

#[test]
fn unknown_workspace_is_durably_local_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.db");
    let workspace = key("ws:repo:0123456789abcdef");
    let binding = WorkspaceMemoryBindingRegistry::open(&path)
        .unwrap()
        .local_only("uid:1000", &workspace, 10)
        .unwrap();
    assert_eq!(binding.state, WorkspaceMemoryBindingState::LocalOnly);
    assert!(binding.write_destination_handle.is_empty());
    drop(binding);
    let reopened = WorkspaceMemoryBindingRegistry::open(&path)
        .unwrap()
        .get("uid:1000", &workspace)
        .unwrap()
        .unwrap();
    assert_eq!(reopened.state, WorkspaceMemoryBindingState::LocalOnly);
}

#[test]
fn exact_verified_grants_activate_and_restart_persist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bindings.db");
    let workspace = key("ws:repo:0123456789abcdef");
    let registry = WorkspaceMemoryBindingRegistry::open(&path).unwrap();
    let preview = registry
        .preview("uid:1000", &workspace, &proposal(), &grant(), 20)
        .unwrap();
    assert!(preview.compatible);
    assert_eq!(preview.binding.state, WorkspaceMemoryBindingState::Active);
    assert!(preview.binding.verified_capability_digest.is_some());
    registry.apply(&preview).unwrap();
    drop(registry);
    let loaded = WorkspaceMemoryBindingRegistry::open(&path)
        .unwrap()
        .get("uid:1000", &workspace)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.state, WorkspaceMemoryBindingState::Active);
    assert_eq!(
        loaded.expected_read_sources,
        proposal().expected_read_sources
    );
}

#[test]
fn mismatch_is_fail_closed_and_stale_preview_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = key("ws:repo:0123456789abcdef");
    let registry = WorkspaceMemoryBindingRegistry::open(dir.path().join("bindings.db")).unwrap();
    let mut wrong = grant();
    wrong.write_source = Some("other".into());
    let incompatible = registry
        .preview("uid:1000", &workspace, &proposal(), &wrong, 20)
        .unwrap();
    assert!(!incompatible.compatible);
    assert_eq!(
        incompatible.binding.state,
        WorkspaceMemoryBindingState::Incompatible
    );
    assert!(incompatible.binding.verified_capability_digest.is_none());
    registry.apply(&incompatible).unwrap();

    let first = registry
        .preview("uid:1000", &workspace, &proposal(), &grant(), 30)
        .unwrap();
    let second = registry
        .preview("uid:1000", &workspace, &proposal(), &grant(), 31)
        .unwrap();
    registry.apply(&first).unwrap();
    assert!(matches!(
        registry.apply(&second),
        Err(WorkspaceMemoryBindingError::Invalid)
    ));
}

#[test]
fn principal_isolation_and_revoke_clear_remote_authority() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = key("ws:repo:0123456789abcdef");
    let registry = WorkspaceMemoryBindingRegistry::open(dir.path().join("bindings.db")).unwrap();
    let preview = registry
        .preview("uid:1000", &workspace, &proposal(), &grant(), 20)
        .unwrap();
    registry.apply(&preview).unwrap();
    assert!(registry.get("uid:1001", &workspace).unwrap().is_none());
    let revoked = registry
        .revoke("uid:1000", &workspace, 40)
        .unwrap()
        .unwrap();
    assert_eq!(revoked.state, WorkspaceMemoryBindingState::Revoked);
    assert!(revoked.verified_capability_digest.is_none());
}
