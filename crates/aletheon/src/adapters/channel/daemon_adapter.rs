//! Production turn executor that delegates to `DaemonTurnOrchestrator`.
//!
//! This adapter bridges the channel router's typed turn Application port
//! with the full daemon turn pipeline, extracting the assistant response
//! text from the JSON-RPC envelope returned by `execute_turn()`.

use std::sync::Arc;

use crate::adapters::approved_apply::ApplyCoordinator;
use crate::daemon::DaemonTurnOrchestrator;
use ::contracts::contract::command::{ClientIntent, StatusIntent, SubmitPromptIntent};
use ::contracts::{
    ApprovalCategory, ApprovalId, ApprovalSnapshot, GoalId, GoalSnapshot, GoalSpec, GoalState,
    PrincipalId, ProcessId,
};
use adapters_google::gmail::GmailGoalDraftCoordinator;
use adapters_sqlite::approval_repository::{
    ApprovalDecision, ApprovalRepository, ApprovalResolutionContext,
};
use adapters_sqlite::goal::ObjectiveStore;
use application::command_dispatcher::{CommandOutput, CommandUseCases};
use gateway::ports::{
    ApprovalApplicationPort, ApprovalResolver, ApprovalResolverRegistry,
    ChannelApprovalCallbackPort, ChannelApprovalDecision, ChannelApprovalDeliveryPort,
    ChannelGoalApplicationPort, ChannelGoalCommandPort, ChannelTurnApplicationPort,
    ChannelTurnRequest,
};
use tokio::sync::Mutex;

/// Wraps a `DaemonTurnOrchestrator` so the channel router can invoke
/// full daemon chat turns without depending on the handler stack.
#[derive(Clone)]
pub struct DaemonChannelTurnApplicationPort {
    orchestrator: Arc<DaemonTurnOrchestrator>,
    /// Canonical session selected by the daemon composition root for the
    /// owner-only channel. Gateway leaves session selection unset; this
    /// adapter must never fall back to using a principal as a session id.
    default_session: Option<::contracts::SessionId>,
}

pub struct DaemonChannelGoalApplicationPort {
    store: Arc<Mutex<ObjectiveStore>>,
}

pub struct DaemonChannelApprovalExecutor {
    coordinator: Arc<ApplyCoordinator>,
    owner_process: ProcessId,
    cancel: tokio_util::sync::CancellationToken,
}

pub struct DaemonExternalDraftApprovalExecutor {
    coordinator: Arc<std::sync::Mutex<GmailGoalDraftCoordinator>>,
}

impl DaemonExternalDraftApprovalExecutor {
    pub fn new(coordinator: Arc<std::sync::Mutex<GmailGoalDraftCoordinator>>) -> Self {
        Self { coordinator }
    }
}

#[async_trait::async_trait]
impl ApprovalResolver for DaemonExternalDraftApprovalExecutor {
    async fn execute_resolved(
        &self,
        approval: &::contracts::ApprovalSnapshot,
        action: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let coordinator = self.coordinator.lock().unwrap_or_else(|e| e.into_inner());
        match action {
            "confirm" => {
                coordinator.confirm(approval, now_ms)?;
            }
            "edit" => {
                coordinator.reject_or_edit(approval, true, now_ms)?;
            }
            "reject" => {
                coordinator.reject_or_edit(approval, false, now_ms)?;
            }
            _ => anyhow::bail!("unsupported Gmail draft approval action"),
        }
        Ok(())
    }

    async fn revise_draft(
        &self,
        owner: &str,
        goal_id: GoalId,
        intent: &str,
        now_ms: i64,
    ) -> anyhow::Result<::contracts::ApprovalSnapshot> {
        self.coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revise(
                goal_id,
                &PrincipalId(owner.to_owned()),
                intent,
                now_ms,
                now_ms.saturating_add(24 * 60 * 60 * 1_000),
            )
            .map(|draft| draft.approval)
    }
}

impl DaemonChannelApprovalExecutor {
    pub fn new(
        coordinator: Arc<ApplyCoordinator>,
        owner_process: ProcessId,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            coordinator,
            owner_process,
            cancel,
        }
    }
}

#[async_trait::async_trait]
impl ApprovalResolver for DaemonChannelApprovalExecutor {
    async fn execute_resolved(
        &self,
        approval: &::contracts::ApprovalSnapshot,
        _action: &str,
        _now_ms: i64,
    ) -> anyhow::Result<()> {
        self.coordinator
            .coordinate(approval.id, self.owner_process, self.cancel.child_token())
            .await?;
        Ok(())
    }
}

impl DaemonChannelGoalApplicationPort {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self { store }
    }

    fn ensure_owner(goal: GoalSnapshot, owner: &str) -> anyhow::Result<GoalSnapshot> {
        if goal.owner.0 != owner {
            anyhow::bail!("goal not found");
        }
        Ok(goal)
    }
}

#[async_trait::async_trait]
impl ChannelGoalApplicationPort for DaemonChannelGoalApplicationPort {
    async fn create_draft(&self, owner: &str, intent: &str) -> anyhow::Result<GoalSnapshot> {
        let store = self.store.lock().await;
        if store
            .list_goals(&[], 100)?
            .into_iter()
            .any(|g| !g.state.is_terminal())
        {
            anyhow::bail!("an active goal already exists");
        }
        let spec = GoalSpec {
            original_intent: intent.into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        };
        store.create_draft_goal(&PrincipalId(owner.into()), owner, "session", &spec)
    }

    async fn list(&self, owner: &str) -> anyhow::Result<Vec<GoalSnapshot>> {
        Ok(self
            .store
            .lock()
            .await
            .list_goals(&[], 100)?
            .into_iter()
            .filter(|g| g.owner.0 == owner)
            .collect())
    }

    async fn show(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot> {
        let goal = self
            .store
            .lock()
            .await
            .get_goal(id)?
            .ok_or_else(|| anyhow::anyhow!("goal not found"))?;
        Self::ensure_owner(goal, owner)
    }

    async fn pause(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot> {
        let store = self.store.lock().await;
        let goal = Self::ensure_owner(
            store
                .get_goal(id)?
                .ok_or_else(|| anyhow::anyhow!("goal not found"))?,
            owner,
        )?;
        Ok(store.transition_goal(
            id,
            goal.version,
            GoalState::Suspended,
            None,
            &serde_json::json!({"action":"pause"}),
        )?)
    }

    async fn resume(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot> {
        let store = self.store.lock().await;
        let goal = Self::ensure_owner(
            store
                .get_goal(id)?
                .ok_or_else(|| anyhow::anyhow!("goal not found"))?,
            owner,
        )?;
        Ok(store.transition_goal(
            id,
            goal.version,
            GoalState::Ready,
            None,
            &serde_json::json!({"action":"resume"}),
        )?)
    }

    async fn cancel(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot> {
        let store = self.store.lock().await;
        let goal = Self::ensure_owner(
            store
                .get_goal(id)?
                .ok_or_else(|| anyhow::anyhow!("goal not found"))?,
            owner,
        )?;
        Ok(store.transition_goal(
            id,
            goal.version,
            GoalState::Cancelled,
            None,
            &serde_json::json!({"action":"cancel"}),
        )?)
    }
}

/// Application route adapter for Gateway Goal syntax. It owns argument
/// validation, projections, and the optional editable-draft resolver; Gateway
/// only forwards the authenticated command envelope.
pub struct DaemonChannelGoalCommandAdapter {
    executor: Arc<dyn ChannelGoalApplicationPort>,
    activate_goal_resolver: Option<Arc<dyn ApprovalResolver>>,
}

impl DaemonChannelGoalCommandAdapter {
    pub fn new(
        executor: Arc<dyn ChannelGoalApplicationPort>,
        activate_goal_resolver: Option<Arc<dyn ApprovalResolver>>,
    ) -> Self {
        Self {
            executor,
            activate_goal_resolver,
        }
    }
}

#[async_trait::async_trait]
impl ChannelGoalCommandPort for DaemonChannelGoalCommandAdapter {
    async fn execute(
        &self,
        owner: &str,
        command: &str,
        args: &str,
        now_ms: i64,
    ) -> anyhow::Result<String> {
        if command == "/edit" {
            let (id, intent) = args
                .trim()
                .split_once(char::is_whitespace)
                .ok_or_else(|| anyhow::anyhow!("usage: /edit <goal-id> <revised-intent>"))?;
            let id = id
                .parse::<i64>()
                .map(GoalId)
                .map_err(|_| anyhow::anyhow!("usage: /edit <goal-id> <revised-intent>"))?;
            if intent.trim().is_empty() {
                anyhow::bail!("usage: /edit <goal-id> <revised-intent>");
            }
            let resolver = self
                .activate_goal_resolver
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("external draft resolver is not configured"))?;
            let approval = resolver
                .revise_draft(owner, id, intent.trim(), now_ms)
                .await?;
            return Ok(format!(
                "Goal {} revised; fresh confirmation {} is pending.",
                id.0, approval.id
            ));
        }
        if command == "/goal" {
            if args.trim().is_empty() {
                anyhow::bail!("usage: /goal <intent>");
            }
            let goal = self.executor.create_draft(owner, args.trim()).await?;
            return Ok(format!(
                "Goal {} created as draft: {}",
                goal.id.0, goal.spec.original_intent
            ));
        }
        if command == "/goals" {
            let goals = self.executor.list(owner).await?;
            if goals.is_empty() {
                return Ok("No goals.".into());
            }
            return Ok(goals
                .iter()
                .map(|g| format!("{} {} {}", g.id.0, g.state, g.spec.original_intent))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        let id = args
            .trim()
            .parse::<i64>()
            .map(GoalId)
            .map_err(|_| anyhow::anyhow!("usage: {command} <goal-id>"))?;
        let goal = match command {
            "/status" => self.executor.show(owner, id).await?,
            "/pause" => self.executor.pause(owner, id).await?,
            "/resume" => self.executor.resume(owner, id).await?,
            "/cancel" => self.executor.cancel(owner, id).await?,
            _ => anyhow::bail!("unsupported goal command"),
        };
        Ok(format!("Goal {}: {}", goal.id.0, goal.state))
    }
}

impl DaemonChannelTurnApplicationPort {
    pub fn new(orchestrator: Arc<DaemonTurnOrchestrator>) -> Self {
        Self {
            orchestrator,
            default_session: None,
        }
    }

    pub fn with_default_session(mut self, session: ::contracts::SessionId) -> Self {
        self.default_session = Some(session);
        self
    }
}

#[derive(serde::Deserialize)]
struct LegacyTurnRpcResponse {
    result: Option<application::command_dispatcher::PromptCompletion>,
    error: Option<LegacyTurnRpcError>,
}

#[derive(serde::Deserialize)]
struct LegacyTurnRpcError {
    message: String,
}

#[async_trait::async_trait]
impl CommandUseCases for DaemonChannelTurnApplicationPort {
    async fn submit_prompt(
        &self,
        intent: &ClientIntent,
        prompt: &SubmitPromptIntent,
    ) -> anyhow::Result<CommandOutput> {
        let permission_mode = prompt.permission_mode;
        let thread_id = prompt
            .session_id
            .as_ref()
            .map(|session_id| ::contracts::ThreadId(session_id.0.clone()))
            .or_else(|| {
                self.default_session
                    .as_ref()
                    .map(|session| ::contracts::ThreadId(session.0.clone()))
            })
            .ok_or_else(|| anyhow::anyhow!("channel turn has no canonical session selected"))?;
        let resp = self
            .orchestrator
            .execute_turn_targeted(
                serde_json::Value::String(intent.correlation_id.clone()),
                &prompt.content,
                ::contracts::PrincipalContext::new(
                    intent.principal.clone(),
                    ::contracts::LocalOsPrincipal {
                        uid: nix::unistd::Uid::effective().as_raw(),
                        gid: nix::unistd::Gid::effective().as_raw(),
                    },
                    ::contracts::ConnectionId::new(),
                    thread_id,
                    prompt.workspace.clone(),
                    if permission_mode.is_full() {
                        ::contracts::PermissionProfileId::danger_full_access()
                    } else {
                        ::contracts::PermissionProfileId::workspace_write()
                    },
                    if permission_mode.is_full() {
                        ::contracts::ApprovalPolicy::Never
                    } else {
                        ::contracts::ApprovalPolicy::OnRequest
                    },
                ),
                prompt.requirements.clone(),
                prompt.task_kind,
                prompt.execution_target.clone(),
                None,
            )
            .await;

        // Sole legacy JSON-RPC adapter for the in-process orchestrator. Decode
        // once into a typed response; no application code dynamically indexes
        // the wire shape after this boundary.
        let response: LegacyTurnRpcResponse = serde_json::from_value(resp)?;
        if let Some(error) = response.error {
            anyhow::bail!("turn failed: {}", error.message);
        }
        let completion = response
            .result
            .ok_or_else(|| anyhow::anyhow!("turn response omitted both result and error"))?;
        Ok(CommandOutput::new(
            intent.correlation_id.clone(),
            ::contracts::contract::command::CommandOutputV1::PromptCompleted(completion),
        ))
    }

    async fn execute_shell(
        &self,
        intent: &ClientIntent,
        shell: &::contracts::contract::command::ExecuteShellIntent,
    ) -> anyhow::Result<CommandOutput> {
        self.submit_prompt(
            intent,
            &SubmitPromptIntent {
                content: format!(
                    "Execute the following user-requested shell command exactly through the `exec_command` capability and report its terminal result:\n{}",
                    shell.command
                ),
                session_id: shell.session_id.clone(),
                workspace: shell.workspace.clone(),
                requirements: vec![::contracts::TurnRequirement::InvokeCapability {
                    name: "exec_command".into(),
                }],
                task_kind: None,
                permission_mode: shell.permission_mode,
                execution_target: ::contracts::ExecutionTargetSelection::default(),
            },
        )
        .await
    }

    async fn status(
        &self,
        _intent: &ClientIntent,
        _status: &StatusIntent,
    ) -> anyhow::Result<CommandOutput> {
        anyhow::bail!("status is not available through the channel turn executor")
    }
}

#[async_trait::async_trait]
impl ChannelTurnApplicationPort for DaemonChannelTurnApplicationPort {
    async fn execute(&self, request: &ChannelTurnRequest) -> anyhow::Result<String> {
        let intent = ClientIntent::v1(
            ::contracts::contract::command::ClientSurface::Gateway,
            request.principal.clone(),
            request.correlation_id.clone(),
            ::contracts::contract::command::ClientCommand::SubmitPrompt(request.prompt.clone()),
        );
        // This is the typed channel edge.  Do not re-enter the presentation
        // dispatcher here: the Gateway has already classified the command,
        // and this adapter can invoke the application use case directly.
        let output = self.submit_prompt(&intent, &request.prompt).await?;
        output.validate()?;
        match output.output {
            ::contracts::contract::command::CommandOutputV1::PromptCompleted(completion) => {
                Ok(completion.response)
            }
            ::contracts::contract::command::CommandOutputV1::PromptAccepted => Ok(String::new()),
            ::contracts::contract::command::CommandOutputV1::CancelRequested(_)
            | ::contracts::contract::command::CommandOutputV1::Status(_)
            | ::contracts::contract::command::CommandOutputV1::StatusProjected(_) => {
                anyhow::bail!("channel turn executor received a non-prompt command")
            }
            ::contracts::contract::command::CommandOutputV1::Rejected(rejection) => {
                anyhow::bail!(rejection.message)
            }
        }
    }
}

/// Adapts the concrete `ApprovalRepository` to the narrow Gateway approval
/// application and delivery ports. Gateway never sees repository types.
pub struct ApprovalRepositoryPort {
    repository: Arc<std::sync::Mutex<ApprovalRepository>>,
}

impl ApprovalRepositoryPort {
    pub fn new(repository: Arc<std::sync::Mutex<ApprovalRepository>>) -> Self {
        Self { repository }
    }
}

impl ApprovalApplicationPort for ApprovalRepositoryPort {
    fn get(&self, id: ApprovalId) -> anyhow::Result<Option<ApprovalSnapshot>> {
        Ok(self
            .repository
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)?)
    }

    fn resolve(
        &self,
        id: ApprovalId,
        expected_version: u64,
        principal: PrincipalId,
        channel: String,
        decision: ChannelApprovalDecision,
        now_ms: i64,
    ) -> anyhow::Result<ApprovalSnapshot> {
        let context = ApprovalResolutionContext {
            principal_id: principal,
            channel,
        };
        let decision = match decision {
            ChannelApprovalDecision::Approve => ApprovalDecision::Approve,
            ChannelApprovalDecision::Reject { reason } => ApprovalDecision::Reject { reason },
        };
        Ok(self
            .repository
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve(id, expected_version, &context, decision, now_ms)?)
    }

    fn list_pending(
        &self,
        principal: &PrincipalId,
        now_ms: i64,
    ) -> anyhow::Result<Vec<ApprovalSnapshot>> {
        Ok(self
            .repository
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .list_pending(principal, now_ms)?)
    }
}

impl ChannelApprovalDeliveryPort for ApprovalRepositoryPort {
    fn record_delivery_pending(
        &self,
        approval_id: ApprovalId,
        channel: &str,
        conversation_id: &str,
        correlation_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        self.repository
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record_delivery_pending(
                approval_id,
                channel,
                conversation_id,
                correlation_id,
                now_ms,
            )?;
        Ok(())
    }

    fn record_delivery_sent(
        &self,
        correlation_id: &str,
        provider_message_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        Ok(self
            .repository
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record_delivery_sent(correlation_id, provider_message_id, now_ms)?)
    }

    fn record_delivery_failed(
        &self,
        correlation_id: &str,
        error: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        Ok(self
            .repository
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record_delivery_failed(correlation_id, error, now_ms)?)
    }
}

/// Application adapter for channel approval callbacks. Gateway forwards the
/// typed callback but does not read, resolve, or execute approval state.
pub struct DaemonChannelApprovalCallbackAdapter {
    approval_port: Arc<dyn ApprovalApplicationPort>,
    resolvers: Arc<ApprovalResolverRegistry>,
}

impl DaemonChannelApprovalCallbackAdapter {
    pub fn new(
        approval_port: Arc<dyn ApprovalApplicationPort>,
        resolvers: Arc<ApprovalResolverRegistry>,
    ) -> Self {
        Self {
            approval_port,
            resolvers,
        }
    }
}

#[async_trait::async_trait]
impl ChannelApprovalCallbackPort for DaemonChannelApprovalCallbackAdapter {
    async fn execute(
        &self,
        principal: &str,
        channel: &str,
        action_data: &str,
        now_ms: i64,
    ) -> anyhow::Result<String> {
        let (id, action) = action_data
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid approval action"))?;
        let id = ApprovalId(uuid::Uuid::parse_str(id)?);
        let principal_id = PrincipalId(principal.to_owned());
        let result = match action {
            "view_diff" => {
                let approval = self
                    .approval_port
                    .get(id)?
                    .ok_or_else(|| anyhow::anyhow!("approval not found"))?;
                let artifact = approval
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == "diff")
                    .ok_or_else(|| anyhow::anyhow!("approval diff artifact is unavailable"))?;
                return Ok(format!(
                    "Verified diff reference: {} (sha256 {}).",
                    artifact.relative_path.display(),
                    artifact.sha256
                ));
            }
            "apply" | "confirm" => {
                let current = self
                    .approval_port
                    .get(id)?
                    .ok_or_else(|| anyhow::anyhow!("approval not found"))?;
                let resolved = self.approval_port.resolve(
                    id,
                    current.version,
                    principal_id,
                    channel.to_owned(),
                    ChannelApprovalDecision::Approve,
                    now_ms,
                )?;
                (resolved, "approved")
            }
            "revision" | "edit" | "reject" => {
                let current = self
                    .approval_port
                    .get(id)?
                    .ok_or_else(|| anyhow::anyhow!("approval not found"))?;
                let reason = matches!(action, "revision" | "edit")
                    .then(|| "owner requested revision".to_owned());
                let resolved = self.approval_port.resolve(
                    id,
                    current.version,
                    principal_id,
                    channel.to_owned(),
                    ChannelApprovalDecision::Reject { reason },
                    now_ms,
                )?;
                (resolved, "rejected")
            }
            _ => anyhow::bail!("unknown approval action"),
        };
        if result.0.category == ApprovalCategory::ActivateGoal {
            let resolver = self
                .resolvers
                .resolve_category_only(ApprovalCategory::ActivateGoal)
                .ok_or_else(|| anyhow::anyhow!("external draft resolver is not configured"))?;
            resolver.execute_resolved(&result.0, action, now_ms).await?;
        } else if let Some(resolver) = self.resolvers.resolve(result.0.category) {
            resolver.execute_resolved(&result.0, action, now_ms).await?;
        }
        Ok(format!("Approval {}: {}", result.0.id, result.1))
    }
}
