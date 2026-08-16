//! Atomic streaming writes for bounded content-addressed artifacts.

use ::contracts::types::episode_report::{
    ArtifactAvailability, ArtifactRetentionTombstone, ArtifactTimeRange, EpisodeArtifactManifest,
};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::schema_migrations as migrations;
use platform::storage_quota::{StorageClass, StorageQuota, StorageReservation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactScanStatus {
    Unscanned,
    Clean,
    Quarantined,
    Rejected,
}

impl ArtifactScanStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unscanned => "unscanned",
            Self::Clean => "clean",
            Self::Quarantined => "quarantined",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArtifactMetadata {
    pub mime_type: String,
    pub provider: String,
    pub account_id: String,
    pub provider_message_id: String,
    pub provider_part_id: String,
    pub source_timestamp_ms: i64,
    pub scan_status: ArtifactScanStatus,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub mime_type: String,
    pub relative_path: PathBuf,
    pub scan_status: ArtifactScanStatus,
}

/// Durable retention tombstone. Content bytes are gone, but the digest and
/// bounded provenance needed to interpret an immutable episode report remain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactTombstone {
    pub artifact_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub mime_type: String,
    pub producer: String,
    pub source_timestamp_ms: i64,
    pub expired_at_unix_ms: i64,
    pub reason: String,
}

impl ArtifactTombstone {
    pub fn episode_tombstone(&self) -> ArtifactRetentionTombstone {
        ArtifactRetentionTombstone {
            uri: format!("artifact://sha256/{}", self.sha256),
            digest: self.sha256.clone(),
            expired_at_unix_ms: self.expired_at_unix_ms,
            reason: self.reason.clone(),
        }
    }
}

pub struct ArtifactStore {
    db: Mutex<Connection>,
    root: PathBuf,
    quota: Option<StorageQuota>,
}

impl ArtifactStore {
    pub fn open(db_path: &Path, root: &Path) -> Result<Self> {
        let db = Connection::open(db_path)?;
        migrations::run_migrations(&db)?;
        std::fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(".upload-") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let store = Self {
            db: Mutex::new(db),
            root,
            quota: None,
        };
        store.purge_expired_content()?;
        Ok(store)
    }

    pub fn with_quota(mut self, quota: StorageQuota) -> Self {
        self.quota = Some(quota);
        self
    }

    pub fn begin(&self, metadata: ArtifactMetadata, max_bytes: u64) -> Result<ArtifactWriter> {
        validate_metadata(&metadata)?;
        anyhow::ensure!(
            (1..=64 * 1_048_576).contains(&max_bytes),
            "invalid artifact cap"
        );
        let temp = self.root.join(format!(".upload-{}", uuid::Uuid::new_v4()));
        let reservation = self
            .quota
            .as_ref()
            .map(|quota| quota.reserve(StorageClass::Artifacts, max_bytes, 1))
            .transpose()?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        Ok(ArtifactWriter {
            temp,
            file: Some(file),
            hasher: Sha256::new(),
            size: 0,
            max_bytes,
            metadata,
            reservation,
        })
    }

    pub fn finish(&self, mut writer: ArtifactWriter) -> Result<ArtifactRecord> {
        writer
            .file
            .take()
            .context("artifact writer already finished")?
            .sync_all()?;
        let sha256 = format!("{:x}", writer.hasher.clone().finalize());
        let artifact_id = format!("sha256:{sha256}");
        let relative = PathBuf::from(&sha256[..2]).join(&sha256);
        let final_path = self.root.join(&relative);
        let parent = final_path.parent().context("artifact path has no parent")?;
        std::fs::create_dir_all(parent)?;
        let canonical_parent = parent.canonicalize()?;
        anyhow::ensure!(
            canonical_parent.starts_with(&self.root),
            "artifact path escaped root"
        );
        if self.retention_tombstone(&artifact_id)?.is_some() {
            std::fs::remove_file(&writer.temp)?;
            anyhow::bail!("retention-expired artifact digest cannot be silently resurrected");
        }
        if final_path.exists() {
            std::fs::remove_file(&writer.temp)?;
        } else {
            std::fs::rename(&writer.temp, &final_path)?;
        }
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        db.execute(
            "INSERT OR IGNORE INTO external_artifacts(
                artifact_id,sha256,size_bytes,mime_type,provider,account_id,
                provider_message_id,provider_part_id,source_timestamp_ms,scan_status,
                relative_path,created_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                artifact_id,
                sha256,
                writer.size,
                writer.metadata.mime_type,
                writer.metadata.provider,
                writer.metadata.account_id,
                writer.metadata.provider_message_id,
                writer.metadata.provider_part_id,
                writer.metadata.source_timestamp_ms,
                writer.metadata.scan_status.as_str(),
                relative.to_string_lossy(),
                writer.metadata.created_at_ms
            ],
        )?;
        db.execute(
            "INSERT OR IGNORE INTO external_artifact_sources(
                artifact_id,provider,account_id,provider_message_id,provider_part_id,
                source_timestamp_ms,created_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                artifact_id,
                writer.metadata.provider,
                writer.metadata.account_id,
                writer.metadata.provider_message_id,
                writer.metadata.provider_part_id,
                writer.metadata.source_timestamp_ms,
                writer.metadata.created_at_ms
            ],
        )?;
        drop(db);
        if let Some(reservation) = writer.reservation.take() {
            reservation.commit();
        }
        self.get(&artifact_id)?
            .context("artifact metadata write failed")
    }

    pub fn get(&self, artifact_id: &str) -> Result<Option<ArtifactRecord>> {
        self.db
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT sha256,size_bytes,mime_type,relative_path,scan_status
                 FROM external_artifacts WHERE artifact_id=?1",
                [artifact_id],
                |row| {
                    let status: String = row.get(4)?;
                    Ok(ArtifactRecord {
                        artifact_id: artifact_id.to_owned(),
                        sha256: row.get(0)?,
                        size_bytes: row.get(1)?,
                        mime_type: row.get(2)?,
                        relative_path: PathBuf::from(row.get::<_, String>(3)?),
                        scan_status: parse_status(&status)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn readable_path(&self, record: &ArtifactRecord) -> Result<Option<PathBuf>> {
        if record.scan_status != ArtifactScanStatus::Clean {
            return Ok(None);
        }
        if self.retention_tombstone(&record.artifact_id)?.is_some() {
            return Ok(None);
        }
        let path = self.root.join(&record.relative_path);
        if !path.exists() {
            return Ok(None);
        }
        let canonical = path.canonicalize()?;
        anyhow::ensure!(
            canonical.starts_with(&self.root),
            "artifact path escaped root"
        );
        Ok(Some(canonical))
    }

    pub fn set_scan_status(&self, artifact_id: &str, next: ArtifactScanStatus) -> Result<bool> {
        anyhow::ensure!(
            matches!(
                next,
                ArtifactScanStatus::Clean
                    | ArtifactScanStatus::Quarantined
                    | ArtifactScanStatus::Rejected
            ),
            "invalid scan transition"
        );
        Ok(self.db.lock().unwrap_or_else(|e| e.into_inner()).execute(
            "UPDATE external_artifacts SET scan_status=?1
             WHERE artifact_id=?2 AND scan_status='unscanned'",
            params![next.as_str(), artifact_id],
        )? == 1)
    }

    /// Delete retained content while preserving its metadata row as a durable
    /// tombstone. Reads fail closed as soon as the transaction commits, even if
    /// filesystem cleanup subsequently reports an error.
    pub fn expire_retention(
        &self,
        artifact_id: &str,
        expired_at_unix_ms: i64,
        reason: &str,
    ) -> Result<Option<ArtifactTombstone>> {
        anyhow::ensure!(expired_at_unix_ms >= 0, "invalid retention expiry time");
        anyhow::ensure!(
            !reason.trim().is_empty() && reason.len() <= 1_024,
            "invalid retention reason"
        );
        let record = match self.get(artifact_id)? {
            Some(record) => record,
            None => return Ok(None),
        };
        {
            let mut db = self.db.lock().unwrap_or_else(|error| error.into_inner());
            let tx = db.transaction()?;
            let changed = tx.execute(
                "UPDATE external_artifacts
                 SET retention_expired_at_ms = ?2, retention_reason = ?3
                 WHERE artifact_id = ?1 AND retention_expired_at_ms IS NULL",
                params![artifact_id, expired_at_unix_ms, reason],
            )?;
            if changed == 0 {
                let existing: (i64, String) = tx.query_row(
                    "SELECT retention_expired_at_ms, retention_reason
                     FROM external_artifacts WHERE artifact_id = ?1",
                    [artifact_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                anyhow::ensure!(
                    existing == (expired_at_unix_ms, reason.to_owned()),
                    "conflicting immutable artifact retention tombstone"
                );
            }
            tx.commit()?;
        }
        let path = self.root.join(&record.relative_path);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("delete retention-expired artifact content"),
        }
        self.retention_tombstone(artifact_id)
    }

    pub fn retention_tombstone(&self, artifact_id: &str) -> Result<Option<ArtifactTombstone>> {
        self.db
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .query_row(
                "SELECT sha256, size_bytes, mime_type, provider, source_timestamp_ms,
                        retention_expired_at_ms, retention_reason
                 FROM external_artifacts
                 WHERE artifact_id = ?1 AND retention_expired_at_ms IS NOT NULL",
                [artifact_id],
                |row| {
                    Ok(ArtifactTombstone {
                        artifact_id: artifact_id.to_owned(),
                        sha256: row.get(0)?,
                        size_bytes: row.get(1)?,
                        mime_type: row.get(2)?,
                        producer: row.get(3)?,
                        source_timestamp_ms: row.get(4)?,
                        expired_at_unix_ms: row.get(5)?,
                        reason: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Complete filesystem cleanup after a crash that happened after the
    /// durable tombstone commit but before content removal.
    fn purge_expired_content(&self) -> Result<()> {
        let paths = {
            let db = self.db.lock().unwrap_or_else(|error| error.into_inner());
            let mut statement = db.prepare(
                "SELECT relative_path FROM external_artifacts
                 WHERE retention_expired_at_ms IS NOT NULL",
            )?;
            let paths = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            paths
        };
        for relative in paths {
            let relative = PathBuf::from(relative);
            anyhow::ensure!(
                relative
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))),
                "artifact retention path is not a safe relative path"
            );
            match std::fs::remove_file(self.root.join(relative)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("purge retention-expired artifact"),
            }
        }
        Ok(())
    }

    /// Resolve current artifact availability into an episode-report projection.
    /// This never mutates the immutable settled report; readers may replace the
    /// original manifest with this bounded projection to state explicitly that
    /// evidence has passed retention.
    pub fn episode_manifest(
        &self,
        artifact_id: &str,
        kind: impl Into<String>,
    ) -> Result<Option<EpisodeArtifactManifest>> {
        let row = self
            .db
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .query_row(
                "SELECT sha256, size_bytes, mime_type, provider, source_timestamp_ms,
                        retention_expired_at_ms, retention_reason
                 FROM external_artifacts WHERE artifact_id = ?1",
                [artifact_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(
            |(sha256, size_bytes, media_type, producer, source_time, expired_at, reason)| {
                let availability = match expired_at {
                    Some(expired_at_unix_ms) => ArtifactAvailability::RetentionExpired {
                        expired_at_unix_ms,
                        reason: reason.unwrap_or_else(|| "retention policy expired".into()),
                    },
                    None => ArtifactAvailability::Available,
                };
                EpisodeArtifactManifest {
                    kind: kind.into(),
                    uri: format!("artifact://sha256/{sha256}"),
                    digest: Some(sha256),
                    media_type: Some(media_type),
                    size_bytes: Some(size_bytes),
                    producer: Some(producer),
                    time_range: Some(ArtifactTimeRange {
                        start_unix_ms: source_time,
                        end_unix_ms: source_time,
                    }),
                    availability,
                }
            },
        ))
    }
}

pub struct ArtifactWriter {
    temp: PathBuf,
    file: Option<File>,
    hasher: Sha256,
    size: u64,
    max_bytes: u64,
    metadata: ArtifactMetadata,
    reservation: Option<StorageReservation>,
}

impl ArtifactWriter {
    pub fn write_chunk(&mut self, chunk: &[u8]) -> Result<()> {
        let next = self.size.saturating_add(chunk.len() as u64);
        anyhow::ensure!(next <= self.max_bytes, "artifact exceeds byte cap");
        self.file
            .as_mut()
            .context("artifact writer closed")?
            .write_all(chunk)?;
        self.hasher.update(chunk);
        self.size = next;
        Ok(())
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

impl Drop for ArtifactWriter {
    fn drop(&mut self) {
        self.file.take();
        let _ = std::fs::remove_file(&self.temp);
    }
}

fn validate_metadata(metadata: &ArtifactMetadata) -> Result<()> {
    anyhow::ensure!(
        !metadata.mime_type.is_empty() && metadata.mime_type.len() <= 256,
        "invalid MIME"
    );
    for value in [
        &metadata.provider,
        &metadata.account_id,
        &metadata.provider_message_id,
        &metadata.provider_part_id,
    ] {
        anyhow::ensure!(
            !value.is_empty() && value.len() <= 1_024,
            "invalid artifact provenance"
        );
    }
    anyhow::ensure!(
        metadata.source_timestamp_ms >= 0 && metadata.created_at_ms >= 0,
        "invalid artifact time"
    );
    Ok(())
}

fn parse_status(value: &str) -> rusqlite::Result<ArtifactScanStatus> {
    match value {
        "unscanned" => Ok(ArtifactScanStatus::Unscanned),
        "clean" => Ok(ArtifactScanStatus::Clean),
        "quarantined" => Ok(ArtifactScanStatus::Quarantined),
        "rejected" => Ok(ArtifactScanStatus::Rejected),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_deletes_content_but_preserves_manifest_tombstone() {
        let temp = tempfile::tempdir().unwrap();
        let store =
            ArtifactStore::open(&temp.path().join("store.db"), &temp.path().join("blobs")).unwrap();
        let mut writer = store
            .begin(
                ArtifactMetadata {
                    mime_type: "application/x-rosbag".into(),
                    provider: "kuavo-bridge".into(),
                    account_id: "robot".into(),
                    provider_message_id: "episode-1".into(),
                    provider_part_id: "rosbag".into(),
                    source_timestamp_ms: 1_000,
                    scan_status: ArtifactScanStatus::Unscanned,
                    created_at_ms: 1_100,
                },
                1_024,
            )
            .unwrap();
        writer.write_chunk(b"bounded rosbag fixture").unwrap();
        let record = store.finish(writer).unwrap();
        assert!(store
            .set_scan_status(&record.artifact_id, ArtifactScanStatus::Clean)
            .unwrap());
        let record = store.get(&record.artifact_id).unwrap().unwrap();
        assert!(store.readable_path(&record).unwrap().is_some());
        let before = store
            .episode_manifest(&record.artifact_id, "rosbag")
            .unwrap()
            .unwrap();
        assert_eq!(before.availability, ArtifactAvailability::Available);
        before.validate().unwrap();

        let tombstone = store
            .expire_retention(&record.artifact_id, 2_000, "episode retention elapsed")
            .unwrap()
            .unwrap();
        assert_eq!(tombstone.sha256, record.sha256);
        assert_eq!(tombstone.size_bytes, record.size_bytes);
        assert!(store.readable_path(&record).unwrap().is_none());
        assert_eq!(
            store
                .expire_retention(&record.artifact_id, 2_000, "episode retention elapsed")
                .unwrap()
                .unwrap(),
            tombstone
        );
        assert!(store
            .expire_retention(&record.artifact_id, 2_001, "different reason")
            .is_err());
        assert!(store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE external_artifacts SET retention_reason = 'tampered'
                 WHERE artifact_id = ?1",
                [&record.artifact_id],
            )
            .is_err());

        let after = store
            .episode_manifest(&record.artifact_id, "rosbag")
            .unwrap()
            .unwrap();
        assert_eq!(after.digest.as_deref(), Some(record.sha256.as_str()));
        assert!(matches!(
            after.availability,
            ArtifactAvailability::RetentionExpired {
                expired_at_unix_ms: 2_000,
                ..
            }
        ));
        after.validate().unwrap();

        // Simulate a crash after the tombstone commit but before filesystem
        // deletion. Reopen reconciles the leftover bytes from durable metadata.
        let orphan = temp.path().join("blobs").join(&record.relative_path);
        std::fs::write(&orphan, b"orphaned after tombstone").unwrap();
        assert!(orphan.exists());
        drop(store);
        let reopened =
            ArtifactStore::open(&temp.path().join("store.db"), &temp.path().join("blobs")).unwrap();
        assert!(!orphan.exists());
        let durable = reopened
            .retention_tombstone(&record.artifact_id)
            .unwrap()
            .unwrap();
        assert_eq!(durable.sha256, record.sha256);
        assert_eq!(durable.episode_tombstone().digest, record.sha256);

        let mut replay = reopened
            .begin(
                ArtifactMetadata {
                    mime_type: "application/x-rosbag".into(),
                    provider: "kuavo-bridge".into(),
                    account_id: "robot".into(),
                    provider_message_id: "episode-2".into(),
                    provider_part_id: "rosbag".into(),
                    source_timestamp_ms: 3_000,
                    scan_status: ArtifactScanStatus::Unscanned,
                    created_at_ms: 3_100,
                },
                1_024,
            )
            .unwrap();
        replay.write_chunk(b"bounded rosbag fixture").unwrap();
        assert!(reopened.finish(replay).is_err());
        assert!(!orphan.exists());
    }
}
