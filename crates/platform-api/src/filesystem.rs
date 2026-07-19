//! FilesystemHost — scoped read/write/watch with confinement.

use crate::error::HostError;
use crate::path::HostPath;
use crate::receipt::HostReceipt;
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct EntryMetadata {
    pub path: HostPath,
    pub is_file: bool,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_unix_ms: i64,
}

#[derive(Clone, Debug)]
pub struct AtomicWrite {
    pub path: HostPath,
    pub content: Vec<u8>,
    pub expected_sha256: Option<String>,
    pub mode: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct WriteReceipt {
    pub bytes_written: u64,
    pub sha256: String,
    pub receipt: HostReceipt,
}

#[derive(Clone, Debug)]
pub enum FsEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

#[async_trait]
pub trait FilesystemHost: Send + Sync {
    async fn metadata(&self, path: &HostPath) -> Result<EntryMetadata, HostError>;
    async fn read(&self, path: &HostPath) -> Result<Vec<u8>, HostError>;
    async fn atomic_write(&self, request: AtomicWrite) -> Result<WriteReceipt, HostError>;
    async fn watch(
        &self,
        root: &HostPath,
    ) -> Result<Box<dyn FsEventStream>, HostError>;
}

/// Stream of filesystem events; backends implement this.
#[async_trait]
pub trait FsEventStream: Send + Sync {
    async fn next(&mut self) -> Option<FsEvent>;
}
