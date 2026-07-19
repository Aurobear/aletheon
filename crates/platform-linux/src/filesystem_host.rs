//! Linux FilesystemHost — root confinement, inotify (H1-03).

use platform_api::error::HostError;
use platform_api::filesystem::{AtomicWrite, EntryMetadata, FilesystemHost, FsEvent, FsEventStream, WriteReceipt};
use platform_api::path::HostPath;
use async_trait::async_trait;

pub struct LinuxFilesystemHost;

impl LinuxFilesystemHost {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl FilesystemHost for LinuxFilesystemHost {
    async fn metadata(&self, _path: &HostPath) -> Result<EntryMetadata, HostError> {
        Err(HostError::unsupported("fs metadata"))
    }
    async fn read(&self, _path: &HostPath) -> Result<Vec<u8>, HostError> {
        Err(HostError::unsupported("fs read"))
    }
    async fn atomic_write(&self, _request: AtomicWrite) -> Result<WriteReceipt, HostError> {
        Err(HostError::unsupported("fs atomic_write"))
    }
    async fn watch(&self, _root: &HostPath) -> Result<Box<dyn FsEventStream>, HostError> {
        Err(HostError::unsupported("fs watch"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn filesystem_host_contract_unimplemented() {
        let host = LinuxFilesystemHost::new();
        let result = host.read(&HostPath::new(std::path::PathBuf::from("/tmp/test"))).await;
        assert!(result.is_err());
    }
}
