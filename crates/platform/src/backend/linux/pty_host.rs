//! Linux PtyHost — Unix98 PTY via /dev/ptmx (H1-04).

use crate::error::{HostError, HostErrorKind};
use crate::pty::{PtyChannel, PtyHost, PtySize};
use crate::receipt::HostReceipt;
use async_trait::async_trait;
use std::os::unix::io::AsRawFd;
use std::time::Instant;

pub struct LinuxPtyHost;

impl LinuxPtyHost {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PtyHost for LinuxPtyHost {
    async fn open(&self, size: PtySize) -> Result<Box<dyn PtyChannel>, HostError> {
        let start = Instant::now();
        let master = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/ptmx")
            .map_err(|e| {
                HostError::new(HostErrorKind::Unsupported(format!("ptmx: {e}")), "pty open")
            })?;
        unsafe {
            libc::grantpt(master.as_raw_fd());
            libc::unlockpt(master.as_raw_fd());
        }
        let pts_name = unsafe {
            let name = libc::ptsname(master.as_raw_fd());
            if name.is_null() {
                return Err(HostError::unsupported("ptsname"));
            }
            std::ffi::CStr::from_ptr(name)
                .to_string_lossy()
                .into_owned()
        };
        let _pts = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pts_name)
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "pts open"))?;
        // Set window size
        let ws = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &ws);
        }
        Ok(Box::new(LinuxPtyChannel {
            master: tokio::fs::File::from_std(master),
            _receipt: HostReceipt::ok("pty_open", start.elapsed().as_micros() as u64),
        }))
    }
}

struct LinuxPtyChannel {
    master: tokio::fs::File,
    _receipt: HostReceipt,
}

#[async_trait]
impl PtyChannel for LinuxPtyChannel {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, HostError> {
        tokio::io::AsyncReadExt::read(&mut self.master, buf)
            .await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "pty read"))
    }
    async fn write(&mut self, data: &[u8]) -> Result<usize, HostError> {
        tokio::io::AsyncWriteExt::write(&mut self.master, data)
            .await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "pty write"))
    }
    async fn resize(&mut self, size: PtySize) -> Result<HostReceipt, HostError> {
        let start = Instant::now();
        let ws = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let result = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
        if result != 0 {
            return Err(HostError::new(
                HostErrorKind::Io(std::io::Error::last_os_error().to_string()),
                "pty resize",
            ));
        }
        Ok(HostReceipt::ok(
            "pty_resize",
            start.elapsed().as_micros() as u64,
        ))
    }
    async fn close(self: Box<Self>) -> Result<HostReceipt, HostError> {
        Ok(HostReceipt::ok("pty_close", 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pty_open_works_on_linux() {
        let host = LinuxPtyHost::new();
        let mut channel = host.open(PtySize { rows: 24, cols: 80 }).await.unwrap();
        let receipt = channel
            .resize(PtySize {
                rows: 40,
                cols: 120,
            })
            .await
            .unwrap();
        assert!(receipt.success);
        channel.close().await.unwrap();
    }
}
