use std::path::PathBuf;

use fabric::WorkspaceIdentity;
use mnemosyne::{
    MemoryAuthority, MemoryScope, MemorySensitivity, RecallPreFilter, ScopeAncestry,
    WorkspaceMemoryKey,
};

fn identity(path: &str, repo_fingerprint: Option<&str>) -> WorkspaceIdentity {
    WorkspaceIdentity {
        canonical_path: PathBuf::from(path),
        repo_fingerprint: repo_fingerprint.map(str::to_owned),
    }
}

#[test]
fn workspace_memory_key_uses_repository_identity_when_available() {
    let first = WorkspaceMemoryKey::derive(
        &identity("/checkout/one", Some("sha256:repository")),
        "machine-a",
    )
    .unwrap();
    let second = WorkspaceMemoryKey::derive(
        &identity("/checkout/two", Some("sha256:repository")),
        "machine-b",
    )
    .unwrap();

    assert_eq!(first.as_str(), "ws:repo:sha256:repository");
    assert_eq!(first, second);
}

#[test]
fn workspace_memory_key_is_deterministic_machine_local_and_opaque_without_repo() {
    let first =
        WorkspaceMemoryKey::derive(&identity("/private/project", None), "machine-a").unwrap();
    let repeat =
        WorkspaceMemoryKey::derive(&identity("/private/project", None), "machine-a").unwrap();
    let other_path =
        WorkspaceMemoryKey::derive(&identity("/private/other", None), "machine-a").unwrap();
    let other_machine =
        WorkspaceMemoryKey::derive(&identity("/private/project", None), "machine-b").unwrap();

    assert_eq!(first, repeat);
    assert_ne!(first, other_path);
    assert_ne!(first, other_machine);
    assert!(first.as_str().starts_with("ws:local:"));
    assert!(!first.as_str().contains("private"));
    assert!(!first.as_str().contains("machine-a"));
}

#[test]
fn workspace_memory_key_rejects_missing_identity_material() {
    assert!(WorkspaceMemoryKey::derive(&identity("/project", None), " ").is_err());
    assert!(WorkspaceMemoryKey::derive(&identity("/project", Some(" ")), "machine").is_err());
    assert!(WorkspaceMemoryKey::from_verified("client-controlled").is_err());
}

#[test]
fn workspace_scope_requires_exact_host_ancestry() {
    let ancestry = ScopeAncestry {
        workspace_id: Some("ws:repo:sha256:repository".into()),
        ..Default::default()
    };

    assert!(MemoryScope::Workspace("ws:repo:sha256:repository".into()).allows(&ancestry));
    assert!(!MemoryScope::Workspace("ws:repo:sha256:other".into()).allows(&ancestry));
    assert_eq!(
        serde_json::to_string(&MemoryScope::Workspace("ws:repo:sha256:repository".into())).unwrap(),
        r#"{"kind":"workspace","id":"ws:repo:sha256:repository"}"#
    );
}

#[test]
fn workspace_scope_reaches_the_common_backend_predicate() {
    let filter = RecallPreFilter {
        ancestry: ScopeAncestry {
            principal_id: Some("principal-a".into()),
            workspace_id: Some("ws:repo:sha256:repository".into()),
            session_id: Some("session-a".into()),
            ..Default::default()
        },
        max_sensitivity: MemorySensitivity::Internal,
        allowed_authorities: vec![MemoryAuthority::VerifiedLocalSemantic],
    };

    let predicate = filter.to_scope_predicate();
    assert!(predicate
        .scope_keys
        .contains(&"workspace:ws:repo:sha256:repository".to_string()));
    assert_eq!(
        predicate
            .scope_keys
            .iter()
            .filter(|key| key.starts_with("workspace:"))
            .count(),
        1
    );
}
