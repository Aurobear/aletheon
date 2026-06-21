use std::sync::atomic::{AtomicU64, Ordering};
use tracing::info;

use crate::ipc::ipc_types::{AgentMessage, IpcBackend, IpcProbeError};

/// Shared memory region for zero-copy IPC.
pub struct SharedMemRegion {
    fd: i32,
    size: usize,
    base: *mut u8,
    /// Write position (producer updates).
    write_pos: AtomicU64,
    /// Read position (consumer updates).
    read_pos: AtomicU64,
}

// SAFETY: SharedMemRegion is used with proper synchronization
unsafe impl Send for SharedMemRegion {}
unsafe impl Sync for SharedMemRegion {}

impl SharedMemRegion {
    /// Create a new shared memory region using memfd.
    pub fn create(name: &str, size: usize) -> Result<Self, anyhow::Error> {
        // Use memfd_create to create an anonymous file in memory
        let fd = unsafe {
            libc::memfd_create(std::ffi::CString::new(name)?.as_ptr(), libc::MFD_CLOEXEC)
        };

        if fd < 0 {
            return Err(anyhow::anyhow!(
                "memfd_create failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        // Set size
        let ret = unsafe { libc::ftruncate(fd, size as i64) };
        if ret < 0 {
            unsafe {
                libc::close(fd);
            }
            return Err(anyhow::anyhow!(
                "ftruncate failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        // Map into memory
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };

        if base == libc::MAP_FAILED {
            unsafe {
                libc::close(fd);
            }
            return Err(anyhow::anyhow!(
                "mmap failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        info!(
            name = name,
            size = size,
            fd = fd,
            "Shared memory region created"
        );

        Ok(Self {
            fd,
            size,
            base: base as *mut u8,
            write_pos: AtomicU64::new(0),
            read_pos: AtomicU64::new(0),
        })
    }

    /// Write a message to the ring buffer.
    pub fn write(&self, msg: &AgentMessage) -> Result<(), anyhow::Error> {
        let bytes = msg.to_bytes();
        let len = bytes.len() as u64;
        let total_len = 8 + len; // 8 bytes for length prefix

        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);

        // Check if there's enough space
        let available = if write_pos >= read_pos {
            self.size as u64 - (write_pos - read_pos)
        } else {
            read_pos - write_pos
        };

        if total_len > available {
            return Err(anyhow::anyhow!("Shared memory buffer full"));
        }

        // Write length prefix
        let len_bytes = (len as u64).to_le_bytes();
        let offset = (write_pos % self.size as u64) as usize;

        unsafe {
            let dst = self.base.add(offset);
            std::ptr::copy_nonoverlapping(len_bytes.as_ptr(), dst, 8);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.add(8), bytes.len());
        }

        self.write_pos
            .store(write_pos + total_len, Ordering::Release);
        Ok(())
    }

    /// Read a message from the ring buffer.
    pub fn read(&self) -> Option<AgentMessage> {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);

        if read_pos >= write_pos {
            return None; // Empty
        }

        let offset = (read_pos % self.size as u64) as usize;

        // Read length prefix
        let mut len_bytes = [0u8; 8];
        unsafe {
            let src = self.base.add(offset);
            std::ptr::copy_nonoverlapping(src, len_bytes.as_mut_ptr(), 8);
        }
        let len = u64::from_le_bytes(len_bytes) as usize;

        // Read payload
        let mut payload = vec![0u8; len];
        unsafe {
            let src = self.base.add(offset + 8);
            std::ptr::copy_nonoverlapping(src, payload.as_mut_ptr(), len);
        }

        self.read_pos
            .store(read_pos + 8 + len as u64, Ordering::Release);

        AgentMessage::from_bytes(&payload)
    }

    /// Current available space.
    pub fn available(&self) -> usize {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        if write_pos >= read_pos {
            self.size - (write_pos - read_pos) as usize
        } else {
            (read_pos - write_pos) as usize
        }
    }
}

impl Drop for SharedMemRegion {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base as *mut libc::c_void, self.size);
            libc::close(self.fd);
        }
    }
}

/// Shared memory based IPC backend (implements IpcBackend trait).
pub struct SharedMemBackend {
    region: tokio::sync::RwLock<Option<SharedMemRegion>>,
}

impl SharedMemBackend {
    pub fn new() -> Self {
        Self {
            region: tokio::sync::RwLock::new(None),
        }
    }

    /// Check if shared memory IPC is available.
    pub fn probe() -> bool {
        cfg!(target_os = "linux")
    }
}

#[async_trait::async_trait]
impl IpcBackend for SharedMemBackend {
    async fn init(&mut self) -> Result<(), IpcProbeError> {
        let region = SharedMemRegion::create("aletheon-shm", 1024 * 1024)
            .map_err(|_| IpcProbeError::NotSupported)?;
        *self.region.write().await = Some(region);
        Ok(())
    }

    async fn send(&self, message: &AgentMessage) -> Result<(), IpcProbeError> {
        let guard = self.region.read().await;
        let region = guard.as_ref().ok_or(IpcProbeError::NotSupported)?;
        region
            .write(message)
            .map_err(|e| IpcProbeError::Other(e.to_string()))
    }

    async fn recv(&self) -> Result<AgentMessage, IpcProbeError> {
        let guard = self.region.read().await;
        let region = guard.as_ref().ok_or(IpcProbeError::NotSupported)?;
        region
            .read()
            .ok_or_else(|| IpcProbeError::Other("no data available".into()))
    }

    async fn try_recv(&self) -> Option<AgentMessage> {
        let guard = self.region.read().await;
        let region = guard.as_ref()?;
        region.read()
    }

    fn is_available(&self) -> bool {
        Self::probe()
    }

    fn name(&self) -> &str {
        "shared_memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ipc_types::IpcPriority;

    #[test]
    fn test_shared_mem_create() {
        let region = SharedMemRegion::create("test", 4096).unwrap();
        assert_eq!(region.size, 4096);
        assert!(region.available() > 0);
    }

    #[test]
    fn test_shared_mem_write_read() {
        let region = SharedMemRegion::create("test_rw", 4096).unwrap();
        let msg = AgentMessage::event(1, 2, IpcPriority::Urgent, b"hello");

        region.write(&msg).unwrap();
        let read_msg = region.read().unwrap();

        assert_eq!(read_msg.sender_id, 1);
        assert_eq!(read_msg.target_id, 2);
        assert_eq!(read_msg.payload, b"hello");
    }

    #[test]
    fn test_shared_mem_empty_read() {
        let region = SharedMemRegion::create("test_empty", 4096).unwrap();
        assert!(region.read().is_none());
    }
}
