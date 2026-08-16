use contracts::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, PermissionProfileId, PrincipalContext,
    PrincipalId, ThreadId, WorkspacePolicy,
};

#[test]
fn workspace_authority_can_be_narrowed_but_not_expanded() {
    let policy =
        WorkspacePolicy::from_resolved_roots(PathBuf::from("/tmp/workspace"), vec![]).unwrap();
    let narrowed = policy
        .clone()
        .narrow_writable_roots(vec![PathBuf::from("/tmp/workspace/src/lib.rs")])
        .unwrap();
    assert_eq!(
        narrowed.writable_roots(),
        &[PathBuf::from("/tmp/workspace/src/lib.rs")]
    );
    assert_eq!(narrowed.cwd(), PathBuf::from("/tmp/workspace"));

    let error = policy
        .narrow_writable_roots(vec![PathBuf::from("/tmp/outside")])
        .unwrap_err();
    assert!(error.contains("exceeds existing authority"));
}

#[test]
fn workspace_authority_can_be_narrowed_to_read_only() {
    let policy =
        WorkspacePolicy::from_resolved_roots(PathBuf::from("/tmp/workspace"), vec![]).unwrap();

    let read_only = policy.narrow_writable_roots(vec![]).unwrap();

    assert_eq!(read_only.cwd(), PathBuf::from("/tmp/workspace"));
    assert!(read_only.writable_roots().is_empty());
}

#[test]
fn declared_paths_resolve_inside_existing_authority() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir(temporary.path().join("src")).unwrap();
    let policy = WorkspacePolicy::from_resolved_roots(
        temporary.path().to_path_buf(),
        vec![temporary.path().to_path_buf()],
    )
    .unwrap();

    let narrowed = policy
        .narrow_to_declared_paths(&["src/lib.rs".into(), "src/lib.rs".into()])
        .unwrap();

    assert_eq!(
        narrowed.writable_roots(),
        &[temporary.path().join("src/lib.rs")]
    );
}

#[test]
fn declared_paths_reject_parent_traversal_and_absolute_escape() {
    let temporary = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let policy = WorkspacePolicy::from_resolved_roots(
        temporary.path().to_path_buf(),
        vec![temporary.path().to_path_buf()],
    )
    .unwrap();

    assert!(policy
        .clone()
        .narrow_to_declared_paths(&["../escape".into()])
        .unwrap_err()
        .contains("invalid declared workspace path"));
    assert!(policy
        .narrow_to_declared_paths(&[outside.path().join("escape").display().to_string()])
        .unwrap_err()
        .contains("exceeds existing authority"));
}

#[cfg(unix)]
#[test]
fn declared_paths_reject_symlink_escape() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("link")).unwrap();
    let policy = WorkspacePolicy::from_resolved_roots(
        root.path().to_path_buf(),
        vec![root.path().to_path_buf()],
    )
    .unwrap();

    assert!(policy
        .narrow_to_declared_paths(&["link/pwned.rs".into()])
        .unwrap_err()
        .contains("exceeds existing authority"));
}

#[test]
fn empty_declared_scope_is_read_only() {
    let temporary = tempfile::tempdir().unwrap();
    let policy = WorkspacePolicy::from_resolved_roots(
        temporary.path().to_path_buf(),
        vec![temporary.path().to_path_buf()],
    )
    .unwrap();

    let read_only = policy.narrow_to_declared_paths(&[]).unwrap();

    assert!(read_only.writable_roots().is_empty());
    assert_eq!(read_only.cwd(), temporary.path());
}
use std::path::PathBuf;

#[test]
fn local_principal_encoding_is_stable() {
    assert_eq!(PrincipalId::local_uid(1001).0, "local-uid:1001");
}

#[test]
fn workspace_is_cwd_first_and_deduplicated() {
    let workspace = WorkspacePolicy::from_resolved_roots(
        PathBuf::from("/tmp/project"),
        vec![PathBuf::from("/tmp/extra"), PathBuf::from("/tmp/project")],
    )
    .unwrap();
    assert_eq!(
        workspace.writable_roots(),
        &[PathBuf::from("/tmp/project"), PathBuf::from("/tmp/extra"),]
    );
}

#[test]
fn principal_context_round_trips_without_mutable_metadata() {
    let context = PrincipalContext::new(
        PrincipalId::local_uid(1001),
        LocalOsPrincipal {
            uid: 1001,
            gid: 1001,
        },
        ConnectionId::new(),
        ThreadId::from("thread-a"),
        WorkspacePolicy::from_resolved_roots(PathBuf::from("/tmp"), vec![]).unwrap(),
        PermissionProfileId::workspace_write(),
        ApprovalPolicy::OnRequest,
    );
    let json = serde_json::to_value(&context).unwrap();
    assert_eq!(json["os_principal"]["uid"], 1001);
    assert!(json.get("metadata").is_none());
}
