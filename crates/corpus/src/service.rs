//! Authoritative Corpus catalog, activation, and invocation boundary.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use fabric::hook::{HookContext, HookResult};
use fabric::{
    AgentId, CapabilityId, CapabilityRequest, CapabilityResult, CapabilityScope, ExecutionPermit,
    ExtensionDescriptor, ExtensionId, ExtensionKind, ExtensionSnapshot, PrincipalId, Registry,
    Tool,
};
use kernel::capability::ToolExecutor;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::catalog::ExtensionCatalog;

/// Trusted, Executive-issued scope used for discovery and activation.
#[derive(Debug, Clone)]
pub struct ExtensionGrant {
    pub grant_id: String,
    pub principal: PrincipalId,
    pub session_id: String,
    pub agent_id: Option<AgentId>,
    pub capabilities: Vec<CapabilityId>,
    pub resources: CapabilityScope,
}

impl ExtensionGrant {
    fn validate(&self) -> Result<(), CorpusError> {
        if self.grant_id.trim().is_empty()
            || self.principal.0.trim().is_empty()
            || self.session_id.trim().is_empty()
        {
            return Err(CorpusError::InvalidGrant(
                "grant id, principal, and session must be non-empty".into(),
            ));
        }
        Ok(())
    }

    fn allows(&self, capability: &CapabilityId) -> bool {
        self.capabilities.iter().any(|item| item == capability)
    }
}

#[derive(Debug, Clone)]
pub struct ActivationRequest {
    pub grant: ExtensionGrant,
    pub extensions: Vec<ExtensionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActivationId(Uuid);

impl ActivationId {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone)]
pub struct ActivationReceipt {
    pub id: ActivationId,
    pub grant_id: String,
    pub extensions: Vec<ExtensionId>,
}

pub struct GovernedInvocation {
    pub activation_id: ActivationId,
    pub extension_id: ExtensionId,
    pub request: CapabilityRequest,
    pub permit: ExecutionPermit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusRetryDisposition {
    Never,
}

#[derive(Debug, Error)]
pub enum CorpusError {
    #[error("invalid extension descriptor: {0}")]
    InvalidDescriptor(String),
    #[error("duplicate extension id: {0}")]
    DuplicateExtension(String),
    #[error(
        "capability '{capability}' is declared by incompatible extensions '{first}' and '{second}'"
    )]
    ConflictingCapability {
        capability: String,
        first: String,
        second: String,
    },
    #[error("invalid extension grant: {0}")]
    InvalidGrant(String),
    #[error("unknown extension: {0}")]
    UnknownExtension(String),
    #[error("extension is not granted: {0}")]
    NotGranted(String),
    #[error("activation does not exist")]
    ActivationNotFound,
    #[error("extension is not active: {0}")]
    NotActivated(String),
    #[error("invocation binding does not match activation")]
    BindingMismatch,
    #[error("invocation exceeds activated resource scope")]
    ScopeExceeded,
    #[error("execution permit does not bind the invocation")]
    PermitMismatch,
    #[error("extension is not executable: {0}")]
    NotExecutable(String),
    #[error("extension catalog is read-only")]
    ReadOnlyCatalog,
}

impl CorpusError {
    pub const fn retry_disposition(&self) -> CorpusRetryDisposition {
        CorpusRetryDisposition::Never
    }
}

#[async_trait]
pub trait CorpusService: Send + Sync {
    async fn catalog(&self, grant: &ExtensionGrant) -> Result<ExtensionSnapshot, CorpusError>;

    async fn activate(&self, request: ActivationRequest) -> Result<ActivationReceipt, CorpusError>;

    async fn invoke(&self, invocation: GovernedInvocation)
        -> Result<CapabilityResult, CorpusError>;

    async fn invoke_streaming(
        &self,
        invocation: GovernedInvocation,
        sink: &mut fabric::ToolEventSink,
    ) -> Result<CapabilityResult, CorpusError> {
        let result = self.invoke(invocation).await?;
        sink.terminal(Ok(fabric::ToolResult {
            content: result.output.clone(),
            is_error: result.is_error,
            metadata: fabric::ToolResultMeta {
                execution_time_ms: result.usage.wall_time_ms,
                truncated: false,
                patch_delta: None,
            },
        }))
        .await;
        Ok(result)
    }

    /// Register a runtime-discovered tool through the authoritative catalog.
    async fn register_tool(&self, _tool: Arc<dyn Tool>) -> Result<(), CorpusError> {
        Err(CorpusError::ReadOnlyCatalog)
    }

    /// Execute lifecycle hooks without exposing the concrete hook registry.
    async fn execute_hook(&self, _context: &HookContext) -> HookResult {
        HookResult::Continue
    }
}

#[derive(Debug, Clone)]
struct ActiveGrant {
    grant: ExtensionGrant,
    extensions: HashSet<ExtensionId>,
}

/// Default adapter around the permit-enforcing Corpus runtime.
pub struct DefaultCorpusService {
    catalog: CatalogBackend,
    executor: Arc<dyn ToolExecutor>,
    hooks: Option<Arc<tokio::sync::Mutex<crate::HookRegistry>>>,
    activations: RwLock<HashMap<ActivationId, ActiveGrant>>,
}

enum CatalogBackend {
    Static(ExtensionCatalog),
    Runtime {
        tools: Arc<tokio::sync::Mutex<crate::ToolRegistry>>,
        extensions: ExtensionCatalog,
    },
}

impl DefaultCorpusService {
    pub fn new(catalog: ExtensionCatalog, executor: Arc<dyn ToolExecutor>) -> Self {
        Self {
            catalog: CatalogBackend::Static(catalog),
            executor,
            hooks: None,
            activations: RwLock::new(HashMap::new()),
        }
    }

    /// Build the production adapter around Corpus-owned registry, runner and hooks.
    pub fn from_runtime(
        registry: Arc<tokio::sync::Mutex<crate::ToolRegistry>>,
        executor: Arc<dyn ToolExecutor>,
        hooks: Arc<tokio::sync::Mutex<crate::HookRegistry>>,
    ) -> Self {
        Self::from_runtime_with_extensions(registry, executor, hooks, ExtensionCatalog::default())
    }

    /// Build the production adapter with non-tool extensions indexed beside
    /// the live tool registry. Discovery remains separate from activation.
    pub fn from_runtime_with_extensions(
        registry: Arc<tokio::sync::Mutex<crate::ToolRegistry>>,
        executor: Arc<dyn ToolExecutor>,
        hooks: Arc<tokio::sync::Mutex<crate::HookRegistry>>,
        extensions: ExtensionCatalog,
    ) -> Self {
        Self {
            catalog: CatalogBackend::Runtime {
                tools: registry,
                extensions,
            },
            executor,
            hooks: Some(hooks),
            activations: RwLock::new(HashMap::new()),
        }
    }

    async fn descriptors(&self) -> Result<BTreeMap<ExtensionId, ExtensionDescriptor>, CorpusError> {
        match &self.catalog {
            CatalogBackend::Static(catalog) => Ok(catalog.entries().clone()),
            CatalogBackend::Runtime { tools, extensions } => {
                let mut catalog = extensions.clone().into_entries();
                for descriptor in crate::discover_tool_extensions(tools).await? {
                    if catalog.insert(descriptor.id.clone(), descriptor).is_some() {
                        return Err(CorpusError::DuplicateExtension(
                            "runtime tool conflicts with a supplemental extension".into(),
                        ));
                    }
                }
                Ok(catalog)
            }
        }
    }

    async fn validate_invocation(
        &self,
        invocation: &GovernedInvocation,
    ) -> Result<(), CorpusError> {
        let active = self
            .activations
            .read()
            .await
            .get(&invocation.activation_id)
            .cloned()
            .ok_or(CorpusError::ActivationNotFound)?;
        if !active.extensions.contains(&invocation.extension_id) {
            return Err(CorpusError::NotActivated(
                invocation.extension_id.as_str().to_string(),
            ));
        }
        let catalog = self.descriptors().await?;
        let descriptor = catalog.get(&invocation.extension_id).ok_or_else(|| {
            CorpusError::UnknownExtension(invocation.extension_id.as_str().to_string())
        })?;
        if !descriptor.is_executable() {
            return Err(CorpusError::NotExecutable(
                invocation.extension_id.as_str().to_string(),
            ));
        }
        validate_binding(&active.grant, &invocation.request)?;
        validate_scope(
            &active.grant.resources,
            &invocation.request.authority.requested_scope,
        )?;
        validate_scope(&active.grant.resources, &invocation.permit.granted_scope)?;
        if descriptor
            .primary_capability()
            .map(|value| value.0.as_str())
            .unwrap_or_default()
            != invocation.request.call.name
            || descriptor
                .primary_capability()
                .is_none_or(|capability| invocation.permit.capability != *capability)
            || invocation.permit.operation_id != invocation.request.call.operation_id
            || invocation.permit.process_id != invocation.request.call.process_id
        {
            return Err(CorpusError::PermitMismatch);
        }
        Ok(())
    }
}

#[async_trait]
impl CorpusService for DefaultCorpusService {
    async fn catalog(&self, grant: &ExtensionGrant) -> Result<ExtensionSnapshot, CorpusError> {
        grant.validate()?;
        let catalog = self.descriptors().await?;
        Ok(ExtensionSnapshot {
            entries: catalog
                .values()
                .filter(|descriptor| {
                    descriptor
                        .capabilities
                        .iter()
                        .all(|capability| grant.allows(capability))
                })
                .cloned()
                .collect(),
        })
    }

    async fn activate(&self, request: ActivationRequest) -> Result<ActivationReceipt, CorpusError> {
        request.grant.validate()?;
        let mut extensions = request.extensions;
        extensions.sort();
        extensions.dedup();
        let catalog = self.descriptors().await?;
        for id in &extensions {
            let descriptor = catalog
                .get(id)
                .ok_or_else(|| CorpusError::UnknownExtension(id.as_str().to_string()))?;
            if !descriptor
                .capabilities
                .iter()
                .all(|capability| request.grant.allows(capability))
            {
                return Err(CorpusError::NotGranted(id.as_str().to_string()));
            }
        }

        let id = ActivationId::new();
        self.activations.write().await.insert(
            id,
            ActiveGrant {
                grant: request.grant.clone(),
                extensions: extensions.iter().cloned().collect(),
            },
        );
        Ok(ActivationReceipt {
            id,
            grant_id: request.grant.grant_id,
            extensions,
        })
    }

    async fn invoke(
        &self,
        invocation: GovernedInvocation,
    ) -> Result<CapabilityResult, CorpusError> {
        self.validate_invocation(&invocation).await?;
        Ok(self
            .executor
            .execute_with_permit(&invocation.request, &invocation.permit)
            .await)
    }

    async fn invoke_streaming(
        &self,
        invocation: GovernedInvocation,
        sink: &mut fabric::ToolEventSink,
    ) -> Result<CapabilityResult, CorpusError> {
        self.validate_invocation(&invocation).await?;
        Ok(self
            .executor
            .execute_streaming_with_permit(&invocation.request, &invocation.permit, sink)
            .await)
    }

    async fn register_tool(&self, tool: Arc<dyn Tool>) -> Result<(), CorpusError> {
        match &self.catalog {
            CatalogBackend::Static(_) => Err(CorpusError::ReadOnlyCatalog),
            CatalogBackend::Runtime { tools, .. } => tools
                .lock()
                .await
                .register(tool)
                .map(|_| ())
                .map_err(|error| CorpusError::InvalidDescriptor(error.to_string())),
        }
    }

    async fn execute_hook(&self, context: &HookContext) -> HookResult {
        match &self.hooks {
            Some(hooks) => hooks.lock().await.execute(context).await,
            None => HookResult::Continue,
        }
    }
}

/// ToolExecutor adapter bound to one explicit Corpus activation.
pub struct ActivatedCorpusExecutor {
    service: Arc<dyn CorpusService>,
    activation_id: ActivationId,
}

impl ActivatedCorpusExecutor {
    pub fn new(service: Arc<dyn CorpusService>, activation_id: ActivationId) -> Self {
        Self {
            service,
            activation_id,
        }
    }
}

#[async_trait]
impl ToolExecutor for ActivatedCorpusExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
    ) -> CapabilityResult {
        let extension_id = match ExtensionId::new(ExtensionKind::Tool, &request.call.name) {
            Ok(id) => id,
            Err(error) => {
                return CapabilityResult {
                    call_id: request.call.call_id.clone(),
                    output: error.to_string(),
                    is_error: true,
                    usage: Default::default(),
                    audit_id: None,
                    patch_delta: None,
                };
            }
        };
        self.service
            .invoke(GovernedInvocation {
                activation_id: self.activation_id,
                extension_id,
                request: request.clone(),
                permit: permit.clone(),
            })
            .await
            .unwrap_or_else(|error| CapabilityResult {
                call_id: request.call.call_id.clone(),
                output: error.to_string(),
                is_error: true,
                usage: Default::default(),
                audit_id: None,
                patch_delta: None,
            })
    }

    async fn execute_streaming_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        sink: &mut fabric::ToolEventSink,
    ) -> CapabilityResult {
        let extension_id = match ExtensionId::new(ExtensionKind::Tool, &request.call.name) {
            Ok(id) => id,
            Err(error) => {
                return CapabilityResult {
                    call_id: request.call.call_id.clone(),
                    output: error.to_string(),
                    is_error: true,
                    usage: Default::default(),
                    audit_id: None,
                    patch_delta: None,
                };
            }
        };
        self.service
            .invoke_streaming(
                GovernedInvocation {
                    activation_id: self.activation_id,
                    extension_id,
                    request: request.clone(),
                    permit: permit.clone(),
                },
                sink,
            )
            .await
            .unwrap_or_else(|error| CapabilityResult {
                call_id: request.call.call_id.clone(),
                output: error.to_string(),
                is_error: true,
                usage: Default::default(),
                audit_id: None,
                patch_delta: None,
            })
    }
}

fn validate_binding(
    grant: &ExtensionGrant,
    request: &CapabilityRequest,
) -> Result<(), CorpusError> {
    let request_agent = request
        .authority
        .agent
        .as_ref()
        .map(|agent| agent.caller_root_agent_id);
    if grant.principal != request.authority.principal
        || grant.session_id != request.authority.session_id
        || grant.agent_id != request_agent
    {
        return Err(CorpusError::BindingMismatch);
    }
    Ok(())
}

fn validate_scope(grant: &CapabilityScope, requested: &CapabilityScope) -> Result<(), CorpusError> {
    let paths_allowed = grant.allowed_paths.is_empty()
        || (!requested.allowed_paths.is_empty()
            && requested
                .allowed_paths
                .iter()
                .all(|path| grant.allowed_paths.contains(path)));
    let targets_allowed = grant.allowed_targets.is_empty()
        || (!requested.allowed_targets.is_empty()
            && requested
                .allowed_targets
                .iter()
                .all(|target| grant.allowed_targets.contains(target)));
    let runtime_allowed = match (grant.max_runtime_ms, requested.max_runtime_ms) {
        (Some(max), Some(value)) => value <= max,
        (Some(_), None) => false,
        (None, _) => true,
    };
    let output_allowed = match (grant.max_output_bytes, requested.max_output_bytes) {
        (Some(max), Some(value)) => value <= max,
        (Some(_), None) => false,
        (None, _) => true,
    };
    if paths_allowed && targets_allowed && runtime_allowed && output_allowed {
        Ok(())
    } else {
        Err(CorpusError::ScopeExceeded)
    }
}
