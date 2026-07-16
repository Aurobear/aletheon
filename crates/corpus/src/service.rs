//! Authoritative Corpus catalog, activation, and invocation boundary.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use aletheon_kernel::capability::ToolExecutor;
use async_trait::async_trait;
use fabric::hook::{HookContext, HookResult};
use fabric::types::admission::RiskLevel;
use fabric::{
    AgentId, CapabilityId, CapabilityRequest, CapabilityResult, CapabilityScope, ExecutionPermit,
    PrincipalId, Registry, Tool, ToolDefinition,
};
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Stable catalog identity. The value is deterministic: `<kind>:<local-name>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionId(String);

impl ExtensionId {
    pub fn new(kind: ExtensionKind, local_name: &str) -> Result<Self, CorpusError> {
        let local_name = local_name.trim();
        if local_name.is_empty()
            || local_name
                .chars()
                .any(|character| character.is_control() || character == ':')
        {
            return Err(CorpusError::InvalidDescriptor(
                "extension name must be non-empty and contain no controls or ':'".into(),
            ));
        }
        Ok(Self(format!("{}:{local_name}", kind.as_str())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionKind {
    Tool,
    Skill,
    Hook,
    Plugin,
    Mcp,
}

impl ExtensionKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Skill => "skill",
            Self::Hook => "hook",
            Self::Plugin => "plugin",
            Self::Mcp => "mcp",
        }
    }
}

/// Immutable metadata returned by catalog discovery.
#[derive(Debug, Clone)]
pub struct ExtensionDescriptor {
    pub id: ExtensionId,
    pub kind: ExtensionKind,
    pub version: String,
    pub description: String,
    pub capability: CapabilityId,
    pub risk: RiskLevel,
    pub tool_definition: Option<ToolDefinition>,
}

impl ExtensionDescriptor {
    pub fn new(
        kind: ExtensionKind,
        local_name: impl AsRef<str>,
        version: impl Into<String>,
        description: impl Into<String>,
        capability: CapabilityId,
        risk: RiskLevel,
    ) -> Result<Self, CorpusError> {
        let local_name = local_name.as_ref();
        if capability.0.trim().is_empty() {
            return Err(CorpusError::InvalidDescriptor(
                "extension capability must be non-empty".into(),
            ));
        }
        let version = version.into();
        if version.trim().is_empty() {
            return Err(CorpusError::InvalidDescriptor(
                "extension version must be non-empty".into(),
            ));
        }
        Ok(Self {
            id: ExtensionId::new(kind, local_name)?,
            kind,
            version,
            description: description.into(),
            capability,
            risk,
            tool_definition: None,
        })
    }

    pub fn with_tool_definition(mut self, definition: ToolDefinition) -> Result<Self, CorpusError> {
        if definition.name != self.capability.0 {
            return Err(CorpusError::InvalidDescriptor(format!(
                "tool definition '{}' does not match capability '{}'",
                definition.name, self.capability.0
            )));
        }
        self.tool_definition = Some(definition);
        Ok(self)
    }

    fn is_executable(&self) -> bool {
        matches!(self.kind, ExtensionKind::Tool | ExtensionKind::Mcp)
            && self.tool_definition.is_some()
    }
}

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

#[derive(Debug, Clone, Default)]
pub struct ExtensionSnapshot {
    pub entries: Vec<ExtensionDescriptor>,
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

/// Deterministic catalog. Discovery never implies activation.
#[derive(Debug, Clone, Default)]
pub struct ExtensionCatalog {
    entries: BTreeMap<ExtensionId, ExtensionDescriptor>,
}

impl ExtensionCatalog {
    pub fn new(
        descriptors: impl IntoIterator<Item = ExtensionDescriptor>,
    ) -> Result<Self, CorpusError> {
        let mut catalog = Self::default();
        for descriptor in descriptors {
            catalog.register(descriptor)?;
        }
        Ok(catalog)
    }

    pub fn register(&mut self, descriptor: ExtensionDescriptor) -> Result<(), CorpusError> {
        let id = descriptor.id.clone();
        if self.entries.contains_key(&id) {
            return Err(CorpusError::DuplicateExtension(id.0));
        }
        self.entries.insert(id, descriptor);
        Ok(())
    }
}

#[async_trait]
pub trait CorpusService: Send + Sync {
    async fn catalog(&self, grant: &ExtensionGrant) -> Result<ExtensionSnapshot, CorpusError>;

    async fn activate(&self, request: ActivationRequest) -> Result<ActivationReceipt, CorpusError>;

    async fn invoke(&self, invocation: GovernedInvocation)
        -> Result<CapabilityResult, CorpusError>;

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
    Runtime(Arc<tokio::sync::Mutex<crate::ToolRegistry>>),
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
        Self {
            catalog: CatalogBackend::Runtime(registry),
            executor,
            hooks: Some(hooks),
            activations: RwLock::new(HashMap::new()),
        }
    }

    async fn descriptors(&self) -> Result<BTreeMap<ExtensionId, ExtensionDescriptor>, CorpusError> {
        let descriptors = match &self.catalog {
            CatalogBackend::Static(catalog) => return Ok(catalog.entries.clone()),
            CatalogBackend::Runtime(registry) => crate::discover_tool_extensions(registry).await?,
        };
        Ok(descriptors
            .into_iter()
            .map(|descriptor| (descriptor.id.clone(), descriptor))
            .collect())
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
                .filter(|descriptor| grant.allows(&descriptor.capability))
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
                .ok_or_else(|| CorpusError::UnknownExtension(id.0.clone()))?;
            if !request.grant.allows(&descriptor.capability) {
                return Err(CorpusError::NotGranted(id.0.clone()));
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
        let active = self
            .activations
            .read()
            .await
            .get(&invocation.activation_id)
            .cloned()
            .ok_or(CorpusError::ActivationNotFound)?;
        if !active.extensions.contains(&invocation.extension_id) {
            return Err(CorpusError::NotActivated(invocation.extension_id.0));
        }
        let catalog = self.descriptors().await?;
        let descriptor = catalog
            .get(&invocation.extension_id)
            .ok_or_else(|| CorpusError::UnknownExtension(invocation.extension_id.0.clone()))?;
        if !descriptor.is_executable() {
            return Err(CorpusError::NotExecutable(invocation.extension_id.0));
        }
        validate_binding(&active.grant, &invocation.request)?;
        validate_scope(
            &active.grant.resources,
            &invocation.request.authority.requested_scope,
        )?;
        validate_scope(&active.grant.resources, &invocation.permit.granted_scope)?;
        if descriptor.capability.0 != invocation.request.call.name
            || invocation.permit.capability != descriptor.capability
            || invocation.permit.operation_id != invocation.request.call.operation_id
            || invocation.permit.process_id != invocation.request.call.process_id
        {
            return Err(CorpusError::PermitMismatch);
        }
        Ok(self
            .executor
            .execute_with_permit(&invocation.request, &invocation.permit)
            .await)
    }

    async fn register_tool(&self, tool: Arc<dyn Tool>) -> Result<(), CorpusError> {
        match &self.catalog {
            CatalogBackend::Static(_) => Err(CorpusError::ReadOnlyCatalog),
            CatalogBackend::Runtime(registry) => registry
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
                }
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
