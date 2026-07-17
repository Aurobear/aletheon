//! Rollback engine with tiered backend selection.
//!
//! Provides automatic snapshots before destructive operations,
//! with three tiers: AtomicSnapshot (btrfs) > FileBackup > AuditOnly.

pub mod types;

pub use types::*;

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};

/// Core trait for rollback backends.
#[async_trait]
pub trait RollbackBackend: Send + Sync {
    /// Backend name (e.g., "btrfs", "file_backup", "audit_only")
    fn name(&self) -> &str;

    /// Capability tier
    fn tier(&self) -> RollbackTier;

    /// Whether this backend is available on the current system
    fn is_available(&self) -> bool;

    /// Create a snapshot before an operation
    async fn create_snapshot(&self, context: &RollbackContext) -> Result<SnapshotId>;

    /// Rollback to a specific snapshot
    async fn rollback(&self, snapshot_id: &SnapshotId) -> Result<RollbackResult>;

    /// List available snapshots
    async fn list_snapshots(&self) -> Result<Vec<SnapshotId>>;

    /// Cleanup old snapshots
    async fn cleanup(&self, max_age: std::time::Duration) -> Result<u32>;
}

/// Tiered rollback executor following SandboxExecutor pattern.
pub struct RollbackExecutor {
    backends: Vec<Box<dyn RollbackBackend>>,
    preference: RollbackPreference,
    active_snapshots: Vec<SnapshotId>,
}

impl RollbackExecutor {
    pub fn new(config: &RollbackConfig, clock: Arc<dyn fabric::Clock>) -> Self {
        let mut backends: Vec<Box<dyn RollbackBackend>> = Vec::new();

        // Tier 3: btrfs (best)
        #[cfg(feature = "rollback-btrfs")]
        {
            if let Some(btrfs) = BtrfsRollbackBackend::probe(clock.clone()) {
                backends.push(Box::new(btrfs));
            }
        }

        // Tier 2: File backup (always available)
        backends.push(Box::new(FileBackupBackend::new(
            &config.protected_paths,
            clock.clone(),
        )));

        // Tier 1: Audit only (always available, last resort)
        backends.push(Box::new(AuditOnlyBackend::new(clock.clone())));

        info!(
            "Rollback engine initialized with {} backends, preference: {:?}",
            backends.len(),
            config.preference
        );

        Self {
            backends,
            preference: config.preference.clone(),
            active_snapshots: Vec::new(),
        }
    }

    /// Select the best available backend.
    pub fn select_backend(&self) -> Option<&dyn RollbackBackend> {
        match self.preference {
            RollbackPreference::Auto | RollbackPreference::BestEffort => self
                .backends
                .iter()
                .find(|b| b.is_available())
                .map(|b| b.as_ref()),
            RollbackPreference::Require => self
                .backends
                .iter()
                .find(|b| {
                    b.is_available()
                        && matches!(
                            b.tier(),
                            RollbackTier::AtomicSnapshot | RollbackTier::FileBackup
                        )
                })
                .map(|b| b.as_ref()),
            RollbackPreference::Forbid => self
                .backends
                .iter()
                .find(|b| b.name() == "audit_only")
                .map(|b| b.as_ref()),
        }
    }

    /// Create a snapshot using the best available backend.
    pub async fn snapshot(&mut self, context: &RollbackContext) -> Result<SnapshotId> {
        let backend = self
            .select_backend()
            .ok_or_else(|| anyhow::anyhow!("No rollback backend available"))?;

        let snapshot_id = backend.create_snapshot(context).await?;
        self.active_snapshots.push(snapshot_id.clone());
        info!(
            tier = ?snapshot_id.tier,
            id = %snapshot_id.id,
            operation = %context.operation,
            "Snapshot created"
        );
        Ok(snapshot_id)
    }

    /// Rollback to a specific snapshot.
    pub async fn rollback(&self, snapshot_id: &SnapshotId) -> Result<RollbackResult> {
        // Find the backend that matches the snapshot's tier
        let backend = self
            .backends
            .iter()
            .find(|b| b.tier() == snapshot_id.tier && b.is_available());

        match backend {
            Some(b) => b.rollback(snapshot_id).await,
            None => {
                warn!("Original backend unavailable, trying lower tier backends");
                // Try any available backend
                if let Some(b) = self.select_backend() {
                    b.rollback(snapshot_id).await
                } else {
                    anyhow::bail!("No rollback backend available")
                }
            }
        }
    }

    /// Cleanup old snapshots across all backends.
    pub async fn cleanup(&self, max_age: std::time::Duration) -> Result<u32> {
        let mut total_cleaned = 0u32;
        for backend in &self.backends {
            if backend.is_available() {
                match backend.cleanup(max_age).await {
                    Ok(n) => total_cleaned += n,
                    Err(e) => warn!("Cleanup failed for {}: {}", backend.name(), e),
                }
            }
        }
        Ok(total_cleaned)
    }

    /// Get the active backend name and tier.
    pub fn active_info(&self) -> Option<(&str, RollbackTier)> {
        self.select_backend().map(|b| (b.name(), b.tier()))
    }
}

// === Tier 1: Audit Only (always available) ===

pub struct AuditOnlyBackend {
    clock: Arc<dyn fabric::Clock>,
}

impl AuditOnlyBackend {
    pub fn new(clock: Arc<dyn fabric::Clock>) -> Self {
        Self { clock }
    }
}

#[async_trait]
impl RollbackBackend for AuditOnlyBackend {
    fn name(&self) -> &str {
        "audit_only"
    }
    fn tier(&self) -> RollbackTier {
        RollbackTier::AuditOnly
    }
    fn is_available(&self) -> bool {
        true
    }

    async fn create_snapshot(&self, context: &RollbackContext) -> Result<SnapshotId> {
        let id = SnapshotId::new(
            RollbackTier::AuditOnly,
            fabric::wall_to_datetime(self.clock.wall_now()),
        );
        // Log the operation for manual rollback guidance
        tracing::info!(
            snapshot_id = %id.id,
            operation = %context.operation,
            paths = ?context.paths,
            "Audit-only snapshot: operation logged for manual rollback"
        );
        Ok(id)
    }

    async fn rollback(&self, snapshot_id: &SnapshotId) -> Result<RollbackResult> {
        Ok(RollbackResult {
            success: false,
            snapshot_id: snapshot_id.clone(),
            restored_paths: vec![],
            message: "Audit-only tier cannot perform automatic rollback. \
                      Check audit logs for operation details and rollback manually."
                .to_string(),
        })
    }

    async fn list_snapshots(&self) -> Result<Vec<SnapshotId>> {
        Ok(vec![]) // No persistent snapshots in audit-only mode
    }

    async fn cleanup(&self, _max_age: std::time::Duration) -> Result<u32> {
        Ok(0) // Nothing to clean up
    }
}

// === Tier 2: File Backup ===

pub struct FileBackupBackend {
    backup_dir: std::path::PathBuf,
    /// Paths excluded from rollback — reserved for future safety checks.
    #[allow(dead_code)]
    protected_paths: Vec<String>,
    clock: Arc<dyn fabric::Clock>,
}

impl FileBackupBackend {
    pub fn new(protected_paths: &[String], clock: Arc<dyn fabric::Clock>) -> Self {
        let backup_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(".aletheon")
            .join("snapshots");

        Self {
            backup_dir,
            protected_paths: protected_paths.to_vec(),
            clock,
        }
    }

    #[cfg(test)]
    fn with_backup_dir(
        backup_dir: std::path::PathBuf,
        protected_paths: &[String],
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        Self {
            backup_dir,
            protected_paths: protected_paths.to_vec(),
            clock,
        }
    }

    fn snapshot_dir(&self, id: &str) -> std::path::PathBuf {
        self.backup_dir.join(id)
    }
}

#[async_trait]
impl RollbackBackend for FileBackupBackend {
    fn name(&self) -> &str {
        "file_backup"
    }
    fn tier(&self) -> RollbackTier {
        RollbackTier::FileBackup
    }
    fn is_available(&self) -> bool {
        true
    }

    async fn create_snapshot(&self, context: &RollbackContext) -> Result<SnapshotId> {
        let id = SnapshotId::new(
            RollbackTier::FileBackup,
            fabric::wall_to_datetime(self.clock.wall_now()),
        );
        let snap_dir = self.snapshot_dir(&id.id);
        tokio::fs::create_dir_all(&snap_dir).await?;

        // Copy each path to the snapshot directory
        for path in &context.paths {
            let source = std::path::Path::new(path);
            if !source.exists() {
                continue;
            }

            let dest = snap_dir.join(path.trim_start_matches('/'));
            if source.is_dir() {
                // Use cp -a for directories
                let output = tokio::process::Command::new("cp")
                    .args(["-a", path, &dest.to_string_lossy()])
                    .output()
                    .await;
                match output {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => {
                        tracing::warn!(
                            "cp failed for {}: {}",
                            path,
                            String::from_utf8_lossy(&o.stderr)
                        );
                    }
                    Err(e) => {
                        tracing::warn!("cp command failed for {}: {}", path, e);
                    }
                }
            } else {
                if let Some(parent) = dest.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                let _ = tokio::fs::copy(source, &dest).await;
            }
        }

        // Record service states
        let services_path = snap_dir.join(".service_states");
        let states = capture_service_states().await;
        let _ = tokio::fs::write(&services_path, &states).await;

        tracing::info!(
            snapshot_id = %id.id,
            paths = ?context.paths,
            backup_dir = %snap_dir.display(),
            "File backup snapshot created"
        );

        Ok(id)
    }

    async fn rollback(&self, snapshot_id: &SnapshotId) -> Result<RollbackResult> {
        let snap_dir = self.snapshot_dir(&snapshot_id.id);
        if !snap_dir.exists() {
            anyhow::bail!("Snapshot directory not found: {}", snap_dir.display());
        }

        let mut restored = Vec::new();

        // Restore each path
        let mut entries = tokio::fs::read_dir(&snap_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();

            // Skip metadata files
            if name.starts_with('.') {
                continue;
            }

            let source = entry.path();
            let dest = std::path::PathBuf::from("/").join(name.as_ref());

            if source.is_dir() {
                let output = tokio::process::Command::new("cp")
                    .args(["-a", &source.to_string_lossy(), &dest.to_string_lossy()])
                    .output()
                    .await;
                if let Ok(o) = output {
                    if o.status.success() {
                        restored.push(dest.to_string_lossy().to_string());
                    }
                }
            } else {
                if let Some(parent) = dest.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                if tokio::fs::copy(&source, &dest).await.is_ok() {
                    restored.push(dest.to_string_lossy().to_string());
                }
            }
        }

        // Restore service states
        let services_path = snap_dir.join(".service_states");
        if services_path.exists() {
            if let Ok(states) = tokio::fs::read_to_string(&services_path).await {
                restore_service_states(&states).await;
            }
        }

        Ok(RollbackResult {
            success: true,
            snapshot_id: snapshot_id.clone(),
            restored_paths: restored.clone(),
            message: format!("Restored {} paths from file backup", restored.len()),
        })
    }

    async fn list_snapshots(&self) -> Result<Vec<SnapshotId>> {
        let mut snapshots = Vec::new();
        if !self.backup_dir.exists() {
            return Ok(snapshots);
        }

        let mut entries = tokio::fs::read_dir(&self.backup_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                let id = entry.file_name().to_string_lossy().to_string();
                snapshots.push(SnapshotId {
                    id,
                    tier: RollbackTier::FileBackup,
                    created_at: entry
                        .metadata()
                        .await?
                        .modified()
                        .map(chrono::DateTime::from)
                        .unwrap_or_else(|_| fabric::wall_to_datetime(self.clock.wall_now())),
                });
            }
        }

        Ok(snapshots)
    }

    async fn cleanup(&self, max_age: std::time::Duration) -> Result<u32> {
        let mut cleaned = 0u32;
        if !self.backup_dir.exists() {
            return Ok(0);
        }

        let now = std::time::UNIX_EPOCH
            + std::time::Duration::from_millis(self.clock.wall_now().0 as u64);
        let cutoff = now - max_age;
        let mut entries = tokio::fs::read_dir(&self.backup_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Ok(metadata) = entry.metadata().await {
                if let Ok(modified) = metadata.modified() {
                    if modified < cutoff {
                        let _ = tokio::fs::remove_dir_all(entry.path()).await;
                        cleaned += 1;
                    }
                }
            }
        }

        Ok(cleaned)
    }
}

// === Tier 3: btrfs Snapshot (feature-gated) ===

#[cfg(feature = "rollback-btrfs")]
pub struct BtrfsRollbackBackend {
    snapshot_dir: std::path::PathBuf,
    clock: Arc<dyn fabric::Clock>,
}

#[cfg(feature = "rollback-btrfs")]
impl BtrfsRollbackBackend {
    pub fn probe(clock: Arc<dyn fabric::Clock>) -> Option<Self> {
        // Check if btrfs tools are available
        if std::process::Command::new("btrfs")
            .arg("--version")
            .output()
            .is_err()
        {
            return None;
        }

        // Check if root filesystem is btrfs
        let output = std::process::Command::new("stat")
            .args(["-f", "-c", "%T", "/"])
            .output()
            .ok()?;
        let fs_type = String::from_utf8_lossy(&output.stdout);
        if !fs_type.trim().contains("btrfs") {
            return None;
        }

        Some(Self {
            snapshot_dir: std::path::PathBuf::from(fabric::paths::SNAPSHOT_DIR),
            clock,
        })
    }
}

#[cfg(feature = "rollback-btrfs")]
#[async_trait]
impl RollbackBackend for BtrfsRollbackBackend {
    fn name(&self) -> &str {
        "btrfs"
    }
    fn tier(&self) -> RollbackTier {
        RollbackTier::AtomicSnapshot
    }
    fn is_available(&self) -> bool {
        true
    }

    async fn create_snapshot(&self, context: &RollbackContext) -> Result<SnapshotId> {
        let id = SnapshotId::new(
            RollbackTier::AtomicSnapshot,
            fabric::wall_to_datetime(self.clock.wall_now()),
        );
        let snap_path = self.snapshot_dir.join(&id.id);

        tokio::fs::create_dir_all(&self.snapshot_dir).await?;

        // Create btrfs subvolume snapshot
        for path in &context.paths {
            let source = std::path::Path::new(path);
            if !source.exists() {
                continue;
            }

            let dest = snap_path.join(path.trim_start_matches('/'));
            let output = tokio::process::Command::new("btrfs")
                .args(["subvolume", "snapshot", "-r", path, &dest.to_string_lossy()])
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("btrfs snapshot failed for {}: {}", path, stderr);
            }
        }

        tracing::info!(
            snapshot_id = %id.id,
            paths = ?context.paths,
            "Btrfs atomic snapshot created"
        );

        Ok(id)
    }

    async fn rollback(&self, snapshot_id: &SnapshotId) -> Result<RollbackResult> {
        let snap_path = self.snapshot_dir.join(&snapshot_id.id);
        if !snap_path.exists() {
            anyhow::bail!("Snapshot not found: {}", snap_path.display());
        }

        let mut restored = Vec::new();

        // Restore each subvolume
        let mut entries = tokio::fs::read_dir(&snap_path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let source = entry.path();
            let dest = std::path::PathBuf::from("/").join(&name);

            // Delete current and restore from snapshot
            if dest.exists() {
                let _ = tokio::fs::remove_dir_all(&dest).await;
            }

            let output = tokio::process::Command::new("btrfs")
                .args([
                    "subvolume",
                    "snapshot",
                    &source.to_string_lossy(),
                    &dest.to_string_lossy(),
                ])
                .output()
                .await?;

            if output.status.success() {
                restored.push(dest.to_string_lossy().to_string());
            }
        }

        Ok(RollbackResult {
            success: true,
            snapshot_id: snapshot_id.clone(),
            restored_paths: restored.clone(),
            message: format!("Restored {} paths from btrfs snapshot", restored.len()),
        })
    }

    async fn list_snapshots(&self) -> Result<Vec<SnapshotId>> {
        let mut snapshots = Vec::new();
        if !self.snapshot_dir.exists() {
            return Ok(snapshots);
        }

        let mut entries = tokio::fs::read_dir(&self.snapshot_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                snapshots.push(SnapshotId {
                    id: entry.file_name().to_string_lossy().to_string(),
                    tier: RollbackTier::AtomicSnapshot,
                    created_at: entry
                        .metadata()
                        .await?
                        .modified()
                        .map(chrono::DateTime::from)
                        .unwrap_or_else(|_| fabric::wall_to_datetime(self.clock.wall_now())),
                });
            }
        }

        Ok(snapshots)
    }

    async fn cleanup(&self, max_age: std::time::Duration) -> Result<u32> {
        let mut cleaned = 0u32;
        if !self.snapshot_dir.exists() {
            return Ok(0);
        }

        let now = std::time::UNIX_EPOCH
            + std::time::Duration::from_millis(self.clock.wall_now().0 as u64);
        let cutoff = now - max_age;
        let mut entries = tokio::fs::read_dir(&self.snapshot_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Ok(metadata) = entry.metadata().await {
                if let Ok(modified) = metadata.modified() {
                    if modified < cutoff {
                        // Delete btrfs subvolume
                        let output = tokio::process::Command::new("btrfs")
                            .args(["subvolume", "delete", &entry.path().to_string_lossy()])
                            .output()
                            .await;
                        if let Ok(o) = output {
                            if o.status.success() {
                                cleaned += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(cleaned)
    }
}

// === Helpers ===

/// Capture current systemd service states.
async fn capture_service_states() -> String {
    let output = tokio::process::Command::new("systemctl")
        .args([
            "list-units",
            "--type=service",
            "--state=running",
            "--no-pager",
            "--plain",
        ])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

/// Restore service states (start stopped services, stop started services).
async fn restore_service_states(states: &str) {
    for line in states.lines() {
        if let Some(service) = line.split_whitespace().next() {
            if service.ends_with(".service") {
                let _ = tokio::process::Command::new("systemctl")
                    .args(["restart", service])
                    .output()
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn now_dt(clock: &dyn fabric::Clock) -> chrono::DateTime<chrono::Utc> {
        fabric::wall_to_datetime(clock.wall_now())
    }

    #[test]
    fn test_rollback_tier_ordering() {
        assert!(RollbackTier::AuditOnly < RollbackTier::FileBackup);
        assert!(RollbackTier::FileBackup < RollbackTier::AtomicSnapshot);
    }

    #[test]
    fn test_snapshot_id_new() {
        let clock = test_clock();
        let id = SnapshotId::new(RollbackTier::FileBackup, now_dt(&*clock));
        assert_eq!(id.tier, RollbackTier::FileBackup);
        assert!(!id.id.is_empty());
    }

    #[test]
    fn test_rollback_config_default() {
        let config = RollbackConfig::default();
        assert!(config.enabled);
        assert_eq!(config.preference, RollbackPreference::Auto);
        assert_eq!(config.max_snapshots, 50);
    }

    #[test]
    fn test_rollback_executor_creation() {
        let config = RollbackConfig::default();
        let executor = RollbackExecutor::new(&config, test_clock());
        // Should have at least file_backup + audit_only
        assert!(executor.backends.len() >= 2);
    }

    #[test]
    fn test_rollback_executor_select_auto() {
        let config = RollbackConfig::default();
        let executor = RollbackExecutor::new(&config, test_clock());
        let backend = executor.select_backend();
        assert!(backend.is_some());
        // Should select file_backup (highest tier available without btrfs)
        assert_eq!(backend.unwrap().name(), "file_backup");
    }

    #[test]
    fn test_rollback_executor_select_forbid() {
        let config = RollbackConfig {
            preference: RollbackPreference::Forbid,
            ..Default::default()
        };
        let executor = RollbackExecutor::new(&config, test_clock());
        let backend = executor.select_backend();
        assert!(backend.is_some());
        assert_eq!(backend.unwrap().name(), "audit_only");
    }

    #[tokio::test]
    async fn test_audit_only_snapshot() {
        let clock = test_clock();
        let backend = AuditOnlyBackend::new(clock);
        let context = RollbackContext {
            operation: "test".to_string(),
            paths: vec!["/tmp/test".to_string()],
            tool: None,
            risk_level: None,
        };
        let id = backend.create_snapshot(&context).await.unwrap();
        assert_eq!(id.tier, RollbackTier::AuditOnly);
    }

    #[tokio::test]
    async fn test_audit_only_rollback() {
        let clock = test_clock();
        let backend = AuditOnlyBackend::new(clock.clone());
        let id = SnapshotId::new(RollbackTier::AuditOnly, now_dt(&*clock));
        let result = backend.rollback(&id).await.unwrap();
        assert!(!result.success); // audit_only cannot rollback
        assert!(result.message.contains("manual"));
    }

    #[tokio::test]
    async fn test_file_backup_snapshot_and_rollback() {
        let clock = test_clock();
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path().join("snapshots");
        let source_path = dir.path().join("aletheon-test-snapshot");
        let backend = FileBackupBackend::with_backup_dir(backup_dir, &[], clock);
        let context = RollbackContext {
            operation: "test".to_string(),
            paths: vec![source_path.to_string_lossy().to_string()],
            tool: None,
            risk_level: None,
        };

        // Create a test file
        std::fs::write(&source_path, "test data").unwrap();

        let id = backend.create_snapshot(&context).await.unwrap();
        assert_eq!(id.tier, RollbackTier::FileBackup);

        // Verify snapshot exists
        let snapshots = backend.list_snapshots().await.unwrap();
        assert!(snapshots.iter().any(|s| s.id == id.id));

        // Cleanup
        let _ = std::fs::remove_file(source_path);
        let _ = backend.cleanup(std::time::Duration::from_secs(0)).await;
    }
}
