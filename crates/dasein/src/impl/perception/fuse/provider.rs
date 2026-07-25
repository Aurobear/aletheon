//! State provider trait for feeding real agent state into the FUSE filesystem.
//!
//! `StateProvider` abstracts data sources so the filesystem can operate with
//! either live system data or mocked test data.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Abstraction for providing agent state data to the virtual filesystem.
///
/// Implementors supply data for dynamic files (sensors, context, logs, agent status).
/// The trait is object-safe so it can be stored as `Arc<dyn StateProvider>`.
#[async_trait]
pub trait StateProvider: Send + Sync {
    /// Fetch sensor data (cpu, memory, disk, network).
    async fn get_sensor_data(&self, sensor: &str) -> Result<Vec<u8>>;

    /// Fetch context data (current conversation, memory state, tools).
    async fn get_context(&self, ctx_type: &str) -> Result<Vec<u8>>;

    /// Fetch log data (agent log, audit log).
    async fn get_log(&self, log_type: &str) -> Result<Vec<u8>>;

    /// Fetch agent status by agent ID.
    async fn get_agent_status(&self, agent_id: &str) -> Result<Vec<u8>>;
}

/// Live implementation that reads real system state.
///
/// Sensors read from `/proc`, context and logs are placeholder implementations
/// ready to be wired to the actual agent subsystems.
pub struct LiveStateProvider {
    clock: Arc<dyn fabric::Clock>,
}

impl LiveStateProvider {
    pub fn new(clock: Arc<dyn fabric::Clock>) -> Self {
        Self { clock }
    }
}

#[cfg(test)]
impl Default for LiveStateProvider {
    fn default() -> Self {
        Self::new(Arc::new(kernel::chronos::TestClock::default()))
    }
}

#[async_trait]
impl StateProvider for LiveStateProvider {
    async fn get_sensor_data(&self, sensor: &str) -> Result<Vec<u8>> {
        match sensor {
            "cpu" => {
                let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
                let info = serde_json::json!({
                    "load_avg": load.trim(),
                    "timestamp": fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
                });
                Ok(serde_json::to_string_pretty(&info)?.into_bytes())
            }
            "memory" => {
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
            "disk" => {
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
                let info = serde_json::json!({
                    "disks": disks,
                    "timestamp": fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
                });
                Ok(serde_json::to_string_pretty(&info)?.into_bytes())
            }
            "network" => {
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
                let info = serde_json::json!({
                    "interfaces": interfaces,
                    "timestamp": fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
                });
                Ok(serde_json::to_string_pretty(&info)?.into_bytes())
            }
            _ => anyhow::bail!("Unknown sensor: {sensor}"),
        }
    }

    async fn get_context(&self, ctx_type: &str) -> Result<Vec<u8>> {
        match ctx_type {
            "current" => Ok(b"# Current Context\n\nNo active conversation.\n".to_vec()),
            "memory" => Ok(b"# Memory State\n\nL1: empty\nL2: 0 entries\nL3: 0 entries\n".to_vec()),
            "tools" => {
                let tools = serde_json::json!([]);
                Ok(serde_json::to_string_pretty(&tools)?.into_bytes())
            }
            _ => anyhow::bail!("Unknown context type: {ctx_type}"),
        }
    }

    async fn get_log(&self, log_type: &str) -> Result<Vec<u8>> {
        match log_type {
            "agent" => Ok(b"# Agent Log\n\nNo log entries.\n".to_vec()),
            "audit" => Ok(b"".to_vec()),
            _ => anyhow::bail!("Unknown log type: {log_type}"),
        }
    }

    async fn get_agent_status(&self, agent_id: &str) -> Result<Vec<u8>> {
        let status = serde_json::json!({
            "agent": agent_id,
            "state": "running",
            "uptime_seconds": 0,
            "timestamp": fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
        });
        Ok(serde_json::to_string_pretty(&status)?.into_bytes())
    }
}

/// Mock implementation for testing.
///
/// Stores data in an in-memory map. Tests can pre-populate responses
/// and verify what was requested.
pub struct MockStateProvider {
    responses: Arc<RwLock<HashMap<String, Result<Vec<u8>>>>>,
    calls: Arc<RwLock<Vec<String>>>,
}

impl MockStateProvider {
    pub fn new() -> Self {
        Self {
            responses: Arc::new(RwLock::new(HashMap::new())),
            calls: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Pre-set a response for a key like "sensor:cpu" or "context:current".
    pub async fn set_response(&self, key: &str, data: Vec<u8>) {
        let mut responses = self.responses.write().await;
        responses.insert(key.to_string(), Ok(data));
    }

    /// Pre-set an error response for a key.
    pub async fn set_error(&self, key: &str, msg: &str) {
        let mut responses = self.responses.write().await;
        responses.insert(key.to_string(), Err(anyhow::anyhow!("{msg}")));
    }

    /// Get all recorded call keys (for assertions).
    pub async fn recorded_calls(&self) -> Vec<String> {
        self.calls.read().await.clone()
    }

    /// Check if a specific key was called.
    pub async fn was_called(&self, key: &str) -> bool {
        self.calls.read().await.iter().any(|c| c == key)
    }

    /// Clear recorded calls.
    pub async fn clear_calls(&self) {
        self.calls.write().await.clear();
    }

    async fn lookup(&self, key: &str) -> Result<Vec<u8>> {
        self.calls.write().await.push(key.to_string());
        let responses = self.responses.read().await;
        match responses.get(key) {
            Some(Ok(data)) => Ok(data.clone()),
            Some(Err(_)) => {
                // Clone the error message since anyhow::Error isn't Clone
                Err(anyhow::anyhow!("mock error for key: {key}"))
            }
            None => anyhow::bail!("No mock response for key: {key}"),
        }
    }
}

impl Default for MockStateProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StateProvider for MockStateProvider {
    async fn get_sensor_data(&self, sensor: &str) -> Result<Vec<u8>> {
        self.lookup(&format!("sensor:{sensor}")).await
    }

    async fn get_context(&self, ctx_type: &str) -> Result<Vec<u8>> {
        self.lookup(&format!("context:{ctx_type}")).await
    }

    async fn get_log(&self, log_type: &str) -> Result<Vec<u8>> {
        self.lookup(&format!("log:{log_type}")).await
    }

    async fn get_agent_status(&self, agent_id: &str) -> Result<Vec<u8>> {
        self.lookup(&format!("agent:{agent_id}")).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_provider_sensor() {
        let mock = MockStateProvider::new();
        mock.set_response("sensor:cpu", b"{\"load\":\"0.5\"}".to_vec())
            .await;

        let data = mock.get_sensor_data("cpu").await.unwrap();
        assert_eq!(data, b"{\"load\":\"0.5\"}");
        assert!(mock.was_called("sensor:cpu").await);
    }

    #[tokio::test]
    async fn test_mock_provider_missing_key() {
        let mock = MockStateProvider::new();
        let result = mock.get_sensor_data("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_provider_context() {
        let mock = MockStateProvider::new();
        mock.set_response("context:current", b"hello context".to_vec())
            .await;

        let data = mock.get_context("current").await.unwrap();
        assert_eq!(data, b"hello context");
        assert!(mock.was_called("context:current").await);
    }

    #[tokio::test]
    async fn test_mock_provider_log() {
        let mock = MockStateProvider::new();
        mock.set_response("log:agent", b"log entries".to_vec())
            .await;

        let data = mock.get_log("agent").await.unwrap();
        assert_eq!(data, b"log entries");
    }

    #[tokio::test]
    async fn test_mock_provider_agent_status() {
        let mock = MockStateProvider::new();
        mock.set_response("agent:main", b"{\"state\":\"running\"}".to_vec())
            .await;

        let data = mock.get_agent_status("main").await.unwrap();
        assert_eq!(data, b"{\"state\":\"running\"}");
        assert!(mock.was_called("agent:main").await);
    }

    #[tokio::test]
    async fn test_mock_provider_error() {
        let mock = MockStateProvider::new();
        mock.set_error("sensor:cpu", "sensor offline").await;

        let result = mock.get_sensor_data("cpu").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mock error"));
    }

    #[tokio::test]
    async fn test_mock_recorded_calls() {
        let mock = MockStateProvider::new();
        mock.set_response("sensor:cpu", b"{}".to_vec()).await;
        mock.set_response("sensor:memory", b"{}".to_vec()).await;

        let _ = mock.get_sensor_data("cpu").await;
        let _ = mock.get_sensor_data("memory").await;
        let _ = mock.get_sensor_data("cpu").await;

        let calls = mock.recorded_calls().await;
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], "sensor:cpu");
        assert_eq!(calls[1], "sensor:memory");
        assert_eq!(calls[2], "sensor:cpu");
    }

    #[tokio::test]
    async fn test_mock_clear_calls() {
        let mock = MockStateProvider::new();
        mock.set_response("sensor:cpu", b"{}".to_vec()).await;
        let _ = mock.get_sensor_data("cpu").await;
        assert!(mock.was_called("sensor:cpu").await);

        mock.clear_calls().await;
        assert!(!mock.was_called("sensor:cpu").await);
    }
}
