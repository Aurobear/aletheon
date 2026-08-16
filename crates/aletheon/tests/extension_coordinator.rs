use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aletheon::extensions::extension_coordinator::{
    ExtensionCoordinator, ExtensionRuntimePublisher,
};
use aletheon::extensions::extension_snapshot::{
    ExtensionRuntimeSnapshot, ExtensionRuntimeView, ExtensionSnapshotCompiler,
};
use async_trait::async_trait;
use flate2::{write::GzEncoder, Compression};
use sha2::Digest;
use tempfile::TempDir;

struct SerialPublisher {
    active_probes: AtomicUsize,
    maximum_active_probes: AtomicUsize,
}

impl SerialPublisher {
    fn new() -> Self {
        Self {
            active_probes: AtomicUsize::new(0),
            maximum_active_probes: AtomicUsize::new(0),
        }
    }

    fn maximum_active_probes(&self) -> usize {
        self.maximum_active_probes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ExtensionRuntimePublisher for SerialPublisher {
    async fn probe(&self, _: &ExtensionRuntimeSnapshot) -> anyhow::Result<()> {
        let active = self.active_probes.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum_active_probes
            .fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(25)).await;
        self.active_probes.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    async fn publish(
        &self,
        _: Arc<ExtensionRuntimeSnapshot>,
        _: Arc<ExtensionRuntimeSnapshot>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

fn package(root: &Path, id: &str, version: &str, skill_name: &str) -> PathBuf {
    let source = root.join(format!("source-{id}-{version}"));
    let skill = source.join("assets/skills/demo/SKILL.md");
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    let manifest = format!(
        r#"schema_version = 1
[package]
id = "{id}"
version = "{version}"
description = "coordinator fixture"
compatibility = {{ min_aletheon = "0.1.0" }}
[[assets]]
kind = "skill"
id = "skill.demo"
path = "assets/skills/demo/SKILL.md"
"#
    );
    let skill_body =
        format!("---\nname: {skill_name}\ndescription: coordinator fixture\n---\n# Fixture\n");
    fs::write(source.join("extension.toml"), &manifest).unwrap();
    fs::write(&skill, &skill_body).unwrap();
    let checksums = format!(
        "{:x}  extension.toml\n{:x}  assets/skills/demo/SKILL.md\n",
        sha2::Sha256::digest(manifest.as_bytes()),
        sha2::Sha256::digest(skill_body.as_bytes())
    );
    fs::write(source.join("checksums.sha256"), checksums).unwrap();

    let archive = root.join(format!("{id}-{version}.tar.gz"));
    let encoder = GzEncoder::new(fs::File::create(&archive).unwrap(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.append_dir_all(".", &source).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    archive
}

fn coordinator(
    root: &Path,
    publisher: Arc<dyn ExtensionRuntimePublisher>,
) -> anyhow::Result<ExtensionCoordinator> {
    ExtensionCoordinator::new(
        root,
        ExtensionSnapshotCompiler::default(),
        publisher,
        ExtensionRuntimeView::default(),
        Arc::new(kernel::chronos::TestClock::default()),
    )
}

#[tokio::test]
async fn failed_upgrade_keeps_old_activation_and_snapshot() {
    let temp = TempDir::new().unwrap();
    let publisher = Arc::new(SerialPublisher::new());
    let coordinator = coordinator(&temp.path().join("store"), publisher).unwrap();
    let first = package(temp.path(), "test.pkg", "1.0.0", "review");
    coordinator
        .install("operator:test", &first, false)
        .await
        .unwrap();
    coordinator
        .enable("operator:test", "test.pkg", true)
        .await
        .unwrap();
    let other = package(temp.path(), "test.other", "1.0.0", "conflict");
    coordinator
        .install("operator:test", &other, false)
        .await
        .unwrap();
    coordinator
        .enable("operator:test", "test.other", true)
        .await
        .unwrap();
    let before = coordinator.view().load().await;
    let before_activation = coordinator.activation("test.pkg").unwrap();

    let broken = package(temp.path(), "test.pkg", "2.0.0", "conflict");
    let error = coordinator
        .upgrade("operator:test", &broken, false, true)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("compiling extension snapshot"));
    assert_eq!(coordinator.view().load().await.digest, before.digest);
    assert_eq!(
        coordinator.activation("test.pkg").unwrap(),
        before_activation
    );
}

#[tokio::test]
async fn mutations_return_snapshot_bound_receipts() {
    let temp = TempDir::new().unwrap();
    let coordinator =
        coordinator(&temp.path().join("store"), Arc::new(SerialPublisher::new())).unwrap();
    let archive = package(temp.path(), "test.receipt", "1.0.0", "receipt-review");

    let receipt = coordinator
        .install("operator:test", &archive, false)
        .await
        .unwrap();

    assert_eq!(receipt.operation, "install");
    assert_eq!(receipt.actor, "operator:test");
    assert_eq!(receipt.package_id, "test.receipt");
    assert!(!receipt.snapshot_digest.is_empty());
    assert_eq!(receipt.previous_snapshot_digest, receipt.snapshot_digest);
    assert_eq!(receipt.schema_version, 1);
}

#[tokio::test]
async fn concurrent_mutations_are_serialized() {
    let temp = TempDir::new().unwrap();
    let publisher = Arc::new(SerialPublisher::new());
    let coordinator = Arc::new(coordinator(&temp.path().join("store"), publisher.clone()).unwrap());
    for (id, skill) in [("test.one", "one-review"), ("test.two", "two-review")] {
        coordinator
            .install(
                "operator:test",
                &package(temp.path(), id, "1.0.0", skill),
                false,
            )
            .await
            .unwrap();
    }

    let one = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move { coordinator.enable("operator:test", "test.one", true).await })
    };
    let two = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move { coordinator.enable("operator:test", "test.two", true).await })
    };
    one.await.unwrap().unwrap();
    two.await.unwrap().unwrap();

    assert_eq!(publisher.maximum_active_probes(), 1);
}
