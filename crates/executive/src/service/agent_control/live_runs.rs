use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use fabric::ipc::envelope_v2::Target;
use fabric::{
    AgentBudget, AgentControlError, AgentControlErrorKind, AgentId, AgentSnapshot,
    BackgroundResourceDecl, WorkspacePolicy, MAX_BACKGROUND_RESOURCES,
};
use tokio::sync::{watch, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use super::execution::BackgroundResourceRegistration;

#[derive(Clone)]
pub struct LiveAgentRun {
    pub snapshots: watch::Sender<AgentSnapshot>,
    pub mailbox_target: Target,
    pub cancellation: CancellationToken,
    accepting_calls: Arc<AtomicBool>,
    resources: Arc<RwLock<HashMap<String, BackgroundResourceDecl>>>,
    managed_resources: Arc<HashMap<String, ManagedResourceState>>,
    reparent_authority: Arc<ReparentAuthority>,
}

#[derive(Clone)]
pub struct ReparentAuthority {
    workspace: Option<WorkspacePolicy>,
    allowed_tools: Vec<String>,
    budget: AgentBudget,
}

impl ReparentAuthority {
    pub fn new(
        workspace: Option<WorkspacePolicy>,
        allowed_tools: Vec<String>,
        budget: AgentBudget,
    ) -> Self {
        Self {
            workspace,
            allowed_tools,
            budget,
        }
    }

    pub fn covers(&self, child: &Self) -> bool {
        let tools_cover = child
            .allowed_tools
            .iter()
            .all(|tool| self.allowed_tools.contains(tool));
        let workspace_covers = match (&self.workspace, &child.workspace) {
            (Some(parent), Some(child)) => child.writable_roots().iter().all(|child_root| {
                parent
                    .writable_roots()
                    .iter()
                    .any(|parent_root| child_root.starts_with(parent_root))
            }),
            (None, None) => true,
            _ => false,
        };
        tools_cover && workspace_covers
    }

    pub fn accepts_budget(&self, child: &Self) -> bool {
        self.budget.max_input_tokens >= child.budget.max_input_tokens
            && self.budget.max_output_tokens >= child.budget.max_output_tokens
            && self.budget.max_tool_calls >= child.budget.max_tool_calls
            && self.budget.max_elapsed_ms >= child.budget.max_elapsed_ms
            && self.budget.max_depth >= child.budget.max_depth
            && match (self.budget.max_cost_usd, child.budget.max_cost_usd) {
                (None, _) => true,
                (Some(parent), Some(child)) => parent >= child,
                (Some(_), None) => false,
            }
    }
}

struct ManagedResourceState {
    registration: BackgroundResourceRegistration,
    owner: Mutex<String>,
    completed_actions: Mutex<std::collections::HashSet<String>>,
}

impl LiveAgentRun {
    pub fn new(
        snapshots: watch::Sender<AgentSnapshot>,
        mailbox_target: Target,
        cancellation: CancellationToken,
        resources: Vec<BackgroundResourceDecl>,
        reparent_authority: ReparentAuthority,
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
        let initial_owner = format!("process:{}", snapshots.borrow().handle.process_id.0);
        let managed_resources = indexed
            .iter()
            .map(|(resource_id, declaration)| {
                // A reviewed survivable resource must not inherit the child
                // token: that would cancel it before reparent can attach the
                // exact producer token to the parent. Non-survivable resources
                // remain structurally parented to child cancellation.
                let resource_token = if declaration.survive_child {
                    CancellationToken::new()
                } else {
                    cancellation.child_token()
                };
                (
                    resource_id.clone(),
                    ManagedResourceState {
                        registration: BackgroundResourceRegistration::new(resource_token),
                        owner: Mutex::new(initial_owner.clone()),
                        completed_actions: Mutex::new(std::collections::HashSet::new()),
                    },
                )
            })
            .collect();
        Ok(Self {
            snapshots,
            mailbox_target,
            cancellation,
            accepting_calls: Arc::new(AtomicBool::new(true)),
            resources: Arc::new(RwLock::new(indexed)),
            managed_resources: Arc::new(managed_resources),
            reparent_authority: Arc::new(reparent_authority),
        })
    }

    pub fn reparent_authority(&self) -> &ReparentAuthority {
        &self.reparent_authority
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

    pub fn has_managed_resource(&self, resource_id: &str) -> bool {
        self.managed_resources.contains_key(resource_id)
    }

    pub async fn resource_cancellation(&self, resource_id: &str) -> Option<CancellationToken> {
        let state = self.managed_resources.get(resource_id)?;
        Some(state.registration.cancellation())
    }

    pub fn resource_registration(
        &self,
        resource_id: &str,
    ) -> Option<BackgroundResourceRegistration> {
        self.managed_resources
            .get(resource_id)
            .map(|state| state.registration.clone())
    }

    pub async fn terminate_managed_resource(&self, resource_id: &str, action_key: &str) -> bool {
        let Some(state) = self.managed_resources.get(resource_id) else {
            return false;
        };
        let mut actions = state.completed_actions.lock().await;
        if actions.insert(action_key.to_string()) {
            state.registration.cancel_and_wait().await;
        }
        true
    }

    pub async fn reparent_managed_resource(
        &self,
        resource_id: &str,
        old_owner: &str,
        new_owner: &str,
        action_key: &str,
        parent_cancellation: Option<&CancellationToken>,
    ) -> Result<(), AgentControlError> {
        let state = self
            .managed_resources
            .get(resource_id)
            .ok_or_else(|| invalid("managed settlement resource is unavailable"))?;
        let mut actions = state.completed_actions.lock().await;
        if actions.contains(action_key) {
            return Ok(());
        }
        let mut owner = state.owner.lock().await;
        if owner.as_str() != old_owner && owner.as_str() != new_owner {
            return Err(invalid("managed settlement resource owner mismatch"));
        }
        *owner = new_owner.to_string();
        if let Some(parent_cancellation) = parent_cancellation {
            // Keep the exact token already handed to the command producer.
            // Replacing it would leave a running producer attached to the old
            // child token and therefore orphan it from parent cancellation.
            let resource_cancellation = state.registration.cancellation();
            let parent_cancellation = parent_cancellation.clone();
            tokio::spawn(async move {
                parent_cancellation.cancelled().await;
                resource_cancellation.cancel();
            });
        }
        actions.insert(action_key.to_string());
        Ok(())
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
            ReparentAuthority::new(
                None,
                vec![],
                AgentBudget {
                    max_input_tokens: 1,
                    max_output_tokens: 1,
                    max_tool_calls: 1,
                    max_elapsed_ms: 1,
                    max_cost_usd: None,
                    max_depth: 1,
                },
            ),
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

    #[tokio::test]
    async fn managed_resource_terminates_independently_and_reparents_to_parent_cancel() {
        let live = run(vec![BackgroundResourceDecl {
            resource_id: "background".into(),
            class: AgentResourceClass::BackgroundCommand,
            survive_child: true,
        }]);
        let token = live.resource_cancellation("background").await.unwrap();
        let registration = live.resource_registration("background").unwrap();
        registration
            .bind(|cancellation| async move { cancellation.cancelled().await })
            .unwrap();
        assert!(!token.is_cancelled());
        let parent = CancellationToken::new();
        live.reparent_managed_resource(
            "background",
            &format!("process:{}", live.snapshots.borrow().handle.process_id.0),
            "agent:parent",
            "reparent-1",
            Some(&parent),
        )
        .await
        .unwrap();
        let reparented = live.resource_cancellation("background").await.unwrap();
        parent.cancel();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            registration.wait_stopped(),
        )
        .await
        .unwrap();
        assert!(reparented.is_cancelled());
        assert!(token.is_cancelled());
        assert!(registration.is_stopped());

        assert!(
            live.terminate_managed_resource("background", "terminate-1")
                .await
        );
        assert!(reparented.is_cancelled());
    }

    #[tokio::test]
    async fn child_cancel_stops_registered_non_surviving_producer() {
        let live = run(vec![BackgroundResourceDecl {
            resource_id: "foreground".into(),
            class: AgentResourceClass::ForegroundCommand,
            survive_child: false,
        }]);
        let registration = live.resource_registration("foreground").unwrap();
        registration
            .bind(|cancellation| async move { cancellation.cancelled().await })
            .unwrap();

        live.cancellation.cancel();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            registration.wait_stopped(),
        )
        .await
        .unwrap();
        assert!(registration.is_stopped());
    }

    #[tokio::test]
    async fn reviewed_survivor_is_not_cancelled_before_reparent_decision() {
        let live = run(vec![BackgroundResourceDecl {
            resource_id: "survivor".into(),
            class: AgentResourceClass::BackgroundCommand,
            survive_child: true,
        }]);
        let registration = live.resource_registration("survivor").unwrap();
        registration
            .bind(|cancellation| async move { cancellation.cancelled().await })
            .unwrap();

        live.cancellation.cancel();
        tokio::task::yield_now().await;
        assert!(!registration.cancellation().is_cancelled());
        assert!(!registration.is_stopped());
        assert!(
            live.terminate_managed_resource("survivor", "settlement-denied")
                .await
        );
        assert!(registration.is_stopped());
    }

    #[tokio::test]
    async fn foreground_settlement_waits_for_registered_producer_cleanup() {
        struct CleanupSignal(Arc<AtomicBool>);
        impl Drop for CleanupSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let live = run(vec![BackgroundResourceDecl {
            resource_id: "foreground".into(),
            class: AgentResourceClass::ForegroundCommand,
            survive_child: false,
        }]);
        let cleaned = Arc::new(AtomicBool::new(false));
        let guard = CleanupSignal(cleaned.clone());
        live.resource_registration("foreground")
            .unwrap()
            .bind(move |cancellation| async move {
                let _guard = guard;
                cancellation.cancelled().await;
            })
            .unwrap();

        assert!(
            live.terminate_managed_resource("foreground", "settle")
                .await
        );
        assert!(cleaned.load(Ordering::Acquire));
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
            ReparentAuthority::new(
                None,
                vec![],
                AgentBudget {
                    max_input_tokens: 1,
                    max_output_tokens: 1,
                    max_tool_calls: 1,
                    max_elapsed_ms: 1,
                    max_cost_usd: None,
                    max_depth: 1,
                }
            ),
        )
        .is_err());
    }

    #[test]
    fn reparent_requires_parent_workspace_tools_and_budget_to_cover_child() {
        let budget = |tokens, cost| AgentBudget {
            max_input_tokens: tokens,
            max_output_tokens: tokens,
            max_tool_calls: tokens as u32,
            max_elapsed_ms: tokens,
            max_cost_usd: cost,
            max_depth: tokens as u16,
        };
        let parent = ReparentAuthority::new(
            Some(WorkspacePolicy::from_resolved_roots("/repo".into(), vec![]).unwrap()),
            vec!["read".into(), "write".into()],
            budget(10, Some(10.0)),
        );
        let child = ReparentAuthority::new(
            Some(WorkspacePolicy::from_resolved_roots("/repo/child".into(), vec![]).unwrap()),
            vec!["read".into()],
            budget(5, Some(5.0)),
        );
        assert!(parent.covers(&child));
        assert!(parent.accepts_budget(&child));

        let excessive = ReparentAuthority::new(
            child.workspace.clone(),
            vec!["shell".into()],
            budget(11, Some(11.0)),
        );
        assert!(!parent.covers(&excessive));
        assert!(!parent.accepts_budget(&excessive));
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
