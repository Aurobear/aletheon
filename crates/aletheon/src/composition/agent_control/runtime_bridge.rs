use std::sync::Arc;

use ::contracts::{AgentControlError, AgentControlErrorKind, AgentId, AgentRunStatus};
use async_trait::async_trait;
use kernel::KernelRuntime;

use super::{
    control_error, AgentHostAdapter, AgentRecoveryRuntimeInput, AgentRunProjection,
    AgentRuntimeLauncher, CANCEL_WAIT,
};
use runtime::RuntimeProcessRegistrationPort;

pub struct DurableRuntimeProcessRegistration {
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) repository: Arc<dyn AgentRunProjection>,
    pub(crate) agent_id: AgentId,
    pub(crate) process_id: ::contracts::ProcessId,
}

/// Runtime-owned lifecycle adapter for the rich AgentControl implementation.
/// It is intentionally a one-way bridge: Runtime supplies the already-minted
/// AgentRun identity and this adapter only performs host operations for that
/// identity. It never creates a second run or generation.
pub struct RuntimeObservedAgentBackend {
    pub(crate) host: std::sync::Weak<dyn AgentHostEffects>,
    /// Optional composition-pinned launcher. When present, Runtime admission
    /// invokes this exact backend instead of resolving a launcher from the
    /// legacy registry during the spawn task.
    pub(crate) launcher: Option<Arc<dyn AgentRuntimeLauncher>>,
}

impl RuntimeObservedAgentBackend {
    pub(crate) fn from_host(
        host: std::sync::Weak<dyn AgentHostEffects>,
        launcher: Option<Arc<dyn AgentRuntimeLauncher>>,
    ) -> Self {
        Self { host, launcher }
    }

    pub fn compatibility(host: &Arc<dyn AgentHostEffects>) -> Self {
        Self {
            host: Arc::downgrade(&host),
            launcher: None,
        }
    }

    pub fn pinned(
        host: &Arc<dyn AgentHostEffects>,
        launcher: Arc<dyn AgentRuntimeLauncher>,
    ) -> Self {
        Self {
            host: Arc::downgrade(&host),
            launcher: Some(launcher),
        }
    }
}

/// Narrow host-effects boundary consumed by Runtime's Agent backend adapter.
///
/// Runtime supplies the already-minted run identity and remains the lifecycle,
/// generation, journal and terminal authority. Implementations may perform
/// concrete launch, process, mailbox and projection effects, but cannot select
/// a runtime or mint a replacement identity through this interface.
#[async_trait]
pub trait AgentHostEffects: Send + Sync {
    async fn launch_admitted(
        &self,
        request: ::contracts::AgentSpawnRequest,
        identity: runtime::DelegateReceipt,
        launcher: Option<Arc<dyn AgentRuntimeLauncher>>,
    ) -> Result<(), ::contracts::AgentControlError>;

    async fn cancel_admitted(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<(), runtime::RuntimeError>;

    async fn send_admitted(
        &self,
        agent_run: &runtime::AgentRunId,
        message: &runtime::DelegateMessage,
    ) -> Result<runtime::DelegateMessageReceipt, runtime::RuntimeError>;

    async fn wait_admitted(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<runtime::TurnTerminal, runtime::RuntimeError>;

    async fn resume_admitted(
        &self,
        recovery: &runtime::DelegateRecoveryRequest,
        launcher: Option<Arc<dyn AgentRuntimeLauncher>>,
    ) -> Result<(), runtime::RuntimeError>;
}

#[async_trait]
impl AgentHostEffects for AgentHostAdapter {
    async fn launch_admitted(
        &self,
        request: ::contracts::AgentSpawnRequest,
        identity: runtime::DelegateReceipt,
        launcher: Option<Arc<dyn AgentRuntimeLauncher>>,
    ) -> Result<(), ::contracts::AgentControlError> {
        match launcher {
            Some(launcher) => self
                .spawn_from_runtime_with_launcher(request, identity, launcher)
                .await
                .map(|_| ()),
            None => self.spawn_from_runtime(request, identity).await.map(|_| ()),
        }
    }

    async fn cancel_admitted(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<(), runtime::RuntimeError> {
        let agent_id = parse_runtime_agent_id(agent_run)?;
        let record = self
            .repository
            .get(agent_id)
            .await
            .map_err(|_| runtime::RuntimeError::Internal)?
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?;
        self.cancel_local(record.root_agent_id(), agent_id)
            .await
            .map(|_| ())
            .map_err(|_| runtime::RuntimeError::Internal)
    }

    async fn send_admitted(
        &self,
        agent_run: &runtime::AgentRunId,
        message: &runtime::DelegateMessage,
    ) -> Result<runtime::DelegateMessageReceipt, runtime::RuntimeError> {
        let agent_id = parse_runtime_agent_id(agent_run)?;
        let record = self
            .repository
            .get(agent_id)
            .await
            .map_err(|_| runtime::RuntimeError::Internal)?
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?;
        let kind = match message.kind.to_ascii_lowercase().as_str() {
            "input" => ::contracts::AgentMessageKind::Input,
            "progress" => ::contracts::AgentMessageKind::Progress,
            "result" => ::contracts::AgentMessageKind::Result,
            "signal" => ::contracts::AgentMessageKind::Signal,
            "request" => ::contracts::AgentMessageKind::Request,
            "response" => ::contracts::AgentMessageKind::Response,
            _ => return Err(runtime::RuntimeError::UnsupportedRequest),
        };
        let start_turn = kind == ::contracts::AgentMessageKind::Input;
        let delivery_id = message
            .delivery_id
            .as_deref()
            .map(uuid::Uuid::parse_str)
            .transpose()
            .map_err(|_| runtime::RuntimeError::UnsupportedRequest)?;
        let receipt = self
            .send_local(::contracts::AgentSendRequest {
                caller_root_agent_id: record.root_agent_id(),
                sender_agent_id: None,
                agent_id,
                kind,
                delivery_id,
                correlation_id: message
                    .correlation
                    .as_deref()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok()),
                deadline_mono_ms: None,
                message: message.content.clone(),
                start_turn,
            })
            .await
            .map_err(|_| runtime::RuntimeError::Internal)?;
        Ok(runtime::DelegateMessageReceipt {
            delivery_id: Some(receipt.delivery_id.to_string()),
            sequence: Some(receipt.sequence),
            delivered: receipt.delivery == ::contracts::AgentMessageDeliveryState::Delivered,
        })
    }

    async fn wait_admitted(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<runtime::TurnTerminal, runtime::RuntimeError> {
        let agent_id = parse_runtime_agent_id(agent_run)?;
        let record = self
            .repository
            .get(agent_id)
            .await
            .map_err(|_| runtime::RuntimeError::Internal)?
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?;
        let snapshot = self
            .wait_local(record.root_agent_id(), agent_id, CANCEL_WAIT)
            .await
            .map_err(|_| runtime::RuntimeError::Internal)?;
        Ok(match snapshot.status {
            AgentRunStatus::Succeeded => runtime::TurnTerminal::Completed,
            AgentRunStatus::Cancelled | AgentRunStatus::Interrupted => {
                runtime::TurnTerminal::Interrupted
            }
            AgentRunStatus::Failed => runtime::TurnTerminal::Failed {
                message: snapshot
                    .last_error
                    .unwrap_or_else(|| "Agent runtime failed".into()),
            },
            AgentRunStatus::Queued | AgentRunStatus::Running | AgentRunStatus::Waiting => {
                return Err(runtime::RuntimeError::Timeout)
            }
        })
    }

    async fn resume_admitted(
        &self,
        recovery: &runtime::DelegateRecoveryRequest,
        launcher: Option<Arc<dyn AgentRuntimeLauncher>>,
    ) -> Result<(), runtime::RuntimeError> {
        let agent_id = parse_runtime_agent_id(&recovery.agent_run)?;
        let record = self
            .repository
            .get(agent_id)
            .await
            .map_err(|_| runtime::RuntimeError::Internal)?
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?;
        let Some(launcher) = launcher else {
            let supervisor = self
                .runtime_agent_supervisor
                .as_ref()
                .ok_or(runtime::RuntimeError::UnsupportedRequest)?;
            let backend = supervisor
                .registry()
                .resolve(&runtime::DelegateBackendId(
                    record.snapshot.handle.runtime_id.0.clone(),
                ))
                .ok_or(runtime::RuntimeError::AgentRunNotFound)?;
            return backend.resume_from_checkpoint(recovery).await;
        };
        if launcher.resumability() != record.resumability {
            return Err(runtime::RuntimeError::UnsupportedRequest);
        }
        launcher
            .resume_from_checkpoint(AgentRecoveryRuntimeInput {
                handle: record.snapshot.handle,
                request: record.request,
                checkpoint_reference: recovery.checkpoint_reference.clone(),
            })
            .await
            .map_err(|_| runtime::RuntimeError::Internal)
    }
}

#[async_trait]
impl runtime::DelegateBackend for RuntimeObservedAgentBackend {
    async fn spawn(
        &self,
        request: &runtime::DelegateSpawnRequest,
        identity: &runtime::DelegateReceipt,
    ) -> Result<runtime::DelegateReceipt, runtime::RuntimeError> {
        let host = self
            .host
            .upgrade()
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?;
        let host_request = request
            .host_request
            .clone()
            .ok_or(runtime::RuntimeError::UnsupportedRequest)?;
        let result = host
            .launch_admitted(host_request, identity.clone(), self.launcher.clone())
            .await;
        result.map_err(|error| {
            tracing::error!(
                agent_run = %identity.agent_run.0,
                kind = ?error.kind,
                message = %error.message,
                "Runtime Delegate host admission failed"
            );
            runtime::RuntimeError::Internal
        })?;
        Ok(identity.clone())
    }

    async fn cancel(&self, agent_run: &runtime::AgentRunId) -> Result<(), runtime::RuntimeError> {
        <Self as runtime::ObservedAgentBackend>::cancel(self, agent_run).await
    }

    async fn send(
        &self,
        agent_run: &runtime::AgentRunId,
        message: &runtime::DelegateMessage,
    ) -> Result<(), runtime::RuntimeError> {
        <Self as runtime::ObservedAgentBackend>::send(self, agent_run, message).await
    }

    async fn send_with_receipt(
        &self,
        agent_run: &runtime::AgentRunId,
        message: &runtime::DelegateMessage,
    ) -> Result<runtime::DelegateMessageReceipt, runtime::RuntimeError> {
        <Self as runtime::ObservedAgentBackend>::send_with_receipt(self, agent_run, message).await
    }

    async fn wait(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<runtime::TurnTerminal, runtime::RuntimeError> {
        <Self as runtime::ObservedAgentBackend>::wait(self, agent_run).await
    }

    async fn resume_from_checkpoint(
        &self,
        recovery: &runtime::DelegateRecoveryRequest,
    ) -> Result<(), runtime::RuntimeError> {
        self.host
            .upgrade()
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?
            .resume_admitted(recovery, self.launcher.clone())
            .await
    }
}

#[async_trait]
impl runtime::ObservedAgentBackend for RuntimeObservedAgentBackend {
    async fn cancel(&self, agent_run: &runtime::AgentRunId) -> Result<(), runtime::RuntimeError> {
        self.host
            .upgrade()
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?
            .cancel_admitted(agent_run)
            .await
    }

    async fn send(
        &self,
        agent_run: &runtime::AgentRunId,
        message: &runtime::DelegateMessage,
    ) -> Result<(), runtime::RuntimeError> {
        self.send_with_receipt(agent_run, message).await.map(|_| ())
    }

    async fn send_with_receipt(
        &self,
        agent_run: &runtime::AgentRunId,
        message: &runtime::DelegateMessage,
    ) -> Result<runtime::DelegateMessageReceipt, runtime::RuntimeError> {
        self.host
            .upgrade()
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?
            .send_admitted(agent_run, message)
            .await
    }

    async fn wait(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<runtime::TurnTerminal, runtime::RuntimeError> {
        self.host
            .upgrade()
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?
            .wait_admitted(agent_run)
            .await
    }

    async fn resume_from_checkpoint(
        &self,
        recovery: &runtime::DelegateRecoveryRequest,
    ) -> Result<(), runtime::RuntimeError> {
        // Reuse the same pinned launcher validation as the normal Runtime
        // backend path.  After a daemon restart the process-local binding is
        // intentionally absent, so Runtime routes recovery through this
        // observed host adapter instead of selecting a new launcher.
        <Self as runtime::DelegateBackend>::resume_from_checkpoint(self, recovery).await
    }
}

fn parse_runtime_agent_id(
    agent_run: &runtime::AgentRunId,
) -> Result<AgentId, runtime::RuntimeError> {
    uuid::Uuid::parse_str(&agent_run.0)
        .map(AgentId)
        .map_err(|_| runtime::RuntimeError::AgentRunNotFound)
}

impl std::fmt::Debug for DurableRuntimeProcessRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableRuntimeProcessRegistration")
            .field("agent_id", &self.agent_id)
            .field("process_id", &self.process_id)
            .finish()
    }
}

#[async_trait]
impl RuntimeProcessRegistrationPort for DurableRuntimeProcessRegistration {
    async fn register(
        &self,
        os_pid: ::contracts::OsProcessId,
        start_time_ticks: u64,
    ) -> Result<::contracts::RuntimeProcessId, AgentControlError> {
        if start_time_ticks == 0 {
            return Err(control_error(
                AgentControlErrorKind::InvalidRequest,
                "runtime process start identity must be nonzero",
            ));
        }
        let logical = self
            .kernel
            .bind_os_process_id(self.process_id, os_pid)
            .await
            .map_err(|error| control_error(AgentControlErrorKind::Conflict, error.to_string()))?;
        if logical.agent_id != self.agent_id {
            return Err(control_error(
                AgentControlErrorKind::Conflict,
                "runtime process registration crossed Agent authority",
            ));
        }
        let identity = ::contracts::RuntimeProcessId {
            agent_id: logical.agent_id,
            process_id: logical.process_id,
            generation: logical.generation,
            os_pid,
            start_time_ticks,
        };
        self.repository.put_runtime_process(&identity).await?;
        Ok(identity)
    }

    async fn clear(
        &self,
        identity: ::contracts::RuntimeProcessId,
    ) -> Result<(), AgentControlError> {
        if identity.agent_id != self.agent_id || identity.process_id != self.process_id {
            return Err(control_error(
                AgentControlErrorKind::Forbidden,
                "runtime process clear crossed Agent authority",
            ));
        }
        self.repository.clear_runtime_process(&identity).await?;
        Ok(())
    }
}
