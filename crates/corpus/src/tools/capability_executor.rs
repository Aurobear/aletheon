use std::{collections::HashMap, sync::Arc, time::Duration};

use ::contracts::tool::ToolCachePolicy;
use ::contracts::{
    AuditEventId, CapabilityId, CapabilityRequest, CapabilityResult, Clock, ExecutionPermit,
    ToolContext, ToolResult, ToolResultMeta, UsageReport,
};
use async_trait::async_trait;
use kernel::capability::ToolExecutor;

use crate::tools::read_only_cache::{
    cache_ttl, dependency_fingerprint, is_cacheable, read_only_cache_key, ReadOnlyToolResultCache,
    ToolCacheIdentity, ToolCacheScope,
};
use crate::{CorpusError, ExtensionDescriptor, ExtensionKind};
use crate::{ToolRegistry, ToolRunnerWithGuard};

pub fn default_tool_registry() -> Arc<tokio::sync::Mutex<ToolRegistry>> {
    Arc::new(tokio::sync::Mutex::new(ToolRegistry::default()))
}

/// Snapshot trusted tool risk metadata without exposing registry access.
pub async fn tool_risk_levels(
    registry: &Arc<tokio::sync::Mutex<ToolRegistry>>,
) -> HashMap<String, ::contracts::types::admission::RiskLevel> {
    let registry = registry.lock().await;
    registry
        .definitions()
        .into_iter()
        .filter_map(|definition| {
            let tool = registry.get(&definition.name)?;
            let risk = match tool.permission_level() {
                ::contracts::tool::PermissionLevel::L0 => {
                    ::contracts::types::admission::RiskLevel::ReadOnly
                }
                ::contracts::tool::PermissionLevel::L1 => {
                    ::contracts::types::admission::RiskLevel::Sandboxed
                }
                ::contracts::tool::PermissionLevel::L2 => {
                    ::contracts::types::admission::RiskLevel::SystemModify
                }
                ::contracts::tool::PermissionLevel::L3 => {
                    ::contracts::types::admission::RiskLevel::Destructive
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
    // Corpus activation and the active Agent profile provide the real
    // disclosure boundary. Index every non-hidden tool here so a profile that
    // explicitly grants a deferred tool can actually receive and invoke it.
    let definitions = registry.profile_definitions();
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

fn permission_risk(
    level: ::contracts::tool::PermissionLevel,
) -> ::contracts::types::admission::RiskLevel {
    match level {
        ::contracts::tool::PermissionLevel::L0 => {
            ::contracts::types::admission::RiskLevel::ReadOnly
        }
        ::contracts::tool::PermissionLevel::L1 => {
            ::contracts::types::admission::RiskLevel::Sandboxed
        }
        ::contracts::tool::PermissionLevel::L2 => {
            ::contracts::types::admission::RiskLevel::SystemModify
        }
        ::contracts::tool::PermissionLevel::L3 => {
            ::contracts::types::admission::RiskLevel::Destructive
        }
    }
}

pub struct CorpusToolExecutor {
    registry: Arc<tokio::sync::Mutex<ToolRegistry>>,
    runner: Arc<tokio::sync::Mutex<ToolRunnerWithGuard>>,
    clock: Arc<dyn Clock>,
    /// Bounded read-only result cache (Phase C7). Only tools that declare a
    /// non-`Never` policy AND are L0 are consulted; see `read_only_cache`.
    read_only_cache: Arc<ReadOnlyToolResultCache>,
    read_only_cache_config: ToolResultCacheConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolResultCacheConfig {
    pub enabled: bool,
    pub capacity: usize,
    pub max_ttl: Duration,
}

impl Default for ToolResultCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            capacity: 256,
            max_ttl: Duration::from_secs(300),
        }
    }
}

impl ToolResultCacheConfig {
    fn governed(self) -> Self {
        Self {
            enabled: self.enabled,
            capacity: self.capacity.clamp(1, 4096),
            max_ttl: self
                .max_ttl
                .clamp(Duration::from_millis(1), Duration::from_secs(3600)),
        }
    }
}

impl CorpusToolExecutor {
    pub fn new(
        registry: Arc<tokio::sync::Mutex<ToolRegistry>>,
        runner: Arc<tokio::sync::Mutex<ToolRunnerWithGuard>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::new_with_cache_config(registry, runner, clock, ToolResultCacheConfig::default())
    }

    pub fn new_with_cache_config(
        registry: Arc<tokio::sync::Mutex<ToolRegistry>>,
        runner: Arc<tokio::sync::Mutex<ToolRunnerWithGuard>>,
        clock: Arc<dyn Clock>,
        cache_config: ToolResultCacheConfig,
    ) -> Self {
        let cache_config = cache_config.governed();
        Self {
            registry,
            runner,
            clock,
            read_only_cache: Arc::new(ReadOnlyToolResultCache::new(cache_config.capacity)),
            read_only_cache_config: cache_config,
        }
    }

    pub fn read_only_cache_metrics(
        &self,
    ) -> crate::tools::read_only_cache::ToolCacheMetricsSnapshot {
        self.read_only_cache.metrics_snapshot()
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
            patch_delta: None,
            served_from_cache: false,
        }
    }

    fn validate(
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        now: ::contracts::MonoTime,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !request.control.cancel.is_cancelled(),
            "capability invocation cancelled before execution"
        );
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

    fn cache_key(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        tool: &dyn ::contracts::Tool,
        cache_policy: ToolCachePolicy,
    ) -> Result<Option<String>, String> {
        if !self.read_only_cache_config.enabled
            || !is_cacheable(cache_policy, tool.permission_level())
        {
            return Ok(None);
        }
        let dependencies = tool.cache_dependencies().ok_or_else(|| {
            format!(
                "tool '{}' declares caching without dependencies",
                tool.name()
            )
        })?;
        let dependency = dependency_fingerprint(
            dependencies,
            &request.call.input,
            &request.authority.workspace,
        )?;
        let schema_digest = ::contracts::tool_schema_digest(&[::contracts::ToolDefinition {
            name: tool.name().to_owned(),
            description: tool.description().to_owned(),
            input_schema: tool.input_schema(),
        }])
        .map_err(|error| error.to_string())?;
        let authority_context = serde_json::to_string(&serde_json::json!({
            "permission_mode": request.authority.permission_mode,
            "requested_scope": request.authority.requested_scope,
            "granted_scope": permit.granted_scope,
            "sandbox_requirement": request.authority.sandbox,
            "sandbox_decision": permit.sandbox,
        }))
        .map_err(|error| format!("cache authority serialization failed: {error}"))?;
        Ok(Some({
            let workspace = serde_json::to_string(&request.authority.workspace)
                .expect("workspace cache identity serializes");
            read_only_cache_key(
                ToolCacheIdentity {
                    tool_name: tool.name(),
                    impl_version: &format!(
                        "corpus/{}/{}",
                        env!("CARGO_PKG_VERSION"),
                        tool.cache_implementation_version()
                    ),
                    schema_digest: &schema_digest,
                    dependency_fingerprint: &dependency,
                    authority_context: &authority_context,
                },
                &request.call.input,
                ToolCacheScope {
                    workspace: &workspace,
                    principal: &request.authority.principal.0,
                    session: &request.authority.session_id,
                    turn: &request.authority.turn_id.0.to_string(),
                },
                cache_policy,
            )
        }))
    }

    fn resolve_cache_key(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        tool: &dyn ::contracts::Tool,
        cache_policy: ToolCachePolicy,
    ) -> Option<String> {
        match self.cache_key(request, permit, tool, cache_policy) {
            Ok(Some(key)) => Some(key),
            Ok(None) => {
                self.read_only_cache.record_bypass(tool.name());
                None
            }
            Err(error) => {
                tracing::debug!(tool = tool.name(), %error, "read-only tool cache bypassed");
                self.read_only_cache.record_rejected_candidate(tool.name());
                None
            }
        }
    }

    fn effective_cache_ttl(&self, policy: ToolCachePolicy) -> Option<Duration> {
        cache_ttl(policy).map(|ttl| ttl.min(self.read_only_cache_config.max_ttl))
    }

    async fn emit_stream_terminal_error(
        sink: &mut ::contracts::ToolEventSink,
        error: ::contracts::ToolExecutionError,
    ) {
        if !sink.terminal_sent() {
            sink.terminal(Err(error)).await;
        }
    }

    fn terminal_error_for(message: &str) -> ::contracts::ToolExecutionError {
        if message == "capability invocation cancelled before execution" {
            ::contracts::ToolExecutionError::Cancelled(message.to_owned())
        } else {
            ::contracts::ToolExecutionError::Failed(message.to_owned())
        }
    }

    async fn authorize_cache_hit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        tool: &dyn ::contracts::Tool,
        context: &ToolContext,
        hit: &mut CapabilityResult,
    ) -> bool {
        let cached_tool_result = ToolResult {
            content: hit.output.clone(),
            is_error: hit.is_error,
            metadata: ToolResultMeta::default(),
        };
        let audit_id = self
            .runner
            .lock()
            .await
            .record_read_only_cache_hit(
                tool,
                &request.call.input,
                context,
                &request.authority.turn_id.0.to_string(),
                &cached_tool_result,
            )
            .await;
        let Ok(audit_id) = audit_id else {
            tracing::warn!(
                tool = tool.name(),
                error = %audit_id.unwrap_err(),
                "cached result rejected by current policy/audit; executing authoritative tool"
            );
            self.read_only_cache.record_rejected_candidate(tool.name());
            return false;
        };
        let saved_latency_ms = hit.usage.wall_time_ms;
        let saved_output_bytes = hit.usage.output_bytes;
        hit.call_id = request.call.call_id.clone();
        hit.usage = UsageReport {
            permit_id: permit.id,
            output_bytes: hit.output.len() as u64,
            exit_code: Some(if hit.is_error { 1 } else { 0 }),
            ..Default::default()
        };
        hit.audit_id = Some(audit_id);
        hit.patch_delta = None;
        hit.served_from_cache = true;
        self.read_only_cache
            .record_hit(tool.name(), saved_latency_ms, saved_output_bytes);
        true
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
            agent: request.authority.agent.clone(),
            approval_authority: Some(::contracts::ToolApprovalAuthority {
                principal_id: request.authority.principal.clone(),
                connection_id: request.authority.connection_id.clone(),
                thread_id: request.authority.thread_id.clone(),
                turn_id: request.authority.turn_id,
                call_id: request.call.call_id.clone(),
                workspace: request.authority.workspace.clone(),
                granted_scope: permit.granted_scope.clone(),
                permission_mode: request.authority.permission_mode,
            }),
            working_dir: request.authority.working_dir.clone(),
            session_id: request.authority.session_id.clone(),
            clock: self.clock.clone(),
            turn_event_sender: request.control.turn_event_sender.clone(),
        };
        // Phase C7 read-only result cache. Only consulted for tools that
        // explicitly declare a non-`Never` policy AND are L0 — the permission
        // gate above already ran, and this cache never bypasses it. A hit is
        // returned as an auditable CapabilityResult marked served_from_cache.
        let cache_policy = tool.cache_policy();
        let cache_key = self.resolve_cache_key(request, permit, tool.as_ref(), cache_policy);
        if let Some(key) = &cache_key {
            if let Some(mut hit) = self.read_only_cache.get(tool.name(), key) {
                if !self
                    .authorize_cache_hit(request, permit, tool.as_ref(), &context, &mut hit)
                    .await
                {
                    // Current policy or audit rejected this candidate. Continue
                    // through the authoritative runner below.
                } else {
                    tracing::info!(
                        tool = tool.name(),
                        "read-only tool result served from cache (underlying tool not executed)"
                    );
                    return hit;
                }
            }
        }
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
                let result_truncated = result.metadata.truncated;
                let wall_time_ms = if result.metadata.execution_time_ms == 0 {
                    self.clock.mono_now().0.saturating_sub(started.0)
                } else {
                    result.metadata.execution_time_ms
                };
                let output_bytes = result.content.len() as u64;
                let capability = CapabilityResult {
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
                    patch_delta: result.metadata.patch_delta,
                    served_from_cache: false,
                };
                // Store successful read-only results under their cache key so a
                // later identical call is served without executing the tool.
                if !capability.is_error
                    && capability.patch_delta.is_none()
                    && !result_truncated
                    && !request.control.cancel.is_cancelled()
                {
                    if let Some(key) = cache_key {
                        if let Some(ttl) = self.effective_cache_ttl(cache_policy) {
                            let mut cached = capability.clone();
                            cached.served_from_cache = true;
                            self.read_only_cache.insert(key, cached, ttl);
                        }
                    }
                }
                capability
            }
            Err(error) => Self::error_result(request, permit, error.to_string(), report.audit_id),
        }
    }

    async fn execute_streaming_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        sink: &mut ::contracts::ToolEventSink,
    ) -> CapabilityResult {
        if let Err(error) = Self::validate(request, permit, self.clock.mono_now()) {
            let message = error.to_string();
            Self::emit_stream_terminal_error(sink, Self::terminal_error_for(&message)).await;
            return Self::error_result(request, permit, message, AuditEventId::new());
        }

        let tool = {
            let registry = self.registry.lock().await;
            registry.get(&request.call.name).cloned()
        };
        let Some(tool) = tool else {
            let message = format!("tool not found: {}", request.call.name);
            Self::emit_stream_terminal_error(
                sink,
                ::contracts::ToolExecutionError::Failed(message.clone()),
            )
            .await;
            return Self::error_result(request, permit, message, AuditEventId::new());
        };

        let context = ToolContext {
            agent: request.authority.agent.clone(),
            approval_authority: Some(::contracts::ToolApprovalAuthority {
                principal_id: request.authority.principal.clone(),
                connection_id: request.authority.connection_id.clone(),
                thread_id: request.authority.thread_id.clone(),
                turn_id: request.authority.turn_id,
                call_id: request.call.call_id.clone(),
                workspace: request.authority.workspace.clone(),
                granted_scope: permit.granted_scope.clone(),
                permission_mode: request.authority.permission_mode,
            }),
            working_dir: request.authority.working_dir.clone(),
            session_id: request.authority.session_id.clone(),
            clock: self.clock.clone(),
            turn_event_sender: request.control.turn_event_sender.clone(),
        };
        let cache_policy = tool.cache_policy();
        let cache_key = self.resolve_cache_key(request, permit, tool.as_ref(), cache_policy);
        if let Some(key) = &cache_key {
            if let Some(mut hit) = self.read_only_cache.get(tool.name(), key) {
                if self
                    .authorize_cache_hit(request, permit, tool.as_ref(), &context, &mut hit)
                    .await
                {
                    if !sink.terminal_sent() {
                        sink.terminal(Ok(ToolResult {
                            content: hit.output.clone(),
                            is_error: hit.is_error,
                            metadata: ToolResultMeta::default(),
                        }))
                        .await;
                    }
                    tracing::info!(
                        tool = tool.name(),
                        "streaming read-only tool result served from cache (underlying tool not executed)"
                    );
                    return hit;
                }
            }
        }
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
                let result_truncated = result.metadata.truncated;
                let wall_time_ms = if result.metadata.execution_time_ms == 0 {
                    self.clock.mono_now().0.saturating_sub(started.0)
                } else {
                    result.metadata.execution_time_ms
                };
                let capability = CapabilityResult {
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
                    patch_delta: result.metadata.patch_delta,
                    served_from_cache: false,
                };
                if !capability.is_error
                    && capability.patch_delta.is_none()
                    && !result_truncated
                    && !request.control.cancel.is_cancelled()
                {
                    if let Some(key) = cache_key {
                        if let Some(ttl) = self.effective_cache_ttl(cache_policy) {
                            let mut cached = capability.clone();
                            cached.served_from_cache = true;
                            self.read_only_cache.insert(key, cached, ttl);
                        }
                    }
                }
                capability
            }
            Err(error) => {
                let message = error.to_string();
                Self::emit_stream_terminal_error(
                    sink,
                    ::contracts::ToolExecutionError::Failed(message.clone()),
                )
                .await;
                Self::error_result(request, permit, message, report.audit_id)
            }
        }
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[tokio::test]
    async fn discovery_includes_deferred_non_hidden_tools() {
        let descriptors = discover_tool_extensions(&default_tool_registry())
            .await
            .unwrap();
        assert!(descriptors.iter().any(|descriptor| {
            descriptor
                .primary_capability()
                .is_some_and(|capability| capability.0.as_str() == "artifact_read")
        }));
    }
}
