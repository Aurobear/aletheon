use std::fs;
use std::path::Path;

use aletheon::extensions::extension_snapshot::ExtensionSnapshotCompiler;
use corpus::extension::asset::AssetKind;
use corpus::extension::package::AssetRef;
use corpus::extension::resolver::{ResolvedPackageAsset, ResolvedPackageSet};
use tempfile::TempDir;

fn resolved_asset(
    root: &Path,
    package_id: &str,
    kind: AssetKind,
    id: &str,
    relative: &str,
    content: &str,
) -> ResolvedPackageAsset {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    ResolvedPackageAsset {
        package_id: package_id.into(),
        package_version: "1.0.0".into(),
        package_hash: format!("{package_id}-hash"),
        asset: AssetRef {
            kind,
            id: id.into(),
            path: relative.into(),
        },
        absolute_path: path.canonicalize().unwrap(),
    }
}

fn skill(root: &Path, package_id: &str, id: &str, name: &str) -> ResolvedPackageAsset {
    resolved_asset(
        root,
        package_id,
        AssetKind::Skill,
        id,
        &format!("assets/{id}/SKILL.md"),
        &format!("---\nname: {name}\ndescription: fixture\n---\nReview carefully."),
    )
}

fn hook(root: &Path, package_id: &str, id: &str) -> ResolvedPackageAsset {
    let script = root.join("payload/audit.sh");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(script, "#!/bin/sh\n").unwrap();
    resolved_asset(
        root,
        package_id,
        AssetKind::Hook,
        id,
        &format!("assets/{id}.toml"),
        "[hook]\nname = \"audit\"\npoint = \"PostTurn\"\nscript = \"payload/audit.sh\"\n",
    )
}

#[test]
fn snapshot_digest_is_order_independent() {
    let temp = TempDir::new().unwrap();
    let package_root = temp.path().join("one");
    fs::create_dir_all(&package_root).unwrap();
    let skill = skill(&package_root, "one", "skill.z", "review");
    let hook = hook(&package_root, "one", "hook.a");
    let compiler = ExtensionSnapshotCompiler::default();

    let a = compiler
        .compile(&ResolvedPackageSet {
            assets: vec![skill.clone(), hook.clone()],
            activation_records: Vec::new(),
        })
        .unwrap();
    let b = compiler
        .compile(&ResolvedPackageSet {
            assets: vec![hook, skill],
            activation_records: Vec::new(),
        })
        .unwrap();
    assert_eq!(a.digest, b.digest);
}

#[test]
fn duplicate_public_names_fail_closed() {
    let temp = TempDir::new().unwrap();
    let one_root = temp.path().join("one");
    let two_root = temp.path().join("two");
    fs::create_dir_all(&one_root).unwrap();
    fs::create_dir_all(&two_root).unwrap();
    let assets = vec![
        skill(&one_root, "one", "skill.review", "review"),
        skill(&two_root, "two", "skill.other", "review"),
    ];

    let error = ExtensionSnapshotCompiler::default()
        .compile(&ResolvedPackageSet {
            assets,
            activation_records: Vec::new(),
        })
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("duplicate public skill name 'review'"));
}

#[test]
fn administrator_mcp_ids_are_reserved() {
    let temp = TempDir::new().unwrap();
    let package_root = temp.path().join("one");
    fs::create_dir_all(&package_root).unwrap();
    let connector = resolved_asset(
        &package_root,
        "one",
        AssetKind::Connector,
        "connector.search",
        "assets/search.json",
        r#"{
            "schema_version": 1,
            "id": "admin-search",
            "transport": {"kind":"streamable_http","url":"https://localhost/mcp"}
        }"#,
    );

    let error = ExtensionSnapshotCompiler::new(["admin-search".to_string()])
        .compile(&ResolvedPackageSet {
            assets: vec![connector],
            activation_records: Vec::new(),
        })
        .unwrap_err();
    assert!(error.to_string().contains("administrator configuration"));
}

#[tokio::test]
async fn runtime_view_publishes_complete_snapshots() {
    let view = aletheon::extensions::extension_snapshot::ExtensionRuntimeView::default();
    let before = view.load().await;
    let mut replacement = aletheon::extensions::extension_snapshot::ExtensionRuntimeSnapshot::empty();
    replacement.digest = "replacement".into();
    view.publish(replacement).await;
    assert_ne!(view.load().await.digest, before.digest);
    assert_eq!(view.load().await.digest, "replacement");
}
