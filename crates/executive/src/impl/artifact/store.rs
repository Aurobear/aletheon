//! Atomic streaming writes for bounded content-addressed artifacts.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::r#impl::goal::migrations;

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

pub struct ArtifactStore {
    db: Mutex<Connection>,
    root: PathBuf,
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
        Ok(Self {
            db: Mutex::new(db),
            root,
        })
    }

    pub fn begin(&self, metadata: ArtifactMetadata, max_bytes: u64) -> Result<ArtifactWriter> {
        validate_metadata(&metadata)?;
        anyhow::ensure!(
            (1..=64 * 1_048_576).contains(&max_bytes),
            "invalid artifact cap"
        );
        let temp = self.root.join(format!(".upload-{}", uuid::Uuid::new_v4()));
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
        if final_path.exists() {
            std::fs::remove_file(&writer.temp)?;
        } else {
            std::fs::rename(&writer.temp, &final_path)?;
        }
        let db = self.db.lock().unwrap();
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
        self.get(&artifact_id)?
            .context("artifact metadata write failed")
    }

    pub fn get(&self, artifact_id: &str) -> Result<Option<ArtifactRecord>> {
        self.db
            .lock()
            .unwrap()
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
        let path = self.root.join(&record.relative_path);
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
        Ok(self.db.lock().unwrap().execute(
            "UPDATE external_artifacts SET scan_status=?1
             WHERE artifact_id=?2 AND scan_status='unscanned'",
            params![next.as_str(), artifact_id],
        )? == 1)
    }
}

pub struct ArtifactWriter {
    temp: PathBuf,
    file: Option<File>,
    hasher: Sha256,
    size: u64,
    max_bytes: u64,
    metadata: ArtifactMetadata,
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
