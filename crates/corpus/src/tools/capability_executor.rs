use std::{collections::HashMap, sync::Arc};

use aletheon_kernel::capability::ToolExecutor;
use async_trait::async_trait;
use fabric::{
    AuditEventId, CapabilityId, CapabilityRequest, CapabilityResult, Clock, ExecutionPermit,
    ToolContext, UsageReport,
};

use crate::{CorpusError, ExtensionDescriptor, ExtensionKind};
use crate::{ToolRegistry, ToolRunnerWithGuard};

pub fn default_tool_registry() -> Arc<tokio::sync::Mutex<ToolRegistry>> {
    Arc::new(tokio::sync::Mutex::new(ToolRegistry::default()))
}

/// Snapshot trusted tool risk metadata without exposing registry access.
pub async fn tool_risk_levels(
    registry: &Arc<tokio::sync::Mutex<ToolRegistry>>,
) -> HashMap<String, fabric::types::admission::RiskLevel> {
    let registry = registry.lock().await;
    registry
        .definitions()
        .into_iter()
        .filter_map(|definition| {
            let tool = registry.get(&definition.name)?;
            let risk = match tool.permission_level() {
                fabric::tool::PermissionLevel::L0 => fabric::types::admission::RiskLevel::ReadOnly,
                fabric::tool::PermissionLevel::L1 => fabric::types::admission::RiskLevel::Sandboxed,
                fabric::tool::PermissionLevel::L2 => {
                    fabric::types::admission::RiskLevel::SystemModify
                }
                fabric::tool::PermissionLevel::L3 => {
                    fabric::types::admission::RiskLevel::Destructive
                }
            };
            Some((definition.name, risk))
        })
        .collect()
}

/// Discover deterministic tool descriptors without activating any tool.
pub async fn discover_tool_extensions(
    registry: &Arc<tokio::sync::Mutex<ToolRegistry>>,
) -> Result<Vec<ExtensionDescriptor>, CorpusError> {
    let registry = registry.lock().await;
    let mut definitions = registry.definitions();
    definitions.sort_by(|left, right| left.name.cmp(&right.name));
    definitions
        .into_iter()
        .map(|definition| {
            let tool = registry
                .get(&definition.name)
                .ok_or_else(|| CorpusError::InvalidDescriptor(definition.name.clone()))?;
            let descriptor = ExtensionDescriptor::new(
                ExtensionKind::Tool,
                &definition.name,
                env!("CARGO_PKG_VERSION"),
                definition.description.clone(),
                CapabilityId(definition.name.clone()),
                permission_risk(tool.permission_level()),
            )
            .map_err(|error| CorpusError::InvalidDescriptor(error.to_string()))?;
            descriptor
                .with_tool_definition(definition)
                .map_err(|error| CorpusError::InvalidDescriptor(error.to_string()))
        })
        .collect()
}

fn permission_risk(level: fabric::tool::PermissionLevel) -> fabric::types::admission::RiskLevel {
    match level {
        fabric::tool::PermissionLevel::L0 => fabric::types::admission::RiskLevel::ReadOnly,
        fabric::tool::PermissionLevel::L1 => fabric::types::admission::RiskLevel::Sandboxed,
        fabric::tool::PermissionLevel::L2 => fabric::types::admission::RiskLevel::SystemModify,
        fabric::tool::PermissionLevel::L3 => fabric::types::admission::RiskLevel::Destructive,
    }
}

pub struct CorpusToolExecutor {
    registry: Arc<tokio::sync::Mutex<ToolRegistry>>,
    runner: Arc<tokio::sync::Mutex<ToolRunnerWithGuard>>,
    clock: Arc<dyn Clock>,
}

impl CorpusToolExecutor {
    pub fn new(
        registry: Arc<tokio::sync::Mutex<ToolRegistry>>,
        runner: Arc<tokio::sync::Mutex<ToolRunnerWithGuard>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            registry,
            runner,
            clock,
        }
    }

    fn error_result(
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        message: impl Into<String>,
        audit_id: AuditEventId,
    ) -> CapabilityResult {
        CapabilityResult {
            call_id: request.call.call_id.clone(),
            output: message.into(),
            is_error: true,
            usage: UsageReport {
                permit_id: permit.id,
                exit_code: Some(1),
                ..Default::default()
            },
            audit_id: Some(audit_id),
        }
    }

    fn validate(
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        now: fabric::MonoTime,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            permit.operation_id == request.call.operation_id
                && permit.process_id == request.call.process_id
                && permit.capability == CapabilityId(request.call.name.clone()),
            "permit does not bind request"
        );
        anyhow::ensure!(
            permit.is_valid_at(now),
            "permit expired or sandbox unavailable"
        );
        Ok(())
    }
}

#[async_trait]
impl ToolExecutor for CorpusToolExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
    ) -> CapabilityResult {
        if let Err(error) = Self::validate(request, permit, self.clock.mono_now()) {
            return Self::error_result(request, permit, error.to_string(), AuditEventId::new());
        }

        let tool = {
            let registry = self.registry.lock().await;
            registry.get(&request.call.name).cloned()
        };
        let Some(tool) = tool else {
            return Self::error_result(
                request,
                permit,
                format!("tool not found: {}", request.call.name),
                AuditEventId::new(),
            );
        };

        let context = ToolContext {
            agent: request.authority.agent,
            approval_authority: Some(fabric::ToolApprovalAuthority {
                principal_id: request.authority.principal.clone(),
                connection_id: request.authority.connection_id.clone(),
                thread_id: request.authority.thread_id.clone(),
                turn_id: request.authority.turn_id,
                call_id: request.call.call_id.clone(),
                workspace: request.authority.workspace.clone(),
            }),
            working_dir: request.authority.working_dir.clone(),
            session_id: request.authority.session_id.clone(),
            clock: self.clock.clone(),
        };
        let started = self.clock.mono_now();
        let report = self
            .runner
            .lock()
            .await
            .execute_tool_report(
                tool.as_ref(),
                request.call.input.clone(),
                &context,
                &request.authority.turn_id.0.to_string(),
            )
            .await;

        match report.result {
            Ok(result) => {
                let wall_time_ms = if result.metadata.execution_time_ms == 0 {
                    self.clock.mono_now().0.saturating_sub(started.0)
                } else {
                    result.metadata.execution_time_ms
                };
                let output_bytes = result.content.len() as u64;
                CapabilityResult {
                    call_id: request.call.call_id.clone(),
                    output: result.content,
                    is_error: result.is_error,
                    usage: UsageReport {
                        permit_id: permit.id,
                        wall_time_ms,
                        output_bytes,
                        exit_code: Some(if result.is_error { 1 } else { 0 }),
                        ..Default::default()
                    },
                    audit_id: Some(report.audit_id),
                }
            }
            Err(error) => Self::error_result(request, permit, error.to_string(), report.audit_id),
        }
    }

    async fn execute_streaming_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        sink: &mut fabric::ToolEventSink,
    ) -> CapabilityResult {
        if let Err(error) = Self::validate(request, permit, self.clock.mono_now()) {
            return Self::error_result(request, permit, error.to_string(), AuditEventId::new());
        }

        let tool = {
            let registry = self.registry.lock().await;
            registry.get(&request.call.name).cloned()
        };
        let Some(tool) = tool else {
            return Self::error_result(
                request,
                permit,
                format!("tool not found: {}", request.call.name),
                AuditEventId::new(),
            );
        };

        let context = ToolContext {
            agent: request.authority.agent,
            approval_authority: Some(fabric::ToolApprovalAuthority {
                principal_id: request.authority.principal.clone(),
                connection_id: request.authority.connection_id.clone(),
                thread_id: request.authority.thread_id.clone(),
                turn_id: request.authority.turn_id,
                call_id: request.call.call_id.clone(),
                workspace: request.authority.workspace.clone(),
            }),
            working_dir: request.authority.working_dir.clone(),
            session_id: request.authority.session_id.clone(),
            clock: self.clock.clone(),
        };
        let started = self.clock.mono_now();
        let report = self
            .runner
            .lock()
            .await
            .execute_tool_streaming_report(
                tool.as_ref(),
                request.call.input.clone(),
                &context,
                &request.authority.turn_id.0.to_string(),
                sink,
            )
            .await;

        match report.result {
            Ok(result) => {
                let wall_time_ms = if result.metadata.execution_time_ms == 0 {
                    self.clock.mono_now().0.saturating_sub(started.0)
                } else {
                    result.metadata.execution_time_ms
                };
                CapabilityResult {
                    call_id: request.call.call_id.clone(),
                    output: result.content.clone(),
                    is_error: result.is_error,
                    usage: UsageReport {
                        permit_id: permit.id,
                        wall_time_ms,
                        output_bytes: result.content.len() as u64,
                        exit_code: Some(if result.is_error { 1 } else { 0 }),
                        ..Default::default()
                    },
                    audit_id: Some(report.audit_id),
                }
            }
            Err(error) => Self::error_result(request, permit, error.to_string(), report.audit_id),
        }
    }
}
