use std::path::PathBuf;

use fabric::types::{workspace_checkpoint, workspace_trust};

fn accepts_canonical(_: workspace_checkpoint::WorkspaceIdentity) {}

#[test]
fn checkpoint_and_trust_paths_share_one_workspace_identity_type() {
    let trust_identity = workspace_trust::WorkspaceIdentity {
        canonical_path: PathBuf::from("/verified/workspace"),
        repo_fingerprint: Some("sha256:abc".into()),
    };

    accepts_canonical(trust_identity.clone());
    assert!(trust_identity.matches(&trust_identity));
}

#[test]
fn serialized_identity_shape_remains_backward_compatible() {
    let identity = workspace_checkpoint::WorkspaceIdentity {
        canonical_path: PathBuf::from("/verified/workspace"),
        repo_fingerprint: Some("sha256:abc".into()),
    };

    let value = serde_json::to_value(identity).unwrap();
    assert_eq!(value["canonical_path"], "/verified/workspace");
    assert_eq!(value["repo_fingerprint"], "sha256:abc");
}
