mod store;

pub use store::{BroadcastReplay, SqliteBroadcastStore};

use crate::CandidatePool;
use async_trait::async_trait;
use fabric::dasein::SelfVersion;
use fabric::{
    BroadcastAck, BroadcastAckStatus, BroadcastDelivery, ProcessId, SelectionResult,
    VisibilityScope, WallTime, WorkspaceBroadcast, WorkspaceCandidate, WORKSPACE_SCHEMA_V1,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinSet;

#[async_trait]
pub trait BroadcastProcessor: Send + Sync {
    async fn receive(&self, delivery: BroadcastDelivery) -> anyhow::Result<Vec<fabric::ContentId>>;
}

#[derive(Clone)]
pub struct ProcessorRegistration {
    pub process: ProcessId,
    pub agent_root: ProcessId,
    pub processor: Arc<dyn BroadcastProcessor>,
}

#[derive(Debug, Clone)]
pub struct BroadcastHubConfig {
    pub max_processors: usize,
    pub max_concurrency: usize,
    pub delivery_timeout: Duration,
}

impl BroadcastHubConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=1024).contains(&self.max_processors),
            "invalid processor capacity"
        );
        anyhow::ensure!(
            (1..=self.max_processors).contains(&self.max_concurrency),
            "invalid broadcast concurrency"
        );
        anyhow::ensure!(!self.delivery_timeout.is_zero(), "delivery timeout is zero");
        Ok(())
    }
}

impl Default for BroadcastHubConfig {
    fn default() -> Self {
        Self {
            max_processors: 64,
            max_concurrency: 8,
            delivery_timeout: Duration::from_secs(5),
        }
    }
}

pub struct BroadcastHub {
    config: BroadcastHubConfig,
    store: Arc<SqliteBroadcastStore>,
    processors: RwLock<HashMap<ProcessId, ProcessorRegistration>>,
}

impl BroadcastHub {
    pub fn new(
        config: BroadcastHubConfig,
        store: Arc<SqliteBroadcastStore>,
    ) -> anyhow::Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            store,
            processors: RwLock::new(HashMap::new()),
        })
    }

    pub async fn register(&self, registration: ProcessorRegistration) -> anyhow::Result<()> {
        let mut processors = self.processors.write().await;
        if processors.contains_key(&registration.process) {
            anyhow::bail!("broadcast processor is already registered");
        }
        anyhow::ensure!(
            processors.len() < self.config.max_processors,
            "broadcast processor capacity exceeded"
        );
        processors.insert(registration.process, registration);
        Ok(())
    }

    pub async fn deliver(
        &self,
        broadcast: &WorkspaceBroadcast,
        observed_at: WallTime,
    ) -> anyhow::Result<Vec<BroadcastAck>> {
        broadcast.validate()?;
        let broadcast_checksum = broadcast.checksum()?;
        let replay = self.store.replay_epoch(&broadcast.space, broadcast.epoch)?;
        if replay.closed_at.is_some() {
            return Ok(replay.acknowledgements);
        }
        let already_acknowledged: std::collections::HashSet<_> = replay
            .acknowledgements
            .iter()
            .map(|ack| ack.processor)
            .collect();
        let registrations: Vec<_> = self.processors.read().await.values().cloned().collect();
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency));
        let mut tasks = JoinSet::new();
        for registration in registrations {
            if already_acknowledged.contains(&registration.process) {
                continue;
            }
            let selected = eligible_candidates(&broadcast.selected, &registration);
            if selected.is_empty() {
                continue;
            }
            let delivery = BroadcastDelivery {
                schema_version: WORKSPACE_SCHEMA_V1,
                epoch: broadcast.epoch,
                space: broadcast.space.clone(),
                recipient: registration.process,
                recipient_agent_root: registration.agent_root,
                broadcast_checksum: broadcast_checksum.clone(),
                dasein_version: broadcast.dasein_version,
                workspace_version: broadcast.workspace_version,
                selected,
            };
            delivery.validate()?;
            let semaphore = semaphore.clone();
            let timeout = self.config.delivery_timeout;
            let store = self.store.clone();
            let space = broadcast.space.clone();
            let epoch = broadcast.epoch;
            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await?;
                let processor = registration.processor.clone();
                let mut receive = tokio::spawn(async move { processor.receive(delivery).await });
                let result = tokio::time::timeout(timeout, &mut receive).await;
                let (status, response_ids, detail) = match result {
                    Ok(Ok(Ok(response_ids))) if valid_response_ids(&response_ids) => {
                        let status = if response_ids.is_empty() {
                            BroadcastAckStatus::Delivered
                        } else {
                            BroadcastAckStatus::Responded
                        };
                        (status, response_ids, None)
                    }
                    Ok(Ok(Ok(_))) => (
                        BroadcastAckStatus::Failed,
                        Vec::new(),
                        Some("processor response IDs are duplicated or excessive".into()),
                    ),
                    Ok(Ok(Err(error))) => (
                        BroadcastAckStatus::Failed,
                        Vec::new(),
                        Some(limit_detail(&error.to_string())),
                    ),
                    Ok(Err(error)) => (
                        BroadcastAckStatus::Failed,
                        Vec::new(),
                        Some(limit_detail(&format!("processor task failed: {error}"))),
                    ),
                    Err(_) => (
                        {
                            receive.abort();
                            BroadcastAckStatus::TimedOut
                        },
                        Vec::new(),
                        Some("processor delivery timed out".into()),
                    ),
                };
                let ack = BroadcastAck {
                    schema_version: WORKSPACE_SCHEMA_V1,
                    space,
                    epoch,
                    processor: registration.process,
                    response_ids,
                    status,
                    observed_at,
                    detail,
                };
                ack.validate()?;
                store.append_ack(&ack)?;
                anyhow::Ok(ack)
            });
        }
        let mut acknowledgements = replay.acknowledgements;
        while let Some(result) = tasks.join_next().await {
            acknowledgements.push(result??);
        }
        acknowledgements.sort_by_key(|ack| ack.processor.0);
        Ok(acknowledgements)
    }
}

pub struct BroadcastCoordinator {
    store: Arc<SqliteBroadcastStore>,
    hub: Arc<BroadcastHub>,
}

impl BroadcastCoordinator {
    pub fn new(store: Arc<SqliteBroadcastStore>, hub: Arc<BroadcastHub>) -> Self {
        Self { store, hub }
    }

    pub async fn broadcast_selection(
        &self,
        pool: &mut CandidatePool,
        selection: SelectionResult,
        dasein_version: SelfVersion,
        workspace_version: u64,
        opened_at: WallTime,
        closed_at: WallTime,
    ) -> anyhow::Result<WorkspaceBroadcast> {
        pool.validate_selection(&selection)?;
        let broadcast = self.store.open_selection(
            selection.clone(),
            dasein_version,
            workspace_version,
            opened_at,
        )?;
        self.hub.deliver(&broadcast, closed_at).await?;
        self.store
            .close_epoch(&broadcast.space, broadcast.epoch, closed_at)?;
        pool.finalize_selection(&selection)?;
        Ok(broadcast)
    }
}

fn eligible_candidates(
    selected: &[WorkspaceCandidate],
    registration: &ProcessorRegistration,
) -> Vec<WorkspaceCandidate> {
    selected
        .iter()
        .filter(|candidate| match candidate.visibility {
            VisibilityScope::Session => true,
            VisibilityScope::PrivateProcess { process } => process == registration.process,
            VisibilityScope::AgentTree { root } => root == registration.agent_root,
        })
        .cloned()
        .collect()
}

fn limit_detail(value: &str) -> String {
    value.chars().take(1024).collect()
}

fn valid_response_ids(ids: &[fabric::ContentId]) -> bool {
    if ids.len() > fabric::MAX_BROADCAST_RESPONSES {
        return false;
    }
    let mut unique = ids.to_vec();
    unique.sort();
    unique.dedup();
    unique.len() == ids.len()
}
