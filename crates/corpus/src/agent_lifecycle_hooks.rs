//! Corpus adapter for Runtime child-agent lifecycle observations.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

pub struct CorpusAgentLifecycleHookSink(pub Arc<dyn crate::CorpusService>);

#[async_trait]
impl runtime::AgentLifecycleHookSink for CorpusAgentLifecycleHookSink {
    async fn emit(&self, observation: runtime::AgentLifecycleObservation) {
        let context = crate::hook::HookContext {
            point: match observation.point {
                runtime::AgentLifecyclePoint::Start => crate::hook::HookPoint::SubagentStart,
                runtime::AgentLifecyclePoint::Stop => crate::hook::HookPoint::SubagentStop,
            },
            session_id: observation.root_agent_id,
            turn_count: 0,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: Some(observation.task),
            metadata: HashMap::from([
                ("agent_id".into(), observation.agent_id),
                (
                    "parent_agent_id".into(),
                    observation.parent_agent_id.unwrap_or_default(),
                ),
                ("operation_id".into(), observation.operation_id),
                ("status".into(), observation.status),
                (
                    "workspace_root".into(),
                    observation.workspace_root.unwrap_or_default(),
                ),
            ]),
        };
        self.0.execute_hook(&context).await;
    }
}
