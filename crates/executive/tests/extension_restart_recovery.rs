use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use executive::application::extension_coordinator::ExtensionRuntimePublisher;
use executive::application::extension_install::ExtensionInstallService;
use executive::application::extension_manage::ExtensionManageService;
use executive::application::extension_snapshot::{
    ExtensionRuntimeSnapshot, ExtensionSnapshotCompiler,
};
use executive::host::daemon::bootstrap::extensions::reconcile_extension_snapshot;
use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::sync::Mutex;

#[derive(Default)]
struct RecordingPublisher {
    current: Mutex<Option<ExtensionRuntimeSnapshot>>,
}

#[async_trait::async_trait]
impl ExtensionRuntimePublisher for RecordingPublisher {
    async fn probe(&self, _: &ExtensionRuntimeSnapshot) -> anyhow::Result<()> {
        Ok(())
    }

    async fn publish(
        &self,
        _: Arc<ExtensionRuntimeSnapshot>,
        candidate: Arc<ExtensionRuntimeSnapshot>,
    ) -> anyhow::Result<()> {
        *self.current.lock().await = Some(candidate.as_ref().clone());
        Ok(())
    }
}

fn package(root: &Path, id: &str, skill: &[u8]) -> PathBuf {
    let source = root.join(format!("source-{id}"));
    let asset_path = "assets/skills/review/SKILL.md";
    fs::create_dir_all(source.join("assets/skills/review")).unwrap();
    let manifest = format!(
        r#"schema_version = 1
[package]
id = "{id}"
version = "1.0.0"
description = "restart recovery fixture"
compatibility = {{ min_aletheon = "0.1.0" }}
[[assets]]
kind = "skill"
id = "skill.review"
path = "{asset_path}"
"#
    );
    fs::write(source.join("extension.toml"), &manifest).unwrap();
    fs::write(source.join(asset_path), skill).unwrap();
    fs::write(
        source.join("checksums.sha256"),
        format!(
            "{:x}  extension.toml\n{:x}  {asset_path}\n",
            Sha256::digest(manifest.as_bytes()),
            Sha256::digest(skill)
        ),
    )
    .unwrap();
    let archive = root.join(format!("{id}.tar.gz"));
    let encoder = GzEncoder::new(fs::File::create(&archive).unwrap(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.append_dir_all(".", source).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    archive
}

#[tokio::test]
async fn restart_quarantines_bad_package_without_losing_healthy_packages() {
    let temp = TempDir::new().unwrap();
    let store_root = temp.path().join("store");
    let install = ExtensionInstallService::new(&store_root).unwrap();
    let healthy = package(
        temp.path(),
        "healthy.pkg",
        b"---\nname: healthy:review\ndescription: Healthy review\n---\n# Review\n",
    );
    let broken = package(
        temp.path(),
        "broken.pkg",
        b"This file has no required Skill frontmatter.\n",
    );
    install.install(&healthy).unwrap();
    install.install(&broken).unwrap();
    let manage = ExtensionManageService::new(&store_root).unwrap();
    manage
        .enable_with_operator_approval("healthy.pkg", "operator:test")
        .unwrap();
    manage
        .enable_with_operator_approval("broken.pkg", "operator:test")
        .unwrap();

    let publisher = RecordingPublisher::default();
    let compiler = ExtensionSnapshotCompiler::default();
    let first = reconcile_extension_snapshot(
        &store_root,
        &compiler,
        &publisher,
        Arc::new(ExtensionRuntimeSnapshot::empty()),
    )
    .await
    .unwrap();
    assert_eq!(
        first
            .snapshot
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>(),
        vec!["healthy:review"]
    );
    assert_eq!(first.quarantined, vec!["broken.pkg"]);
    assert_eq!(first.snapshot.package_digests.len(), 1);
    let digest = first.snapshot.digest.clone();

    let second =
        reconcile_extension_snapshot(&store_root, &compiler, &publisher, Arc::new(first.snapshot))
            .await
            .unwrap();
    assert_eq!(second.snapshot.digest, digest);
    assert_eq!(second.quarantined, vec!["broken.pkg"]);
    let activation = coordinator_activation(&store_root, "broken.pkg");
    assert!(!activation.enabled);
    assert_eq!(activation.health, "quarantined");
}

fn coordinator_activation(root: &Path, id: &str) -> corpus::extension::store::ActivationRecord {
    corpus::extension::store::PackageStore::new(root.to_owned())
        .unwrap()
        .read_activation(id)
        .unwrap()
}
