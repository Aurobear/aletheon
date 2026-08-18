//! Governed Memory Gateway adapter for synchronous turn-context recall.

use std::sync::Arc;

use async_trait::async_trait;
use contracts::protocol::memory::MemoryRecallRequestV1;
use mnemosyne::MemoryGatewayService;

use application::turn::context::{
    ContextMemoryRecallItem, ContextMemoryRecallPort, ContextMemoryRecallRequest,
    ContextMemoryRecallSet,
};

pub struct MemoryGatewayContextRecall {
    gateway: Arc<MemoryGatewayService>,
}

impl MemoryGatewayContextRecall {
    pub fn new(gateway: Arc<MemoryGatewayService>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl ContextMemoryRecallPort for MemoryGatewayContextRecall {
    async fn recall(
        &self,
        request: ContextMemoryRecallRequest,
    ) -> anyhow::Result<ContextMemoryRecallSet> {
        let recalled = self
            .gateway
            .recall(
                &request.principal_id,
                "turn_context",
                MemoryRecallRequestV1 {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    client_session_id: request.session_id,
                    working_dir: request.working_dir,
                    query: request.query,
                    max_items: request.max_items,
                    max_content_bytes: request.max_content_bytes,
                    include_historical: false,
                    requested_kinds: None,
                },
            )
            .await?;
        Ok(ContextMemoryRecallSet {
            items: recalled
                .items
                .into_iter()
                .map(|item| ContextMemoryRecallItem {
                    record_id: item.record_id,
                    source: item.source,
                    source_id: item.source_id,
                    score: item.score,
                    content: item.content,
                    sensitivity: item.sensitivity,
                })
                .collect(),
            degraded_sources: recalled.degraded_sources,
        })
    }
}
pub async fn observe_native_user(
    memory: &mnemosyne::MemoryGatewayService,
    principal: &contracts::PrincipalId,
    session_id: &str,
    turn_id: &str,
    working_dir: &std::path::Path,
    message: &str,
) -> anyhow::Result<()> {
    memory
        .observe(
            principal,
            "aletheon_native",
            contracts::protocol::memory::MemoryObservationRequestV1 {
                observation_id: format!("native-{turn_id}-user"),
                client_session_id: session_id.to_string(),
                client_turn_id: Some(turn_id.to_string()),
                working_dir: working_dir.to_path_buf(),
                kind: contracts::protocol::memory::MemoryObservationKindV1::UserMessage,
                content: message.to_string(),
                occurred_at: None,
                source_refs: vec![format!("turn:{turn_id}:user")],
                sensitivity_hint: contracts::protocol::memory::MemorySensitivityV1::Internal,
                explicit_user_action: false,
            },
        )
        .await
        .map(|_| ())
}

pub async fn observe_native_assistant(
    memory: &mnemosyne::MemoryGatewayService,
    principal: &contracts::PrincipalId,
    session_id: &str,
    turn_id: &str,
    working_dir: &std::path::Path,
    assistant_item_id: &str,
    output: &str,
) -> anyhow::Result<()> {
    memory
        .observe(
            principal,
            "aletheon_native",
            contracts::protocol::memory::MemoryObservationRequestV1 {
                observation_id: format!("native-{turn_id}-assistant"),
                client_session_id: session_id.to_string(),
                client_turn_id: Some(turn_id.to_string()),
                working_dir: working_dir.to_path_buf(),
                kind: contracts::protocol::memory::MemoryObservationKindV1::AssistantMessage,
                content: output.to_string(),
                occurred_at: None,
                source_refs: vec![assistant_item_id.to_string()],
                sensitivity_hint: contracts::protocol::memory::MemorySensitivityV1::Internal,
                explicit_user_action: false,
            },
        )
        .await
        .map(|_| ())
}
