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
    MemorySensitivityV1, MemoryTemporalStateV1, MAX_MEMORY_RECALL_CONTENT_BYTES,
    MAX_MEMORY_RECALL_ITEMS,
};
use fabric::{Clock, PermissionProfileId, PrincipalId, WorkspacePolicy, WorkspaceSelection};
use mnemosyne::{
    GovernedMemoryObservation, MemoryAuthority, MemoryIntakeLedger, MemoryKind, MemoryScope,
    MemorySensitivity, RecallPreFilter, RecallRequest, ScopeAncestry, TemporalState,
    WorkspaceMemoryKey,
};
use sha2::{Digest, Sha256};

const MEMORY_GATEWAY_DIR: &str = "memory-gateway";
const INSTALLATION_ID_FILE: &str = "installation-id";
const INTAKE_DB_FILE: &str = "intake-v1.db";

pub struct MemoryGatewayService {
    ledger: Arc<MemoryIntakeLedger>,
    memory: Arc<dyn mnemosyne::MemoryService>,
    clock: Arc<dyn Clock>,
    installation_id: String,
}

impl MemoryGatewayService {
    pub(crate) fn intake_ledger(&self) -> Arc<MemoryIntakeLedger> {
        self.ledger.clone()
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
        Self::from_parts(ledger, memory, clock, installation_id)
    }

    pub fn from_parts(
        ledger: Arc<MemoryIntakeLedger>,
        memory: Arc<dyn mnemosyne::MemoryService>,
        clock: Arc<dyn Clock>,
        installation_id: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let installation_id = installation_id.into();
        let installation_id = installation_id.trim().to_owned();
        uuid::Uuid::parse_str(&installation_id)
            .map_err(|_| anyhow::anyhow!("invalid memory gateway installation ID"))?;
        Ok(Self {
            ledger,
            memory,
            clock,
            installation_id,
        })
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
        tokio::task::spawn_blocking(move || ledger.observe(&observation))
            .await
            .map_err(|error| anyhow::anyhow!("memory intake task failed: {error}"))?
            .map_err(anyhow::Error::from)
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
            "schema": "aletheon.memory.feedback/v1",
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
        let recalled = self
            .memory
            .recall_with_prefilter(local_request, &prefilter)
            .await?;
        let items = recalled
            .items
            .into_iter()
            .filter(|item| {
                requested_kinds
                    .as_ref()
                    .is_none_or(|kinds| kinds.contains(&record_kind(item.kind)))
            })
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

    fn resolve_workspace_key(&self, working_dir: &Path) -> anyhow::Result<WorkspaceMemoryKey> {
        let workspace = resolve_memory_workspace(working_dir)?;
        let identity = crate::application::workspace_trust::workspace_identity(workspace.cwd());
        WorkspaceMemoryKey::derive(&identity, &self.installation_id)
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
