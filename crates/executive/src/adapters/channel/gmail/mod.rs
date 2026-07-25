//! Authenticated Gmail channel ingress and durable classification.

pub mod classifier;
pub mod event_ingress;
pub mod goal_draft;
pub mod ingest;
pub mod report;
pub mod sender_policy;

use self::classifier::{classify_verified_subject, GmailClassification};
use self::sender_policy::{GmailHeader, GmailSenderPolicy};
use crate::compatibility::persistence_migrations as migrations;
pub use event_ingress::{load_gmail_ingress_policies, GmailGoalEventIngress, GmailIngressPolicy};
use fabric::{ExternalIdentityId, PrincipalId};
pub use goal_draft::{GmailGoalDraft, GmailGoalDraftCoordinator};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct GmailChannelMessage {
    pub account_id: ExternalIdentityId,
    pub message_id: String,
    pub thread_id: String,
    pub headers: Vec<GmailHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailInboxRecord {
    pub account_id: ExternalIdentityId,
    pub message_id: String,
    pub thread_id: String,
    pub verified_principal: Option<PrincipalId>,
    pub sender_address: Option<String>,
    pub sender_policy_version: Option<u64>,
    pub classification: GmailClassification,
    pub evidence_hash: String,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmailInsertOutcome {
    Inserted,
    Duplicate,
}

pub struct GmailChannelStore {
    db: Connection,
}

impl GmailChannelStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Connection::open(path)?;
        migrations::run_migrations(&db)?;
        Ok(Self { db })
    }

    pub fn authenticate_and_persist(
        &self,
        message: &GmailChannelMessage,
        policy: Option<&GmailSenderPolicy>,
        now_ms: i64,
    ) -> anyhow::Result<(GmailInsertOutcome, GmailInboxRecord)> {
        validate_message(message, now_ms)?;
        if let Some(existing) = self.get(message.account_id, &message.message_id)? {
            return Ok((GmailInsertOutcome::Duplicate, existing));
        }
        let subject = header_value(&message.headers, "subject").unwrap_or_default();
        let bound_principal: Option<String> = self
            .db
            .query_row(
                "SELECT i.principal_id FROM external_identities i
                 JOIN capability_grants g USING(identity_id)
                 WHERE i.identity_id=?1 AND i.state='active' AND g.state='active'",
                [message.account_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let verified = policy
            .filter(|policy| bound_principal.as_deref() == Some(policy.principal.0.as_str()))
            .and_then(|policy| policy.verify(&message.headers).ok());
        let (principal, address, version, classification, evidence_hash, status) =
            if let Some(sender) = verified {
                (
                    Some(sender.principal),
                    Some(sender.address),
                    Some(sender.policy_version),
                    classify_verified_subject(subject),
                    sender.evidence_hash,
                    "accepted".to_owned(),
                )
            } else {
                (
                    None,
                    None,
                    None,
                    GmailClassification::Quarantine,
                    hash_unverified_evidence(&message.headers),
                    "quarantined".to_owned(),
                )
            };
        let record = GmailInboxRecord {
            account_id: message.account_id,
            message_id: message.message_id.clone(),
            thread_id: message.thread_id.clone(),
            verified_principal: principal,
            sender_address: address,
            sender_policy_version: version,
            classification,
            evidence_hash,
            status,
        };
        self.db.execute(
            "INSERT INTO gmail_channel_inbox(
                account_id,message_id,thread_id,verified_principal_id,sender_address,
                sender_policy_version,classification,evidence_hash,status,created_at_ms,updated_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
            params![
                record.account_id.to_string(),
                record.message_id,
                record.thread_id,
                record.verified_principal.as_ref().map(|value| value.0.as_str()),
                record.sender_address,
                record.sender_policy_version,
                record.classification.as_str(),
                record.evidence_hash,
                record.status,
                now_ms
            ],
        )?;
        Ok((GmailInsertOutcome::Inserted, record))
    }

    pub fn get(
        &self,
        account_id: ExternalIdentityId,
        message_id: &str,
    ) -> anyhow::Result<Option<GmailInboxRecord>> {
        self.db
            .query_row(
                "SELECT thread_id,verified_principal_id,sender_address,sender_policy_version,
                        classification,evidence_hash,status
                 FROM gmail_channel_inbox WHERE account_id=?1 AND message_id=?2",
                params![account_id.to_string(), message_id],
                |row| {
                    let classification: String = row.get(4)?;
                    Ok(GmailInboxRecord {
                        account_id,
                        message_id: message_id.to_owned(),
                        thread_id: row.get(0)?,
                        verified_principal: row.get::<_, Option<String>>(1)?.map(PrincipalId),
                        sender_address: row.get(2)?,
                        sender_policy_version: row.get(3)?,
                        classification: parse_classification(&classification)?,
                        evidence_hash: row.get(5)?,
                        status: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }
}

fn validate_message(message: &GmailChannelMessage, now_ms: i64) -> anyhow::Result<()> {
    anyhow::ensure!(now_ms >= 0, "invalid Gmail ingress time");
    anyhow::ensure!(
        !message.message_id.is_empty() && message.message_id.len() <= 1_024,
        "invalid Gmail message ID"
    );
    anyhow::ensure!(
        !message.thread_id.is_empty() && message.thread_id.len() <= 1_024,
        "invalid Gmail thread ID"
    );
    Ok(())
}

fn header_value<'a>(headers: &'a [GmailHeader], name: &str) -> Option<&'a str> {
    let mut values = headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name));
    let first = values.next()?;
    values.next().is_none().then_some(first.value.as_str())
}

fn hash_unverified_evidence(headers: &[GmailHeader]) -> String {
    let mut material = String::new();
    for header in headers.iter().take(200) {
        material.push_str(&header.name.to_ascii_lowercase());
        material.push(':');
        material.push_str(&header.value.chars().take(16 * 1024).collect::<String>());
        material.push('\n');
    }
    format!("{:x}", Sha256::digest(material.as_bytes()))
}

fn parse_classification(value: &str) -> rusqlite::Result<GmailClassification> {
    match value {
        "ask" => Ok(GmailClassification::Ask),
        "goal" => Ok(GmailClassification::Goal),
        "memory" => Ok(GmailClassification::Memory),
        "doc" => Ok(GmailClassification::Doc),
        "notification" => Ok(GmailClassification::Notification),
        "quarantine" => Ok(GmailClassification::Quarantine),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
