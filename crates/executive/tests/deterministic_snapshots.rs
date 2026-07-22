use std::path::{Path, PathBuf};

use fabric::dasein::SelfVersion;
use fabric::{AgoraSpaceId, BroadcastEpoch, ContentId, ContextProjectionReceipt};

fn assert_checked_in_snapshot(name: &str, rendered: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(name);
    assert_eq!(
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display())),
        rendered,
        "deterministic snapshot drifted: {}",
        path.display()
    );
}

#[test]
fn context_projection_receipt_matches_checked_in_snapshot() {
    let receipt = ContextProjectionReceipt {
        space: AgoraSpaceId("snapshot-space".into()),
        broadcast_epoch: Some(BroadcastEpoch(7)),
        workspace_version: Some(11),
        dasein_version: SelfVersion(5),
        content_ids: vec![
            ContentId(uuid::Uuid::from_u128(1)),
            ContentId(uuid::Uuid::from_u128(2)),
        ],
    };
    receipt.validate().unwrap();
    let rendered = serde_json::to_string_pretty(&receipt).unwrap() + "\n";
    assert_checked_in_snapshot("context_projection_receipt.snap", &rendered);
}

#[test]
fn app_config_schema_matches_checked_in_repository_snapshot() {
    let generated = executive::composition::config::schema::generated_schema_json();
    assert_eq!(
        generated,
        executive::composition::config::schema::generated_schema_json(),
        "schema generation must be deterministic"
    );
    let checked_in = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/schema/aletheon-config.schema.json");
    assert_eq!(
        std::fs::read_to_string(&checked_in)
            .unwrap_or_else(|error| panic!("reading {}: {error}", checked_in.display())),
        generated,
        "AppConfig schema snapshot drifted: regenerate and review {}",
        checked_in.display()
    );
}
