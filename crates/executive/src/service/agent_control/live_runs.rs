use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use fabric::ipc::envelope_v2::Target;
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentId, AgentSnapshot, BackgroundResourceDecl,
    MAX_BACKGROUND_RESOURCES,
};
use tokio::sync::{watch, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct LiveAgentRun {
    pub snapshots: watch::Sender<AgentSnapshot>,
    pub mailbox_target: Target,
    pub cancellation: CancellationToken,
    accepting_calls: Arc<AtomicBool>,
    resources: Arc<RwLock<HashMap<String, BackgroundResourceDecl>>>,
}

impl LiveAgentRun {
    pub fn new(
        snapshots: watch::Sender<AgentSnapshot>,
        mailbox_target: Target,
        cancellation: CancellationToken,
        resources: Vec<BackgroundResourceDecl>,
    ) -> Result<Self, AgentControlError> {
        if resources.len() > MAX_BACKGROUND_RESOURCES {
            return Err(invalid("too many background resources"));
        }
        let mut indexed = HashMap::with_capacity(resources.len());
        for resource in resources {
            if resource.resource_id.trim().is_empty() {
                return Err(invalid("background resource ID must not be empty"));
            }
            if indexed
                .insert(resource.resource_id.clone(), resource)
                .is_some()
            {
                return Err(invalid("background resource IDs must be unique"));
            }
        }
        Ok(Self {
            snapshots,
            mailbox_target,
            cancellation,
            accepting_calls: Arc::new(AtomicBool::new(true)),
            resources: Arc::new(RwLock::new(indexed)),
        })
    }

    /// Returns false once settlement has entered Quiescing.
    pub fn accepting_calls(&self) -> bool {
        self.accepting_calls.load(Ordering::Acquire)
    }

    /// Atomically closes admission to new calls and returns a deterministic
    /// snapshot of resources to settle.
    pub async fn begin_quiescing(&self) -> Vec<BackgroundResourceDecl> {
        self.accepting_calls.store(false, Ordering::Release);
        let mut resources = self
            .resources
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        resources.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
        resources
    }

    pub async fn remove_resource(&self, resource_id: &str) -> Option<BackgroundResourceDecl> {
        self.resources.write().await.remove(resource_id)
    }
}

#[derive(Default)]
pub struct LiveAgentRuns {
    runs: RwLock<HashMap<AgentId, LiveAgentRun>>,
}

fn invalid(message: &str) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::InvalidRequest,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{
        AgentHandle, AgentProfileId, AgentResourceClass, AgentRunStatus, OperationId, ProcessId,
        RuntimeId,
    };

    fn run(resources: Vec<BackgroundResourceDecl>) -> LiveAgentRun {
        let snapshot = AgentSnapshot {
            handle: AgentHandle {
                agent_id: AgentId::new(),
                root_agent_id: AgentId::new(),
                parent_agent_id: None,
                process_id: ProcessId::new(),
                operation_id: OperationId::new(),
                runtime_id: RuntimeId("test".into()),
                profile_id: AgentProfileId("test".into()),
            },
            status: AgentRunStatus::Running,
            result: None,
            created_at_ms: 0,
            started_at_ms: Some(0),
            ended_at_ms: None,
            last_error: None,
        };
        let (snapshots, _) = watch::channel(snapshot);
        LiveAgentRun::new(
            snapshots,
            Target::from("agent:test"),
            CancellationToken::new(),
            resources,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn quiesce_closes_admission_and_sorts_resource_snapshot() {
        let live = run(vec![
            BackgroundResourceDecl {
                resource_id: "z".into(),
                class: AgentResourceClass::BackgroundCommand,
                survive_child: false,
            },
            BackgroundResourceDecl {
                resource_id: "a".into(),
                class: AgentResourceClass::ForegroundCommand,
                survive_child: false,
            },
        ]);
        assert!(live.accepting_calls());
        let resources = live.begin_quiescing().await;
        assert!(!live.accepting_calls());
        assert_eq!(resources[0].resource_id, "a");
        assert_eq!(resources[1].resource_id, "z");
    }

    #[test]
    fn duplicate_resource_is_rejected() {
        let resource = BackgroundResourceDecl {
            resource_id: "same".into(),
            class: AgentResourceClass::BackgroundCommand,
            survive_child: false,
        };
        assert!(LiveAgentRun::new(
            watch::channel(AgentSnapshot {
                handle: AgentHandle {
                    agent_id: AgentId::new(),
                    root_agent_id: AgentId::new(),
                    parent_agent_id: None,
                    process_id: ProcessId::new(),
                    operation_id: OperationId::new(),
                    runtime_id: RuntimeId("test".into()),
                    profile_id: AgentProfileId("test".into()),
                },
                status: AgentRunStatus::Running,
                result: None,
                created_at_ms: 0,
                started_at_ms: Some(0),
                ended_at_ms: None,
                last_error: None,
            })
            .0,
            Target::from("agent:test"),
            CancellationToken::new(),
            vec![resource.clone(), resource],
        )
        .is_err());
    }
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
