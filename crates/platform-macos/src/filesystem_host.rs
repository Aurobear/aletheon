//! macOS FilesystemHost — FSEvents, path confinement (H3).

use platform_api::error::HostError;
use platform_api::filesystem::{AtomicWrite, EntryMetadata, FilesystemHost, FsEventStream, WriteReceipt};
use platform_api::path::HostPath;
use async_trait::async_trait;

pub struct MacOSFilesystemHost;
impl MacOSFilesystemHost { pub fn new() -> Self { Self } }

#[async_trait]
impl FilesystemHost for MacOSFilesystemHost {
    async fn metadata(&self, _p: &HostPath) -> Result<EntryMetadata, HostError> { Err(HostError::unsupported("fs")) }
    async fn read(&self, _p: &HostPath) -> Result<Vec<u8>, HostError> { Err(HostError::unsupported("fs")) }
    async fn atomic_write(&self, _r: AtomicWrite) -> Result<WriteReceipt, HostError> { Err(HostError::unsupported("fs")) }
    async fn watch(&self, _root: &HostPath) -> Result<Box<dyn FsEventStream>, HostError> { Err(HostError::unsupported("fs")) }
}
#[cfg(test)] mod tests { use super::*; #[tokio::test] async fn contract() { assert!(MacOSFilesystemHost::new().read(&HostPath::new(std::path::PathBuf::from("/tmp/t"))).await.is_err()); } }
