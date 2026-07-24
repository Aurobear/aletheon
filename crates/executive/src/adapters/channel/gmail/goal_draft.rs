//! Authenticated Gmail Goal drafts and Telegram confirmation boundary.

use super::ingest::ExternalEventIngestResult;
use super::sender_policy::GmailSenderPolicy;
use super::{GmailClassification, GmailInboxRecord};
use crate::application::approval::{ApprovalCreate, ApprovalRepository};
use crate::application::goal::{migrations, ObjectiveStore};
use fabric::{
    ApprovalArtifactRef, ApprovalCategory, ApprovalId, ApprovalRisk, ApprovalSnapshot,
    ApprovalStatus, ApprovalSubject, ExternalIdentityId, GoalBudget, GoalId, GoalSnapshot,
    GoalSpec, GoalState, PrincipalId,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MAX_GOAL_INTENT_BYTES: usize = 32 * 1024;
const MAX_SOURCE_EVENT_ID_BYTES: usize = 1_024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct GmailDraftArtifactEvidence {
    filename: String,
    artifact_id: Option<String>,
    sha256: Option<String>,
    relative_path: Option<PathBuf>,
    available: bool,
    unavailable_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GmailGoalDraft {
    pub goal: GoalSnapshot,
    pub approval: ApprovalSnapshot,
    pub account_id: ExternalIdentityId,
    pub message_id: String,
    pub source_event_id: String,
    pub revision: u64,
}

/// Restart-safe boundary between authenticated Gmail ingress and executable Goals.
/// Draft identity is `(account_id, message_id)`; approval identity is derived from
/// the immutable draft revision and uses the shared M5 approval repository.
pub struct GmailGoalDraftCoordinator {
    db: Connection,
    db_path: PathBuf,
    approvals: Arc<Mutex<ApprovalRepository>>,
}

impl GmailGoalDraftCoordinator {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = Connection::open(path)?;
        migrations::run_migrations(&db)?;
        let approvals = Arc::new(Mutex::new(ApprovalRepository::open(path)?));
        Ok(Self {
            db,
            db_path: path.to_path_buf(),
            approvals,
        })
    }

    pub fn approval_repository(&self) -> Arc<Mutex<ApprovalRepository>> {
        self.approvals.clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_draft(
        &mut self,
        inbox: &GmailInboxRecord,
        current_policy: &GmailSenderPolicy,
        ingested: &ExternalEventIngestResult,
        source_event_id: &str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> anyhow::Result<GmailGoalDraft> {
        validate_goal_ingress(inbox, current_policy, ingested, source_event_id, now_ms)?;
        anyhow::ensure!(expires_at_ms > now_ms, "invalid review expiry");
        self.ensure_account_active(inbox)?;

        let account_id = inbox.account_id.to_string();
        let evidence = artifact_evidence(ingested);
        let evidence_json = serde_json::to_string(&evidence)?;
        anyhow::ensure!(
            evidence_json.len() <= 64 * 1024,
            "artifact summary exceeds cap"
        );
        let spec = goal_spec(&ingested.body_text);
        let spec_json = serde_json::to_string(&spec)?;
        let intent_hash = sha256_hex(ingested.body_text.as_bytes());

        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT objective_id FROM gmail_goal_drafts WHERE account_id=?1 AND message_id=?2",
                params![account_id, inbox.message_id],
                |row| row.get(0),
            )
            .optional()?;
        let goal_id = if let Some(id) = existing {
            GoalId(id)
        } else {
            tx.execute(
                "INSERT INTO objectives(description,session_id,scope,owner_id,goal_state,spec_json,version)
                 VALUES(?1,?2,'session',?3,'draft',?4,0)",
                params![
                    ingested.body_text,
                    format!("gmail:{}", inbox.thread_id),
                    inbox.verified_principal.as_ref().expect("validated").0,
                    spec_json
                ],
            )?;
            let goal_id = GoalId(tx.last_insert_rowid());
            tx.execute(
                "INSERT INTO goal_events(objective_id,version,event_type,payload_json)
                 VALUES(?1,0,'created',?2)",
                params![
                    goal_id.0,
                    serde_json::json!({
                        "source":"gmail", "account_id":account_id,
                        "message_id":inbox.message_id, "source_event_id":source_event_id
                    })
                    .to_string()
                ],
            )?;
            tx.execute(
                "INSERT INTO gmail_goal_draft_revisions(
                    objective_id,revision,intent,intent_sha256,created_at_ms)
                 VALUES(?1,1,?2,?3,?4)",
                params![goal_id.0, ingested.body_text, intent_hash, now_ms],
            )?;
            tx.execute(
                "INSERT INTO gmail_goal_drafts(
                    account_id,message_id,thread_id,source_event_id,objective_id,principal_id,
                    sender_address,sender_policy_version,current_revision,status,
                    artifact_summary_json,created_at_ms,updated_at_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,'pending',?9,?10,?10)",
                params![
                    account_id,
                    inbox.message_id,
                    inbox.thread_id,
                    source_event_id,
                    goal_id.0,
                    inbox.verified_principal.as_ref().expect("validated").0,
                    inbox.sender_address.as_deref().expect("validated"),
                    inbox.sender_policy_version.expect("validated"),
                    evidence_json,
                    now_ms
                ],
            )?;
            goal_id
        };
        tx.commit()?;
        self.ensure_review(goal_id, now_ms, expires_at_ms)
    }

    pub fn resume_review(
        &self,
        goal_id: GoalId,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> anyhow::Result<GmailGoalDraft> {
        self.ensure_review(goal_id, now_ms, expires_at_ms)
    }

    pub fn confirm(
        &self,
        approval: &ApprovalSnapshot,
        now_ms: i64,
    ) -> anyhow::Result<GoalSnapshot> {
        validate_draft_resolution(approval, ApprovalStatus::Approved)?;
        anyhow::ensure!(
            approval
                .resolution
                .as_ref()
                .and_then(|r| r.channel.as_deref())
                == Some("telegram"),
            "Gmail Goal confirmation must come from Telegram"
        );
        let row = self.draft_row(approval.goal_id)?;
        anyhow::ensure!(row.approval_id == Some(approval.id), "stale draft approval");
        self.ensure_row_account_active(&row)?;
        let store = ObjectiveStore::open(&self.db_path)?;
        let goal = store
            .get_goal(approval.goal_id)?
            .ok_or_else(|| anyhow::anyhow!("draft Goal not found"))?;
        let next = if goal.state == GoalState::Ready {
            goal
        } else {
            anyhow::ensure!(
                goal.state == GoalState::Draft,
                "draft is no longer confirmable"
            );
            store.transition_goal(
                goal.id,
                goal.version,
                GoalState::Ready,
                None,
                &serde_json::json!({
                    "action":"gmail_draft_confirmed",
                    "approval_id":approval.id.to_string(),
                    "principal_id":approval.resolution.as_ref().and_then(|r| r.principal_id.as_ref()).map(|p| p.0.as_str()),
                    "channel":"telegram"
                }),
            )?
        };
        self.db.execute(
            "UPDATE gmail_goal_drafts SET status='confirmed',updated_at_ms=?1
             WHERE objective_id=?2 AND current_approval_id=?3",
            params![now_ms, next.id.0, approval.id.to_string()],
        )?;
        Ok(next)
    }

    pub fn reject_or_edit(
        &self,
        approval: &ApprovalSnapshot,
        edit: bool,
        now_ms: i64,
    ) -> anyhow::Result<GoalSnapshot> {
        validate_draft_resolution(approval, ApprovalStatus::Rejected)?;
        let row = self.draft_row(approval.goal_id)?;
        anyhow::ensure!(row.approval_id == Some(approval.id), "stale draft approval");
        let store = ObjectiveStore::open(&self.db_path)?;
        let goal = store
            .get_goal(approval.goal_id)?
            .ok_or_else(|| anyhow::anyhow!("draft Goal not found"))?;
        if edit {
            anyhow::ensure!(
                goal.state == GoalState::Draft,
                "draft is no longer editable"
            );
            self.db.execute(
                "UPDATE gmail_goal_drafts SET status='awaiting_edit',updated_at_ms=?1
                 WHERE objective_id=?2 AND current_approval_id=?3",
                params![now_ms, goal.id.0, approval.id.to_string()],
            )?;
            Ok(goal)
        } else {
            let next = if goal.state == GoalState::Cancelled {
                goal
            } else {
                anyhow::ensure!(
                    goal.state == GoalState::Draft,
                    "draft is no longer rejectable"
                );
                store.transition_goal(
                    goal.id,
                    goal.version,
                    GoalState::Cancelled,
                    None,
                    &serde_json::json!({"action":"gmail_draft_rejected","approval_id":approval.id.to_string()}),
                )?
            };
            self.db.execute(
                "UPDATE gmail_goal_drafts SET status='rejected',updated_at_ms=?1
                 WHERE objective_id=?2 AND current_approval_id=?3",
                params![now_ms, next.id.0, approval.id.to_string()],
            )?;
            Ok(next)
        }
    }

    pub fn revise(
        &mut self,
        goal_id: GoalId,
        principal: &PrincipalId,
        revised_intent: &str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> anyhow::Result<GmailGoalDraft> {
        validate_intent(revised_intent)?;
        anyhow::ensure!(expires_at_ms > now_ms, "invalid review expiry");
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (owner, state, version, spec_json, revision, status): (
            String,
            String,
            u64,
            String,
            u64,
            String,
        ) = tx.query_row(
            "SELECT o.owner_id,o.goal_state,o.version,o.spec_json,d.current_revision,d.status
             FROM objectives o JOIN gmail_goal_drafts d ON d.objective_id=o.objective_id
             WHERE o.objective_id=?1",
            [goal_id.0],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        anyhow::ensure!(owner == principal.0, "draft owner mismatch");
        anyhow::ensure!(
            state == "draft" && status == "awaiting_edit",
            "draft is not awaiting edit"
        );
        let mut spec: GoalSpec = serde_json::from_str(&spec_json)?;
        spec.original_intent = revised_intent.to_owned();
        let next_revision = revision + 1;
        let next_version = version + 1;
        tx.execute(
            "UPDATE objectives SET description=?1,spec_json=?2,version=?3,updated_at=datetime('now')
             WHERE objective_id=?4 AND version=?5 AND goal_state='draft'",
            params![revised_intent, serde_json::to_string(&spec)?, next_version, goal_id.0, version],
        )?;
        tx.execute(
            "INSERT INTO goal_events(objective_id,version,event_type,payload_json)
             VALUES(?1,?2,'draft_revised',?3)",
            params![goal_id.0,next_version,serde_json::json!({"revision":next_revision,"intent_sha256":sha256_hex(revised_intent.as_bytes())}).to_string()],
        )?;
        tx.execute(
            "INSERT INTO gmail_goal_draft_revisions(objective_id,revision,intent,intent_sha256,created_at_ms)
             VALUES(?1,?2,?3,?4,?5)",
            params![goal_id.0,next_revision,revised_intent,sha256_hex(revised_intent.as_bytes()),now_ms],
        )?;
        tx.execute(
            "UPDATE gmail_goal_drafts SET current_revision=?1,current_approval_id=NULL,
             status='pending',updated_at_ms=?2 WHERE objective_id=?3",
            params![next_revision, now_ms, goal_id.0],
        )?;
        tx.commit()?;
        self.ensure_review(goal_id, now_ms, expires_at_ms)
    }

    fn ensure_review(
        &self,
        goal_id: GoalId,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> anyhow::Result<GmailGoalDraft> {
        let row = self.draft_row(goal_id)?;
        anyhow::ensure!(row.status == "pending", "draft is not pending review");
        self.ensure_row_account_active(&row)?;
        let store = ObjectiveStore::open(&self.db_path)?;
        let goal = store
            .get_goal(goal_id)?
            .ok_or_else(|| anyhow::anyhow!("draft Goal not found"))?;
        anyhow::ensure!(goal.state == GoalState::Draft, "Goal is not a draft");
        if let Some(id) = row.approval_id {
            let approval = self
                .approvals
                .lock()
                .unwrap()
                .get(id)?
                .ok_or_else(|| anyhow::anyhow!("draft approval not found"))?;
            return Ok(row.into_result(goal, approval));
        }
        let artifacts = row
            .artifacts
            .iter()
            .filter(|item| item.available)
            .filter_map(|item| {
                Some(ApprovalArtifactRef {
                    kind: "gmail_evidence".into(),
                    relative_path: item.relative_path.clone()?,
                    sha256: item.sha256.clone()?,
                })
            })
            .collect::<Vec<_>>();
        let unavailable = row.artifacts.iter().filter(|item| !item.available).count();
        let intent_preview: String = goal.spec.original_intent.chars().take(2_000).collect();
        let approval = self.approvals.lock().unwrap().create(ApprovalCreate {
            subject: ApprovalSubject {
                category: ApprovalCategory::ActivateGoal,
                goal_id,
                attempt_id: None,
                job_id: None,
                attributes: BTreeMap::from([
                    ("provider".into(), "gmail".into()),
                    ("account_id".into(), row.account_id.to_string()),
                    ("message_id".into(), row.message_id.clone()),
                    ("source_event_id".into(), row.source_event_id.clone()),
                    ("revision".into(), row.revision.to_string()),
                    ("intent_sha256".into(), sha256_hex(goal.spec.original_intent.as_bytes())),
                ]),
                allowed_scope: vec![],
                apply_target: None,
            },
            risk: ApprovalRisk::Medium,
            summary: format!(
                "Email Goal intent: {intent_preview}\nSource: Gmail {}/{}\nArtifacts: {} clean, {unavailable} unavailable",
                row.account_id,
                row.message_id,
                artifacts.len()
            ),
            artifacts,
            created_at_ms: now_ms,
            expires_at_ms,
        })?;
        self.db.execute(
            "UPDATE gmail_goal_drafts SET current_approval_id=?1,updated_at_ms=?2
             WHERE objective_id=?3 AND current_approval_id IS NULL",
            params![approval.id.to_string(), now_ms, goal_id.0],
        )?;
        self.db.execute(
            "UPDATE gmail_goal_draft_revisions SET approval_id=?1
             WHERE objective_id=?2 AND revision=?3 AND approval_id IS NULL",
            params![approval.id.to_string(), goal_id.0, row.revision],
        )?;
        let current = self.draft_row(goal_id)?;
        Ok(current.into_result(goal, approval))
    }

    fn ensure_account_active(&self, inbox: &GmailInboxRecord) -> anyhow::Result<()> {
        let active: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM external_identities i JOIN capability_grants g USING(identity_id)
             WHERE i.identity_id=?1 AND i.principal_id=?2 AND i.state='active' AND g.state='active')",
            params![inbox.account_id.to_string(), inbox.verified_principal.as_ref().expect("validated").0],
            |row| row.get(0),
        )?;
        anyhow::ensure!(active, "Google account or grant is revoked");
        Ok(())
    }

    fn ensure_row_account_active(&self, row: &DraftRow) -> anyhow::Result<()> {
        let active: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM external_identities i JOIN capability_grants g USING(identity_id)
             WHERE i.identity_id=?1 AND i.principal_id=?2 AND i.state='active' AND g.state='active')",
            params![row.account_id.to_string(), row.principal.0],
            |sql_row| sql_row.get(0),
        )?;
        anyhow::ensure!(active, "Google account or grant is revoked");
        Ok(())
    }

    fn draft_row(&self, goal_id: GoalId) -> anyhow::Result<DraftRow> {
        self.db
            .query_row(
                "SELECT account_id,message_id,source_event_id,principal_id,current_revision,
                    current_approval_id,status,artifact_summary_json
             FROM gmail_goal_drafts WHERE objective_id=?1",
                [goal_id.0],
                |row| {
                    let account: String = row.get(0)?;
                    let approval: Option<String> = row.get(5)?;
                    let artifacts: String = row.get(7)?;
                    Ok(DraftRow {
                        account_id: ExternalIdentityId(uuid::Uuid::parse_str(&account).map_err(
                            |e| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    0,
                                    rusqlite::types::Type::Text,
                                    Box::new(e),
                                )
                            },
                        )?),
                        message_id: row.get(1)?,
                        source_event_id: row.get(2)?,
                        principal: PrincipalId(row.get(3)?),
                        revision: row.get(4)?,
                        approval_id: approval
                            .map(|id| uuid::Uuid::parse_str(&id).map(ApprovalId))
                            .transpose()
                            .map_err(|e| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    5,
                                    rusqlite::types::Type::Text,
                                    Box::new(e),
                                )
                            })?,
                        status: row.get(6)?,
                        artifacts: serde_json::from_str(&artifacts).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                7,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        })?,
                    })
                },
            )
            .map_err(Into::into)
    }
}

struct DraftRow {
    account_id: ExternalIdentityId,
    message_id: String,
    source_event_id: String,
    principal: PrincipalId,
    revision: u64,
    approval_id: Option<ApprovalId>,
    status: String,
    artifacts: Vec<GmailDraftArtifactEvidence>,
}

impl DraftRow {
    fn into_result(self, goal: GoalSnapshot, approval: ApprovalSnapshot) -> GmailGoalDraft {
        GmailGoalDraft {
            goal,
            approval,
            account_id: self.account_id,
            message_id: self.message_id,
            source_event_id: self.source_event_id,
            revision: self.revision,
        }
    }
}

fn validate_goal_ingress(
    inbox: &GmailInboxRecord,
    policy: &GmailSenderPolicy,
    ingested: &ExternalEventIngestResult,
    source_event_id: &str,
    now_ms: i64,
) -> anyhow::Result<()> {
    anyhow::ensure!(now_ms >= 0, "invalid Gmail Goal time");
    anyhow::ensure!(
        inbox.status == "accepted",
        "Gmail message is not authenticated"
    );
    anyhow::ensure!(
        inbox.classification == GmailClassification::Goal,
        "Gmail message is not a Goal"
    );
    let principal = inbox
        .verified_principal
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Gmail Goal has no bound principal"))?;
    let sender = inbox
        .sender_address
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Gmail Goal has no authenticated sender"))?;
    anyhow::ensure!(
        principal == &policy.principal,
        "sender policy principal changed"
    );
    anyhow::ensure!(
        inbox.sender_policy_version == Some(policy.version),
        "sender policy changed"
    );
    let domain = sender
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .ok_or_else(|| anyhow::anyhow!("invalid authenticated sender"))?;
    anyhow::ensure!(
        policy.allowed_addresses.contains(sender) || policy.allowed_domains.contains(domain),
        "authenticated sender is no longer allowed"
    );
    anyhow::ensure!(
        inbox.account_id == ingested.original.account_id,
        "Gmail account mismatch"
    );
    anyhow::ensure!(
        inbox.message_id == ingested.original.message_id,
        "Gmail message mismatch"
    );
    anyhow::ensure!(
        inbox.thread_id == ingested.original.thread_id,
        "Gmail thread mismatch"
    );
    anyhow::ensure!(
        !source_event_id.is_empty() && source_event_id.len() <= MAX_SOURCE_EVENT_ID_BYTES,
        "invalid source event ID"
    );
    validate_intent(&ingested.body_text)
}

fn validate_intent(intent: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !intent.trim().is_empty() && intent.len() <= MAX_GOAL_INTENT_BYTES,
        "invalid Gmail Goal intent"
    );
    anyhow::ensure!(!intent.contains('\0'), "invalid Gmail Goal intent");
    Ok(())
}

fn goal_spec(intent: &str) -> GoalSpec {
    GoalSpec {
        original_intent: intent.to_owned(),
        desired_state: vec![],
        constraints: vec!["Requires Telegram owner confirmation before execution".into()],
        acceptance_criteria: vec![],
        budget: GoalBudget::default(),
    }
}

fn artifact_evidence(ingested: &ExternalEventIngestResult) -> Vec<GmailDraftArtifactEvidence> {
    ingested
        .attachments
        .iter()
        .map(|attachment| {
            let available = attachment.available_to_model();
            GmailDraftArtifactEvidence {
                filename: attachment.filename.chars().take(512).collect(),
                artifact_id: attachment
                    .artifact
                    .as_ref()
                    .map(|item| item.artifact_id.clone()),
                sha256: attachment.artifact.as_ref().map(|item| item.sha256.clone()),
                relative_path: attachment
                    .artifact
                    .as_ref()
                    .map(|item| item.relative_path.clone()),
                available,
                unavailable_reason: (!available).then(|| {
                    attachment
                        .unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "not_clean".into())
                        .chars()
                        .take(256)
                        .collect()
                }),
            }
        })
        .collect()
}

fn validate_draft_resolution(
    approval: &ApprovalSnapshot,
    expected: ApprovalStatus,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        approval.category == ApprovalCategory::ActivateGoal,
        "wrong approval category"
    );
    anyhow::ensure!(approval.status == expected, "approval has wrong resolution");
    anyhow::ensure!(
        approval.resolution.is_some(),
        "approval resolution is missing"
    );
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
