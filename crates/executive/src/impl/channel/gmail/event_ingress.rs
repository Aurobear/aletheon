//! Production bridge from durable Gmail history events to authenticated Goal drafts.

use super::ingest::{
    GmailAttachmentFetcher, GmailIngestConfig, GmailIngestMessage, GmailMessageIngester,
    GmailMimePart,
};
use super::sender_policy::{GmailHeader, GmailSenderPolicy};
use super::{
    GmailChannelMessage, GmailChannelStore, GmailClassification, GmailGoalDraftCoordinator,
};
use crate::r#impl::artifact::ArtifactStore;
use async_trait::async_trait;
use corpus::tools::google::{
    GmailIngressCapability, GmailIngressMessage, GmailIngressPart, GoogleApiError,
};
use fabric::{ExternalEvent, ExternalEventEnvelope, ExternalIdentityId, PrincipalId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const REVIEW_TTL_MS: i64 = 7 * 86_400_000;
const MAX_PROVIDER_ATTACHMENT_BYTES: usize = 8 * 1_048_576;

/// Account-specific policy binding. Construction requires the account owner to
/// match the policy principal; missing accounts remain deny-by-default.
#[derive(Debug, Clone)]
pub struct GmailIngressPolicy {
    pub account_id: ExternalIdentityId,
    pub sender: GmailSenderPolicy,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GmailIngressPolicyFile {
    policies: Vec<GmailIngressPolicyEntry>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GmailIngressPolicyEntry {
    account_id: ExternalIdentityId,
    principal: PrincipalId,
    version: u64,
    #[serde(default)]
    allowed_addresses: std::collections::HashSet<String>,
    #[serde(default)]
    allowed_domains: std::collections::HashSet<String>,
    trusted_authserv_ids: std::collections::HashSet<String>,
    authentication: super::sender_policy::AuthenticationRequirement,
}

pub fn load_gmail_ingress_policies(
    path: &Path,
    active_owners: &HashMap<ExternalIdentityId, PrincipalId>,
) -> anyhow::Result<Vec<GmailIngressPolicy>> {
    let metadata = std::fs::metadata(path)?;
    anyhow::ensure!(metadata.is_file(), "Gmail ingress policy is not a file");
    anyhow::ensure!(
        metadata.len() <= 1_048_576,
        "Gmail ingress policy exceeds cap"
    );
    let bytes = std::fs::read(path)?;
    let file: GmailIngressPolicyFile = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        file.policies.len() <= 1_000,
        "too many Gmail ingress policies"
    );
    file.policies
        .into_iter()
        .map(|entry| {
            anyhow::ensure!(
                active_owners.get(&entry.account_id) == Some(&entry.principal),
                "Gmail ingress policy owner mismatch"
            );
            let sender = GmailSenderPolicy {
                principal: entry.principal,
                version: entry.version,
                allowed_addresses: entry.allowed_addresses,
                allowed_domains: entry.allowed_domains,
                trusted_authserv_ids: entry.trusted_authserv_ids,
                authentication: entry.authentication,
            };
            sender
                .validate()
                .map_err(|_| anyhow::anyhow!("invalid Gmail ingress sender policy"))?;
            Ok(GmailIngressPolicy {
                account_id: entry.account_id,
                sender,
            })
        })
        .collect()
}

pub struct GmailGoalEventIngress {
    provider: Arc<dyn GmailIngressCapability>,
    objective_db_path: PathBuf,
    artifact_root: PathBuf,
    policies: HashMap<ExternalIdentityId, GmailSenderPolicy>,
    review_ttl_ms: i64,
    storage_quota: Option<crate::r#impl::storage_quota::StorageQuota>,
}

impl GmailGoalEventIngress {
    pub fn new(
        provider: Arc<dyn GmailIngressCapability>,
        objective_db_path: &Path,
        artifact_root: &Path,
        policies: Vec<GmailIngressPolicy>,
    ) -> anyhow::Result<Self> {
        let mut by_account = HashMap::new();
        for policy in policies {
            anyhow::ensure!(
                by_account
                    .insert(policy.account_id, policy.sender)
                    .is_none(),
                "duplicate Gmail ingress policy"
            );
        }
        Ok(Self {
            provider,
            objective_db_path: objective_db_path.to_path_buf(),
            artifact_root: artifact_root.to_path_buf(),
            policies: by_account,
            review_ttl_ms: REVIEW_TTL_MS,
            storage_quota: None,
        })
    }

    pub fn with_storage_quota(mut self, quota: crate::r#impl::storage_quota::StorageQuota) -> Self {
        self.storage_quota = Some(quota);
        self
    }

    pub async fn ingest(
        &self,
        event: &ExternalEventEnvelope,
        cancel: &CancellationToken,
    ) -> Result<bool, String> {
        let ExternalEvent::MailReceived(change) = &event.event else {
            return Ok(false);
        };
        let Some(policy) = self.policies.get(&event.account_id) else {
            return Ok(false);
        };
        if cancel.is_cancelled() {
            return Err("gmail_ingress_cancelled".into());
        }
        let raw = self
            .provider
            .read_ingress_message(
                &policy.principal,
                event.account_id,
                &change.message.source.provider_object_id,
                cancel,
            )
            .await
            .map_err(stable_provider_error)?;
        validate_event_binding(event, &raw)?;

        let channel_message = GmailChannelMessage {
            account_id: raw.account_id,
            message_id: raw.message_id.clone(),
            thread_id: raw.thread_id.clone(),
            headers: raw
                .headers
                .iter()
                .map(|header| GmailHeader {
                    name: header.name.clone(),
                    value: header.value.clone(),
                })
                .collect(),
        };
        let channel = GmailChannelStore::open(&self.objective_db_path)
            .map_err(|_| "gmail_ingress_store_unavailable".to_owned())?;
        let (_, inbox) = channel
            .authenticate_and_persist(&channel_message, Some(policy), event.observed_at_ms)
            .map_err(|_| "gmail_ingress_auth_persist_failed".to_owned())?;
        if inbox.status != "accepted" || inbox.classification != GmailClassification::Goal {
            return Ok(false);
        }

        let ingest_message = GmailIngestMessage {
            account_id: raw.account_id,
            message_id: raw.message_id.clone(),
            thread_id: raw.thread_id.clone(),
            source_timestamp_ms: raw.source_timestamp_ms,
            root: convert_part(raw.root),
        };
        let fetcher = ProviderAttachmentFetcher::new(
            self.provider.clone(),
            policy.principal.clone(),
            raw.account_id,
            raw.message_id,
        );
        let artifacts = ArtifactStore::open(&self.objective_db_path, &self.artifact_root)
            .map(|store| match &self.storage_quota {
                Some(quota) => store.with_quota(quota.clone()),
                None => store,
            })
            .map_err(|_| "gmail_ingress_artifact_store_unavailable".to_owned())?;
        let ingested = GmailMessageIngester::new(GmailIngestConfig::default())
            .map_err(|_| "gmail_ingress_policy_invalid".to_owned())?
            .ingest(
                &ingest_message,
                &fetcher,
                &artifacts,
                event.observed_at_ms,
                cancel,
            )
            .await
            .map_err(|_| "gmail_ingress_content_rejected".to_owned())?;
        let mut drafts = GmailGoalDraftCoordinator::open(&self.objective_db_path)
            .map_err(|_| "gmail_ingress_draft_store_unavailable".to_owned())?;
        drafts
            .create_draft(
                &inbox,
                policy,
                &ingested,
                &event.id.to_string(),
                event.observed_at_ms,
                event.observed_at_ms.saturating_add(self.review_ttl_ms),
            )
            .map_err(|_| "gmail_ingress_draft_rejected".to_owned())?;
        Ok(true)
    }
}

fn validate_event_binding(
    event: &ExternalEventEnvelope,
    message: &GmailIngressMessage,
) -> Result<(), String> {
    let ExternalEvent::MailReceived(change) = &event.event else {
        return Err("gmail_ingress_event_mismatch".into());
    };
    if message.account_id != event.account_id
        || message.message_id != change.message.source.provider_object_id.as_str()
        || message.thread_id != change.message.thread_id.as_str()
        || message.source_timestamp_ms != event.source_timestamp_ms
    {
        return Err("gmail_ingress_event_mismatch".into());
    }
    Ok(())
}

fn convert_part(part: GmailIngressPart) -> GmailMimePart {
    GmailMimePart {
        part_id: part.part_id,
        mime_type: part.mime_type,
        filename: part.filename,
        declared_size: part.declared_size,
        inline_body: part.inline_body,
        attachment_id: part.attachment_id,
        parts: part.parts.into_iter().map(convert_part).collect(),
    }
}

fn stable_provider_error(error: GoogleApiError) -> String {
    format!("gmail_ingress_provider_{error}")
}

struct ProviderAttachmentFetcher {
    provider: Arc<dyn GmailIngressCapability>,
    principal: PrincipalId,
    account_id: ExternalIdentityId,
    message_id: String,
    cache: Mutex<HashMap<String, Vec<u8>>>,
}

impl ProviderAttachmentFetcher {
    fn new(
        provider: Arc<dyn GmailIngressCapability>,
        principal: PrincipalId,
        account_id: ExternalIdentityId,
        message_id: String,
    ) -> Self {
        Self {
            provider,
            principal,
            account_id,
            message_id,
            cache: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl GmailAttachmentFetcher for ProviderAttachmentFetcher {
    async fn next_chunk(
        &self,
        attachment_id: &str,
        offset: u64,
        max_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut cache = self.cache.lock().await;
        if !cache.contains_key(attachment_id) {
            let bytes = self
                .provider
                .read_ingress_attachment(
                    &self.principal,
                    self.account_id,
                    &self.message_id,
                    attachment_id,
                    MAX_PROVIDER_ATTACHMENT_BYTES,
                    cancel,
                )
                .await
                .map_err(stable_provider_error)?;
            cache.insert(attachment_id.to_owned(), bytes);
        }
        let bytes = cache.get(attachment_id).expect("inserted");
        let start = usize::try_from(offset).map_err(|_| "gmail_ingress_offset_invalid")?;
        if start >= bytes.len() {
            return Ok(None);
        }
        let end = start.saturating_add(max_bytes).min(bytes.len());
        Ok(Some(bytes[start..end].to_vec()))
    }
}
