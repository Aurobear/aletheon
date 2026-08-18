use std::sync::Arc;

use contracts::{MonoDeadlineMillis, OperationId, PrincipalId, ProcessId};
use tokio_util::sync::CancellationToken;

use super::settings::ResolvedTurnProfile;

#[derive(Clone, Debug)]
pub struct TurnCommand {
    pub input: String,
    pub execution_target: contracts::ExecutionTargetSelection,
    pub model_policy: Option<String>,
    pub deadline: Option<MonoDeadlineMillis>,
    pub requirements: Vec<contracts::TurnRequirement>,
    pub requested_task_kind: Option<contracts::TaskKind>,
}

#[derive(Clone, Debug)]
pub struct TurnContext {
    pub principal_id: PrincipalId,
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub workspace: Arc<contracts::WorkspacePolicy>,
    pub profile: ResolvedTurnProfile,
    pub cancel_token: CancellationToken,
    /// Transport-neutral event delivery port supplied by the admitting adapter.
    pub notification: Option<Arc<dyn super::service::TurnNotificationPort>>,
    /// Exact host-authenticated context resolved at the trusted transport edge.
    pub principal_context: Option<contracts::PrincipalContext>,
}

impl TurnContext {
    pub fn require_principal_context(
        &self,
    ) -> Result<contracts::PrincipalContext, super::service::TurnServiceError> {
        let context = self.principal_context.clone().ok_or_else(|| {
            super::service::TurnServiceError::InvalidContext(
                "authenticated principal context is missing".into(),
            )
        })?;
        if context.principal_id != self.principal_id {
            return Err(super::service::TurnServiceError::InvalidContext(
                "authenticated principal does not match engine principal".into(),
            ));
        }
        Ok(context)
    }
}

pub fn role_graph_requested(
    request: &contracts::TurnRequest,
    settings: &super::settings::TurnRuntimeSettings,
) -> bool {
    request
        .requirements
        .iter()
        .any(|requirement| matches!(requirement, contracts::TurnRequirement::RunRoleGraph { .. }))
        || (request.requested_task_kind == Some(contracts::TaskKind::Coding)
            && settings.multi_agent_enabled
            && settings.automatic_multi_agent_for_coding)
}

/// Derive the main agent's delegation envelope from snapshots already
/// authorized for this turn. This is policy, not host composition.
pub fn main_delegation_authority(
    workspace: contracts::WorkspacePolicy,
    profile: &ResolvedTurnProfile,
    settings: &super::settings::TurnRuntimeSettings,
) -> contracts::AgentDelegationAuthority {
    let mut allowed_tools = profile.delegated_tools.iter().cloned().collect::<Vec<_>>();
    allowed_tools.sort();
    contracts::AgentDelegationAuthority::new(
        Some(workspace),
        allowed_tools,
        contracts::AgentBudget {
            max_input_tokens: profile.max_input_tokens,
            max_output_tokens: profile.max_output_tokens,
            max_tool_calls: profile.max_tool_calls,
            max_elapsed_ms: profile.max_elapsed_ms,
            max_cost_usd: None,
            max_depth: settings.max_agent_depth,
        },
    )
}
