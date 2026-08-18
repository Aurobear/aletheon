use std::sync::Arc;

use ::contracts::{
    CapabilityCall, CapabilityResult, ItemPayload, RecallRequest, RecallSet, TurnRequest,
    TurnServices,
};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use application::turn::coordinator::TurnCoordinator;
use application::turn::settings::ResolvedTurnProfile;
use application::turn::{TurnEngine, TurnEngineContext, TurnEngineRequest, TurnEngineStream};

/// Thin compatibility adapter from the existing exec caller shape to the
/// authoritative TurnEngine entry point. It owns no turn orchestration.
pub struct TurnService {
    engine: Arc<dyn TurnEngine>,
    coordinator: Arc<TurnCoordinator>,
    profile: ResolvedTurnProfile,
    cancellation: CancellationToken,
}

impl TurnService {
    pub fn from_engine(
        engine: Arc<dyn TurnEngine>,
        coordinator: Arc<TurnCoordinator>,
        profile: ResolvedTurnProfile,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            engine,
            coordinator,
            profile,
            cancellation,
        }
    }

    /// Request authoritative cancellation for every active turn owned by the
    /// authenticated principal.
    pub async fn cancel_active_for_principal(
        &self,
        principal_id: &::contracts::PrincipalId,
    ) -> usize {
        self.coordinator
            .cancel_active_for_principal(principal_id)
            .await
    }

    pub async fn submit<E>(
        &self,
        request: TurnRequest,
        events: &E,
    ) -> Result<::contracts::TurnResult>
    where
        E: TurnEngineStream,
    {
        let principal_context = request.context.clone();
        let engine_request = TurnEngineRequest {
            input: request.input,
            execution_target: request.execution_target,
            model_policy: request.model_policy,
            deadline: request.deadline,
            requirements: request.requirements,
            requested_task_kind: request.requested_task_kind,
        };
        let engine_context = TurnEngineContext {
            principal_id: principal_context.principal_id.clone(),
            operation_id: request.operation_id,
            process_id: request.process_id,
            workspace: Arc::new(principal_context.workspace.clone()),
            profile: self.profile.clone(),
            cancel_token: self.cancellation.clone(),
            notification: None,
            principal_context: Some(principal_context),
        };
        let result = self
            .engine
            .execute_with_events(engine_request, engine_context, events)
            .await
            .map_err(anyhow::Error::new)?;
        Ok(result
            .coordinator_execution
            .context("exec TurnEngine omitted its coordinator execution")?
            .result)
    }
}

pub(crate) struct RecordingTurnServices {
    inner: Arc<dyn TurnServices>,
    items: Mutex<Vec<ItemPayload>>,
    canonical_seed: Vec<::contracts::Message>,
}

impl RecordingTurnServices {
    pub(crate) fn new(
        inner: Arc<dyn TurnServices>,
        canonical_seed: Vec<::contracts::Message>,
    ) -> Self {
        Self {
            inner,
            items: Mutex::new(Vec::new()),
            canonical_seed,
        }
    }
    pub(crate) async fn take_items(&self) -> Vec<ItemPayload> {
        std::mem::take(&mut *self.items.lock().await)
    }
}

#[async_trait]
impl TurnServices for RecordingTurnServices {
    async fn recall(&self, request: RecallRequest) -> Result<RecallSet> {
        self.inner.recall(request).await
    }
    async fn dasein_view(
        &self,
        process: ::contracts::ProcessId,
    ) -> Result<::contracts::DaseinView> {
        self.inner.dasein_view(process).await
    }
    async fn agora_view(&self, session_id: &str) -> Result<::contracts::AgoraView> {
        self.inner.agora_view(session_id).await
    }
    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        self.items.lock().await.push(ItemPayload::ToolCall {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            input: call.input.clone(),
        });
        let result = self.inner.invoke(call).await;
        self.items.lock().await.push(ItemPayload::ToolResult {
            call_id: result.call_id.clone(),
            content: result.output.clone(),
            is_error: result.is_error,
            permit_id: (result.usage.permit_id != ::contracts::PermitId::default())
                .then_some(result.usage.permit_id),
            audit_id: result.audit_id,
        });
        result
    }
    async fn record_capability_receipt(&self, receipt: ::contracts::CapabilityTerminalReceipt) {
        self.items
            .lock()
            .await
            .push(ItemPayload::CapabilityReceipt {
                receipt: receipt.clone(),
            });
        self.inner.record_capability_receipt(receipt).await;
    }
    async fn record_model_context_projection(
        &self,
        mut receipt: ::contracts::model_projection::ModelContextProjectionReceipt,
    ) {
        corpus::tools::artifact::materialize_model_context_projection(&mut receipt);
        self.items
            .lock()
            .await
            .push(ItemPayload::ModelContextProjection {
                receipt: receipt.clone(),
            });
        self.inner.record_model_context_projection(receipt).await;
    }
    async fn record_inference_receipt(
        &self,
        receipt: ::contracts::types::inference_receipt::InferenceTerminalReceipt,
    ) {
        self.items.lock().await.push(ItemPayload::InferenceReceipt {
            receipt: receipt.clone(),
        });
        self.inner.record_inference_receipt(receipt).await;
    }
    fn llm_provider(&self) -> Option<&dyn ::contracts::LlmProvider> {
        self.inner.llm_provider()
    }
    fn tool_definitions(&self) -> Vec<::contracts::ToolDefinition> {
        self.inner.tool_definitions()
    }
    fn seed_messages(&self, request: &TurnRequest) -> Vec<::contracts::Message> {
        let mut seed = self.inner.seed_messages(request);
        seed.extend(self.canonical_seed.clone());
        seed
    }
    fn turn_requirements(&self, request: &TurnRequest) -> Vec<::contracts::TurnRequirement> {
        self.inner.turn_requirements(request)
    }

    async fn plan_capability_batch(
        &self,
        calls: Vec<CapabilityCall>,
    ) -> anyhow::Result<::contracts::CapabilityBatchPlan> {
        self.inner.plan_capability_batch(calls).await
    }
}

#[cfg(test)]
mod receipt_tests {
    use super::*;

    #[tokio::test]
    async fn recording_services_persist_terminal_receipt_as_distinct_item() {
        let services =
            RecordingTurnServices::new(Arc::new(::contracts::StubTurnServices), Vec::new());
        let receipt = ::contracts::CapabilityTerminalReceipt {
            invocation_id: "call".into(),
            operation_id: ::contracts::OperationId::new(),
            process_id: ::contracts::ProcessId::new(),
            capability: "validation_run".into(),
            status: ::contracts::CapabilityTerminalStatus::Succeeded,
            started_at: ::contracts::MonoTime(1),
            finished_at: ::contracts::MonoTime(2),
            exit_code: Some(0),
            error_class: None,
            artifact_ids: Vec::new(),
            evidence_ids: vec!["evidence".into()],
            output_ref: Some("command-session:id".into()),
            truncated: false,
            retry_disposition: ::contracts::CapabilityRetryDisposition::Never,
            audit_id: None,
        };

        services.record_capability_receipt(receipt.clone()).await;

        let items = services.take_items().await;
        assert_eq!(items.len(), 1);
        assert!(matches!(
            &items[0],
            ItemPayload::CapabilityReceipt { receipt: stored } if stored == &receipt
        ));
    }

    #[tokio::test]
    async fn recording_services_persist_model_projection_as_control_item() {
        let services =
            RecordingTurnServices::new(Arc::new(::contracts::StubTurnServices), Vec::new());
        let receipt = ::contracts::model_projection::ModelContextProjectionReceipt {
            inference_id: "inference-1".into(),
            operation_id: "operation-1".into(),
            system_prefix_digest: "sha256:system".into(),
            tool_schema_digest: "sha256:tools".into(),
            role: "worker".into(),
            stage: "execute".into(),
            task_node_id: Some("task-1".into()),
            fragments: vec![::contracts::model_projection::ModelContextFragmentReceipt {
                fragment_id: "fragment-1".into(),
                source: "system_prompt".into(),
                source_version: "version-1".into(),
                artifact_ref: None,
                inline_content: Some("{\"role\":\"system\"}".into()),
                selection_reason: "role_instruction".into(),
                classification:
                    ::contracts::model_projection::ModelContextClassification::Instruction,
                truncated: false,
                bytes: 17,
            }],
            omitted_fragment_ids: Vec::new(),
            message_bytes: 10,
            tool_schema_bytes: 20,
        };

        services
            .record_model_context_projection(receipt.clone())
            .await;

        let items = services.take_items().await;
        let [ItemPayload::ModelContextProjection { receipt: stored }] = items.as_slice() else {
            panic!("projection receipt was not persisted as a distinct item")
        };
        assert!(stored.fragments[0].inline_content.is_none());
        assert!(stored.fragments[0]
            .artifact_ref
            .as_deref()
            .is_some_and(|reference| reference.starts_with("artifact://sha256/")));
    }
}
