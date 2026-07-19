//! Windows FilesystemHost — path confinement, ReadDirectoryChangesW (H2).

use platform_api::error::HostError;
use platform_api::filesystem::{AtomicWrite, EntryMetadata, FilesystemHost, FsEventStream, WriteReceipt};
use platform_api::path::HostPath;
use async_trait::async_trait;

pub struct WindowsFilesystemHost;
impl WindowsFilesystemHost { pub fn new() -> Self { Self } }

#[async_trait]
impl FilesystemHost for WindowsFilesystemHost {
    async fn metadata(&self, _p: &HostPath) -> Result<EntryMetadata, HostError> { Err(HostError::unsupported("fs metadata")) }
    async fn read(&self, _p: &HostPath) -> Result<Vec<u8>, HostError> { Err(HostError::unsupported("fs read")) }
    async fn atomic_write(&self, _req: AtomicWrite) -> Result<WriteReceipt, HostError> { Err(HostError::unsupported("fs atomic_write")) }
    async fn watch(&self, _root: &HostPath) -> Result<Box<dyn FsEventStream>, HostError> { Err(HostError::unsupported("fs watch")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test] async fn contract_unimplemented() { assert!(WindowsFilesystemHost::new().read(&HostPath::new(std::path::PathBuf::from("C:\\test"))).await.is_err()); }
}
