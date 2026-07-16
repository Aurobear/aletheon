use std::collections::HashMap;

use fabric::ipc::envelope_v2::Target;
use fabric::{AgentId, AgentSnapshot};
use tokio::sync::{watch, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct LiveAgentRun {
    pub snapshots: watch::Sender<AgentSnapshot>,
    pub mailbox_target: Target,
    pub cancellation: CancellationToken,
}

#[derive(Default)]
pub struct LiveAgentRuns {
    runs: RwLock<HashMap<AgentId, LiveAgentRun>>,
}

impl LiveAgentRuns {
    pub async fn insert(&self, agent: AgentId, run: LiveAgentRun) -> bool {
        self.runs.write().await.insert(agent, run).is_none()
    }

    pub async fn get(&self, agent: AgentId) -> Option<LiveAgentRun> {
        self.runs.read().await.get(&agent).cloned()
    }

    pub async fn remove(&self, agent: AgentId) -> Option<LiveAgentRun> {
        self.runs.write().await.remove(&agent)
    }

    pub async fn all(&self) -> Vec<LiveAgentRun> {
        self.runs.read().await.values().cloned().collect()
    }
}
