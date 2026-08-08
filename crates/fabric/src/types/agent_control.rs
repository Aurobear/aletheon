use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::agent_settlement::{BackgroundResourceDecl, MAX_BACKGROUND_RESOURCES};
use super::attempt::{AttemptEvidence, AttemptUsage, RuntimeId};
use super::operation::{OperationId, ProcessId};
use super::process::{AgentId, AgentProfileId};
use super::space::AgoraSpaceId;
use super::workspace::{BroadcastEpoch, ContentId};

pub const MAX_AGENT_TASK_BYTES: usize = 64 * 1024;
pub const MAX_AGENT_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_AGENT_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_CONTEXT_ITEMS: usize = 64;
pub const MAX_EVIDENCE_ITEMS: usize = 128;
pub const MAX_ARTIFACTS: usize = 128;
pub const MAX_LIST_ITEMS: usize = 1000;
pub const MAX_AGENT_BROADCAST_REFS: usize = 64;
pub const AGENT_MESSAGE_SCHEMA_V1: u16 = 1;

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeCapability {
    CodeRead,
    CodeSearch,
    CodeEdit,
    Shell,
    Test,
    Git,
    Diagnostics,
    Browser,
    DeviceObserve,
    DeviceCommand,
    MemoryProposal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentInteractionMode {
    OneShot,
    Resident,
    Steering,
    FollowUp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkspaceMode {
    WorkspaceLess,
    SharedReadOnly,
    SharedWritable,
    IsolatedWorktree,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskEncoding {
    NaturalLanguage,
    StructuredJson,
}

/// Risk tier for agent profiles. Each tier is cumulative: higher tiers include
/// lower-tier capabilities but add additional risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    /// L0 tools only — read-only, no side effects.
    ReadOnly = 0,
    /// L0 + L1 tools — sandboxed writes.
    Sandboxed = 1,
    /// L0 + L1 + L2 tools — system-level changes.
    System = 2,
    /// All tools — L3 destructive operations included.
    Unrestricted = 3,
}

/// Per-profile approval policy for agent tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentApprovalPolicy {
    /// Deny without prompting — for restricted profiles.
    AutoDeny,
    /// Prompt the user for confirmation.
    PromptUser,
    /// Approve without prompting — for fully trusted profiles.
    AutoApprove,
}

/// Parent-child profile restriction: the child's capabilities must not exceed
/// the parent's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentRestriction {
    /// Child must use the same profile as the parent.
    Same,
    /// Child may use the same-or-safer profile (risk tier ≤ parent's).
    #[default]
    SameOrSafer,
    /// No restriction on child profile.
    None,
}

/// Durable task lineage owned by AgentControl. Callers may describe a task,
/// but may not choose the identifier used for memory authority or recovery.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentTaskId(pub String);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RuntimeResumability {
    #[default]
    Never,
    Checkpointed {
        reference: String,
    },
}

impl RuntimeResumability {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if let Self::Checkpointed { reference } = self {
            ensure_text(reference, 4096, "runtime checkpoint reference")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRecoveryDecision {
    Interrupt,
    Resume,
    Finalize,
    Reclaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRecoveryReceipt {
    pub decision: AgentRecoveryDecision,
    pub daemon_generation: String,
    pub recovered_at_ms: i64,
    pub idempotency_key: String,
}

/// Immutable receipt for workspace content explicitly made available to a
/// child Agent. The triple is required so a content ID cannot be replayed from
/// a different space or broadcast epoch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentBroadcastRef {
    pub space: AgoraSpaceId,
    pub epoch: BroadcastEpoch,
    pub content_id: ContentId,
}

impl AgentBroadcastRef {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.space.0, 1024, "broadcast space ID")?;
        if self.epoch.0 == 0 {
            return Err(AgentControlError::invalid(
                "broadcast epoch must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: AgentProfileId,
    pub system_prompt: String,
    pub model: String,
    pub allowed_tools: Vec<String>,
    /// Tools this profile may grant to a child Agent. This can be broader than
    /// `allowed_tools` for a pure orchestrator, but it never grants the parent
    /// permission to invoke those tools directly.
    #[serde(default)]
    pub delegated_tools: Vec<String>,
    pub max_iterations: usize,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,

    // ── Phase 2 profile metadata ──────────────────────────────────────────
    /// Human-readable name (e.g. "code-agent", "safe-agent").
    pub profile_name: String,
    /// Risk tier derived from both callable and delegable tools.
    pub risk_tier: RiskTier,
    /// Per-profile tool approval policy.
    pub approval_policy: AgentApprovalPolicy,
    /// Per-tool-call timeout in milliseconds.
    pub tool_timeout_ms: u64,
    /// Whether child agents may inherit this profile.
    pub inheritable: bool,
    /// Parent-child profile restriction policy.
    pub parent_restriction: ParentRestriction,
}

impl AgentProfile {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.id.0, 512, "profile ID")?;
        ensure_text(
            &self.system_prompt,
            MAX_AGENT_MESSAGE_BYTES,
            "profile system prompt",
        )?;
        ensure_text(&self.model, 512, "profile model")?;
        ensure_text(&self.profile_name, 512, "profile name")?;
        ensure_count(self.allowed_tools.len(), 256, "profile tools")?;
        for tool in &self.allowed_tools {
            ensure_text(tool, 512, "profile tool")?;
        }
        ensure_count(self.delegated_tools.len(), 256, "profile delegated tools")?;
        for tool in &self.delegated_tools {
            ensure_text(tool, 512, "profile delegated tool")?;
        }
        if self
            .allowed_tools
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != self.allowed_tools.len()
        {
            return Err(AgentControlError::invalid(
                "Agent profile callable tools must be unique",
            ));
        }
        if self
            .delegated_tools
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != self.delegated_tools.len()
        {
            return Err(AgentControlError::invalid(
                "Agent profile delegated tools must be unique",
            ));
        }
        if self.max_iterations == 0
            || self.max_input_tokens == 0
            || self.max_output_tokens == 0
            || self.max_tool_calls == 0
            || self.max_elapsed_ms == 0
        {
            return Err(AgentControlError::invalid(
                "Agent profile limits must be nonzero",
            ));
        }
        if self.tool_timeout_ms == 0 {
            return Err(AgentControlError::invalid("tool timeout must be nonzero"));
        }
        Ok(())
    }

    /// Returns true if the child profile's risk tier does not exceed the parent's.
    pub fn allows_child(&self, child: &AgentProfile) -> bool {
        match self.parent_restriction {
            ParentRestriction::None => true,
            ParentRestriction::Same => child.id == self.id,
            ParentRestriction::SameOrSafer => {
                if child.risk_tier > self.risk_tier {
                    return false;
                }
                for tool in &child.allowed_tools {
                    if !self.delegated_tools.contains(tool) {
                        return false;
                    }
                }
                true
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AgentContextFork {
    None,
    LastTurns { count: u16 },
    SelectedProjection { items: Vec<String> },
}

impl Default for AgentContextFork {
    fn default() -> Self {
        Self::SelectedProjection { items: Vec::new() }
    }
}

impl AgentContextFork {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        match self {
            Self::None => Ok(()),
            Self::LastTurns { count } if *count == 0 || *count > 100 => Err(
                AgentControlError::invalid("last-turn count must be between 1 and 100"),
            ),
            Self::LastTurns { .. } => Ok(()),
            Self::SelectedProjection { items } => {
                ensure_count(items.len(), MAX_CONTEXT_ITEMS, "context items")?;
                for item in items {
                    ensure_text(item, MAX_AGENT_MESSAGE_BYTES, "context item")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentBudget {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,
    pub max_cost_usd: Option<f64>,
    pub max_depth: u16,
}

impl AgentBudget {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.max_input_tokens == 0
            || self.max_output_tokens == 0
            || self.max_elapsed_ms == 0
            || self.max_depth == 0
        {
            return Err(AgentControlError::invalid(
                "Agent budget values must be nonzero",
            ));
        }
        if self
            .max_cost_usd
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err(AgentControlError::invalid("Agent cost budget is invalid"));
        }
        Ok(())
    }
}

/// Host-minted authority that bounds every capability a child Agent may receive.
/// This value is never accepted from model-visible input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDelegationAuthority {
    pub workspace: Option<crate::WorkspacePolicy>,
    pub allowed_tools: Vec<String>,
    pub budget: AgentBudget,
}

impl PartialEq for AgentDelegationAuthority {
    fn eq(&self, other: &Self) -> bool {
        self.workspace == other.workspace
            && self.allowed_tools == other.allowed_tools
            && self.budget.max_input_tokens == other.budget.max_input_tokens
            && self.budget.max_output_tokens == other.budget.max_output_tokens
            && self.budget.max_tool_calls == other.budget.max_tool_calls
            && self.budget.max_elapsed_ms == other.budget.max_elapsed_ms
            && self.budget.max_depth == other.budget.max_depth
            && self.budget.max_cost_usd.map(f64::to_bits)
                == other.budget.max_cost_usd.map(f64::to_bits)
    }
}

impl Eq for AgentDelegationAuthority {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentBudgetField {
    MaxInputTokens,
    MaxOutputTokens,
    MaxToolCalls,
    MaxElapsedMs,
    MaxCostUsd,
    MaxDepth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAttenuationReport {
    pub dropped_tools: Vec<String>,
    pub dropped_writable_roots: Vec<std::path::PathBuf>,
    pub inherited_protected_paths: Vec<std::path::PathBuf>,
    pub capped_budget_fields: Vec<AgentBudgetField>,
    pub requested_sha256: String,
    pub effective_sha256: String,
}

impl AgentDelegationAuthority {
    pub fn new(
        workspace: Option<crate::WorkspacePolicy>,
        allowed_tools: Vec<String>,
        budget: AgentBudget,
    ) -> Self {
        Self {
            workspace,
            allowed_tools,
            budget,
        }
    }

    pub fn covers(&self, child: &Self) -> bool {
        let tools_cover = child
            .allowed_tools
            .iter()
            .all(|tool| self.allowed_tools.contains(tool));
        let workspace_covers = match (&self.workspace, &child.workspace) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(parent), Some(child)) => {
                child.writable_roots().iter().all(|root| {
                    parent
                        .writable_roots()
                        .iter()
                        .any(|authority| root.starts_with(authority))
                }) && parent
                    .protected_paths()
                    .credential_paths()
                    .iter()
                    .all(|path| child.protected_paths().credential_paths().contains(path))
            }
        };
        tools_cover && workspace_covers && self.accepts_budget(child)
    }

    pub fn accepts_budget(&self, child: &Self) -> bool {
        self.budget.max_input_tokens >= child.budget.max_input_tokens
            && self.budget.max_output_tokens >= child.budget.max_output_tokens
            && self.budget.max_tool_calls >= child.budget.max_tool_calls
            && self.budget.max_elapsed_ms >= child.budget.max_elapsed_ms
            && self.budget.max_depth >= child.budget.max_depth
            && match (self.budget.max_cost_usd, child.budget.max_cost_usd) {
                (None, _) => true,
                (Some(parent), Some(child)) => parent >= child,
                (Some(_), None) => false,
            }
    }

    pub fn attenuate(
        &self,
        requested: &Self,
    ) -> Result<(Self, AgentAttenuationReport), AgentControlError> {
        let allowed_tools = requested
            .allowed_tools
            .iter()
            .filter(|tool| self.allowed_tools.contains(*tool))
            .cloned()
            .collect::<Vec<_>>();
        let dropped_tools = requested
            .allowed_tools
            .iter()
            .filter(|tool| !self.allowed_tools.contains(*tool))
            .cloned()
            .collect::<Vec<_>>();

        let mut dropped_writable_roots = Vec::new();
        let mut inherited_protected_paths = Vec::new();
        let workspace = match (&self.workspace, &requested.workspace) {
            (_, None) | (None, Some(_)) => {
                if let Some(child) = &requested.workspace {
                    dropped_writable_roots.extend(child.writable_roots().iter().cloned());
                }
                None
            }
            (Some(parent), Some(child)) => {
                let kept = child
                    .writable_roots()
                    .iter()
                    .filter(|root| {
                        parent
                            .writable_roots()
                            .iter()
                            .any(|authority| root.starts_with(authority))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                dropped_writable_roots.extend(
                    child
                        .writable_roots()
                        .iter()
                        .filter(|root| !kept.contains(root))
                        .cloned(),
                );
                let mut protected = child.protected_paths().credential_paths().to_vec();
                for path in parent.protected_paths().credential_paths() {
                    if !protected.contains(path) {
                        protected.push(path.clone());
                        inherited_protected_paths.push(path.clone());
                    }
                }
                let protected = crate::ProtectedPathPolicy::new(protected)
                    .map_err(AgentControlError::invalid)?;
                Some(
                    child
                        .clone()
                        .narrow_writable_roots(kept)
                        .map_err(AgentControlError::invalid)?
                        .with_protected_paths(protected),
                )
            }
        };

        let mut capped_budget_fields = Vec::new();
        macro_rules! cap {
            ($field:ident, $variant:ident) => {{
                if requested.budget.$field > self.budget.$field {
                    capped_budget_fields.push(AgentBudgetField::$variant);
                }
                requested.budget.$field.min(self.budget.$field)
            }};
        }
        let max_cost_usd = match (self.budget.max_cost_usd, requested.budget.max_cost_usd) {
            (None, child) => child,
            (Some(parent), Some(child)) => {
                if child > parent {
                    capped_budget_fields.push(AgentBudgetField::MaxCostUsd);
                }
                Some(parent.min(child))
            }
            (Some(parent), None) => {
                capped_budget_fields.push(AgentBudgetField::MaxCostUsd);
                Some(parent)
            }
        };
        let effective = Self::new(
            workspace,
            allowed_tools,
            AgentBudget {
                max_input_tokens: cap!(max_input_tokens, MaxInputTokens),
                max_output_tokens: cap!(max_output_tokens, MaxOutputTokens),
                max_tool_calls: cap!(max_tool_calls, MaxToolCalls),
                max_elapsed_ms: cap!(max_elapsed_ms, MaxElapsedMs),
                max_cost_usd,
                max_depth: cap!(max_depth, MaxDepth),
            },
        );
        let digest = |authority: &Self| -> Result<String, AgentControlError> {
            let bytes = serde_json::to_vec(authority)
                .map_err(|error| AgentControlError::invalid(error.to_string()))?;
            Ok(format!("{:x}", Sha256::digest(bytes)))
        };
        let report = AgentAttenuationReport {
            dropped_tools,
            dropped_writable_roots,
            inherited_protected_paths,
            capped_budget_fields,
            requested_sha256: digest(requested)?,
            effective_sha256: digest(&effective)?,
        };
        debug_assert!(self.covers(&effective));
        Ok((effective, report))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpawnRequest {
    pub root_agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub parent_process_id: Option<ProcessId>,
    pub profile_id: AgentProfileId,
    pub runtime_id: RuntimeId,
    /// Host-injected workspace authority. This field is deliberately absent
    /// from serialized/model-visible requests and must be minted from the
    /// authenticated ToolContext at the capability boundary.
    #[serde(skip)]
    pub trusted_workspace: Option<crate::WorkspacePolicy>,
    /// Exact host-only authority of the delegating Agent. Required for
    /// non-root spawns whose parent is outside AgentControl's live registry.
    #[serde(skip)]
    pub delegator_authority: Option<AgentDelegationAuthority>,
    /// Optional host-only task authority admitted atomically after process
    /// allocation and before the runtime is launched.
    #[serde(skip)]
    pub cognitive_binding: Option<crate::cognitive_workflow::CognitiveTaskRuntimeBinding>,
    pub task: String,
    pub context: AgentContextFork,
    #[serde(default)]
    pub broadcast_refs: Vec<AgentBroadcastRef>,
    pub allowed_tools: Vec<String>,
    pub budget: AgentBudget,
    /// Host-reviewed resource declarations fixed at spawn time. A child may
    /// not add `survive_child` authorization while it is terminating.
    #[serde(default)]
    pub background_decls: Vec<BackgroundResourceDecl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpawnIntent {
    pub root_agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub parent_process_id: Option<ProcessId>,
    pub profile_id: AgentProfileId,
    #[serde(default)]
    pub runtime_override: Option<String>,
    #[serde(default)]
    pub required_capabilities: Vec<AgentRuntimeCapability>,
    /// Host-injected workspace authority. Model-visible input cannot mint it.
    #[serde(skip)]
    pub trusted_workspace: Option<crate::WorkspacePolicy>,
    #[serde(skip)]
    pub delegator_authority: Option<AgentDelegationAuthority>,
    pub task: String,
    pub context: AgentContextFork,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub budget: AgentBudget,
}

impl AgentSpawnIntent {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.profile_id.0, 512, "profile ID")?;
        if let Some(runtime_override) = &self.runtime_override {
            ensure_text(runtime_override, 512, "runtime override")?;
        }
        ensure_text(&self.task, MAX_AGENT_TASK_BYTES, "Agent task")?;
        ensure_count(self.allowed_tools.len(), 256, "allowed tools")?;
        for tool in &self.allowed_tools {
            ensure_text(tool, 512, "allowed tool")?;
        }
        let mut capabilities = self.required_capabilities.clone();
        capabilities.sort();
        let original_len = capabilities.len();
        capabilities.dedup();
        if capabilities.len() != original_len {
            return Err(AgentControlError::invalid(
                "required runtime capabilities contain duplicates",
            ));
        }
        self.context.validate()?;
        self.budget.validate()
    }
}

impl AgentSpawnRequest {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.profile_id.0, 512, "profile ID")?;
        ensure_text(&self.runtime_id.0, 512, "runtime ID")?;
        ensure_text(&self.task, MAX_AGENT_TASK_BYTES, "Agent task")?;
        ensure_count(self.allowed_tools.len(), 256, "allowed tools")?;
        for tool in &self.allowed_tools {
            ensure_text(tool, 512, "allowed tool")?;
        }
        self.context.validate()?;
        if let Some(binding) = &self.cognitive_binding {
            if binding.task_node_id.0.trim().is_empty()
                || binding.role_profile.version == 0
                || binding.role_profile.id.trim().is_empty()
            {
                return Err(AgentControlError::invalid(
                    "cognitive task runtime binding is invalid",
                ));
            }
            binding
                .budget
                .validate()
                .map_err(|error| AgentControlError::invalid(error.to_string()))?;
        }
        ensure_count(
            self.background_decls.len(),
            MAX_BACKGROUND_RESOURCES,
            "background resource declarations",
        )?;
        let mut resource_ids = std::collections::HashSet::new();
        for declaration in &self.background_decls {
            ensure_text(&declaration.resource_id, 1024, "background resource ID")?;
            if !resource_ids.insert(&declaration.resource_id) {
                return Err(AgentControlError::invalid(
                    "background resource declarations contain duplicate resource IDs",
                ));
            }
        }
        ensure_count(
            self.broadcast_refs.len(),
            MAX_AGENT_BROADCAST_REFS,
            "broadcast references",
        )?;
        for reference in &self.broadcast_refs {
            reference.validate()?;
        }
        let mut unique = self.broadcast_refs.clone();
        unique.sort_by(|left, right| {
            (&left.space.0, left.epoch, left.content_id).cmp(&(
                &right.space.0,
                right.epoch,
                right.content_id,
            ))
        });
        unique.dedup();
        if unique.len() != self.broadcast_refs.len() {
            return Err(AgentControlError::invalid(
                "broadcast references contain duplicates",
            ));
        }
        self.budget.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWaitRequest {
    pub caller_root_agent_id: AgentId,
    pub agent_id: AgentId,
    pub timeout_ms: u64,
}

impl AgentWaitRequest {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.timeout_ms == 0 {
            return Err(AgentControlError::invalid(
                "Agent wait timeout must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSendRequest {
    pub caller_root_agent_id: AgentId,
    /// Trusted immediate sender. `None` denotes the external/root caller.
    #[serde(default)]
    pub sender_agent_id: Option<AgentId>,
    pub agent_id: AgentId,
    #[serde(default)]
    pub kind: AgentMessageKind,
    /// Caller-generated delivery identity used for idempotent retries.
    #[serde(default)]
    pub delivery_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub correlation_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub deadline_mono_ms: Option<u64>,
    pub message: String,
    pub start_turn: bool,
}

impl AgentSendRequest {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.message, MAX_AGENT_MESSAGE_BYTES, "Agent message")?;
        if self.delivery_id.is_some_and(|id| id.is_nil()) {
            return Err(AgentControlError::invalid(
                "Agent message delivery ID must not be nil",
            ));
        }
        if self.start_turn && self.kind != AgentMessageKind::Input {
            return Err(AgentControlError::invalid(
                "only Agent input messages may start a turn",
            ));
        }
        if self.kind == AgentMessageKind::Response && self.correlation_id.is_none() {
            return Err(AgentControlError::invalid(
                "Agent response requires a correlation ID",
            ));
        }
        if self.deadline_mono_ms == Some(0) {
            return Err(AgentControlError::invalid(
                "Agent message deadline must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentListRequest {
    pub caller_root_agent_id: AgentId,
    pub status: Option<AgentRunStatus>,
    pub limit: usize,
}

impl AgentListRequest {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.limit == 0 || self.limit > MAX_LIST_ITEMS {
            return Err(AgentControlError::invalid("Agent list limit is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Queued,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl AgentRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    /// Project an external Agent runtime terminal into the canonical Turn
    /// terminal semantics without reusing the Turn event schema for runtime
    /// progress. Non-terminal runtime states intentionally have no projection.
    pub fn turn_terminal_status(self) -> Option<crate::TurnTerminalStatus> {
        match self {
            Self::Succeeded => Some(crate::TurnTerminalStatus::Completed),
            Self::Failed => Some(crate::TurnTerminalStatus::Failed),
            Self::Cancelled | Self::Interrupted => Some(crate::TurnTerminalStatus::Interrupted),
            Self::Queued | Self::Running | Self::Waiting => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub root_agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub process_id: ProcessId,
    pub operation_id: OperationId,
    pub runtime_id: RuntimeId,
    pub profile_id: AgentProfileId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    #[default]
    Input,
    Progress,
    Result,
    Signal,
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageDeliveryState {
    Pending,
    Delivered,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessagePayload {
    pub schema_version: u16,
    pub kind: AgentMessageKind,
    pub content: String,
    pub start_turn: bool,
    #[serde(default)]
    pub correlation_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub deadline_mono_ms: Option<u64>,
}

impl AgentMessagePayload {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.schema_version != AGENT_MESSAGE_SCHEMA_V1 {
            return Err(AgentControlError::invalid(
                "unsupported Agent message payload schema",
            ));
        }
        ensure_text(&self.content, MAX_AGENT_MESSAGE_BYTES, "Agent message")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentControlMessage {
    pub delivery_id: uuid::Uuid,
    pub sequence: u64,
    pub from: AgentId,
    pub to: AgentId,
    pub kind: AgentMessageKind,
    pub delivery: AgentMessageDeliveryState,
    pub content: String,
}

impl AgentControlMessage {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.content, MAX_AGENT_MESSAGE_BYTES, "Agent message")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessageReceipt {
    pub delivery_id: uuid::Uuid,
    pub agent_id: AgentId,
    pub sequence: u64,
    pub delivery: AgentMessageDeliveryState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentArtifact {
    pub kind: String,
    pub reference: String,
    pub content_hash: String,
}

impl AgentArtifact {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.kind, 512, "artifact kind")?;
        ensure_text(&self.reference, 4096, "artifact reference")?;
        ensure_text(&self.content_hash, 512, "artifact content hash")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentResult {
    pub output: String,
    pub usage: AttemptUsage,
    pub evidence: Vec<AttemptEvidence>,
    pub artifacts: Vec<AgentArtifact>,
}

impl AgentResult {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.output.len() > MAX_AGENT_OUTPUT_BYTES {
            return Err(AgentControlError::invalid(
                "Agent output exceeds byte limit",
            ));
        }
        ensure_count(self.evidence.len(), MAX_EVIDENCE_ITEMS, "Agent evidence")?;
        ensure_count(self.artifacts.len(), MAX_ARTIFACTS, "Agent artifacts")?;
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub handle: AgentHandle,
    pub status: AgentRunStatus,
    pub result: Option<AgentResult>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentControlErrorKind {
    InvalidRequest,
    NotFound,
    Forbidden,
    Capacity,
    Conflict,
    Timeout,
    Terminal,
    Persistence,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("Agent control {kind:?}: {message}")]
pub struct AgentControlError {
    pub kind: AgentControlErrorKind,
    pub message: String,
}

impl AgentControlError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: AgentControlErrorKind::InvalidRequest,
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait AgentControlPort: Send + Sync {
    async fn spawn_intent(
        &self,
        _intent: AgentSpawnIntent,
    ) -> Result<AgentHandle, AgentControlError> {
        Err(AgentControlError {
            kind: AgentControlErrorKind::Runtime,
            message: "generic subagent selection is unavailable".into(),
        })
    }

    async fn spawn(&self, request: AgentSpawnRequest) -> Result<AgentHandle, AgentControlError>;
    async fn wait(&self, request: AgentWaitRequest) -> Result<AgentSnapshot, AgentControlError>;
    async fn send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError>;
    async fn cancel(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError>;
    async fn inspect(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError>;
    async fn list(
        &self,
        request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError>;
}

fn ensure_text(value: &str, max: usize, label: &str) -> Result<(), AgentControlError> {
    if value.trim().is_empty() || value.len() > max {
        return Err(AgentControlError::invalid(format!(
            "{label} is empty or exceeds byte limit"
        )));
    }
    Ok(())
}

fn ensure_count(value: usize, max: usize, label: &str) -> Result<(), AgentControlError> {
    if value > max {
        return Err(AgentControlError::invalid(format!(
            "{label} count exceeds limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod delegation_tests {
    use super::*;

    fn budget(tokens: u64, cost: Option<f64>) -> AgentBudget {
        AgentBudget {
            max_input_tokens: tokens,
            max_output_tokens: tokens,
            max_tool_calls: tokens as u32,
            max_elapsed_ms: tokens,
            max_cost_usd: cost,
            max_depth: tokens as u16,
        }
    }

    #[test]
    fn attenuation_intersects_tools_roots_protections_and_budget() {
        let parent_workspace =
            crate::WorkspacePolicy::from_resolved_roots("/tmp/aletheon-parent".into(), vec![])
                .unwrap()
                .with_protected_paths(
                    crate::ProtectedPathPolicy::new(vec!["/tmp/secret".into()]).unwrap(),
                );
        let child_workspace = crate::WorkspacePolicy::from_resolved_roots(
            "/tmp/aletheon-parent/child".into(),
            vec!["/var/tmp/outside".into()],
        )
        .unwrap();
        let parent = AgentDelegationAuthority::new(
            Some(parent_workspace),
            vec!["read".into(), "test".into()],
            budget(10, Some(2.0)),
        );
        let requested = AgentDelegationAuthority::new(
            Some(child_workspace),
            vec!["shell".into(), "read".into()],
            budget(20, None),
        );

        let (effective, report) = parent.attenuate(&requested).unwrap();
        assert!(parent.covers(&effective));
        assert_eq!(effective.allowed_tools, vec!["read"]);
        assert_eq!(effective.budget.max_input_tokens, 10);
        assert_eq!(effective.budget.max_cost_usd, Some(2.0));
        assert_eq!(report.dropped_tools, vec!["shell"]);
        assert_eq!(
            report.dropped_writable_roots,
            vec![std::path::PathBuf::from("/var/tmp/outside")]
        );
        assert_eq!(
            report.inherited_protected_paths,
            vec![std::path::PathBuf::from("/tmp/secret")]
        );
        assert_eq!(report.requested_sha256.len(), 64);
        assert_eq!(report.effective_sha256.len(), 64);
    }

    #[test]
    fn no_child_workspace_is_always_narrower_but_parentless_is_not() {
        let parent = AgentDelegationAuthority::new(
            Some(crate::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap()),
            vec!["read".into()],
            budget(10, None),
        );
        let read_only = AgentDelegationAuthority::new(None, vec!["read".into()], budget(5, None));
        assert!(parent.covers(&read_only));
        assert!(!read_only.covers(&parent));
    }
}
