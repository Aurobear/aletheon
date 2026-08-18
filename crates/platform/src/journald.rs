//! Linux journal transport owned by the host platform layer.

use crate::{HostError, HostErrorKind};

#[cfg(target_os = "linux")]
pub struct JournalLineStream {
    lines: tokio::sync::mpsc::Receiver<String>,
}

#[cfg(not(target_os = "linux"))]
pub struct JournalLineStream;

impl JournalLineStream {
    #[cfg(target_os = "linux")]
    pub async fn start() -> Result<Self, HostError> {
        use std::process::Stdio;
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = tokio::process::Command::new("journalctl")
            .args(["-f", "-o", "json", "--no-pager"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                HostError::new(HostErrorKind::Io(error.to_string()), "start journal stream")
            })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            HostError::new(
                HostErrorKind::Io("journalctl stdout pipe is unavailable".into()),
                "start journal stream",
            )
        })?;
        let (tx, lines) = tokio::sync::mpsc::channel(256);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).await.is_err() {
                    break;
                }
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
        });
        Ok(Self { lines })
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn start() -> Result<Self, HostError> {
        Err(HostError::unsupported("journald stream"))
    }

    #[cfg(target_os = "linux")]
    pub fn try_recv(&mut self) -> Option<String> {
        self.lines.try_recv().ok()
    }

    #[cfg(not(target_os = "linux"))]
    pub fn try_recv(&mut self) -> Option<String> {
        None
    }

    #[cfg(target_os = "linux")]
    pub fn is_available() -> bool {
        std::process::Command::new("journalctl")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }

    #[cfg(not(target_os = "linux"))]
    pub fn is_available() -> bool {
        false
    }
}
