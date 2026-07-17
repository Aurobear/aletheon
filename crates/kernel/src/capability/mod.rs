//! Capability invoker implementation — Phase 5B.
//!
//! Wraps the admission controller to enforce permit-before-execution for all
//! tool invocations. The production path goes through `DefaultCapabilityInvoker`;
//! direct tool calls that bypass this are forbidden.

use async_trait::async_trait;
use fabric::{
    AdmissionController, AdmissionRequest, AuditEventId, CapabilityInvoker, CapabilityRequest,
    CapabilityResult, ExecutionPermit, SandboxDecision, UsageReport,
};
use std::sync::Arc;

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
    A: AdmissionController + ?Sized,
    E: ToolExecutor + ?Sized,
{
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult {
        // 1. Build admission request.
        let admission_req = AdmissionRequest {
            operation_id: request.call.operation_id,
            process_id: request.call.process_id,
            principal: request.authority.principal.clone(),
            capability: fabric::CapabilityId(request.call.name.clone()),
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
                };
            }
        };

        // 2b. Sandbox check — fail closed.  SandboxFirst mandates that when
        // sandbox infrastructure is unavailable, execution must be denied even
        // if the permit was otherwise granted.  (M0-PR-0E)
        if matches!(permit.sandbox, SandboxDecision::Required) {
            let _ = self
                .admission
                .revoke(permit.id, fabric::RevokeReason::OperationCancelled)
                .await;
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
            };
        }

        // 3. Execute.
        let mut result = tokio::select! {
            result = self.executor.execute_with_permit(&request, &permit) => result,
            _ = request.control.cancel.cancelled() => {
                let _ = self
                    .admission
                    .revoke(permit.id, fabric::RevokeReason::OperationCancelled)
                    .await;
                return CapabilityResult {
                    call_id: request.call.call_id.clone(),
                    output: "capability invocation cancelled".into(),
                    is_error: true,
                    usage: UsageReport { permit_id: permit.id, ..Default::default() },
                    audit_id: Some(AuditEventId::new()),
                };
            }
        };
        result.usage.permit_id = permit.id;
        if result.audit_id.is_none() {
            result.audit_id = Some(AuditEventId::new());
        }

        // 4. Settle with the usage emitted by the executor. Settlement failure
        // is returned as a structured capability error so double-settle / budget
        // accounting bugs cannot silently pass.
        if let Err(err) = self.admission.settle(permit.id, result.usage.clone()).await {
            return CapabilityResult {
                call_id: request.call.call_id.clone(),
                output: format!("settlement failed: {err}"),
                is_error: true,
                usage: result.usage,
                audit_id: result.audit_id,
            };
        }

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
        }
    }
}
