//! Windows FilesystemHost — path confinement, ReadDirectoryChangesW (H2).

use crate::error::HostError;
use crate::filesystem::{
    AtomicWrite, EntryMetadata, FilesystemHost, FilesystemScope, FsEventStream, WriteReceipt,
};
use crate::path::HostPath;
use async_trait::async_trait;

pub struct WindowsFilesystemHost;
impl WindowsFilesystemHost {
    pub fn scoped(scope: FilesystemScope) -> Result<Self, HostError> {
        if scope.roots.is_empty() && scope.readable_paths.is_empty() {
            return Err(HostError::unsupported("empty Windows filesystem scope"));
        }
        Ok(Self)
    }
}

#[async_trait]
impl FilesystemHost for WindowsFilesystemHost {
    async fn metadata(&self, _p: &HostPath) -> Result<EntryMetadata, HostError> {
        Err(HostError::unsupported("fs metadata"))
    }
    async fn read(&self, _p: &HostPath) -> Result<Vec<u8>, HostError> {
        Err(HostError::unsupported("fs read"))
    }
    async fn create_dir_all(&self, _p: &HostPath) -> Result<crate::HostReceipt, HostError> {
        Err(HostError::unsupported("fs create_dir_all"))
    }
    async fn atomic_write(&self, _req: AtomicWrite) -> Result<WriteReceipt, HostError> {
        Err(HostError::unsupported("fs atomic_write"))
    }
    async fn remove_file(
        &self,
        _request: crate::RemoveFile,
    ) -> Result<crate::HostReceipt, HostError> {
        Err(HostError::unsupported("fs remove_file"))
    }
    async fn watch(&self, _root: &HostPath) -> Result<Box<dyn FsEventStream>, HostError> {
        Err(HostError::unsupported("fs watch"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn contract_unimplemented() {
        let root = tempfile::tempdir().unwrap();
        let host = WindowsFilesystemHost::scoped(FilesystemScope {
            roots: vec![HostPath::new(root.path().to_path_buf())],
            readable_paths: vec![],
            access: crate::FilesystemAccess::ReadOnly,
            symlink_policy: crate::SymlinkPolicy::WithinRoot,
        })
        .unwrap();
        assert!(host
            .read(&HostPath::new(std::path::PathBuf::from("C:\\test")))
            .await
            .is_err());
    }
}
