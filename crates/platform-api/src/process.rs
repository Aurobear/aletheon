//! ProcessHost — create, inspect, signal, and terminate OS process trees.

use crate::error::HostError;
use crate::receipt::HostReceipt;
use async_trait::async_trait;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u32);

#[derive(Clone, Debug)]
pub struct SpawnSpec {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub working_dir: Option<crate::path::HostPath>,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ProcessSnapshot {
    pub id: ProcessId,
    pub running: bool,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Copy, Debug)]
pub enum ProcessSignal {
    Interrupt,
    Terminate,
    Kill,
}

#[async_trait]
pub trait ProcessHost: Send + Sync {
    async fn spawn(&self, spec: SpawnSpec) -> Result<(ProcessId, HostReceipt), HostError>;
    async fn inspect(&self, id: ProcessId) -> Result<ProcessSnapshot, HostError>;
    async fn signal(&self, id: ProcessId, signal: ProcessSignal) -> Result<HostReceipt, HostError>;
    async fn terminate_tree(&self, id: ProcessId, grace_ms: u64) -> Result<HostReceipt, HostError>;
}
