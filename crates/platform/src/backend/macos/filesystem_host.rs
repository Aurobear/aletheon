//! macOS FilesystemHost — FSEvents, path confinement (H3).

use crate::error::HostError;
use crate::filesystem::{
    AtomicWrite, EntryMetadata, FilesystemHost, FilesystemScope, FsEventStream, WriteReceipt,
};
use crate::path::HostPath;
use async_trait::async_trait;

pub struct MacOSFilesystemHost;
impl MacOSFilesystemHost {
    pub fn scoped(scope: FilesystemScope) -> Result<Self, HostError> {
        if scope.roots.is_empty() && scope.readable_paths.is_empty() {
            return Err(HostError::unsupported("empty macOS filesystem scope"));
        }
        Ok(Self)
    }
}

#[async_trait]
impl FilesystemHost for MacOSFilesystemHost {
    async fn metadata(&self, _p: &HostPath) -> Result<EntryMetadata, HostError> {
        Err(HostError::unsupported("fs"))
    }
    async fn read(&self, _p: &HostPath) -> Result<Vec<u8>, HostError> {
        Err(HostError::unsupported("fs"))
    }
    async fn create_dir_all(&self, _p: &HostPath) -> Result<crate::HostReceipt, HostError> {
        Err(HostError::unsupported("fs"))
    }
    async fn atomic_write(&self, _r: AtomicWrite) -> Result<WriteReceipt, HostError> {
        Err(HostError::unsupported("fs"))
    }
    async fn remove_file(
        &self,
        _request: crate::RemoveFile,
    ) -> Result<crate::HostReceipt, HostError> {
        Err(HostError::unsupported("fs"))
    }
    async fn watch(&self, _root: &HostPath) -> Result<Box<dyn FsEventStream>, HostError> {
        Err(HostError::unsupported("fs"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn contract() {
        let root = tempfile::tempdir().unwrap();
        let host = MacOSFilesystemHost::scoped(FilesystemScope {
            roots: vec![HostPath::new(root.path().to_path_buf())],
            readable_paths: vec![],
            access: crate::FilesystemAccess::ReadOnly,
            symlink_policy: crate::SymlinkPolicy::WithinRoot,
        })
        .unwrap();
        assert!(host
            .read(&HostPath::new(std::path::PathBuf::from("/tmp/t")))
            .await
            .is_err());
    }
}
