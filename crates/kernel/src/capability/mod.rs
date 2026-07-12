//! Capability invoker implementation — Phase 5B.
//!
//! Wraps the admission controller to enforce permit-before-execution for all
//! tool invocations. The production path goes through `DefaultCapabilityInvoker`;
//! direct tool calls that bypass this are forbidden.

use async_trait::async_trait;
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionController, AdmissionRequest, CapabilityInvoker, CapabilityRequest, CapabilityResult,
    CapabilityScope, ExecutionPermit, SandboxDecision, SandboxRequirement,
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
    E: ToolExecutor,
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
    E: ToolExecutor,
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
    E: ToolExecutor,
{
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult {
        // 1. Build admission request.
        let admission_req = AdmissionRequest {
            operation_id: fabric::OperationId::new(),
            process_id: request.process_id,
            principal: fabric::PrincipalId("agent".into()),
            capability: fabric::CapabilityId(request.name.clone()),
            action: request.name.clone(),
            input_summary: format!("{:?}", request.input).chars().take(200).collect(),
            risk: RiskLevel::ReadOnly,
            requested_scope: CapabilityScope::default(),
            budget: None,
            lease: None,
            sandbox: SandboxRequirement::NotRequired,
        };

        // 2. Admit.
        let permit = match self.admission.admit(admission_req).await {
            Ok(p) => p,
            Err(e) => {
                return CapabilityResult {
                    call_id: request.call_id.clone(),
                    output: format!("admission denied: {e}"),
                    is_error: true,
                };
            }
        };

        // 2b. Sandbox check — fail closed.  SandboxFirst mandates that when
        // sandbox infrastructure is unavailable, execution must be denied even
        // if the permit was otherwise granted.  (M0-PR-0E)
        if matches!(permit.sandbox, SandboxDecision::Required) {
            return CapabilityResult {
                call_id: request.call_id.clone(),
                output: format!(
                    "Sandbox required but execution infrastructure not available for '{}'",
                    request.name
                ),
                is_error: true,
            };
        }

        // 3. Execute.
        let result = self.executor.execute_with_permit(&request, &permit).await;

        // 4. Settle (ignore settlement errors in this simplified impl).
        let _ = self.admission.settle(permit.id, Default::default()).await;

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
            call_id: request.call_id.clone(),
            output: format!("stub: executed {}", request.name),
            is_error: false,
        }
    }
}
