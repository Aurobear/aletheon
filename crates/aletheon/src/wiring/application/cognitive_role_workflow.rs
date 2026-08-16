//! Retired cognitive role workflow — one-way compatibility re-export.
//!
//! The typed Planner → Explorer → Executor workflow moved to
//! `agora::cognitive_role_workflow` (cognitive domain owner). This module
//! re-exports the public surface so existing Aletheon-internal callers and
//! compatibility tests keep compiling while the cutover completes. No new
//! implementation is added here.

pub use agora::cognitive_role_workflow::{
    AcceptanceWorkflowReceipt, AcceptanceWorkflowRequest, AgentControlRoleInvoker,
    CodingWorkflowReceipt, CodingWorkflowRequest, CognitiveRoleWorkflow, FullCodingWorkflowReceipt,
    RoleInvocationTerminal, RoleLaunchProfile, RoleWorkflowFactory, TurnRoleLaunchContext,
    classify_task_risk,
};
