//! Composition-only adapter from extension providers to Aletheon's legacy
//! host launcher seam.
//!
//! The extension crate owns provider routing and deliberately does not know
//! Aletheon. This adapter is assembled at the Aletheon composition root,
//! where the compatibility host lifecycle is still available.

use std::sync::Arc;

use crate::composition::agent_control::{
    AgentEventSink, AgentRuntimeEvent, AgentRuntimeInput, AgentRuntimeLauncher,
};
use async_trait::async_trait;

pub struct ExtensionProviderLauncher {
    router: Arc<crate::extensions::extension_runtime_router::ExtensionRuntimeRouter>,
}

impl ExtensionProviderLauncher {
    pub fn new(
        router: Arc<crate::extensions::extension_runtime_router::ExtensionRuntimeRouter>,
    ) -> Self {
        Self { router }
    }
}

fn runtime_error(error: impl std::fmt::Display) -> ::contracts::AgentControlError {
    ::contracts::AgentControlError {
        kind: ::contracts::AgentControlErrorKind::Runtime,
        message: error.to_string(),
    }
}

#[async_trait]
impl AgentRuntimeLauncher for ExtensionProviderLauncher {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<::contracts::AgentResult, ::contracts::AgentControlError> {
        events
            .emit(AgentRuntimeEvent::Started {
                agent_id: input.handle.agent_id,
                process_id: input.handle.process_id,
                operation_id: input.handle.operation_id,
            })
            .await;
        let runtime_handle = self
            .router
            .start(input.request)
            .await
            .map_err(runtime_error)?;
        let value = tokio::select! {
            result = self.router.wait(&runtime_handle) => result.map_err(runtime_error)?,
            _ = input.cancellation.cancelled() => {
                self.router
                    .cancel(&runtime_handle, "executive cancellation")
                    .await
                    .map_err(runtime_error)?;
                events
                    .emit(AgentRuntimeEvent::Terminal {
                        agent_id: input.handle.agent_id,
                        process_id: input.handle.process_id,
                        operation_id: input.handle.operation_id,
                        status: ::contracts::AgentRunStatus::Cancelled,
                        result: None,
                    })
                    .await;
                return Err(::contracts::AgentControlError {
                    kind: ::contracts::AgentControlErrorKind::Terminal,
                    message: "extension runtime was cancelled".into(),
                });
            }
        };
        let result: ::contracts::AgentResult =
            serde_json::from_value(value).map_err(runtime_error)?;
        result.validate()?;
        events
            .emit(AgentRuntimeEvent::Terminal {
                agent_id: input.handle.agent_id,
                process_id: input.handle.process_id,
                operation_id: input.handle.operation_id,
                status: ::contracts::AgentRunStatus::Succeeded,
                result: Some(result.clone()),
            })
            .await;
        Ok(result)
    }
}
