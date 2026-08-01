//! Host-authorized application service for the versioned Memory Gateway.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use fabric::protocol::memory::{
    MemoryAuthorityV1, MemoryEvidenceV1, MemoryFeedbackReceiptV1, MemoryFeedbackRequestV1,
    MemoryFeedbackSignalV1, MemoryLifecycleReceiptV1, MemoryObservationKindV1,
    MemoryObservationReceiptV1, MemoryRecallItemV1, MemoryRecallRequestV1, MemoryRecallResultV1,
    MemoryReceiptGetRequestV1, MemoryRecordKindV1, MemoryScopeKindV1, MemoryScopeViewV1,
    MemorySensitivityV1, MemoryTemporalStateV1, MemoryWorkspaceBindRequestV1,
    MemoryWorkspaceBindingPreviewV1, MemoryWorkspaceBindingSpecV1, MemoryWorkspaceBindingStateV1,
    MemoryWorkspaceBindingViewV1, MemoryWorkspacePreviewBindRequestV1, MemoryWorkspaceStateV1,
    MemoryWorkspaceUnbindRequestV1, MAX_MEMORY_RECALL_CONTENT_BYTES, MAX_MEMORY_RECALL_ITEMS,
    MEMORY_FEEDBACK_RECORD_SCHEMA_V1,
};
use fabric::{Clock, PermissionProfileId, PrincipalId, WorkspacePolicy, WorkspaceSelection};
use mnemosyne::{
    GovernedMemoryObservation, MemoryAuthority, MemoryIntakeLedger, MemoryKind, MemoryScope,
    MemorySensitivity, RecallPreFilter, RecallRequest, ScopeAncestry, SupplementalCapabilityGrant,
    TemporalState, WorkspaceMemoryBinding, WorkspaceMemoryBindingPreview,
    WorkspaceMemoryBindingProposal, WorkspaceMemoryBindingRegistry, WorkspaceMemoryBindingState,
    WorkspaceMemoryKey,
};
use sha2::{Digest, Sha256};

const MEMORY_GATEWAY_DIR: &str = "memory-gateway";
const INSTALLATION_ID_FILE: &str = "installation-id";
const INTAKE_DB_FILE: &str = "intake-v1.db";
const BINDINGS_DB_FILE: &str = "workspace-bindings-v1.db";

#[async_trait::async_trait]
pub trait SupplementalBindingNegotiator: Send + Sync {
    async fn negotiate(
        &self,
        destination_handle: &str,
        backend_id: &str,
        expected_source: &str,
    ) -> anyhow::Result<SupplementalCapabilityGrant>;
}

#[async_trait::async_trait]
pub trait SupplementalBindingRecallPort: Send + Sync {
    async fn recall(
        &self,
        binding: &WorkspaceMemoryBinding,
        request: &RecallRequest,
    ) -> anyhow::Result<mnemosyne::RecallSet>;
}

pub struct MemoryGatewayService {
    ledger: Arc<MemoryIntakeLedger>,
    memory_service: Arc<dyn mnemosyne::MemoryService>,
    clock: Arc<dyn Clock>,
    installation_id: String,
    bindings: Arc<WorkspaceMemoryBindingRegistry>,
    binding_negotiator: Option<Arc<dyn SupplementalBindingNegotiator>>,
    supplemental_recall: Option<Arc<dyn SupplementalBindingRecallPort>>,
}

impl MemoryGatewayService {
    pub(crate) fn intake_ledger(&self) -> Arc<MemoryIntakeLedger> {
        self.ledger.clone()
    }

    pub(crate) fn binding_registry(&self) -> Arc<WorkspaceMemoryBindingRegistry> {
        self.bindings.clone()
    }

    pub fn open(
        state_root: impl AsRef<Path>,
        memory: Arc<dyn mnemosyne::MemoryService>,
        clock: Arc<dyn Clock>,
    ) -> anyhow::Result<Self> {
        let root = state_root.as_ref().join(MEMORY_GATEWAY_DIR);
        std::fs::create_dir_all(&root)?;
        let root_metadata = std::fs::symlink_metadata(&root)?;
        anyhow::ensure!(
            root_metadata.file_type().is_dir(),
            "memory gateway state root must be a real directory"
        );
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        let installation_id = load_or_create_installation_id(&root)?;
        let ledger = Arc::new(MemoryIntakeLedger::open(root.join(INTAKE_DB_FILE))?);
        let bindings = Arc::new(WorkspaceMemoryBindingRegistry::open(
            root.join(BINDINGS_DB_FILE),
        )?);
        Self::from_parts(ledger, memory, clock, installation_id, bindings, None, None)
    }

    pub fn from_parts(
        ledger: Arc<MemoryIntakeLedger>,
        memory: Arc<dyn mnemosyne::MemoryService>,
        clock: Arc<dyn Clock>,
        installation_id: impl Into<String>,
        bindings: Arc<WorkspaceMemoryBindingRegistry>,
        binding_negotiator: Option<Arc<dyn SupplementalBindingNegotiator>>,
        supplemental_recall: Option<Arc<dyn SupplementalBindingRecallPort>>,
    ) -> anyhow::Result<Self> {
        let installation_id = installation_id.into();
        let installation_id = installation_id.trim().to_owned();
        uuid::Uuid::parse_str(&installation_id)
            .map_err(|_| anyhow::anyhow!("invalid memory gateway installation ID"))?;
        Ok(Self {
            ledger,
            memory_service: memory,
            clock,
            installation_id,
            bindings,
            binding_negotiator,
            supplemental_recall,
        })
    }

    pub fn with_supplemental_recall(
        mut self,
        supplemental_recall: Arc<dyn SupplementalBindingRecallPort>,
    ) -> Self {
        self.supplemental_recall = Some(supplemental_recall);
        self
    }

    pub fn with_binding_negotiator(
        mut self,
        binding_negotiator: Arc<dyn SupplementalBindingNegotiator>,
    ) -> Self {
        self.binding_negotiator = Some(binding_negotiator);
        self
    }

    pub async fn observe(
        &self,
        principal_id: &PrincipalId,
        connection_kind: &str,
        request: fabric::protocol::memory::MemoryObservationRequestV1,
    ) -> anyhow::Result<MemoryObservationReceiptV1> {
        request.validate()?;
        for value in [
            request.observation_id.as_str(),
            request.client_session_id.as_str(),
        ]
        .into_iter()
        .chain(request.client_turn_id.iter().map(String::as_str))
        .chain(request.occurred_at.iter().map(String::as_str))
        .chain(request.source_refs.iter().map(String::as_str))
        {
            ensure_identifier_has_no_sensitive_material(value)?;
        }
        let workspace_key = self.resolve_workspace_key(&request.working_dir)?;
        let binding = self
            .ensure_binding(&principal_id.0, workspace_key.clone())
            .await?;
        let content_fingerprint = content_fingerprint(&self.installation_id, &request.content);
        let governed = fabric::types::data_governance::scrub_for_projection(
            &request.content,
            fabric::types::data_governance::ContentTrust::ExternalUntrusted,
        );
        let sensitivity = max_sensitivity(
            request.sensitivity_hint,
            classification_sensitivity(governed.classification),
        );
        let observation = GovernedMemoryObservation {
            observation_id: request.observation_id,
            client_session_id: request.client_session_id,
            client_turn_id: request.client_turn_id,
            kind: request.kind,
            content: governed.content,
            content_fingerprint,
            scrub_policy_version: governed.scrub_policy_version,
            scrub_redactions: governed.redactions.try_into().unwrap_or(u32::MAX),
            occurred_at: request.occurred_at,
            source_refs: request.source_refs,
            sensitivity,
            explicit_user_action: request.explicit_user_action,
            principal_id: principal_id.0.clone(),
            workspace_key,
            connection_kind: connection_kind.to_owned(),
            observed_at_ms: self.clock.wall_now().0.max(0),
        };
        let ledger = self.ledger.clone();
        let mut receipt = tokio::task::spawn_blocking(move || ledger.observe(&observation))
            .await
            .map_err(|error| anyhow::anyhow!("memory intake task failed: {error}"))?
            .map_err(anyhow::Error::from)?;
        receipt.workspace_state = wire_workspace_state(binding.state);
        Ok(receipt)
    }

    pub async fn receipt(
        &self,
        principal_id: &PrincipalId,
        request: MemoryReceiptGetRequestV1,
    ) -> anyhow::Result<MemoryLifecycleReceiptV1> {
        request.validate()?;
        let ledger = self.ledger.clone();
        let principal_id = principal_id.0.clone();
        let durable_intake_id = request.durable_intake_id;
        tokio::task::spawn_blocking(move || {
            ledger.receipt_for_principal(&principal_id, &durable_intake_id)
        })
        .await
        .map_err(|error| anyhow::anyhow!("memory receipt task failed: {error}"))??
        .ok_or_else(|| anyhow::anyhow!("memory intake was not found for this principal"))
    }

    pub async fn feedback(
        &self,
        principal_id: &PrincipalId,
        connection_kind: &str,
        request: MemoryFeedbackRequestV1,
    ) -> anyhow::Result<MemoryFeedbackReceiptV1> {
        request.validate()?;
        let workspace_key = self.resolve_workspace_key(&request.working_dir)?;
        let ledger = self.ledger.clone();
        let principal = principal_id.0.clone();
        let target_record_id = request.target_record_id.clone();
        let workspace = workspace_key.as_str().to_owned();
        let visible = tokio::task::spawn_blocking(move || {
            ledger.is_record_visible(&principal, &workspace, &target_record_id)
        })
        .await
        .map_err(|error| anyhow::anyhow!("memory visibility task failed: {error}"))??;
        anyhow::ensure!(
            visible,
            "feedback target is not visible in this principal and workspace ancestry"
        );
        let content = serde_json::to_string(&serde_json::json!({
            "schema": MEMORY_FEEDBACK_RECORD_SCHEMA_V1,
            "target_record_id": request.target_record_id,
            "signal": request.signal,
            "correction_text": request.correction_text,
        }))?;
        let sensitivity_hint = if request.signal == MemoryFeedbackSignalV1::Sensitive {
            MemorySensitivityV1::Restricted
        } else {
            MemorySensitivityV1::Internal
        };
        let observation = self
            .observe(
                principal_id,
                connection_kind,
                fabric::protocol::memory::MemoryObservationRequestV1 {
                    observation_id: request.observation_id,
                    client_session_id: request.client_session_id,
                    client_turn_id: None,
                    working_dir: request.working_dir,
                    kind: MemoryObservationKindV1::Feedback,
                    content,
                    occurred_at: None,
                    source_refs: vec![request.target_record_id],
                    sensitivity_hint,
                    explicit_user_action: true,
                },
            )
            .await?;
        let lifecycle = self
            .receipt(
                principal_id,
                MemoryReceiptGetRequestV1 {
                    durable_intake_id: observation.durable_intake_id.clone(),
                },
            )
            .await?;
        Ok(MemoryFeedbackReceiptV1 {
            observation,
            lifecycle,
        })
    }

    pub async fn recall(
        &self,
        principal_id: &PrincipalId,
        connection_kind: &str,
        request: MemoryRecallRequestV1,
    ) -> anyhow::Result<MemoryRecallResultV1> {
        request.validate()?;
        let workspace_key = self.resolve_workspace_key(&request.working_dir)?;
        let binding = self
            .ensure_binding(&principal_id.0, workspace_key.clone())
            .await?;
        let session_scope =
            client_session_scope(&principal_id.0, connection_kind, &request.client_session_id);
        let requested_kinds = request.requested_kinds.clone();
        let local_request = RecallRequest {
            session: session_scope.clone(),
            query: request.query.clone(),
            max_items: request.max_items.min(MAX_MEMORY_RECALL_ITEMS),
            max_content_bytes: request
                .max_content_bytes
                .min(MAX_MEMORY_RECALL_CONTENT_BYTES),
            current_at: None,
            include_historical: request.include_historical,
            mode: None,
        };
        let prefilter = RecallPreFilter {
            ancestry: ScopeAncestry {
                principal_id: Some(principal_id.0.clone()),
                workspace_id: Some(workspace_key.as_str().to_owned()),
                session_id: Some(session_scope),
                ..Default::default()
            },
            max_sensitivity: MemorySensitivity::Restricted,
            allowed_authorities: all_authorities(),
        };
        let mut recalled = self
            .memory_service
            .recall_with_prefilter(local_request.clone(), &prefilter)
            .await?;
        if binding.state == WorkspaceMemoryBindingState::Active {
            match self.verify_active_binding(&binding).await {
                Ok(true) => {
                    if let Some(port) = &self.supplemental_recall {
                        match port.recall(&binding, &local_request).await {
                            Ok(remote) => {
                                recalled.items.extend(remote.items);
                                recalled.degraded_sources.extend(remote.degraded_sources);
                            }
                            Err(error) => {
                                tracing::warn!(%error, workspace = %binding.workspace_key, "bound supplemental recall degraded");
                                recalled.degraded_sources.push("supplemental".into());
                            }
                        }
                    } else {
                        recalled.degraded_sources.push("supplemental".into());
                    }
                }
                Ok(false) => recalled
                    .degraded_sources
                    .push("supplemental_incompatible".into()),
                Err(error) => {
                    tracing::warn!(%error, workspace = %binding.workspace_key, "supplemental grant revalidation unavailable");
                    recalled.degraded_sources.push("supplemental".into());
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut used_bytes = 0usize;
        let items = recalled
            .items
            .into_iter()
            .filter(|item| {
                requested_kinds
                    .as_ref()
                    .is_none_or(|kinds| kinds.contains(&record_kind(item.kind)))
            })
            .filter(|item| seen.insert(item.metadata.record_id.clone()))
            .filter(|item| {
                let next = used_bytes.saturating_add(item.content.len());
                if next > local_request.max_content_bytes {
                    return false;
                }
                used_bytes = next;
                true
            })
            .take(local_request.max_items)
            .map(memory_item)
            .collect::<Vec<_>>();
        let visible_record_ids = items
            .iter()
            .map(|item| item.record_id.clone())
            .collect::<Vec<_>>();
        let ledger = self.ledger.clone();
        let principal = principal_id.0.clone();
        let workspace = workspace_key.as_str().to_owned();
        let seen_at_ms = self.clock.wall_now().0.max(0);
        tokio::task::spawn_blocking(move || {
            ledger.remember_visible_records(&principal, &workspace, &visible_record_ids, seen_at_ms)
        })
        .await
        .map_err(|error| anyhow::anyhow!("memory visibility task failed: {error}"))??;
        Ok(MemoryRecallResultV1 {
            request_id: request.request_id,
            items,
            degraded_sources: recalled.degraded_sources,
        })
    }

    pub async fn preview_workspace_bind(
        &self,
        principal_id: &PrincipalId,
        request: MemoryWorkspacePreviewBindRequestV1,
    ) -> anyhow::Result<MemoryWorkspaceBindingPreviewV1> {
        request.validate()?;
        validate_binding_spec_safety(&request.binding)?;
        let workspace_key = self.resolve_workspace_key(&request.working_dir)?;
        let proposal = binding_proposal(request.binding);
        let grant = self.negotiate_binding(&proposal).await?;
        let registry = self.bindings.clone();
        let principal = principal_id.0.clone();
        let now_ms = self.clock.wall_now().0.max(0);
        let preview = tokio::task::spawn_blocking(move || {
            registry.preview(&principal, &workspace_key, &proposal, &grant, now_ms)
        })
        .await
        .map_err(|error| anyhow::anyhow!("memory binding preview task failed: {error}"))??;
        Ok(binding_preview_view(preview))
    }

    pub async fn bind_workspace(
        &self,
        principal_id: &PrincipalId,
        request: MemoryWorkspaceBindRequestV1,
    ) -> anyhow::Result<MemoryWorkspaceBindingViewV1> {
        request.validate()?;
        validate_binding_spec_safety(&request.binding)?;
        let workspace_key = self.resolve_workspace_key(&request.working_dir)?;
        let proposal = binding_proposal(request.binding);
        let grant = self.negotiate_binding(&proposal).await?;
        let registry = self.bindings.clone();
        let principal = principal_id.0.clone();
        let now_ms = self.clock.wall_now().0.max(0);
        let expected_digest = request.expected_capability_digest;
        let binding = tokio::task::spawn_blocking(move || {
            let preview =
                registry.preview(&principal, &workspace_key, &proposal, &grant, now_ms)?;
            if !preview.compatible
                || preview.binding.verified_capability_digest.as_deref()
                    != Some(expected_digest.as_str())
            {
                return Err(mnemosyne::WorkspaceMemoryBindingError::Invalid);
            }
            registry.apply(&preview)
        })
        .await
        .map_err(|error| anyhow::anyhow!("memory binding apply task failed: {error}"))??;
        Ok(binding_view(binding))
    }

    pub async fn unbind_workspace(
        &self,
        principal_id: &PrincipalId,
        request: MemoryWorkspaceUnbindRequestV1,
    ) -> anyhow::Result<MemoryWorkspaceBindingViewV1> {
        request.validate()?;
        let workspace_key = self.resolve_workspace_key(&request.working_dir)?;
        let registry = self.bindings.clone();
        let principal = principal_id.0.clone();
        let now_ms = self.clock.wall_now().0.max(0);
        let binding = tokio::task::spawn_blocking(move || {
            registry.local_only(&principal, &workspace_key, now_ms)?;
            registry
                .revoke(&principal, &workspace_key, now_ms)?
                .ok_or(mnemosyne::WorkspaceMemoryBindingError::Corrupt)
        })
        .await
        .map_err(|error| anyhow::anyhow!("memory binding revoke task failed: {error}"))??;
        Ok(binding_view(binding))
    }

    async fn ensure_binding(
        &self,
        principal_id: &str,
        workspace_key: WorkspaceMemoryKey,
    ) -> anyhow::Result<WorkspaceMemoryBinding> {
        let registry = self.bindings.clone();
        let principal = principal_id.to_owned();
        let now_ms = self.clock.wall_now().0.max(0);
        tokio::task::spawn_blocking(move || registry.local_only(&principal, &workspace_key, now_ms))
            .await
            .map_err(|error| anyhow::anyhow!("memory binding lookup task failed: {error}"))?
            .map_err(anyhow::Error::from)
    }

    async fn negotiate_binding(
        &self,
        proposal: &WorkspaceMemoryBindingProposal,
    ) -> anyhow::Result<SupplementalCapabilityGrant> {
        let negotiator = self
            .binding_negotiator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("supplemental binding adapter is unavailable"))?;
        let write = negotiator
            .negotiate(
                &proposal.write_destination_handle,
                &proposal.backend_id,
                &proposal.expected_write_source,
            )
            .await?;
        anyhow::ensure!(
            proposal.read_destination_handles.len() == proposal.expected_read_sources.len(),
            "supplemental read destinations must pair one-to-one with expected sources"
        );
        let mut reads = std::collections::BTreeSet::new();
        let mut can_read = true;
        for (handle, expected_source) in proposal
            .read_destination_handles
            .iter()
            .zip(&proposal.expected_read_sources)
        {
            let grant = negotiator
                .negotiate(handle, &proposal.backend_id, expected_source)
                .await?;
            anyhow::ensure!(
                grant.backend_id == proposal.backend_id,
                "supplemental binding backend identity changed during negotiation"
            );
            can_read &= grant.can_read;
            reads.extend(grant.read_sources);
        }
        Ok(SupplementalCapabilityGrant {
            backend_id: write.backend_id,
            write_source: write.write_source,
            read_sources: reads.into_iter().collect(),
            can_read,
            can_write: write.can_write,
        })
    }

    async fn verify_active_binding(
        &self,
        binding: &WorkspaceMemoryBinding,
    ) -> anyhow::Result<bool> {
        let proposal = WorkspaceMemoryBindingProposal {
            backend_id: binding.backend_id.clone(),
            write_destination_handle: binding.write_destination_handle.clone(),
            read_destination_handles: binding.read_destination_handles.clone(),
            expected_write_source: binding.expected_write_source.clone(),
            expected_read_sources: binding.expected_read_sources.clone(),
            credential_ref: binding.credential_ref.clone(),
        };
        let grant = self.negotiate_binding(&proposal).await?;
        let registry = self.bindings.clone();
        let binding = binding.clone();
        let now_ms = self.clock.wall_now().0.max(0);
        tokio::task::spawn_blocking(move || {
            let preview = registry.preview(
                &binding.principal_id,
                &binding.workspace_key,
                &proposal,
                &grant,
                now_ms,
            )?;
            let unchanged = preview.compatible
                && preview.binding.verified_capability_digest == binding.verified_capability_digest;
            if !unchanged {
                let _ = registry.mark_incompatible(
                    &binding.principal_id,
                    &binding.workspace_key,
                    binding.revision,
                    now_ms,
                )?;
            }
            Ok::<_, mnemosyne::WorkspaceMemoryBindingError>(unchanged)
        })
        .await
        .map_err(|error| anyhow::anyhow!("memory binding verification task failed: {error}"))?
        .map_err(anyhow::Error::from)
    }

    fn resolve_workspace_key(&self, working_dir: &Path) -> anyhow::Result<WorkspaceMemoryKey> {
        let workspace = resolve_memory_workspace(working_dir)?;
        let identity = crate::application::workspace_trust::workspace_identity(workspace.cwd());
        WorkspaceMemoryKey::derive(&identity, &self.installation_id)
    }
}

fn binding_proposal(value: MemoryWorkspaceBindingSpecV1) -> WorkspaceMemoryBindingProposal {
    WorkspaceMemoryBindingProposal {
        backend_id: value.backend_id,
        write_destination_handle: value.write_destination_handle,
        read_destination_handles: value.read_destination_handles,
        expected_write_source: value.expected_write_source,
        expected_read_sources: value.expected_read_sources,
        credential_ref: value.credential_ref,
    }
}

fn validate_binding_spec_safety(value: &MemoryWorkspaceBindingSpecV1) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.read_destination_handles.len() == value.expected_read_sources.len(),
        "memory read destinations must pair one-to-one with expected sources"
    );
    for field in [
        value.backend_id.as_str(),
        value.write_destination_handle.as_str(),
        value.expected_write_source.as_str(),
        value.credential_ref.as_str(),
    ]
    .into_iter()
    .chain(value.read_destination_handles.iter().map(String::as_str))
    .chain(value.expected_read_sources.iter().map(String::as_str))
    {
        ensure_identifier_has_no_sensitive_material(field)?;
    }
    anyhow::ensure!(
        ["mcp-server:", "systemd:", "secret-store:"]
            .iter()
            .any(|prefix| value.credential_ref.starts_with(prefix)),
        "memory credential_ref must be an opaque supported secret-store reference"
    );
    Ok(())
}

fn binding_preview_view(value: WorkspaceMemoryBindingPreview) -> MemoryWorkspaceBindingPreviewV1 {
    MemoryWorkspaceBindingPreviewV1 {
        binding: binding_view(value.binding),
        compatible: value.compatible,
        reason_codes: value.reason_codes,
    }
}

fn binding_view(value: WorkspaceMemoryBinding) -> MemoryWorkspaceBindingViewV1 {
    MemoryWorkspaceBindingViewV1 {
        workspace_key: value.workspace_key.as_str().to_owned(),
        backend_id: value.backend_id,
        write_destination_handle: value.write_destination_handle,
        read_destination_handles: value.read_destination_handles,
        expected_write_source: value.expected_write_source,
        expected_read_sources: value.expected_read_sources,
        credential_ref: value.credential_ref,
        state: match value.state {
            WorkspaceMemoryBindingState::Active => MemoryWorkspaceBindingStateV1::Active,
            WorkspaceMemoryBindingState::LocalOnly => MemoryWorkspaceBindingStateV1::LocalOnly,
            WorkspaceMemoryBindingState::Incompatible => {
                MemoryWorkspaceBindingStateV1::Incompatible
            }
            WorkspaceMemoryBindingState::Revoked => MemoryWorkspaceBindingStateV1::Revoked,
        },
        verified_capability_digest: value.verified_capability_digest,
        revision: value.revision,
    }
}

fn wire_workspace_state(value: WorkspaceMemoryBindingState) -> MemoryWorkspaceStateV1 {
    match value {
        WorkspaceMemoryBindingState::Active => MemoryWorkspaceStateV1::Bound,
        WorkspaceMemoryBindingState::Incompatible => MemoryWorkspaceStateV1::Incompatible,
        WorkspaceMemoryBindingState::LocalOnly | WorkspaceMemoryBindingState::Revoked => {
            MemoryWorkspaceStateV1::LocalOnly
        }
    }
}

fn resolve_memory_workspace(working_dir: &Path) -> anyhow::Result<WorkspacePolicy> {
    anyhow::ensure!(
        working_dir.is_absolute(),
        "memory working_dir must be absolute"
    );
    WorkspaceSelection::new(Some(working_dir.to_path_buf()), Vec::new())
        .resolve_with_profile(Path::new("/"), &PermissionProfileId::workspace_write())
        .map_err(anyhow::Error::from)
}

fn load_or_create_installation_id(root: &Path) -> anyhow::Result<String> {
    let path = root.join(INSTALLATION_ID_FILE);
    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                let value = uuid::Uuid::new_v4().to_string();
                file.write_all(value.as_bytes())?;
                file.write_all(b"\n")?;
                file.sync_all()?;
                return Ok(value);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(&path)?;
                anyhow::ensure!(
                    metadata.file_type().is_file() && metadata.len() <= 128,
                    "memory installation ID must be a bounded regular file"
                );
                let mut file = File::open(&path)?;
                let mut value = String::new();
                file.read_to_string(&mut value)?;
                let value = value.trim();
                uuid::Uuid::parse_str(value)
                    .map_err(|_| anyhow::anyhow!("invalid persisted memory installation ID"))?;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
                return Ok(value.to_owned());
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn client_session_scope(
    principal_id: &str,
    connection_kind: &str,
    client_session_id: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"aletheon.memory-client-session.v1\0");
    for value in [principal_id, connection_kind, client_session_id] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    format!("memory-session:sha256:{:x}", hasher.finalize())
}

fn content_fingerprint(installation_id: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"aletheon.memory-content-fingerprint.v1\0");
    for value in [installation_id.as_bytes(), content.as_bytes()] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value);
    }
    format!("keyed-sha256:{:x}", hasher.finalize())
}

fn ensure_identifier_has_no_sensitive_material(value: &str) -> anyhow::Result<()> {
    let governed = fabric::types::data_governance::scrub_for_projection(
        value,
        fabric::types::data_governance::ContentTrust::ExternalUntrusted,
    );
    anyhow::ensure!(
        governed.redactions == 0,
        "memory identifier contains sensitive material"
    );
    Ok(())
}

fn classification_sensitivity(
    value: fabric::types::data_governance::DataClassification,
) -> MemorySensitivityV1 {
    use fabric::types::data_governance::DataClassification;
    match value {
        DataClassification::Public => MemorySensitivityV1::Public,
        DataClassification::Internal => MemorySensitivityV1::Internal,
        DataClassification::Confidential => MemorySensitivityV1::Confidential,
        DataClassification::Restricted => MemorySensitivityV1::Restricted,
    }
}

fn max_sensitivity(left: MemorySensitivityV1, right: MemorySensitivityV1) -> MemorySensitivityV1 {
    let rank = |value| match value {
        MemorySensitivityV1::Public => 0,
        MemorySensitivityV1::Internal => 1,
        MemorySensitivityV1::Confidential => 2,
        MemorySensitivityV1::Restricted => 3,
    };
    if rank(left) >= rank(right) {
        left
    } else {
        right
    }
}

fn all_authorities() -> Vec<MemoryAuthority> {
    vec![
        MemoryAuthority::ApprovedCore,
        MemoryAuthority::VerifiedLocalSemantic,
        MemoryAuthority::LocalEpisode,
        MemoryAuthority::AletheonExternal,
        MemoryAuthority::ExternalReference,
        MemoryAuthority::RawExperience,
    ]
}

fn memory_item(item: mnemosyne::RecallItem) -> MemoryRecallItemV1 {
    MemoryRecallItemV1 {
        record_id: item.metadata.record_id,
        kind: record_kind(item.kind),
        scope: scope_view(item.scope),
        authority: authority(item.authority),
        temporal_state: temporal_state(item.temporal_state),
        evidence: item.evidence.map(evidence),
        score: if item.score.is_finite() {
            item.score
        } else {
            0.0
        },
        sensitivity: sensitivity(item.metadata.sensitivity),
        source: item.metadata.provenance.source,
        source_id: item.metadata.provenance.source_id,
        content: item.content,
        untrusted_reference: matches!(
            item.authority,
            MemoryAuthority::AletheonExternal | MemoryAuthority::ExternalReference
        ),
    }
}

fn record_kind(kind: MemoryKind) -> MemoryRecordKindV1 {
    match kind {
        MemoryKind::Message => MemoryRecordKindV1::Message,
        MemoryKind::ToolOutcome => MemoryRecordKindV1::ToolOutcome,
        MemoryKind::GoalOutcome => MemoryRecordKindV1::GoalOutcome,
        MemoryKind::Reflection => MemoryRecordKindV1::Reflection,
        MemoryKind::Episodic => MemoryRecordKindV1::Episodic,
        MemoryKind::SemanticFact => MemoryRecordKindV1::SemanticFact,
        MemoryKind::Procedure => MemoryRecordKindV1::Procedure,
        MemoryKind::CoreState => MemoryRecordKindV1::CoreState,
        MemoryKind::ArchitectureDecision => MemoryRecordKindV1::ArchitectureDecision,
        MemoryKind::ExternalReference => MemoryRecordKindV1::ExternalReference,
    }
}

fn scope_view(scope: MemoryScope) -> MemoryScopeViewV1 {
    let (kind, id) = match scope {
        MemoryScope::Global => (MemoryScopeKindV1::Global, None),
        MemoryScope::Principal(id) => (MemoryScopeKindV1::Principal, Some(id)),
        MemoryScope::Workspace(id) => (MemoryScopeKindV1::Workspace, Some(id)),
        MemoryScope::Session(id) => (MemoryScopeKindV1::Session, Some(id)),
        MemoryScope::Goal(id) => (MemoryScopeKindV1::Goal, Some(id)),
        MemoryScope::Agent(id) => (MemoryScopeKindV1::Agent, Some(id)),
        MemoryScope::Task(id) => (MemoryScopeKindV1::Task, Some(id)),
    };
    MemoryScopeViewV1 { kind, id }
}

fn authority(value: MemoryAuthority) -> MemoryAuthorityV1 {
    match value {
        MemoryAuthority::ApprovedCore => MemoryAuthorityV1::ApprovedCore,
        MemoryAuthority::VerifiedLocalSemantic => MemoryAuthorityV1::VerifiedLocalSemantic,
        MemoryAuthority::LocalEpisode => MemoryAuthorityV1::LocalEpisode,
        MemoryAuthority::AletheonExternal => MemoryAuthorityV1::AletheonExternal,
        MemoryAuthority::ExternalReference => MemoryAuthorityV1::ExternalReference,
        MemoryAuthority::RawExperience => MemoryAuthorityV1::RawExperience,
    }
}

fn temporal_state(value: TemporalState) -> MemoryTemporalStateV1 {
    match value {
        TemporalState::Current => MemoryTemporalStateV1::Current,
        TemporalState::Superseded => MemoryTemporalStateV1::Superseded,
        TemporalState::Expired => MemoryTemporalStateV1::Expired,
        TemporalState::Unknown => MemoryTemporalStateV1::Unknown,
    }
}

fn evidence(value: mnemosyne::EvidenceLevel) -> MemoryEvidenceV1 {
    match value {
        mnemosyne::EvidenceLevel::AliasHit => MemoryEvidenceV1::AliasHit,
        mnemosyne::EvidenceLevel::ExactTitleMatch => MemoryEvidenceV1::ExactTitleMatch,
        mnemosyne::EvidenceLevel::HighVectorMatch => MemoryEvidenceV1::HighVectorMatch,
        mnemosyne::EvidenceLevel::KeywordExact => MemoryEvidenceV1::KeywordExact,
        mnemosyne::EvidenceLevel::WeakSemantic => MemoryEvidenceV1::WeakSemantic,
    }
}

fn sensitivity(value: MemorySensitivity) -> MemorySensitivityV1 {
    match value {
        MemorySensitivity::Public => MemorySensitivityV1::Public,
        MemorySensitivity::Internal => MemorySensitivityV1::Internal,
        MemorySensitivity::Confidential => MemorySensitivityV1::Confidential,
        MemorySensitivity::Restricted => MemorySensitivityV1::Restricted,
    }
}
