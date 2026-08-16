//! RA-05 AgentSupervisor + DelegateBackendRegistry owner seam (Agent Kernel V2).
//!
//! The single Agent spawn/send/wait/cancel/recovery entry contract.  This seam
//! defines the generic `DelegateBackendRegistry` (backend id → typed launcher)
//! and the AgentSupervisor shape. Pi concrete files are **not** migrated here
//! (that is E6-K6d); this seam is generic only.

use crate::error::RuntimeError;
use crate::ids::{AgentRunId, Generation, SessionId, TurnId};
use std::collections::HashMap;
use std::sync::Arc;

/// Read-only historical input for deterministic selection among Runtime IDs
/// that already passed capability and workspace eligibility checks.
pub trait RuntimePreferenceHistory: Send + Sync {
    fn preferred_runtime(&self, profile_id: &str, eligible: &[String]) -> Option<String>;
}

pub type AgentProfileResolver =
    dyn Fn(&::contracts::AgentProfileId) -> Option<::contracts::AgentProfile> + Send + Sync;

/// Runtime-owned, effect-free policy inputs for Agent backend selection.
#[derive(Default)]
pub struct AgentRuntimeSelectionPolicy {
    profiles: HashMap<::contracts::AgentProfileId, ::contracts::AgentProfile>,
    profile_resolver: Option<Arc<AgentProfileResolver>>,
    runtime_requirements:
        HashMap<::contracts::AgentProfileId, Vec<::contracts::AgentRuntimeCapability>>,
    capability_history: Option<Arc<dyn RuntimePreferenceHistory>>,
}

impl AgentRuntimeSelectionPolicy {
    pub fn set_profiles(&mut self, profiles: HashMap<String, ::contracts::AgentProfile>) {
        self.profiles = profiles
            .into_values()
            .map(|profile| (profile.id.clone(), profile))
            .collect();
    }

    pub fn set_profile_resolver(&mut self, resolver: Arc<AgentProfileResolver>) {
        self.profile_resolver = Some(resolver);
    }

    pub fn set_runtime_requirements(
        &mut self,
        requirements: HashMap<
            ::contracts::AgentProfileId,
            Vec<::contracts::AgentRuntimeCapability>,
        >,
    ) {
        self.runtime_requirements = requirements;
    }

    pub fn set_capability_history(&mut self, history: Arc<dyn RuntimePreferenceHistory>) {
        self.capability_history = Some(history);
    }

    pub fn resolve_profile(
        &self,
        id: &::contracts::AgentProfileId,
    ) -> Option<::contracts::AgentProfile> {
        self.profile_resolver
            .as_ref()
            .and_then(|resolve| resolve(id))
            .or_else(|| self.profiles.get(id).cloned())
    }

    pub fn runtime_requirements(
        &self,
        id: &::contracts::AgentProfileId,
    ) -> Vec<::contracts::AgentRuntimeCapability> {
        self.runtime_requirements
            .get(id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn preferred_runtime(
        &self,
        profile_id: &::contracts::AgentProfileId,
        catalog: &[crate::RuntimeManifest],
    ) -> Option<String> {
        let eligible = catalog
            .iter()
            .map(|manifest| manifest.id.clone())
            .collect::<Vec<_>>();
        self.capability_history
            .as_ref()
            .and_then(|history| history.preferred_runtime(&profile_id.0, &eligible))
    }
}

/// Host execution context supplied to a delegate adapter. It is descriptive
/// input only: Runtime still owns Agent/Process/Operation identity and
/// terminal settlement.
#[derive(Debug, Clone)]
pub struct DelegateExecutionContext {
    pub process_id: ::contracts::ProcessId,
    pub operation_id: ::contracts::OperationId,
    pub session_id: String,
    pub working_dir: std::path::PathBuf,
}

#[async_trait::async_trait]
pub trait DelegateTaskBackend: Send + Sync {
    fn capabilities(&self) -> std::collections::BTreeSet<crate::RuntimeCapability> {
        std::collections::BTreeSet::new()
    }

    fn resource_requirements(&self) -> crate::RuntimeResourceRequirements {
        crate::RuntimeResourceRequirements::default()
    }

    async fn run_attempt(
        &self,
        task: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<::contracts::RuntimeResult, ::contracts::RuntimeFailure>;

    async fn run_attempt_in_context(
        &self,
        task: &str,
        cancel: tokio_util::sync::CancellationToken,
        _context: DelegateExecutionContext,
    ) -> Result<::contracts::RuntimeResult, ::contracts::RuntimeFailure> {
        self.run_attempt(task, cancel).await
    }
}

/// Stable delegate backend id (e.g. "native", "pi-coder").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DelegateBackendId(pub String);

/// Typed delegate spawn request.  The Runtime assigns AgentRunId/Generation.
#[derive(Debug, Clone)]
pub struct DelegateSpawnRequest {
    pub parent_session: SessionId,
    /// Optional canonical parent turn. Background/goal work may be admitted
    /// without a parent turn; callers must not fabricate a `TurnId` merely to
    /// satisfy the compatibility shape.
    pub parent_turn: Option<TurnId>,
    pub backend: DelegateBackendId,
    pub profile: Option<String>,
    /// Optional host-only command payload. It carries the caller's spawn
    /// authority and task, but never a child AgentRunId or generation. The
    /// Runtime remains the only identity allocator.
    pub host_request: Option<::contracts::AgentSpawnRequest>,
    /// Optional typed command for non-AgentControl Runtime work.  Commands
    /// are host-authenticated values; the Runtime still owns identity,
    /// generation and terminal settlement.
    pub command: Option<DelegateCommand>,
}

/// Runtime-owned command payloads that are not rich AgentControl requests.
/// Compatibility adapters may translate an old task into one of these
/// commands, but may not mint identities or settle lifecycle state.
#[derive(Debug, Clone)]
pub enum DelegateCommand {
    GoalAttempt { task: String },
}

/// A bounded Runtime-owned mailbox payload. The backend may translate this
/// into its native AgentStream message, but it cannot change the target run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateMessage {
    pub kind: String,
    pub content: String,
    pub correlation: Option<String>,
    /// Opaque caller idempotency key. Runtime forwards it to the host adapter
    /// but never mints or interprets the value as an Agent identity.
    pub delivery_id: Option<String>,
}

/// Bounded delivery receipt returned by the Runtime mailbox entry point. The
/// richer Fabric message remains owned by the host adapter; Runtime exposes
/// only transport-neutral delivery evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateMessageReceipt {
    pub delivery_id: Option<String>,
    pub sequence: Option<u64>,
    pub delivered: bool,
}

/// Typed result projection for Runtime commands.  The Runtime owns terminal
/// settlement; this value is only execution evidence consumed after the
/// authoritative wait fence.
#[derive(Debug, Clone)]
pub enum DelegateResult {
    Succeeded(::contracts::RuntimeResult),
    Failed(::contracts::RuntimeFailure),
}

impl DelegateMessage {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.kind.trim().is_empty()
            || self.kind.len() > 256
            || self.content.is_empty()
            || self.content.len() > 64 * 1024
            || self
                .correlation
                .as_ref()
                .is_some_and(|value| value.len() > 512)
        {
            return Err(RuntimeError::UnsupportedRequest);
        }
        Ok(())
    }
}

/// Typed delegate receipt returned after spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateReceipt {
    pub agent_run: AgentRunId,
    pub generation: Generation,
}

/// A backend launcher registered in the registry.  A running AgentRun is
/// pinned to its backend generation; a reload must not swap the binding.
#[async_trait::async_trait]
pub trait DelegateBackend: Send + Sync {
    async fn spawn(
        &self,
        request: &DelegateSpawnRequest,
        identity: &DelegateReceipt,
    ) -> Result<DelegateReceipt, RuntimeError>;
    async fn cancel(&self, agent_run: &AgentRunId) -> Result<(), RuntimeError>;
    /// Optional AgentStream/mailbox operation. Backends that have not yet
    /// exposed a native mailbox fail closed rather than silently dropping the
    /// message.
    async fn send(
        &self,
        _agent_run: &AgentRunId,
        _message: &DelegateMessage,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::UnsupportedRequest)
    }
    async fn send_with_receipt(
        &self,
        agent_run: &AgentRunId,
        message: &DelegateMessage,
    ) -> Result<DelegateMessageReceipt, RuntimeError> {
        self.send(agent_run, message).await?;
        Ok(DelegateMessageReceipt {
            delivery_id: message.delivery_id.clone(),
            sequence: None,
            delivered: true,
        })
    }
    async fn wait(
        &self,
        agent_run: &AgentRunId,
    ) -> Result<crate::event::TurnTerminal, RuntimeError>;
    /// Resume a checkpoint through the already-pinned backend generation.
    /// Runtime owns the run identity and only forwards the typed recovery
    /// receipt; concrete checkpoint semantics remain with the backend.
    async fn resume_from_checkpoint(
        &self,
        _request: &DelegateRecoveryRequest,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::UnsupportedRequest)
    }
    async fn result(&self, _agent_run: &AgentRunId) -> Result<DelegateResult, RuntimeError> {
        Err(RuntimeError::UnsupportedRequest)
    }
}

/// Runtime-owned recovery request forwarded to a pinned delegate backend.
/// The checkpoint reference is opaque to Runtime and never used for routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateRecoveryRequest {
    pub agent_run: AgentRunId,
    pub generation: Generation,
    pub checkpoint_reference: String,
}

/// Generic DelegateBackend registry.  Registration rejects duplicate ids.
/// A running binding is pinned to its backend generation.
#[derive(Default)]
pub struct DelegateBackendRegistry {
    backends: std::sync::RwLock<HashMap<DelegateBackendId, Arc<dyn DelegateBackend>>>,
    /// Selectable backend contracts live beside the backend bindings.  The
    /// host may provide the concrete launcher, but it must not become a
    /// second runtime-selection authority.
    manifests: std::sync::RwLock<HashMap<DelegateBackendId, crate::RuntimeManifest>>,
}

impl DelegateBackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        id: DelegateBackendId,
        backend: Arc<dyn DelegateBackend>,
    ) -> Result<(), RuntimeError> {
        if id.0.trim().is_empty() {
            return Err(RuntimeError::Internal);
        }
        let mut backends = self.backends.write().unwrap();
        if backends.contains_key(&id) {
            return Err(RuntimeError::Internal); // duplicate backend
        }
        backends.insert(id, backend);
        Ok(())
    }

    /// Register the immutable capability contract for an already-bound
    /// backend. Registration is explicit and duplicate-safe so a package
    /// reload cannot silently replace a selectable runtime.
    pub fn register_manifest(
        &self,
        id: DelegateBackendId,
        manifest: crate::RuntimeManifest,
    ) -> Result<(), RuntimeError> {
        if manifest.id != id.0 || manifest.validate().is_err() {
            return Err(RuntimeError::UnsupportedRequest);
        }
        let mut manifests = self.manifests.write().unwrap();
        if manifests.contains_key(&id) {
            return Err(RuntimeError::Internal);
        }
        manifests.insert(id, manifest);
        Ok(())
    }

    /// Bind a backend and its selectable contract as one composition-root
    /// operation. A failed manifest registration rolls back the backend
    /// binding, preserving the duplicate/invalid fail-closed invariant.
    pub fn register_with_manifest(
        &self,
        id: DelegateBackendId,
        backend: Arc<dyn DelegateBackend>,
        manifest: crate::RuntimeManifest,
    ) -> Result<(), RuntimeError> {
        self.register(id.clone(), backend)?;
        if let Err(error) = self.register_manifest(id.clone(), manifest) {
            self.backends.write().unwrap().remove(&id);
            return Err(error);
        }
        Ok(())
    }

    /// Replace only a selectable capability manifest during reload. The
    /// backend binding itself is untouched, so running AgentRuns keep their
    /// pinned generation while future admission sees the new contract.
    pub fn replace_manifest(
        &self,
        id: DelegateBackendId,
        manifest: crate::RuntimeManifest,
    ) -> Result<(), RuntimeError> {
        if manifest.id != id.0 || manifest.validate().is_err() {
            return Err(RuntimeError::UnsupportedRequest);
        }
        if self.resolve(&id).is_none() {
            return Err(RuntimeError::AgentRunNotFound);
        }
        self.manifests.write().unwrap().insert(id, manifest);
        Ok(())
    }

    pub fn resolve(&self, id: &DelegateBackendId) -> Option<Arc<dyn DelegateBackend>> {
        self.backends.read().unwrap().get(id).cloned()
    }

    /// Remove a backend from future admission. Existing RuntimeAgentSupervisor
    /// bindings retain their Arc and therefore continue on the pinned backend
    /// generation; reload never swaps an already-running AgentRun.
    pub fn unregister(&self, id: &DelegateBackendId) -> bool {
        let removed = self.backends.write().unwrap().remove(id).is_some();
        self.manifests.write().unwrap().remove(id);
        removed
    }

    pub fn catalog(&self) -> Vec<crate::RuntimeManifest> {
        let mut manifests = self
            .manifests
            .read()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        manifests.sort_by(|left, right| left.id.cmp(&right.id));
        manifests
    }

    pub fn select(
        &self,
        request: &crate::RuntimeSelectionRequest,
    ) -> Result<(DelegateBackendId, crate::RuntimeSelectionDecision), RuntimeError> {
        let decision = request
            .select(self.catalog().iter())
            .map_err(|_| RuntimeError::AgentRunNotFound)?;
        Ok((
            DelegateBackendId(decision.selected_runtime_id.clone()),
            decision,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    struct StaticHistory(&'static str);

    impl RuntimePreferenceHistory for StaticHistory {
        fn preferred_runtime(&self, _profile_id: &str, eligible: &[String]) -> Option<String> {
            eligible
                .iter()
                .any(|id| id == self.0)
                .then(|| self.0.into())
        }
    }

    #[test]
    fn selection_policy_bounds_history_to_eligible_runtime_manifests() {
        let profile_id = ::contracts::AgentProfileId("reviewer".into());
        let mut policy = AgentRuntimeSelectionPolicy::default();
        policy.set_runtime_requirements(HashMap::from([(
            profile_id.clone(),
            vec![::contracts::AgentRuntimeCapability::CodeRead],
        )]));
        policy.set_capability_history(Arc::new(StaticHistory("native")));

        let manifest = |id: &str| crate::RuntimeManifest {
            id: id.into(),
            aliases: vec![],
            display_name: id.into(),
            capabilities: BTreeSet::new(),
            interaction_modes: BTreeSet::new(),
            workspace_modes: BTreeSet::new(),
            task_encodings: BTreeSet::new(),
            supported_profiles: None,
            tool_governance: crate::ToolGovernance::Mediated,
            priority: 0,
            max_context_tokens: None,
            resource_requirements: Default::default(),
        };

        assert_eq!(
            policy.runtime_requirements(&profile_id),
            vec![::contracts::AgentRuntimeCapability::CodeRead]
        );
        assert_eq!(
            policy.preferred_runtime(&profile_id, &[manifest("native"), manifest("pi")]),
            Some("native".into())
        );
        assert_eq!(
            policy.preferred_runtime(&profile_id, &[manifest("pi")]),
            None
        );
    }

    struct StubBackend;

    #[async_trait::async_trait]
    impl DelegateBackend for StubBackend {
        async fn spawn(
            &self,
            _r: &DelegateSpawnRequest,
            identity: &DelegateReceipt,
        ) -> Result<DelegateReceipt, RuntimeError> {
            Ok(DelegateReceipt {
                agent_run: identity.agent_run.clone(),
                generation: identity.generation.clone(),
            })
        }
        async fn cancel(&self, _a: &AgentRunId) -> Result<(), RuntimeError> {
            Ok(())
        }
        async fn wait(&self, _a: &AgentRunId) -> Result<crate::event::TurnTerminal, RuntimeError> {
            Ok(crate::event::TurnTerminal::Completed)
        }
    }

    #[test]
    fn registry_rejects_duplicate_backend() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend))
            .unwrap();
        assert!(registry
            .register(DelegateBackendId("native".into()), Arc::new(StubBackend),)
            .is_err());
    }

    #[test]
    fn registry_selects_only_registered_manifested_backends() {
        let registry = DelegateBackendRegistry::new();
        registry
            .register_with_manifest(
                DelegateBackendId("native".into()),
                Arc::new(StubBackend),
                crate::RuntimeManifest {
                    id: "native".into(),
                    aliases: vec!["default".into()],
                    display_name: "Native".into(),
                    capabilities: BTreeSet::from([crate::RuntimeCapability::CodeRead]),
                    interaction_modes: BTreeSet::from([crate::InteractionMode::Resident]),
                    workspace_modes: BTreeSet::from([crate::WorkspaceMode::WorkspaceLess]),
                    task_encodings: BTreeSet::from([crate::TaskEncoding::NaturalLanguage]),
                    supported_profiles: None,
                    tool_governance: crate::ToolGovernance::Mediated,
                    priority: 0,
                    max_context_tokens: None,
                    resource_requirements: Default::default(),
                },
            )
            .unwrap();
        let request = crate::RuntimeSelectionRequest {
            selector: crate::RuntimeSelector::Alias("default".into()),
            profile_id: "code-agent".into(),
            required_capabilities: vec![crate::RuntimeCapability::CodeRead],
            interaction_mode: crate::InteractionMode::Resident,
            workspace_mode: crate::WorkspaceMode::WorkspaceLess,
            task_encoding: crate::TaskEncoding::NaturalLanguage,
            max_input_tokens: 1,
        };
        let (backend, decision) = registry.select(&request).unwrap();
        assert_eq!(backend.0, "native");
        assert_eq!(decision.selected_runtime_id, "native");
        assert_eq!(registry.catalog().len(), 1);
    }

    #[test]
    fn reload_withdraws_admission_but_keeps_manifest_updates_explicit() {
        let registry = DelegateBackendRegistry::new();
        let id = DelegateBackendId("package-runtime".into());
        let manifest = crate::RuntimeManifest {
            id: id.0.clone(),
            aliases: vec![],
            display_name: "Package runtime".into(),
            capabilities: BTreeSet::new(),
            interaction_modes: BTreeSet::from([crate::InteractionMode::Resident]),
            workspace_modes: BTreeSet::from([crate::WorkspaceMode::WorkspaceLess]),
            task_encodings: BTreeSet::from([crate::TaskEncoding::NaturalLanguage]),
            supported_profiles: None,
            tool_governance: crate::ToolGovernance::Mediated,
            priority: 0,
            max_context_tokens: None,
            resource_requirements: Default::default(),
        };
        registry
            .register_with_manifest(id.clone(), Arc::new(StubBackend), manifest.clone())
            .unwrap();
        let mut replacement = manifest;
        replacement.display_name = "Reloaded package runtime".into();
        registry.replace_manifest(id.clone(), replacement).unwrap();
        assert_eq!(
            registry.catalog()[0].display_name,
            "Reloaded package runtime"
        );
        assert!(registry.unregister(&id));
        assert!(registry.resolve(&id).is_none());
        assert!(registry.catalog().is_empty());
    }
}
