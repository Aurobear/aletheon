//! FilesystemHost — scoped read/write/watch with confinement.

use crate::error::HostError;
use crate::path::HostPath;
use crate::receipt::HostReceipt;
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymlinkPolicy {
    Deny,
    WithinRoot,
}

/// Operation-scoped filesystem authority projected by Executive from the
/// admitted workspace and capability permit.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FilesystemScope {
    /// Directory roots that may be traversed within this operation.
    pub roots: Vec<HostPath>,
    /// Individually admitted read-only files outside `roots`.
    #[serde(default)]
    pub readable_paths: Vec<HostPath>,
    pub access: FilesystemAccess,
    pub symlink_policy: SymlinkPolicy,
}

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
pub struct RemoveFile {
    pub path: HostPath,
    pub expected_sha256: Option<String>,
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
    async fn create_dir_all(&self, path: &HostPath) -> Result<HostReceipt, HostError>;
    async fn atomic_write(&self, request: AtomicWrite) -> Result<WriteReceipt, HostError>;
    async fn remove_file(&self, request: RemoveFile) -> Result<HostReceipt, HostError>;
    async fn watch(&self, root: &HostPath) -> Result<Box<dyn FsEventStream>, HostError>;
}

/// Stream of filesystem events; backends implement this.
#[async_trait]
pub trait FsEventStream: Send + Sync {
    async fn next(&mut self) -> Option<FsEvent>;
}
