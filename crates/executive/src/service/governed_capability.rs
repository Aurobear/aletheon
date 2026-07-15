//! Executive-owned authorization boundary for cognitive capability calls.
//!
//! Cognit supplies only a [`CapabilityCall`].  This module attaches trusted
//! application authority and cancellation before delegating to Kernel's
//! admit/execute/settle pipeline.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use aletheon_kernel::capability::{DefaultCapabilityInvoker, ToolExecutor};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionController, CapabilityAuthority, CapabilityCall, CapabilityInvoker, CapabilityResult,
    CapabilityScope, InvocationControl, PrincipalId, SandboxRequirement, UsageReport,
};
use tokio_util::sync::CancellationToken;

/// Trusted execution context attached by Executive, never by model input.
#[derive(Clone)]
pub struct CapabilityExecutionContext {
    pub process_id: fabric::ProcessId,
    pub operation_id: fabric::OperationId,
    pub principal: PrincipalId,
    pub session_id: String,
    pub working_dir: PathBuf,
    pub sandbox: SandboxRequirement,
    pub cancel: CancellationToken,
    pub turn_count: usize,
}

/// Canonical application capability entry point used outside the turn pipeline.
///
/// An existing lifecycle context is supplied by native sub-agents. Callers such
/// as MCP and durable goal workers pass `None`; the Executive implementation
/// creates and cleans up a transient Kernel Process/Operation around the call.
#[async_trait]
pub trait CapabilityService: Send + Sync {
    async fn invoke(
        &self,
        context: Option<CapabilityExecutionContext>,
        call: CapabilityCall,
        cancel: CancellationToken,
    ) -> CapabilityResult;
}

/// Trusted application result of authorizing a model-originated call.
pub struct AuthorizedInvocation {
    pub authority: CapabilityAuthority,
    pub control: InvocationControl,
}

#[async_trait]
pub trait TurnAuthorityProvider: Send + Sync {
    async fn authorize(&self, call: &CapabilityCall) -> Result<AuthorizedInvocation>;
}

/// The only capability surface exposed to a turn implementation.
#[async_trait]
pub trait TurnCapabilityInvoker: Send + Sync {
    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult;
}

pub struct GovernedCapabilityInvoker {
    inner: Arc<dyn CapabilityInvoker>,
    authority: Arc<dyn TurnAuthorityProvider>,
}

impl GovernedCapabilityInvoker {
    pub fn new(
        inner: Arc<dyn CapabilityInvoker>,
        authority: Arc<dyn TurnAuthorityProvider>,
    ) -> Self {
        Self { inner, authority }
    }
}

#[async_trait]
impl TurnCapabilityInvoker for GovernedCapabilityInvoker {
    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult {
        let authorized = match self.authority.authorize(&call).await {
            Ok(authorized) => authorized,
            Err(error) => {
                return CapabilityResult {
                    call_id: call.call_id,
                    output: format!("capability authorization denied: {error}"),
                    is_error: true,
                    usage: UsageReport::default(),
                    audit_id: None,
                };
            }
        };
        self.inner
            .invoke(fabric::CapabilityRequest {
                call,
                authority: authorized.authority,
                control: authorized.control,
            })
            .await
    }
}

/// Per-runtime composition factory shared by daemon and exec composition roots.
pub struct CapabilityRuntimeFactory;

impl CapabilityRuntimeFactory {
    pub fn build(
        admission: Arc<dyn AdmissionController>,
        executor: Arc<dyn ToolExecutor>,
        authority: Arc<dyn TurnAuthorityProvider>,
    ) -> Arc<dyn TurnCapabilityInvoker> {
        let kernel: Arc<dyn CapabilityInvoker> =
            Arc::new(DefaultCapabilityInvoker::new(admission, executor));
        Arc::new(GovernedCapabilityInvoker::new(kernel, authority))
    }
}

/// Registry-backed policy adapter. Unknown tools are rejected before admission;
/// known tools derive risk from their declared permission level.
pub struct RegistryAuthorityProvider {
    risk_by_tool: HashMap<String, RiskLevel>,
    principal: PrincipalId,
    session_id: String,
    working_dir: PathBuf,
    sandbox: SandboxRequirement,
    cancel: CancellationToken,
}

impl RegistryAuthorityProvider {
    pub fn new(
        risk_by_tool: HashMap<String, RiskLevel>,
        principal: PrincipalId,
        session_id: String,
        working_dir: PathBuf,
        sandbox: SandboxRequirement,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            risk_by_tool,
            principal,
            session_id,
            working_dir,
            sandbox,
            cancel,
        }
    }
}

#[async_trait]
impl TurnAuthorityProvider for RegistryAuthorityProvider {
    async fn authorize(&self, call: &CapabilityCall) -> Result<AuthorizedInvocation> {
        let risk = *self
            .risk_by_tool
            .get(&call.name)
            .ok_or_else(|| anyhow!("unknown tool '{}'", call.name))?;
        Ok(AuthorizedInvocation {
            authority: CapabilityAuthority {
                principal: self.principal.clone(),
                action: call.name.clone(),
                requested_scope: CapabilityScope::default(),
                risk,
                budget: None,
                lease: None,
                sandbox: self.sandbox,
                session_id: self.session_id.clone(),
                working_dir: self.working_dir.clone(),
            },
            control: InvocationControl {
                cancel: self.cancel.clone(),
            },
        })
    }
}
