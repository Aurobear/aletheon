//! Deterministic host-authored activation policy for the canonical role graph.

use fabric::{
    AgentBudget, AgentDelegationAuthority, AgentRuntimeCapability, TaskKind, TurnRequirement,
};

#[derive(Debug, Clone, PartialEq)]
pub enum TaskDecomposition {
    Simple,
    RoleGraph {
        objective: String,
        workspace_scope: Vec<String>,
        allowed_capabilities: Vec<AgentRuntimeCapability>,
        expected_evidence: Vec<String>,
    },
}

/// Typed host state. `objective` is workflow data only and is never inspected
/// when deciding whether decomposition is enabled.
#[derive(Debug, Clone)]
pub struct DecompositionContext {
    pub task_kind: Option<TaskKind>,
    pub requirements: Vec<TurnRequirement>,
    pub multi_agent_enabled: bool,
    pub automatic_for_coding: bool,
    pub agora_available: bool,
    pub parent_authority: Option<AgentDelegationAuthority>,
    pub remaining_budget: AgentBudget,
    pub objective: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecompositionError {
    #[error("explicit role graph requires Agora")]
    AgoraUnavailable,
    #[error("explicit role graph requires delegation authority")]
    DelegationAuthorityUnavailable,
    #[error("remaining role graph budget is invalid: {0}")]
    InvalidBudget(String),
}

pub trait TaskDecompositionPolicy: Send + Sync {
    fn decompose(
        &self,
        ctx: &DecompositionContext,
    ) -> Result<TaskDecomposition, DecompositionError>;
}

#[derive(Debug, Default)]
pub struct DeterministicTaskDecompositionPolicy;

impl TaskDecompositionPolicy for DeterministicTaskDecompositionPolicy {
    fn decompose(
        &self,
        ctx: &DecompositionContext,
    ) -> Result<TaskDecomposition, DecompositionError> {
        let explicit = ctx
            .requirements
            .iter()
            .find_map(|requirement| match requirement {
                TurnRequirement::RunRoleGraph {
                    workspace_scope,
                    allowed_capabilities,
                    expected_evidence,
                } => Some((
                    workspace_scope.clone(),
                    allowed_capabilities.clone(),
                    expected_evidence.clone(),
                )),
                _ => None,
            });
        let automatic = ctx.task_kind == Some(TaskKind::Coding)
            && ctx.multi_agent_enabled
            && ctx.automatic_for_coding;
        if explicit.is_none() && !automatic {
            return Ok(TaskDecomposition::Simple);
        }
        if !ctx.agora_available {
            return if explicit.is_some() {
                Err(DecompositionError::AgoraUnavailable)
            } else {
                Ok(TaskDecomposition::Simple)
            };
        }
        let Some(authority) = &ctx.parent_authority else {
            return if explicit.is_some() {
                Err(DecompositionError::DelegationAuthorityUnavailable)
            } else {
                Ok(TaskDecomposition::Simple)
            };
        };
        ctx.remaining_budget
            .validate()
            .map_err(|error| DecompositionError::InvalidBudget(error.to_string()))?;
        let (workspace_scope, allowed_capabilities, expected_evidence) =
            explicit.unwrap_or_else(|| {
                let roots = authority
                    .workspace
                    .as_ref()
                    .map(|workspace| {
                        workspace
                            .writable_roots()
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    roots,
                    Vec::new(),
                    vec![
                        "plan".into(),
                        "change".into(),
                        "test".into(),
                        "review".into(),
                    ],
                )
            });
        Ok(TaskDecomposition::RoleGraph {
            objective: ctx.objective.clone(),
            workspace_scope,
            allowed_capabilities,
            expected_evidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn budget() -> AgentBudget {
        AgentBudget {
            max_input_tokens: 10,
            max_output_tokens: 10,
            max_tool_calls: 10,
            max_elapsed_ms: 10,
            max_cost_usd: None,
            max_depth: 2,
        }
    }
    fn authority() -> AgentDelegationAuthority {
        AgentDelegationAuthority::new(None, vec![], budget())
    }
    fn context() -> DecompositionContext {
        DecompositionContext {
            task_kind: None,
            requirements: vec![],
            multi_agent_enabled: true,
            automatic_for_coding: true,
            agora_available: true,
            parent_authority: Some(authority()),
            remaining_budget: budget(),
            objective: "arbitrary prose".into(),
        }
    }

    #[test]
    fn prose_never_activates_a_general_task() {
        assert_eq!(
            DeterministicTaskDecompositionPolicy
                .decompose(&context())
                .unwrap(),
            TaskDecomposition::Simple
        );
    }
    #[test]
    fn typed_coding_activates_when_configured() {
        let mut ctx = context();
        ctx.task_kind = Some(TaskKind::Coding);
        assert!(matches!(
            DeterministicTaskDecompositionPolicy
                .decompose(&ctx)
                .unwrap(),
            TaskDecomposition::RoleGraph { .. }
        ));
    }
    #[test]
    fn explicit_requirement_fails_closed_without_prerequisites() {
        let mut ctx = context();
        ctx.requirements.push(TurnRequirement::RunRoleGraph {
            workspace_scope: vec![],
            allowed_capabilities: vec![],
            expected_evidence: vec![],
        });
        ctx.agora_available = false;
        assert_eq!(
            DeterministicTaskDecompositionPolicy.decompose(&ctx),
            Err(DecompositionError::AgoraUnavailable)
        );
        ctx.agora_available = true;
        ctx.parent_authority = None;
        assert_eq!(
            DeterministicTaskDecompositionPolicy.decompose(&ctx),
            Err(DecompositionError::DelegationAuthorityUnavailable)
        );
    }
    #[test]
    fn automatic_activation_falls_back_before_spawning() {
        let mut ctx = context();
        ctx.task_kind = Some(TaskKind::Coding);
        ctx.parent_authority = None;
        assert_eq!(
            DeterministicTaskDecompositionPolicy
                .decompose(&ctx)
                .unwrap(),
            TaskDecomposition::Simple
        );
    }
}
