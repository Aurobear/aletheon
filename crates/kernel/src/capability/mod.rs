//! Capability invoker implementation — Phase 5B.
//!
//! Wraps the admission controller to enforce permit-before-execution for all
//! tool invocations. The production path goes through `DefaultCapabilityInvoker`;
//! direct tool calls that bypass this are forbidden.

mod invoker;
pub use invoker::CapabilityInvoker;
pub mod governed;
pub mod registry;
pub mod verifier;

use ::contracts::{
    AdmissionRequest, AuditEventId, CapabilityRequest, CapabilityResult, ExecutionPermit,
    SandboxDecision, UsageReport,
};
use async_trait::async_trait;
use std::sync::Arc;

use crate::admission::AdmissionController;

/// Async resources cannot be released directly from `Drop`, so the guard
/// schedules the idempotent admission revoke on the current Tokio runtime.
/// Explicit settle/revoke paths disarm it only after their terminal call has
/// been observed.
struct PermitCleanupGuard<A>
where
    A: AdmissionController + ?Sized + 'static,
{
    admission: Arc<A>,
    permit_id: Option<::contracts::PermitId>,
}

impl<A> PermitCleanupGuard<A>
where
    A: AdmissionController + ?Sized + 'static,
{
    fn new(admission: Arc<A>, permit_id: ::contracts::PermitId) -> Self {
        Self {
            admission,
            permit_id: Some(permit_id),
        }
    }

    fn disarm(&mut self) {
        self.permit_id = None;
    }
}

impl<A> Drop for PermitCleanupGuard<A>
where
    A: AdmissionController + ?Sized + 'static,
{
    fn drop(&mut self) {
        let Some(permit_id) = self.permit_id.take() else {
            return;
        };
        let admission = self.admission.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!(
                permit = %permit_id.0,
                "capability permit cleanup lost its async runtime"
            );
            return;
        };
        runtime.spawn(async move {
            if let Err(error) = admission
                .revoke(permit_id, ::contracts::RevokeReason::OperationCancelled)
                .await
            {
                tracing::warn!(
                    permit = %permit_id.0,
                    %error,
                    "capability permit Drop fallback revoke failed"
                );
            }
        });
    }
}

/// Capability invoker that enforces admission control.
///
/// Every `invoke()` call:
/// 1. Builds an `AdmissionRequest` from the `CapabilityRequest`
/// 2. Calls `admission.admit()` to get an `ExecutionPermit`
/// 3. Executes the actual capability (delegated to inner executor)
/// 4. Calls `admission.settle()` with usage
///
/// If admission is denied, returns a `CapabilityResult` with `is_error = true`
/// and the denial reason as output.
#[derive(Debug)]
pub struct DefaultCapabilityInvoker<A, E>
where
    A: AdmissionController + ?Sized,
    E: ToolExecutor + ?Sized,
{
    admission: Arc<A>,
    executor: Arc<E>,
}

/// Trait for the actual tool execution layer.
///
/// Separated from the admission wrapper so the executor can be tested
/// independently. In production, this delegates to the existing ToolRunner.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool with a valid permit.
    ///
    /// The permit has already been validated by the admission controller.
    /// The executor MUST NOT execute without a permit.
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
    ) -> CapabilityResult;

    /// Additive streaming execution. Legacy executors retain their exact
    /// execution path and emit a terminal-only stream.
    async fn execute_streaming_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        sink: &mut ::contracts::ToolEventSink,
    ) -> CapabilityResult {
        let result = self.execute_with_permit(request, permit).await;
        sink.terminal(Ok(::contracts::ToolResult {
            content: result.output.clone(),
            is_error: result.is_error,
            metadata: ::contracts::ToolResultMeta {
                execution_time_ms: result.usage.wall_time_ms,
                truncated: false,
                patch_delta: result.patch_delta.clone(),
            },
        }))
        .await;
        result
    }
}

/// Canonical construction point for Kernel's admit-execute-settle invoker.
/// Domain adapters supply only the executor and cannot assemble an alternate
/// admission lifecycle.
pub fn canonical_capability_invoker(
    admission: Arc<dyn AdmissionController>,
    executor: Arc<dyn ToolExecutor>,
) -> Arc<dyn CapabilityInvoker> {
    Arc::new(DefaultCapabilityInvoker::new(admission, executor))
}

impl<A, E> DefaultCapabilityInvoker<A, E>
where
    A: AdmissionController + ?Sized,
    E: ToolExecutor + ?Sized,
{
    pub fn new(admission: Arc<A>, executor: Arc<E>) -> Self {
        Self {
            admission,
            executor,
        }
    }
}

#[async_trait]
impl<A, E> CapabilityInvoker for DefaultCapabilityInvoker<A, E>
where
    A: AdmissionController + ?Sized + 'static,
    E: ToolExecutor + ?Sized,
{
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult {
        self.invoke_inner(request, None).await
    }

    async fn invoke_streaming(
        &self,
        request: CapabilityRequest,
        sink: &mut ::contracts::ToolEventSink,
    ) -> CapabilityResult {
        self.invoke_inner(request, Some(sink)).await
    }
}

impl<A, E> DefaultCapabilityInvoker<A, E>
where
    A: AdmissionController + ?Sized + 'static,
    E: ToolExecutor + ?Sized,
{
    async fn invoke_inner(
        &self,
        request: CapabilityRequest,
        mut sink: Option<&mut ::contracts::ToolEventSink>,
    ) -> CapabilityResult {
        // 1. Build admission request.
        let admission_req = AdmissionRequest {
            operation_id: request.call.operation_id,
            process_id: request.call.process_id,
            principal: request.authority.principal.clone(),
            capability: ::contracts::CapabilityId(request.call.name.clone()),
            action: request.authority.action.clone(),
            input_summary: format!("{:?}", request.call.input)
                .chars()
                .take(200)
                .collect(),
            risk: request.authority.risk,
            requested_scope: request.authority.requested_scope.clone(),
            budget: request.authority.budget.clone(),
            lease: request.authority.lease.clone(),
            sandbox: request.authority.sandbox,
        };

        // 2. Admit.
        let permit = match self.admission.admit(admission_req).await {
            Ok(p) => p,
            Err(e) => {
                return CapabilityResult {
                    call_id: request.call.call_id.clone(),
                    output: format!("admission denied: {e}"),
                    is_error: true,
                    usage: UsageReport::default(),
                    audit_id: None,
                    patch_delta: None,
                    served_from_cache: false,
                };
            }
        };
        let mut permit_cleanup = PermitCleanupGuard::new(self.admission.clone(), permit.id);

        // 2b. Sandbox check — fail closed.  SandboxFirst mandates that when
        // sandbox infrastructure is unavailable, execution must be denied even
        // if the permit was otherwise granted.  (M0-PR-0E)
        if matches!(permit.sandbox, SandboxDecision::Required) {
            let _ = self
                .admission
                .revoke(permit.id, ::contracts::RevokeReason::OperationCancelled)
                .await;
            permit_cleanup.disarm();
            return CapabilityResult {
                call_id: request.call.call_id.clone(),
                output: format!(
                    "Sandbox required but execution infrastructure not available for '{}'",
                    request.call.name
                ),
                is_error: true,
                usage: UsageReport {
                    permit_id: permit.id,
                    ..Default::default()
                },
                audit_id: Some(AuditEventId::new()),
                patch_delta: None,
                served_from_cache: false,
            };
        }

        // 3. Execute.
        let execution = async {
            match sink.as_deref_mut() {
                Some(sink) => {
                    self.executor
                        .execute_streaming_with_permit(&request, &permit, sink)
                        .await
                }
                None => self.executor.execute_with_permit(&request, &permit).await,
            }
        };
        let mut result = tokio::select! {
            result = execution => result,
            _ = request.control.cancel.cancelled() => {
                let _ = self
                    .admission
                    .revoke(permit.id, ::contracts::RevokeReason::OperationCancelled)
                    .await;
                permit_cleanup.disarm();
                return CapabilityResult {
                    call_id: request.call.call_id.clone(),
                    output: "capability invocation cancelled".into(),
                    is_error: true,
                    usage: UsageReport { permit_id: permit.id, ..Default::default() },
                    audit_id: Some(AuditEventId::new()),
                    patch_delta: None,
                    served_from_cache: false,
                };
            }
        };
        if let Some(sink) = sink {
            if !sink.terminal_sent() {
                sink.terminal(Ok(::contracts::ToolResult {
                    content: result.output.clone(),
                    is_error: result.is_error,
                    metadata: ::contracts::ToolResultMeta {
                        execution_time_ms: result.usage.wall_time_ms,
                        truncated: false,
                        patch_delta: result.patch_delta.clone(),
                    },
                }))
                .await;
            }
        }
        result.usage.permit_id = permit.id;
        if result.audit_id.is_none() {
            result.audit_id = Some(AuditEventId::new());
        }

        // 4. Settle with the usage emitted by the executor. Settlement failure
        // is returned as a structured capability error so double-settle / budget
        // accounting bugs cannot silently pass.
        if let Err(err) = self.admission.settle(permit.id, result.usage.clone()).await {
            // AlreadySettled is itself an authoritative terminal receipt. For
            // any other controller failure, revoke defensively so a failed
            // settlement cannot retain a live budget or lease hold.
            if !matches!(err, ::contracts::AdmissionError::AlreadySettled) {
                let _ = self
                    .admission
                    .revoke(permit.id, ::contracts::RevokeReason::OperationCancelled)
                    .await;
            }
            permit_cleanup.disarm();
            return CapabilityResult {
                call_id: request.call.call_id.clone(),
                output: format!("settlement failed: {err}"),
                is_error: true,
                usage: result.usage,
                audit_id: result.audit_id,
                patch_delta: result.patch_delta,
                served_from_cache: false,
            };
        }
        permit_cleanup.disarm();

        result
    }
}

// ---------------------------------------------------------------------------
// Stub executor for testing
// ---------------------------------------------------------------------------

/// A stub tool executor that always succeeds.
///
/// Useful for testing the admission → execute → settle pipeline without
/// involving real tool implementations.
pub struct StubToolExecutor;

#[async_trait]
impl ToolExecutor for StubToolExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        _permit: &ExecutionPermit,
    ) -> CapabilityResult {
        CapabilityResult {
            call_id: request.call.call_id.clone(),
            output: format!("stub: executed {}", request.call.name),
            is_error: false,
            usage: UsageReport {
                permit_id: _permit.id,
                output_bytes: request.call.name.len() as u64,
                ..Default::default()
            },
            audit_id: Some(AuditEventId::new()),
            patch_delta: None,
            served_from_cache: false,
        }
    }
}

/// Narrow permit lifecycle for governed system mutations.
#[async_trait::async_trait]
pub trait GovernedPermitIssuer: Send + Sync {
    async fn admit_system_modify(
        &self,
        principal: ::contracts::PrincipalId,
        capability: ::contracts::CapabilityId,
        action: String,
        input_summary: String,
    ) -> anyhow::Result<::contracts::ExecutionPermit>;
    async fn settle_system_modify(
        &self,
        permit: &::contracts::ExecutionPermit,
        success: bool,
    ) -> anyhow::Result<()>;
}

struct KernelPermitIssuer {
    admission: std::sync::Arc<dyn crate::AdmissionController>,
}

pub fn canonical_permit_issuer(
    admission: std::sync::Arc<dyn crate::AdmissionController>,
) -> std::sync::Arc<dyn GovernedPermitIssuer> {
    std::sync::Arc::new(KernelPermitIssuer { admission })
}

#[async_trait::async_trait]
impl GovernedPermitIssuer for KernelPermitIssuer {
    async fn admit_system_modify(
        &self,
        principal: ::contracts::PrincipalId,
        capability: ::contracts::CapabilityId,
        action: String,
        input_summary: String,
    ) -> anyhow::Result<::contracts::ExecutionPermit> {
        self.admission
            .admit(::contracts::AdmissionRequest {
                operation_id: ::contracts::OperationId::new(),
                process_id: ::contracts::ProcessId::new(),
                principal,
                capability,
                action,
                input_summary,
                risk: ::contracts::types::admission::RiskLevel::SystemModify,
                requested_scope: ::contracts::CapabilityScope::default(),
                budget: None,
                lease: None,
                sandbox: ::contracts::SandboxRequirement::NotRequired,
            })
            .await
            .map_err(Into::into)
    }

    async fn settle_system_modify(
        &self,
        permit: &::contracts::ExecutionPermit,
        success: bool,
    ) -> anyhow::Result<()> {
        self.admission
            .settle(
                permit.id,
                ::contracts::UsageReport {
                    permit_id: permit.id,
                    exit_code: Some(if success { 0 } else { 1 }),
                    ..Default::default()
                },
            )
            .await
            .map_err(Into::into)
    }
}
