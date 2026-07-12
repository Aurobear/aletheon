//! Agent virtual filesystem mounted at /mnt/agent/
//!
//! Directory structure:
//!   /mnt/agent/
//!   ├── context/     - Agent context (current conversation, memory state)
//!   ├── controls/    - Control commands (pause, resume)
//!   ├── sensors/     - Real-time system metrics
//!   ├── logs/        - Agent and audit logs
//!   └── agents/      - Multi-agent status views

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Node types in the virtual filesystem.
#[derive(Debug, Clone)]
pub enum FsNode {
    Directory {
        children: Vec<String>,
    },
    File {
        content: Vec<u8>,
        writable: bool,
    },
    DynamicFile {
        /// Generator function name (for lazy evaluation)
        generator: String,
        writable: bool,
    },
}

/// Agent virtual filesystem.
pub struct AgentFs {
    /// FUSE mount point — reserved for future filesystem integration.
    #[allow(dead_code)]
    mount_point: std::path::PathBuf,
    nodes: Arc<RwLock<HashMap<String, FsNode>>>,
    paused: Arc<RwLock<bool>>,
    clock: Arc<dyn fabric::Clock>,
}

impl AgentFs {
    pub fn new(mount_point: std::path::PathBuf, clock: Arc<dyn fabric::Clock>) -> Self {
        let mut nodes = HashMap::new();

        // Create directory structure
        nodes.insert(
            "/".to_string(),
            FsNode::Directory {
                children: vec![
                    "context".to_string(),
                    "controls".to_string(),
                    "sensors".to_string(),
                    "logs".to_string(),
                    "agents".to_string(),
                ],
            },
        );

        nodes.insert(
            "/context".to_string(),
            FsNode::Directory {
                children: vec![
                    "current.md".to_string(),
                    "memory.md".to_string(),
                    "tools.json".to_string(),
                ],
            },
        );

        nodes.insert(
            "/context/current.md".to_string(),
            FsNode::DynamicFile {
                generator: "context_current".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/context/memory.md".to_string(),
            FsNode::DynamicFile {
                generator: "context_memory".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/context/tools.json".to_string(),
            FsNode::DynamicFile {
                generator: "context_tools".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/controls".to_string(),
            FsNode::Directory {
                children: vec![
                    "pause".to_string(),
                    "resume".to_string(),
                    "config.toml".to_string(),
                ],
            },
        );

        nodes.insert(
            "/controls/pause".to_string(),
            FsNode::File {
                content: b"0".to_vec(),
                writable: true,
            },
        );

        nodes.insert(
            "/controls/resume".to_string(),
            FsNode::File {
                content: b"0".to_vec(),
                writable: true,
            },
        );

        nodes.insert(
            "/sensors".to_string(),
            FsNode::Directory {
                children: vec![
                    "cpu.json".to_string(),
                    "memory.json".to_string(),
                    "disk.json".to_string(),
                    "network.json".to_string(),
                ],
            },
        );

        nodes.insert(
            "/sensors/cpu.json".to_string(),
            FsNode::DynamicFile {
                generator: "sensor_cpu".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/sensors/memory.json".to_string(),
            FsNode::DynamicFile {
                generator: "sensor_memory".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/sensors/disk.json".to_string(),
            FsNode::DynamicFile {
                generator: "sensor_disk".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/sensors/network.json".to_string(),
            FsNode::DynamicFile {
                generator: "sensor_network".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/logs".to_string(),
            FsNode::Directory {
                children: vec!["agent.log".to_string(), "audit.jsonl".to_string()],
            },
        );

        nodes.insert(
            "/logs/agent.log".to_string(),
            FsNode::DynamicFile {
                generator: "log_agent".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/logs/audit.jsonl".to_string(),
            FsNode::DynamicFile {
                generator: "log_audit".to_string(),
                writable: false,
            },
        );

        nodes.insert(
            "/agents".to_string(),
            FsNode::Directory {
                children: vec!["main".to_string()],
            },
        );

        nodes.insert(
            "/agents/main".to_string(),
            FsNode::Directory {
                children: vec!["status.json".to_string()],
            },
        );

        nodes.insert(
            "/agents/main/status.json".to_string(),
            FsNode::DynamicFile {
                generator: "agent_status_main".to_string(),
                writable: false,
            },
        );

        Self {
            mount_point,
            nodes: Arc::new(RwLock::new(nodes)),
            paused: Arc::new(RwLock::new(false)),
            clock,
        }
    }

    /// Read a file from the virtual filesystem.
    pub async fn read(&self, path: &str) -> Result<Vec<u8>> {
        let nodes = self.nodes.read().await;
        match nodes.get(path) {
            Some(FsNode::File { content, .. }) => Ok(content.clone()),
            Some(FsNode::DynamicFile { generator, .. }) => self.generate_content(generator).await,
            Some(FsNode::Directory { children }) => Ok(children.join("\n").into_bytes()),
            None => anyhow::bail!("File not found: {}", path),
        }
    }

    /// Write to a file in the virtual filesystem.
    pub async fn write(&self, path: &str, data: &[u8]) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        match nodes.get_mut(path) {
            Some(FsNode::File {
                content,
                writable: true,
            }) => {
                *content = data.to_vec();

                // Handle control commands
                if path == "/controls/pause" && data == b"1" {
                    *self.paused.write().await = true;
                    info!("Agent paused via FUSE");
                } else if path == "/controls/resume" && data == b"1" {
                    *self.paused.write().await = false;
                    info!("Agent resumed via FUSE");
                }

                Ok(())
            }
            Some(FsNode::File {
                writable: false, ..
            }) => {
                anyhow::bail!("File is read-only: {}", path)
            }
            Some(FsNode::DynamicFile {
                writable: false, ..
            }) => {
                anyhow::bail!("File is read-only: {}", path)
            }
            None => anyhow::bail!("File not found: {}", path),
            _ => anyhow::bail!("Cannot write to directory: {}", path),
        }
    }

    /// List directory contents.
    pub async fn readdir(&self, path: &str) -> Result<Vec<String>> {
        let nodes = self.nodes.read().await;
        match nodes.get(path) {
            Some(FsNode::Directory { children }) => Ok(children.clone()),
            _ => anyhow::bail!("Not a directory: {}", path),
        }
    }

    /// Check if agent is paused.
    pub async fn is_paused(&self) -> bool {
        *self.paused.read().await
    }

    /// Generate content for dynamic files.
    async fn generate_content(&self, generator: &str) -> Result<Vec<u8>> {
        match generator {
            "sensor_cpu" => {
                let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
                let cpu_info = serde_json::json!({
                    "load_avg": load.trim(),
                    "timestamp": fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
                });
                Ok(serde_json::to_string_pretty(&cpu_info)?.into_bytes())
            }
            "sensor_memory" => {
                let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
                let mut mem = serde_json::json!({});
                for line in meminfo.lines().take(5) {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() == 2 {
                        mem[parts[0].trim()] =
                            serde_json::Value::String(parts[1].trim().to_string());
                    }
                }
                mem["timestamp"] = serde_json::Value::String(
                    fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
                );
                Ok(serde_json::to_string_pretty(&mem)?.into_bytes())
            }
            "sensor_disk" => {
                let diskstats = std::fs::read_to_string("/proc/diskstats").unwrap_or_default();
                let mut disks = vec![];
                for line in diskstats.lines().take(10) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 14 {
                        disks.push(serde_json::json!({
                            "device": parts[2],
                            "reads": parts[3],
                            "writes": parts[7],
                        }));
                    }
                }
                let disk_info = serde_json::json!({
                    "disks": disks,
                    "timestamp": fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
                });
                Ok(serde_json::to_string_pretty(&disk_info)?.into_bytes())
            }
            "sensor_network" => {
                let net_dev = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
                let mut interfaces = vec![];
                for line in net_dev.lines().skip(2) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 10 {
                        interfaces.push(serde_json::json!({
                            "interface": parts[0].trim_end_matches(':'),
                            "rx_bytes": parts[1],
                            "tx_bytes": parts[9],
                        }));
                    }
                }
                let net_info = serde_json::json!({
                    "interfaces": interfaces,
                    "timestamp": fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
                });
                Ok(serde_json::to_string_pretty(&net_info)?.into_bytes())
            }
            "context_current" => Ok(b"# Current Context\n\nNo active conversation.\n".to_vec()),
            "context_memory" => {
                Ok(b"# Memory State\n\nL1: empty\nL2: 0 entries\nL3: 0 entries\n".to_vec())
            }
            "context_tools" => {
                let tools = serde_json::json!([]);
                Ok(serde_json::to_string_pretty(&tools)?.into_bytes())
            }
            "log_agent" => Ok(b"# Agent Log\n\nNo log entries.\n".to_vec()),
            "log_audit" => Ok(b"".to_vec()),
            "agent_status_main" => {
                let status = serde_json::json!({
                    "agent": "main",
                    "state": "running",
                    "uptime_seconds": 0,
                    "timestamp": fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
                });
                Ok(serde_json::to_string_pretty(&status)?.into_bytes())
            }
            _ => {
                warn!("Unknown generator: {}", generator);
                Ok(b"{}".to_vec())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use std::path::PathBuf;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    #[tokio::test]
    async fn test_fuse_read_root() {
        let fs = AgentFs::new(PathBuf::from("/tmp/test-fuse"), test_clock());
        let content = fs.read("/").await.unwrap();
        let listing = String::from_utf8(content).unwrap();
        assert!(listing.contains("context"));
        assert!(listing.contains("controls"));
        assert!(listing.contains("sensors"));
    }

    #[tokio::test]
    async fn test_fuse_read_sensor_cpu() {
        let fs = AgentFs::new(PathBuf::from("/tmp/test-fuse"), test_clock());
        let content = fs.read("/sensors/cpu.json").await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&content).unwrap();
        assert!(json["load_avg"].is_string());
    }

    #[tokio::test]
    async fn test_fuse_write_pause() {
        let fs = AgentFs::new(PathBuf::from("/tmp/test-fuse"), test_clock());
        assert!(!fs.is_paused().await);

        fs.write("/controls/pause", b"1").await.unwrap();
        assert!(fs.is_paused().await);

        fs.write("/controls/resume", b"1").await.unwrap();
        assert!(!fs.is_paused().await);
    }

    #[tokio::test]
    async fn test_fuse_readdir() {
        let fs = AgentFs::new(PathBuf::from("/tmp/test-fuse"), test_clock());
        let entries = fs.readdir("/sensors").await.unwrap();
        assert!(entries.contains(&"cpu.json".to_string()));
        assert!(entries.contains(&"memory.json".to_string()));
    }

    #[tokio::test]
    async fn test_fuse_readonly_file() {
        let fs = AgentFs::new(PathBuf::from("/tmp/test-fuse"), test_clock());
        let result = fs.write("/sensors/cpu.json", b"hack").await;
        assert!(result.is_err());
    }
}
