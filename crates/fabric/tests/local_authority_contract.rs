use fabric::{
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
