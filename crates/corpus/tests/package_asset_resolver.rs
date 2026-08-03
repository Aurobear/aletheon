use std::fs;

use corpus::extension::resolver::PackageAssetResolver;
use corpus::extension::store::{
    ActivationRecord, InstalledPackageRecord, PackageSourceRecord, PackageStore,
};
use fabric::types::extension_asset::AssetKind;
use fabric::types::extension_package::{AssetRef, PermissionRequestSet};
use tempfile::TempDir;

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn record(assets: Vec<AssetRef>) -> InstalledPackageRecord {
    InstalledPackageRecord {
        schema_version: 1,
        id: "aurb.core".into(),
        version: "1.0.0".into(),
        description: "fixture".into(),
        hash: HASH.into(),
        file_count: assets.len(),
        total_size: 1,
        installed_at: "2026-08-03T00:00:00Z".into(),
        assets,
        requested_permissions: PermissionRequestSet::default(),
        source: PackageSourceRecord::LocalArchive,
        workspace_trust: None,
    }
}

fn asset(kind: AssetKind, id: &str, path: &str) -> AssetRef {
    AssetRef {
        kind,
        id: id.into(),
        path: path.into(),
    }
}

#[test]
fn resolves_assets_in_stable_kind_and_id_order() {
    let temp = TempDir::new().unwrap();
    let store = PackageStore::new(temp.path().to_path_buf()).unwrap();
    let package_root = store.package_path(HASH).unwrap();
    for relative in [
        "assets/skills/review/SKILL.md",
        "assets/hooks/audit.toml",
        "assets/connectors/gbrain.json",
        "assets/agents/reviewer.md",
    ] {
        let path = package_root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "fixture").unwrap();
    }
    store
        .put_installed(&record(vec![
            asset(
                AssetKind::Skill,
                "skill.review",
                "assets/skills/review/SKILL.md",
            ),
            asset(AssetKind::Hook, "hook.audit", "assets/hooks/audit.toml"),
            asset(
                AssetKind::Connector,
                "connector.gbrain",
                "assets/connectors/gbrain.json",
            ),
            asset(
                AssetKind::AgentProfile,
                "agent.reviewer",
                "assets/agents/reviewer.md",
            ),
        ]))
        .unwrap();
    store
        .write_activation(&ActivationRecord {
            schema_version: 1,
            package_id: "aurb.core".into(),
            enabled: true,
            current: Some(HASH.into()),
            health: "healthy".into(),
            ..ActivationRecord::default()
        })
        .unwrap();

    let resolved = PackageAssetResolver::new(store).resolve_enabled().unwrap();
    assert_eq!(
        resolved.asset_ids(),
        [
            "agent.reviewer",
            "connector.gbrain",
            "hook.audit",
            "skill.review"
        ]
    );
    let canonical_root = package_root.canonicalize().unwrap();
    assert!(resolved
        .iter()
        .all(|asset| asset.path().starts_with(&canonical_root)));
}

#[test]
fn enabled_activation_requires_installed_projection() {
    let temp = TempDir::new().unwrap();
    let store = PackageStore::new(temp.path().to_path_buf()).unwrap();
    store
        .write_activation(&ActivationRecord {
            schema_version: 1,
            package_id: "aurb.core".into(),
            enabled: true,
            current: Some(HASH.into()),
            ..ActivationRecord::default()
        })
        .unwrap();

    let error = PackageAssetResolver::new(store)
        .resolve_enabled()
        .unwrap_err();
    assert!(error.to_string().contains("no installed projection"));
}

#[test]
fn enabled_activation_requires_content_addressed_package_root() {
    let temp = TempDir::new().unwrap();
    let store = PackageStore::new(temp.path().to_path_buf()).unwrap();
    store
        .put_installed(&record(vec![asset(
            AssetKind::Skill,
            "skill.review",
            "assets/skills/review/SKILL.md",
        )]))
        .unwrap();
    store
        .write_activation(&ActivationRecord {
            schema_version: 1,
            package_id: "aurb.core".into(),
            enabled: true,
            current: Some(HASH.into()),
            ..ActivationRecord::default()
        })
        .unwrap();

    let error = PackageAssetResolver::new(store)
        .resolve_enabled()
        .unwrap_err();
    assert!(error.to_string().contains("content is missing"));
}

#[test]
fn package_skill_and_hook_parsers_use_contained_paths() {
    let temp = TempDir::new().unwrap();
    let skill_dir = temp.path().join("assets/skills/review");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: review\ndescription: Review changes\n---\nReview carefully.",
    )
    .unwrap();
    let (loaded, plugin) = corpus::skill::loader::load_skill_dir(&skill_dir).unwrap();
    assert_eq!(loaded.name, "review");
    assert_eq!(plugin.skill_dir, skill_dir);

    let hook_manifest = temp.path().join("assets/hooks/audit.toml");
    fs::create_dir_all(hook_manifest.parent().unwrap()).unwrap();
    fs::create_dir_all(temp.path().join("payload/bin")).unwrap();
    fs::write(temp.path().join("payload/bin/audit.sh"), "#!/bin/sh\n").unwrap();
    fs::write(
        &hook_manifest,
        "[hook]\nname = \"audit\"\npoint = \"PostTurn\"\nscript = \"payload/bin/audit.sh\"\ntimeout_ms = 2500\n",
    )
    .unwrap();
    let hook = corpus::hook::loader::load_hook_path(&hook_manifest, Some(temp.path())).unwrap();
    assert_eq!(hook.timeout_ms, Some(2500));
    assert!(hook.script.starts_with(temp.path().canonicalize().unwrap()));

    fs::write(
        &hook_manifest,
        "[hook]\nname = \"audit\"\npoint = \"PostTurn\"\nscript = \"../outside.sh\"\n",
    )
    .unwrap();
    assert!(corpus::hook::loader::load_hook_path(&hook_manifest, Some(temp.path())).is_err());
}
