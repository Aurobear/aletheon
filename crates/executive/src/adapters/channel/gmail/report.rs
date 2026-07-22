//! Local-first report artifacts and approval-bound Gmail delivery.

use crate::application::approval::{ApprovalCreate, ApprovalRepository};
use crate::adapters::artifact::{
    ArtifactMetadata, ArtifactRecord, ArtifactScanStatus, ArtifactStore,
};
use crate::application::goal::{migrations, ObjectiveStore};
use async_trait::async_trait;
use fabric::{
    ApprovalCategory, ApprovalId, ApprovalRisk, ApprovalSnapshot, ApprovalStatus, ApprovalSubject,
    ExternalCapabilityId, ExternalIdentityId, GoalId, PrincipalId,
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MAX_SUBJECT_BYTES: usize = 998;
const MAX_BODY_BYTES: usize = 4 * 1_048_576;
const MAX_RECIPIENT_BYTES: usize = 320;

#[derive(Debug, Clone)]
pub struct LocalGmailReport {
    pub goal_id: GoalId,
    pub owner: PrincipalId,
    pub account_id: ExternalIdentityId,
    pub subject: String,
    pub body: String,
    pub artifact: ArtifactRecord,
    pub telegram_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmailSendResult {
    Sent { provider_message_id: String },
    AmbiguousTimeout,
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmailReconciliation {
    Found { provider_message_id: String },
    NotFound,
    Unknown,
}

#[async_trait]
pub trait GmailReportProvider: Send + Sync {
    async fn send(
        &self,
        account_id: ExternalIdentityId,
        recipient: &str,
        subject: &str,
        body: &str,
        idempotency_key: &str,
    ) -> GmailSendResult;

    async fn reconcile(
        &self,
        account_id: ExternalIdentityId,
        idempotency_key: &str,
    ) -> GmailReconciliation;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmailDeliveryOutcome {
    Sent { provider_message_id: String },
    AlreadySent { provider_message_id: String },
    Ambiguous,
    ReconciliationRequired,
    Failed,
}

pub struct GmailReportBoundary {
    db: Connection,
    db_path: PathBuf,
    artifacts: ArtifactStore,
    approvals: Arc<Mutex<ApprovalRepository>>,
}

impl GmailReportBoundary {
    pub fn open(db_path: &Path, artifact_root: &Path) -> anyhow::Result<Self> {
        let db = Connection::open(db_path)?;
        migrations::run_migrations(&db)?;
        Ok(Self {
            db,
            db_path: db_path.to_path_buf(),
            artifacts: ArtifactStore::open(db_path, artifact_root)?,
            approvals: Arc::new(Mutex::new(ApprovalRepository::open(db_path)?)),
        })
    }

    pub fn approval_repository(&self) -> Arc<Mutex<ApprovalRepository>> {
        self.approvals.clone()
    }

    /// Create a trusted local artifact. This operation never sends email and
    /// needs no Gmail write grant; Telegram is the default delivery surface.
    pub fn create_local_report(
        &self,
        goal_id: GoalId,
        account_id: ExternalIdentityId,
        subject: &str,
        body: &str,
        now_ms: i64,
    ) -> anyhow::Result<LocalGmailReport> {
        validate_subject(subject)?;
        validate_body(body)?;
        anyhow::ensure!(now_ms >= 0, "invalid report time");
        let goal = ObjectiveStore::open(&self.db_path)?
            .get_goal(goal_id)?
            .ok_or_else(|| anyhow::anyhow!("report Goal not found"))?;
        let mut writer = self.artifacts.begin(
            ArtifactMetadata {
                mime_type: "text/plain".into(),
                provider: "aletheon".into(),
                account_id: account_id.to_string(),
                provider_message_id: format!("goal-report:{}", goal_id.0),
                provider_part_id: "body".into(),
                source_timestamp_ms: now_ms,
                scan_status: ArtifactScanStatus::Clean,
                created_at_ms: now_ms,
            },
            MAX_BODY_BYTES as u64,
        )?;
        writer.write_chunk(body.as_bytes())?;
        let artifact = self.artifacts.finish(writer)?;
        let subject_preview: String = subject.chars().take(256).collect();
        Ok(LocalGmailReport {
            goal_id,
            owner: goal.owner,
            account_id,
            subject: subject.to_owned(),
            body: body.to_owned(),
            telegram_summary: format!(
                "Report ready for Goal {}: {} (sha256 {})",
                goal_id.0, subject_preview, artifact.sha256
            ),
            artifact,
        })
    }

    /// Request a separate M5 approval for an explicitly supplied recipient.
    /// No recipient is ever parsed or inferred from report or inbound content.
    pub fn request_send_approval(
        &self,
        report: &LocalGmailReport,
        recipient: &str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> anyhow::Result<ApprovalSnapshot> {
        validate_report(report)?;
        validate_recipient(recipient)?;
        anyhow::ensure!(expires_at_ms > now_ms, "invalid send approval expiry");
        self.ensure_send_grant(report.account_id, &report.owner)?;
        let subject_hash = sha256_hex(report.subject.as_bytes());
        let body_hash = sha256_hex(report.body.as_bytes());
        self.approvals
            .lock()
            .unwrap()
            .create(ApprovalCreate {
                subject: ApprovalSubject {
                    category: ApprovalCategory::SendMail,
                    goal_id: report.goal_id,
                    attempt_id: None,
                    job_id: None,
                    attributes: BTreeMap::from([
                        ("account_id".into(), report.account_id.to_string()),
                        ("recipient".into(), recipient.to_owned()),
                        ("subject_sha256".into(), subject_hash),
                        ("body_sha256".into(), body_hash),
                        ("report_sha256".into(), report.artifact.sha256.clone()),
                    ]),
                    allowed_scope: vec![],
                    apply_target: None,
                },
                risk: ApprovalRisk::High,
                summary: format!(
                    "Send Goal {} report to {}\nSubject: {}\nReport sha256: {}",
                    report.goal_id.0, recipient, report.subject, report.artifact.sha256
                )
                .chars()
                .take(4_096)
                .collect(),
                artifacts: vec![fabric::ApprovalArtifactRef {
                    kind: "report".into(),
                    relative_path: report.artifact.relative_path.clone(),
                    sha256: report.artifact.sha256.clone(),
                }],
                created_at_ms: now_ms,
                expires_at_ms,
            })
            .map_err(Into::into)
    }

    pub async fn deliver(
        &self,
        approval_id: ApprovalId,
        report: &LocalGmailReport,
        recipient: &str,
        now_ms: i64,
        provider: &dyn GmailReportProvider,
    ) -> anyhow::Result<GmailDeliveryOutcome> {
        validate_report(report)?;
        validate_recipient(recipient)?;
        anyhow::ensure!(now_ms >= 0, "invalid delivery time");
        let approval = self
            .approvals
            .lock()
            .unwrap()
            .get(approval_id)?
            .ok_or_else(|| anyhow::anyhow!("send approval not found"))?;
        validate_send_approval(&approval, report, recipient, now_ms)?;
        self.ensure_send_grant(report.account_id, &report.owner)?;
        let idempotency_key = format!("{}:{}", approval.id, report.artifact.sha256);
        self.db.execute(
            "INSERT OR IGNORE INTO gmail_report_outbox(
                approval_id,report_sha256,account_id,recipient,subject_sha256,body_sha256,
                idempotency_key,status,created_at_ms,updated_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?8)",
            params![
                approval.id.to_string(),
                report.artifact.sha256,
                report.account_id.to_string(),
                recipient,
                sha256_hex(report.subject.as_bytes()),
                sha256_hex(report.body.as_bytes()),
                idempotency_key,
                now_ms
            ],
        )?;
        let mut row = self.delivery_row(approval.id, &report.artifact.sha256)?;
        if row.status == "sent" {
            return Ok(GmailDeliveryOutcome::AlreadySent {
                provider_message_id: row
                    .provider_message_id
                    .ok_or_else(|| anyhow::anyhow!("sent delivery has no provider ID"))?,
            });
        }
        if row.status == "ambiguous" {
            match provider
                .reconcile(report.account_id, &row.idempotency_key)
                .await
            {
                GmailReconciliation::Found {
                    provider_message_id,
                } => {
                    self.mark_sent(
                        approval.id,
                        &report.artifact.sha256,
                        &provider_message_id,
                        now_ms,
                    )?;
                    return Ok(GmailDeliveryOutcome::AlreadySent {
                        provider_message_id,
                    });
                }
                GmailReconciliation::Unknown => {
                    return Ok(GmailDeliveryOutcome::ReconciliationRequired)
                }
                GmailReconciliation::NotFound => {
                    self.db.execute(
                        "UPDATE gmail_report_outbox SET status='pending',reconciled_at_ms=?1,
                         updated_at_ms=?1 WHERE approval_id=?2 AND report_sha256=?3 AND status='ambiguous'",
                        params![now_ms,approval.id.to_string(),report.artifact.sha256],
                    )?;
                    row.status = "pending".into();
                }
            }
        }
        anyhow::ensure!(
            matches!(row.status.as_str(), "pending" | "failed"),
            "invalid delivery state"
        );
        self.db.execute(
            "UPDATE gmail_report_outbox SET attempt_count=attempt_count+1,last_error=NULL,
             updated_at_ms=?1 WHERE approval_id=?2 AND report_sha256=?3",
            params![now_ms, approval.id.to_string(), report.artifact.sha256],
        )?;
        match provider
            .send(
                report.account_id,
                recipient,
                &report.subject,
                &report.body,
                &idempotency_key,
            )
            .await
        {
            GmailSendResult::Sent {
                provider_message_id,
            } => {
                self.mark_sent(
                    approval.id,
                    &report.artifact.sha256,
                    &provider_message_id,
                    now_ms,
                )?;
                Ok(GmailDeliveryOutcome::Sent {
                    provider_message_id,
                })
            }
            GmailSendResult::AmbiguousTimeout => {
                self.db.execute(
                    "UPDATE gmail_report_outbox SET status='ambiguous',last_error='ambiguous_provider_timeout',
                     reconciled_at_ms=NULL,updated_at_ms=?1 WHERE approval_id=?2 AND report_sha256=?3",
                    params![now_ms,approval.id.to_string(),report.artifact.sha256],
                )?;
                Ok(GmailDeliveryOutcome::Ambiguous)
            }
            GmailSendResult::Failed { error } => {
                let bounded: String = error.chars().take(1_024).collect();
                self.db.execute(
                    "UPDATE gmail_report_outbox SET status='failed',last_error=?1,updated_at_ms=?2
                     WHERE approval_id=?3 AND report_sha256=?4",
                    params![
                        bounded,
                        now_ms,
                        approval.id.to_string(),
                        report.artifact.sha256
                    ],
                )?;
                Ok(GmailDeliveryOutcome::Failed)
            }
        }
    }

    fn ensure_send_grant(
        &self,
        account_id: ExternalIdentityId,
        owner: &PrincipalId,
    ) -> anyhow::Result<()> {
        let grant: Option<String> = self
            .db
            .query_row(
                "SELECT g.scopes_json FROM external_identities i JOIN capability_grants g USING(identity_id)
                 WHERE i.identity_id=?1 AND i.principal_id=?2 AND i.state='active' AND g.state='active'",
                params![account_id.to_string(), owner.0],
                |row| row.get(0),
            )
            .optional()?;
        let scopes: Vec<ExternalCapabilityId> = grant
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        anyhow::ensure!(
            scopes.contains(&ExternalCapabilityId::new("mail.send").unwrap()),
            "active gmail.send grant required"
        );
        Ok(())
    }

    fn delivery_row(
        &self,
        approval_id: ApprovalId,
        report_sha256: &str,
    ) -> anyhow::Result<DeliveryRow> {
        self.db
            .query_row(
                "SELECT status,provider_message_id,idempotency_key FROM gmail_report_outbox
                 WHERE approval_id=?1 AND report_sha256=?2",
                params![approval_id.to_string(), report_sha256],
                |row| {
                    Ok(DeliveryRow {
                        status: row.get(0)?,
                        provider_message_id: row.get(1)?,
                        idempotency_key: row.get(2)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    fn mark_sent(
        &self,
        approval_id: ApprovalId,
        report_sha256: &str,
        provider_message_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !provider_message_id.is_empty() && provider_message_id.len() <= 1_024,
            "invalid provider message ID"
        );
        self.db.execute(
            "UPDATE gmail_report_outbox SET status='sent',provider_message_id=?1,last_error=NULL,
             updated_at_ms=?2 WHERE approval_id=?3 AND report_sha256=?4",
            params![
                provider_message_id,
                now_ms,
                approval_id.to_string(),
                report_sha256
            ],
        )?;
        Ok(())
    }
}

struct DeliveryRow {
    status: String,
    provider_message_id: Option<String>,
    idempotency_key: String,
}

fn validate_report(report: &LocalGmailReport) -> anyhow::Result<()> {
    validate_subject(&report.subject)?;
    validate_body(&report.body)?;
    anyhow::ensure!(
        report.artifact.scan_status == ArtifactScanStatus::Clean,
        "report artifact is not clean"
    );
    anyhow::ensure!(
        report.artifact.sha256 == sha256_hex(report.body.as_bytes()),
        "report body hash changed"
    );
    Ok(())
}

fn validate_send_approval(
    approval: &ApprovalSnapshot,
    report: &LocalGmailReport,
    recipient: &str,
    now_ms: i64,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        approval.category == ApprovalCategory::SendMail,
        "wrong approval category"
    );
    anyhow::ensure!(
        approval.status == ApprovalStatus::Approved,
        "send approval is not approved"
    );
    anyhow::ensure!(
        !approval.is_expired_at(now_ms) && now_ms < approval.expires_at_ms,
        "send approval expired"
    );
    anyhow::ensure!(
        approval.goal_id == report.goal_id && approval.owner_id == report.owner,
        "send approval owner mismatch"
    );
    let expected = BTreeMap::from([
        ("account_id", report.account_id.to_string()),
        ("recipient", recipient.to_owned()),
        ("subject_sha256", sha256_hex(report.subject.as_bytes())),
        ("body_sha256", sha256_hex(report.body.as_bytes())),
        ("report_sha256", report.artifact.sha256.clone()),
    ]);
    for (key, value) in expected {
        anyhow::ensure!(
            approval.subject.attributes.get(key) == Some(&value),
            "approved report fields changed"
        );
    }
    Ok(())
}

fn validate_subject(subject: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !subject.trim().is_empty() && subject.len() <= MAX_SUBJECT_BYTES,
        "invalid report subject"
    );
    anyhow::ensure!(
        !subject.contains(['\r', '\n', '\0']),
        "invalid report subject"
    );
    Ok(())
}

fn validate_body(body: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !body.trim().is_empty() && body.len() <= MAX_BODY_BYTES,
        "invalid report body"
    );
    anyhow::ensure!(!body.contains('\0'), "invalid report body");
    Ok(())
}

fn validate_recipient(recipient: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        recipient.is_ascii() && recipient.len() <= MAX_RECIPIENT_BYTES,
        "invalid recipient"
    );
    anyhow::ensure!(
        !recipient.contains([',', '<', '>', '\r', '\n', '\0']),
        "invalid recipient"
    );
    let (local, domain) = recipient
        .rsplit_once('@')
        .ok_or_else(|| anyhow::anyhow!("invalid recipient"))?;
    anyhow::ensure!(
        !local.is_empty() && !domain.is_empty() && domain.contains('.'),
        "invalid recipient"
    );
    anyhow::ensure!(
        recipient == recipient.to_ascii_lowercase(),
        "recipient must be canonical"
    );
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
