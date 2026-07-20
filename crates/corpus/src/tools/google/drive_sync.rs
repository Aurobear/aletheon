//! Bounded selected-file Google Drive changes synchronization.

use super::{GoogleApiError, GoogleDriveAdapter};
use chrono::DateTime;
use fabric::{
    DriveFileMetadata, ExternalContentRef, ExternalEventDraft, ExternalEventEnvelope,
    ExternalIdentityId, ExternalObjectRef, GoogleEvent, IdentityProvider, PrincipalId,
    ProviderRecordRef,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

const HARD_MAX_PAGES: usize = 100;
const HARD_MAX_CHANGES: usize = 10_000;
const HARD_MAX_CONTENT_BYTES: usize = 16 * 1_048_576;

#[derive(Debug, Clone)]
pub struct DriveSyncConfig {
    pub selected_file_ids: HashSet<String>,
    pub content_mime_allowlist: HashSet<String>,
    pub download_content: bool,
    pub max_content_bytes: usize,
    pub max_pages: usize,
    pub max_changes: usize,
    pub page_size: u16,
}

impl DriveSyncConfig {
    fn validate(&self) -> Result<(), GoogleApiError> {
        let ids_valid = self
            .selected_file_ids
            .iter()
            .all(|id| !id.is_empty() && id.len() <= 1_024);
        let mimes_valid = self
            .content_mime_allowlist
            .iter()
            .all(|mime| !mime.is_empty() && mime.len() <= 256);
        if !ids_valid
            || !mimes_valid
            || self.max_content_bytes == 0
            || self.max_content_bytes > HARD_MAX_CONTENT_BYTES
            || self.max_pages == 0
            || self.max_pages > HARD_MAX_PAGES
            || self.max_changes == 0
            || self.max_changes > HARD_MAX_CHANGES
            || !(1..=1_000).contains(&self.page_size)
        {
            Err(GoogleApiError::InvalidRequest)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveContentArtifact {
    pub reference: ExternalContentRef,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveSyncHealthEvent {
    CursorExpiredBaselineReset,
}

#[derive(Debug, Clone)]
pub struct DriveSyncBatch {
    pub input_cursor: Option<String>,
    pub successor_cursor: String,
    pub events: Vec<ExternalEventEnvelope>,
    pub artifacts: Vec<DriveContentArtifact>,
    pub health_events: Vec<DriveSyncHealthEvent>,
    pub baseline_only: bool,
    pub reconciled: bool,
}

#[derive(Debug, Clone)]
pub struct DriveSynchronizer {
    drive: GoogleDriveAdapter,
    config: DriveSyncConfig,
}

impl DriveSynchronizer {
    pub fn new(drive: GoogleDriveAdapter, config: DriveSyncConfig) -> Result<Self, GoogleApiError> {
        config.validate()?;
        Ok(Self { drive, config })
    }

    pub async fn synchronize(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        input_cursor: Option<&str>,
        known_dedup_keys: &HashSet<String>,
        cancel: &CancellationToken,
    ) -> Result<DriveSyncBatch, GoogleApiError> {
        if input_cursor.is_some_and(|cursor| cursor.is_empty() || cursor.len() > 16 * 1024) {
            return Err(GoogleApiError::InvalidRequest);
        }
        let Some(cursor) = input_cursor else {
            return self.baseline(principal, account, None, false, cancel).await;
        };
        match self
            .changes(principal, account, cursor, known_dedup_keys, cancel)
            .await
        {
            Ok(batch) => Ok(batch),
            Err(GoogleApiError::CursorExpired) => {
                self.baseline(principal, account, Some(cursor), true, cancel)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    async fn baseline(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        input_cursor: Option<&str>,
        reconciled: bool,
        cancel: &CancellationToken,
    ) -> Result<DriveSyncBatch, GoogleApiError> {
        let response: StartPageToken = self
            .drive
            .get_json(
                principal,
                account,
                "changes/startPageToken",
                &[("supportsAllDrives", "true")],
                cancel,
            )
            .await?;
        validate_token(&response.start_page_token)?;
        Ok(DriveSyncBatch {
            input_cursor: input_cursor.map(str::to_owned),
            successor_cursor: response.start_page_token,
            events: Vec::new(),
            artifacts: Vec::new(),
            health_events: reconciled
                .then_some(DriveSyncHealthEvent::CursorExpiredBaselineReset)
                .into_iter()
                .collect(),
            baseline_only: true,
            reconciled,
        })
    }

    async fn changes(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        input_cursor: &str,
        known_dedup_keys: &HashSet<String>,
        cancel: &CancellationToken,
    ) -> Result<DriveSyncBatch, GoogleApiError> {
        let mut page_token = input_cursor.to_owned();
        let mut pages = 0;
        let mut examined = 0;
        let mut events = Vec::new();
        let mut artifacts = Vec::new();
        let mut emitted = HashSet::new();
        let successor = loop {
            if pages >= self.config.max_pages {
                return Err(GoogleApiError::ResponseTooLarge);
            }
            let page_size = self.config.page_size.to_string();
            let page: ChangePage = self
                .drive
                .get_json(
                    principal,
                    account,
                    "changes",
                    &[
                        ("pageToken", page_token.as_str()),
                        ("pageSize", page_size.as_str()),
                        ("includeItemsFromAllDrives", "true"),
                        ("supportsAllDrives", "true"),
                        ("fields", "nextPageToken,newStartPageToken,changes(fileId,removed,time,file(id,name,mimeType,size,modifiedTime,version,trashed))"),
                    ],
                    cancel,
                )
                .await?;
            pages += 1;
            if page.changes.len() > usize::from(self.config.page_size) {
                return Err(GoogleApiError::MalformedResponse);
            }
            for change in page.changes {
                examined += 1;
                if examined > self.config.max_changes {
                    return Err(GoogleApiError::ResponseTooLarge);
                }
                if !self.config.selected_file_ids.contains(&change.file_id) {
                    continue;
                }
                let (event, artifact) = self
                    .normalize_change(principal, account, change, cancel)
                    .await?;
                if !known_dedup_keys.contains(&event.dedup_key)
                    && emitted.insert(event.dedup_key.clone())
                {
                    events.push(event);
                    if let Some(artifact) = artifact {
                        artifacts.push(artifact);
                    }
                }
            }
            if let Some(next) = page.next_page_token {
                validate_token(&next)?;
                page_token = next;
            } else {
                let token = page
                    .new_start_page_token
                    .ok_or(GoogleApiError::MalformedResponse)?;
                validate_token(&token)?;
                break token;
            }
        };
        Ok(DriveSyncBatch {
            input_cursor: Some(input_cursor.into()),
            successor_cursor: successor,
            events,
            artifacts,
            health_events: Vec::new(),
            baseline_only: false,
            reconciled: false,
        })
    }

    async fn normalize_change(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        change: RawChange,
        cancel: &CancellationToken,
    ) -> Result<(ExternalEventEnvelope, Option<DriveContentArtifact>), GoogleApiError> {
        validate_id(&change.file_id)?;
        let now = chrono::Utc::now().timestamp_millis();
        let change_ms = change
            .time
            .as_deref()
            .map(parse_time)
            .transpose()?
            .unwrap_or(now);
        if change.removed || change.file.as_ref().is_some_and(|file| file.trashed) {
            let version = change.time.unwrap_or_else(|| change_ms.to_string());
            let object = object(account, &change.file_id, &version);
            let envelope = envelope(
                account,
                change.file_id,
                object.clone(),
                change_ms,
                GoogleEvent::DriveFileDeleted(object),
            )?;
            return Ok((envelope, None));
        }
        let file = change.file.ok_or(GoogleApiError::MalformedResponse)?;
        if file.id != change.file_id {
            return Err(GoogleApiError::MalformedResponse);
        }
        validate_id(&file.id)?;
        let modified_ms = parse_time(&file.modified_time)?;
        let version = file.version.unwrap_or_else(|| file.modified_time.clone());
        let object = object(account, &file.id, &version);
        let declared_size = file
            .size
            .as_deref()
            .map(str::parse::<u64>)
            .transpose()
            .map_err(|_| GoogleApiError::MalformedResponse)?;
        let eligible = self.config.download_content
            && self.config.content_mime_allowlist.contains(&file.mime_type)
            && declared_size.is_some_and(|size| size <= self.config.max_content_bytes as u64);
        let artifact = if eligible {
            let bytes = self
                .drive
                .download(
                    principal,
                    account,
                    &file.id,
                    self.config.max_content_bytes,
                    cancel,
                )
                .await?;
            if declared_size != Some(bytes.len() as u64) {
                return Err(GoogleApiError::MalformedResponse);
            }
            let sha256 = format!("{:x}", Sha256::digest(&bytes));
            let reference = ExternalContentRef {
                artifact_id: format!("sha256:{sha256}"),
                sha256,
                size_bytes: bytes.len() as u64,
                mime_type: file.mime_type.clone(),
            };
            Some(DriveContentArtifact { reference, bytes })
        } else {
            None
        };
        let metadata = DriveFileMetadata {
            object: object.clone(),
            name: file.name,
            mime_type: file.mime_type,
            size_bytes: declared_size,
            modified_at_ms: modified_ms,
            selected: true,
            content: artifact.as_ref().map(|artifact| artifact.reference.clone()),
        };
        let event = envelope(
            account,
            file.id,
            object,
            change_ms,
            GoogleEvent::DriveFileUpdated(metadata),
        )?;
        Ok((event, artifact))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartPageToken {
    start_page_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangePage {
    #[serde(default)]
    changes: Vec<RawChange>,
    next_page_token: Option<String>,
    new_start_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawChange {
    file_id: String,
    #[serde(default)]
    removed: bool,
    time: Option<String>,
    file: Option<RawFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFile {
    id: String,
    #[serde(default)]
    name: String,
    mime_type: String,
    size: Option<String>,
    modified_time: String,
    version: Option<String>,
    #[serde(default)]
    trashed: bool,
}

fn object(account: ExternalIdentityId, id: &str, version: &str) -> ExternalObjectRef {
    ExternalObjectRef {
        provider: IdentityProvider::Google,
        account_id: account,
        object_id: id.into(),
        object_version: version.into(),
    }
}

fn envelope(
    account: ExternalIdentityId,
    provider_event_id: String,
    object: ExternalObjectRef,
    source_ms: i64,
    event: GoogleEvent,
) -> Result<ExternalEventEnvelope, GoogleApiError> {
    let now = chrono::Utc::now().timestamp_millis();
    ExternalEventEnvelope::from_draft(ExternalEventDraft {
        provider: IdentityProvider::Google,
        account_id: account,
        provider_event_id: Some(provider_event_id),
        provenance: ProviderRecordRef {
            account_id: account,
            provider_object_id: object.object_id.clone(),
            fetched_at_ms: now,
            source_timestamp_ms: source_ms,
            etag_or_history: Some(object.object_version.clone()),
        },
        object,
        observed_at_ms: now,
        source_timestamp_ms: source_ms,
        event,
    })
    .map_err(|_| GoogleApiError::MalformedResponse)
}

fn parse_time(value: &str) -> Result<i64, GoogleApiError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.timestamp_millis())
        .map_err(|_| GoogleApiError::MalformedResponse)
}

fn validate_id(value: &str) -> Result<(), GoogleApiError> {
    if value.is_empty() || value.len() > 1_024 {
        Err(GoogleApiError::MalformedResponse)
    } else {
        Ok(())
    }
}

fn validate_token(value: &str) -> Result<(), GoogleApiError> {
    if value.is_empty() || value.len() > 16 * 1_024 {
        Err(GoogleApiError::MalformedResponse)
    } else {
        Ok(())
    }
}
