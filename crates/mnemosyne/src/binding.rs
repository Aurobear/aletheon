//! Durable, product-neutral workspace-to-supplemental-memory bindings.
//!
//! A binding stores opaque adapter and credential handles, never credentials.
//! Remote authority becomes active only after a backend capability handshake
//! exactly matches the operator-declared source set.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::WorkspaceMemoryKey;

const SCHEMA_VERSION: i64 = 1;
const MAX_ID_BYTES: usize = 512;
const MAX_READ_SOURCES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMemoryBindingState {
    Active,
    LocalOnly,
    Incompatible,
    Revoked,
}

impl WorkspaceMemoryBindingState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::LocalOnly => "local_only",
            Self::Incompatible => "incompatible",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> Result<Self, WorkspaceMemoryBindingError> {
        match value {
            "active" => Ok(Self::Active),
            "local_only" => Ok(Self::LocalOnly),
            "incompatible" => Ok(Self::Incompatible),
            "revoked" => Ok(Self::Revoked),
            _ => Err(WorkspaceMemoryBindingError::Corrupt),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMemoryBinding {
    pub schema_version: u16,
    pub workspace_key: WorkspaceMemoryKey,
    pub principal_id: String,
    pub backend_id: String,
    pub write_destination_handle: String,
    pub read_destination_handles: Vec<String>,
    pub expected_write_source: String,
    pub expected_read_sources: Vec<String>,
    pub credential_ref: String,
    pub state: WorkspaceMemoryBindingState,
    pub verified_capability_digest: Option<String>,
    pub revision: u64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMemoryBindingProposal {
    pub backend_id: String,
    pub write_destination_handle: String,
    pub read_destination_handles: Vec<String>,
    pub expected_write_source: String,
    pub expected_read_sources: Vec<String>,
    pub credential_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplementalCapabilityGrant {
    pub backend_id: String,
    pub write_source: Option<String>,
    pub read_sources: Vec<String>,
    pub can_read: bool,
    pub can_write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMemoryBindingPreview {
    pub binding: WorkspaceMemoryBinding,
    pub compatible: bool,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceMemoryBindingError {
    #[error("workspace memory binding input is invalid")]
    Invalid,
    #[error("workspace memory binding storage is corrupt")]
    Corrupt,
    #[error("workspace memory binding storage error")]
    Storage(#[from] rusqlite::Error),
    #[error("workspace memory binding filesystem error")]
    Io(#[from] std::io::Error),
}

pub struct WorkspaceMemoryBindingRegistry {
    path: PathBuf,
}

impl WorkspaceMemoryBindingRegistry {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WorkspaceMemoryBindingError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(WorkspaceMemoryBindingError::Invalid);
        }
        let registry = Self { path };
        let connection = registry.connection()?;
        migrate(&connection)?;
        fs::set_permissions(&registry.path, fs::Permissions::from_mode(0o600))?;
        Ok(registry)
    }

    pub fn local_only(
        &self,
        principal_id: &str,
        workspace_key: &WorkspaceMemoryKey,
        now_ms: i64,
    ) -> Result<WorkspaceMemoryBinding, WorkspaceMemoryBindingError> {
        validate_id(principal_id)?;
        validate_id(workspace_key.as_str())?;
        if let Some(binding) = self.get(principal_id, workspace_key)? {
            return Ok(binding);
        }
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO workspace_memory_bindings(
                principal_id,workspace_key,backend_id,write_destination_handle,
                read_destination_handles_json,expected_write_source,
                expected_read_sources_json,credential_ref,state,
                verified_capability_digest,revision,updated_at_ms
             ) VALUES(?1,?2,'','','[]','','[]','','local_only',NULL,1,?3)",
            params![principal_id, workspace_key.as_str(), now_ms],
        )?;
        self.get(principal_id, workspace_key)?
            .ok_or(WorkspaceMemoryBindingError::Corrupt)
    }

    pub fn preview(
        &self,
        principal_id: &str,
        workspace_key: &WorkspaceMemoryKey,
        proposal: &WorkspaceMemoryBindingProposal,
        grant: &SupplementalCapabilityGrant,
        now_ms: i64,
    ) -> Result<WorkspaceMemoryBindingPreview, WorkspaceMemoryBindingError> {
        validate_proposal(principal_id, workspace_key, proposal, grant)?;
        let mut reasons = Vec::new();
        if grant.backend_id != proposal.backend_id {
            reasons.push("backend_mismatch".to_owned());
        }
        if !grant.can_write {
            reasons.push("write_scope_missing".to_owned());
        }
        if !grant.can_read {
            reasons.push("read_scope_missing".to_owned());
        }
        if grant.write_source.as_deref() != Some(proposal.expected_write_source.as_str()) {
            reasons.push("write_source_mismatch".to_owned());
        }
        let expected_reads = normalized_set(&proposal.expected_read_sources);
        let actual_reads = normalized_set(&grant.read_sources);
        if expected_reads != actual_reads {
            reasons.push("read_sources_mismatch".to_owned());
        }
        let compatible = reasons.is_empty();
        let prior_revision = self
            .get(principal_id, workspace_key)?
            .map_or(0, |binding| binding.revision);
        let digest = compatible.then(|| capability_digest(grant));
        Ok(WorkspaceMemoryBindingPreview {
            binding: WorkspaceMemoryBinding {
                schema_version: SCHEMA_VERSION as u16,
                workspace_key: workspace_key.clone(),
                principal_id: principal_id.to_owned(),
                backend_id: proposal.backend_id.clone(),
                write_destination_handle: proposal.write_destination_handle.clone(),
                read_destination_handles: proposal.read_destination_handles.clone(),
                expected_write_source: proposal.expected_write_source.clone(),
                expected_read_sources: proposal.expected_read_sources.clone(),
                credential_ref: proposal.credential_ref.clone(),
                state: if compatible {
                    WorkspaceMemoryBindingState::Active
                } else {
                    WorkspaceMemoryBindingState::Incompatible
                },
                verified_capability_digest: digest,
                revision: prior_revision.saturating_add(1),
                updated_at_ms: now_ms,
            },
            compatible,
            reason_codes: reasons,
        })
    }

    /// Atomically persists the exact preview only if the previous revision has
    /// not changed. Callers must obtain a fresh backend handshake for each
    /// preview; this method never trusts config-only source declarations.
    pub fn apply(
        &self,
        preview: &WorkspaceMemoryBindingPreview,
    ) -> Result<WorkspaceMemoryBinding, WorkspaceMemoryBindingError> {
        let value = &preview.binding;
        let expected_prior = value.revision.saturating_sub(1);
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_revision: Option<u64> = tx
            .query_row(
                "SELECT revision FROM workspace_memory_bindings WHERE principal_id=?1 AND workspace_key=?2",
                params![value.principal_id, value.workspace_key.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if current_revision.unwrap_or(0) != expected_prior {
            return Err(WorkspaceMemoryBindingError::Invalid);
        }
        tx.execute(
            "INSERT INTO workspace_memory_bindings(
                principal_id,workspace_key,backend_id,write_destination_handle,
                read_destination_handles_json,expected_write_source,
                expected_read_sources_json,credential_ref,state,
                verified_capability_digest,revision,updated_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(principal_id,workspace_key) DO UPDATE SET
                backend_id=excluded.backend_id,
                write_destination_handle=excluded.write_destination_handle,
                read_destination_handles_json=excluded.read_destination_handles_json,
                expected_write_source=excluded.expected_write_source,
                expected_read_sources_json=excluded.expected_read_sources_json,
                credential_ref=excluded.credential_ref,state=excluded.state,
                verified_capability_digest=excluded.verified_capability_digest,
                revision=excluded.revision,updated_at_ms=excluded.updated_at_ms",
            params![
                value.principal_id,
                value.workspace_key.as_str(),
                value.backend_id,
                value.write_destination_handle,
                serde_json::to_string(&value.read_destination_handles)
                    .map_err(|_| WorkspaceMemoryBindingError::Invalid)?,
                value.expected_write_source,
                serde_json::to_string(&value.expected_read_sources)
                    .map_err(|_| WorkspaceMemoryBindingError::Invalid)?,
                value.credential_ref,
                value.state.as_str(),
                value.verified_capability_digest,
                value.revision,
                value.updated_at_ms,
            ],
        )?;
        tx.commit()?;
        Ok(value.clone())
    }

    pub fn revoke(
        &self,
        principal_id: &str,
        workspace_key: &WorkspaceMemoryKey,
        now_ms: i64,
    ) -> Result<Option<WorkspaceMemoryBinding>, WorkspaceMemoryBindingError> {
        validate_id(principal_id)?;
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE workspace_memory_bindings SET state='revoked',
             verified_capability_digest=NULL,revision=revision+1,updated_at_ms=?3
             WHERE principal_id=?1 AND workspace_key=?2",
            params![principal_id, workspace_key.as_str(), now_ms],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.get(principal_id, workspace_key)
    }

    pub fn get(
        &self,
        principal_id: &str,
        workspace_key: &WorkspaceMemoryKey,
    ) -> Result<Option<WorkspaceMemoryBinding>, WorkspaceMemoryBindingError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT backend_id,write_destination_handle,read_destination_handles_json,
                        expected_write_source,expected_read_sources_json,credential_ref,state,
                        verified_capability_digest,revision,updated_at_ms
                 FROM workspace_memory_bindings WHERE principal_id=?1 AND workspace_key=?2",
                params![principal_id, workspace_key.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, u64>(8)?,
                        row.get::<_, i64>(9)?,
                    ))
                },
            )
            .optional()?
            .map(|row| decode_binding(principal_id, workspace_key, row))
            .transpose()
    }

    fn connection(&self) -> Result<Connection, WorkspaceMemoryBindingError> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(connection)
    }
}

type StoredBinding = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    u64,
    i64,
);

fn decode_binding(
    principal_id: &str,
    workspace_key: &WorkspaceMemoryKey,
    row: StoredBinding,
) -> Result<WorkspaceMemoryBinding, WorkspaceMemoryBindingError> {
    Ok(WorkspaceMemoryBinding {
        schema_version: SCHEMA_VERSION as u16,
        workspace_key: workspace_key.clone(),
        principal_id: principal_id.to_owned(),
        backend_id: row.0,
        write_destination_handle: row.1,
        read_destination_handles: serde_json::from_str(&row.2)
            .map_err(|_| WorkspaceMemoryBindingError::Corrupt)?,
        expected_write_source: row.3,
        expected_read_sources: serde_json::from_str(&row.4)
            .map_err(|_| WorkspaceMemoryBindingError::Corrupt)?,
        credential_ref: row.5,
        state: WorkspaceMemoryBindingState::parse(&row.6)?,
        verified_capability_digest: row.7,
        revision: row.8,
        updated_at_ms: row.9,
    })
}

fn validate_proposal(
    principal_id: &str,
    workspace_key: &WorkspaceMemoryKey,
    proposal: &WorkspaceMemoryBindingProposal,
    grant: &SupplementalCapabilityGrant,
) -> Result<(), WorkspaceMemoryBindingError> {
    validate_id(principal_id)?;
    validate_id(workspace_key.as_str())?;
    for value in [
        proposal.backend_id.as_str(),
        proposal.write_destination_handle.as_str(),
        proposal.expected_write_source.as_str(),
        proposal.credential_ref.as_str(),
        grant.backend_id.as_str(),
    ] {
        validate_id(value)?;
    }
    if proposal.read_destination_handles.is_empty()
        || proposal.expected_read_sources.is_empty()
        || proposal.read_destination_handles.len() > MAX_READ_SOURCES
        || proposal.expected_read_sources.len() > MAX_READ_SOURCES
        || grant.read_sources.len() > MAX_READ_SOURCES
        || proposal.read_destination_handles.len() != proposal.expected_read_sources.len()
    {
        return Err(WorkspaceMemoryBindingError::Invalid);
    }
    for value in proposal
        .read_destination_handles
        .iter()
        .chain(proposal.expected_read_sources.iter())
        .chain(grant.read_sources.iter())
    {
        validate_id(value)?;
    }
    if normalized_set(&proposal.expected_read_sources).len() != proposal.expected_read_sources.len()
        || normalized_set(&proposal.read_destination_handles).len()
            != proposal.read_destination_handles.len()
    {
        return Err(WorkspaceMemoryBindingError::Invalid);
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<(), WorkspaceMemoryBindingError> {
    if value.trim().is_empty() || value.len() > MAX_ID_BYTES || value.chars().any(char::is_control)
    {
        return Err(WorkspaceMemoryBindingError::Invalid);
    }
    Ok(())
}

fn normalized_set(values: &[String]) -> BTreeSet<&str> {
    values.iter().map(String::as_str).collect()
}

pub fn capability_digest(grant: &SupplementalCapabilityGrant) -> String {
    let reads = normalized_set(&grant.read_sources);
    let mut hasher = Sha256::new();
    hasher.update(b"aletheon.supplemental-capability.v1\0");
    for value in [
        grant.backend_id.as_str(),
        grant.write_source.as_deref().unwrap_or(""),
    ] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    for value in reads {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.update([u8::from(grant.can_read), u8::from(grant.can_write)]);
    format!("sha256:{:x}", hasher.finalize())
}

fn migrate(connection: &Connection) -> Result<(), WorkspaceMemoryBindingError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(WorkspaceMemoryBindingError::Corrupt);
    }
    if version == 0 {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE workspace_memory_bindings(
                principal_id TEXT NOT NULL,
                workspace_key TEXT NOT NULL,
                backend_id TEXT NOT NULL,
                write_destination_handle TEXT NOT NULL,
                read_destination_handles_json TEXT NOT NULL,
                expected_write_source TEXT NOT NULL,
                expected_read_sources_json TEXT NOT NULL,
                credential_ref TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('active','local_only','incompatible','revoked')),
                verified_capability_digest TEXT,
                revision INTEGER NOT NULL CHECK(revision > 0),
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY(principal_id,workspace_key)
             );
             PRAGMA user_version=1;
             COMMIT;",
        )?;
    }
    Ok(())
}
