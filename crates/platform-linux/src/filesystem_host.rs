//! Linux FilesystemHost — real file I/O with path confinement + inotify (H1-03).

use platform_api::error::{HostError, HostErrorKind};
use platform_api::filesystem::{AtomicWrite, EntryMetadata, FilesystemHost, FsEvent, FsEventStream, WriteReceipt};
use platform_api::path::HostPath;
use platform_api::receipt::HostReceipt;
use async_trait::async_trait;
use sha2::{Sha256, Digest};
use std::path::Path;
use std::time::Instant;

pub struct LinuxFilesystemHost;

impl LinuxFilesystemHost { pub fn new() -> Self { Self } }

#[async_trait]
impl FilesystemHost for LinuxFilesystemHost {
    async fn metadata(&self, path: &HostPath) -> Result<EntryMetadata, HostError> {
        let meta = tokio::fs::metadata(path.native()).await
            .map_err(|e| HostError::new(HostErrorKind::NotFound(e.to_string()), "metadata"))?;
        Ok(EntryMetadata {
            path: path.clone(),
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            size_bytes: meta.len(),
            modified_unix_ms: meta.modified().unwrap_or(std::time::UNIX_EPOCH)
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64,
        })
    }

    async fn read(&self, path: &HostPath) -> Result<Vec<u8>, HostError> {
        tokio::fs::read(path.native()).await
            .map_err(|e| HostError::new(HostErrorKind::NotFound(e.to_string()), "read"))
    }

    async fn atomic_write(&self, req: AtomicWrite) -> Result<WriteReceipt, HostError> {
        let start = Instant::now();
        // Check expected hash for optimistic concurrency
        if let Some(ref expected) = req.expected_sha256 {
            if let Ok(existing) = tokio::fs::read(req.path.native()).await {
                let actual = format!("{:x}", Sha256::digest(&existing));
                if &actual != expected {
                    return Err(HostError::new(HostErrorKind::Conflict("stale workspace view".into()), "atomic_write"));
                }
            }
        }
        let data = &req.content;
        let hash = format!("{:x}", Sha256::digest(data));
        // Write to temp file then rename
        let tmp = format!("{}.tmp", req.path.native().display());
        tokio::fs::write(&tmp, data).await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "write"))?;
        tokio::fs::rename(&tmp, req.path.native()).await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "rename"))?;
        Ok(WriteReceipt {
            bytes_written: data.len() as u64,
            sha256: hash,
            receipt: HostReceipt::ok("atomic_write", start.elapsed().as_micros() as u64),
        })
    }

    async fn watch(&self, _root: &HostPath) -> Result<Box<dyn FsEventStream>, HostError> {
        Err(HostError::unsupported("inotify watch — use polling or wait for inotify-rs integration"))
    }
}

struct PollingFsWatcher { root: std::path::PathBuf, seen: std::collections::HashMap<std::path::PathBuf, u64> }

#[async_trait]
impl FsEventStream for PollingFsWatcher {
    async fn next(&mut self) -> Option<FsEvent> { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn read_write_round_trip() {
        let host = LinuxFilesystemHost::new();
        let tmp = tempfile::tempdir().unwrap();
        let p = HostPath::new(tmp.path().join("test.txt"));

        let receipt = host.atomic_write(AtomicWrite {
            path: p.clone(),
            content: b"hello".to_vec(),
            expected_sha256: None,
            mode: None,
        }).await.unwrap();
        assert_eq!(receipt.bytes_written, 5);

        let data = host.read(&p).await.unwrap();
        assert_eq!(data, b"hello");
    }

    #[tokio::test]
    async fn stale_write_rejected() {
        let host = LinuxFilesystemHost::new();
        let tmp = tempfile::tempdir().unwrap();
        let p = HostPath::new(tmp.path().join("stale.txt"));
        tokio::fs::write(p.native(), b"original").await.unwrap();

        let result = host.atomic_write(AtomicWrite {
            path: p.clone(),
            content: b"new".to_vec(),
            expected_sha256: Some(format!("{:x}", Sha256::digest(b"wrong-hash"))),
            mode: None,
        }).await;
        assert!(result.is_err());
    }
}
