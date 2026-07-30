//! Lifecycle hook boundary for child-agent execution.

use std::sync::Arc;

use async_trait::async_trait;

use super::AgentRuntimeInput;

#[async_trait]
pub trait AgentLifecycleHookSink: Send + Sync {
    async fn emit(&self, context: fabric::hook::HookContext);
}

pub(super) struct NoopAgentLifecycleHookSink;

#[async_trait]
impl AgentLifecycleHookSink for NoopAgentLifecycleHookSink {
    async fn emit(&self, _context: fabric::hook::HookContext) {}
}

pub struct CorpusAgentLifecycleHookSink(pub Arc<dyn corpus::CorpusService>);

#[async_trait]
impl AgentLifecycleHookSink for CorpusAgentLifecycleHookSink {
    async fn emit(&self, context: fabric::hook::HookContext) {
        self.0.execute_hook(&context).await;
    }
}

pub(super) fn agent_lifecycle_hook_context(
    point: fabric::hook::HookPoint,
    input: &AgentRuntimeInput,
    status: &str,
) -> fabric::hook::HookContext {
    fabric::hook::HookContext {
        point,
        session_id: input.handle.root_agent_id.0.to_string(),
        turn_count: 0,
        tool_name: None,
        tool_input: None,
        tool_result: None,
        message: Some(input.request.task.clone()),
        metadata: std::collections::HashMap::from([
            ("agent_id".into(), input.handle.agent_id.0.to_string()),
            (
                "parent_agent_id".into(),
                input
                    .handle
                    .parent_agent_id
                    .map(|id| id.0.to_string())
                    .unwrap_or_default(),
            ),
            (
                "operation_id".into(),
                input.handle.operation_id.0.to_string(),
            ),
            ("status".into(), status.into()),
            (
                "workspace_root".into(),
                input
                    .workspace
                    .as_ref()
                    .map(|workspace| workspace.cwd().to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
        ]),
    }
}
