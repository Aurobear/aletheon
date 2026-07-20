//! Atomic normalized Google event, projection, outbox, and cursor persistence.

use crate::r#impl::goal::migrations;
use fabric::{ExternalEventEnvelope, ExternalEventId, ExternalIdentityId, PrincipalId};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::time::Duration;

const MAX_EVENT_JSON_BYTES: usize = 1_048_576;
const MAX_PROJECTION_JSON_BYTES: usize = 512 * 1_024;
const MAX_CURSOR_BYTES: usize = 16 * 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStream {
    GmailHistory,
    Calendar,
    DriveChanges,
}

impl SyncStream {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GmailHistory => "gmail_history",
            Self::Calendar => "calendar",
            Self::DriveChanges => "drive_changes",
        }
    }

    fn parse(value: &str) -> Result<Self, rusqlite::Error> {
        match value {
            "gmail_history" => Ok(Self::GmailHistory),
            "calendar" => Ok(Self::Calendar),
            "drive_changes" => Ok(Self::DriveChanges),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleSyncCursor {
    pub account_id: ExternalIdentityId,
    pub stream: SyncStream,
    pub token: Option<String>,
    pub generation: u64,
    pub last_success_ms: Option<i64>,
    pub last_error_ms: Option<i64>,
    pub retry_count: u32,
    pub retry_after_ms: Option<i64>,
    pub health_state: String,
    pub version: u64,
}

#[derive(Debug, Clone)]
pub struct ProjectionWrite {
    pub json: serde_json::Value,
    pub tombstone: bool,
}

#[derive(Debug, Clone)]
pub struct SyncCommit {
    pub account_id: ExternalIdentityId,
    pub stream: SyncStream,
    pub expected_cursor_token: Option<String>,
    pub expected_cursor_version: u64,
    pub successor_cursor_token: String,
    pub cursor_generation: u64,
    pub events: Vec<(ExternalEventEnvelope, ProjectionWrite)>,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEventOutcome {
    pub event_id: ExternalEventId,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCommitOutcome {
    pub events: Vec<CommitEventOutcome>,
    pub cursor: GoogleSyncCursor,
}

#[derive(Debug, Clone)]
pub struct GoogleOutboxClaim {
    pub outbox_id: String,
    pub event: ExternalEventEnvelope,
    pub attempt_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleSubscriptionQuery {
    pub object_id: Option<String>,
    pub important_only: bool,
    pub source_after_ms: Option<i64>,
    pub source_before_ms: Option<i64>,
    pub telegram_conversation_id: Option<String>,
    pub current_task_id: Option<String>,
    pub propose_memory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleSubscription {
    pub subscription_id: String,
    pub principal_id: PrincipalId,
    pub account_id: ExternalIdentityId,
    pub stream: SyncStream,
    pub event_kinds: Vec<String>,
    pub query: GoogleSubscriptionQuery,
    pub cursor_generation: u64,
    pub state: String,
    pub version: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

pub struct GoogleSyncStore {
    db: Connection,
    #[cfg(test)]
    fail_after_statement: std::cell::Cell<Option<usize>>,
}

impl GoogleSyncStore {
    pub fn open(path: &Path) -> Result<Self, SyncStoreError> {
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        migrations::run_migrations(&db)
            .map_err(|error| SyncStoreError::Storage(error.to_string()))?;
        Ok(Self {
            db,
            #[cfg(test)]
            fail_after_statement: std::cell::Cell::new(None),
        })
    }

    pub fn initialize_cursor(
        &self,
        account_id: ExternalIdentityId,
        stream: SyncStream,
        token: Option<&str>,
        generation: u64,
    ) -> Result<GoogleSyncCursor, SyncStoreError> {
        validate_cursor(token)?;
        self.db.execute(
            "INSERT OR IGNORE INTO google_sync_cursors(
                account_id,stream,cursor_token,generation,health_state,version
             ) VALUES(?1,?2,?3,?4,'healthy',0)",
            params![account_id.to_string(), stream.as_str(), token, generation],
        )?;
        self.cursor(account_id, stream)?
            .ok_or_else(|| SyncStoreError::Storage("cursor initialization failed".into()))
    }

    pub fn cursor(
        &self,
        account_id: ExternalIdentityId,
        stream: SyncStream,
    ) -> Result<Option<GoogleSyncCursor>, SyncStoreError> {
        self.db
            .query_row(
                "SELECT cursor_token,generation,last_success_ms,last_error_ms,retry_count,
                        retry_after_ms,health_state,version
                 FROM google_sync_cursors WHERE account_id=?1 AND stream=?2",
                params![account_id.to_string(), stream.as_str()],
                |row| decode_cursor(row, account_id, stream),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn commit(&self, commit: SyncCommit) -> Result<SyncCommitOutcome, SyncStoreError> {
        validate_commit(&commit)?;
        let tx = self.db.unchecked_transaction()?;
        let cursor = tx
            .query_row(
                "SELECT cursor_token,generation,last_success_ms,last_error_ms,retry_count,
                        retry_after_ms,health_state,version
                 FROM google_sync_cursors WHERE account_id=?1 AND stream=?2",
                params![commit.account_id.to_string(), commit.stream.as_str()],
                |row| decode_cursor(row, commit.account_id, commit.stream),
            )
            .optional()?
            .ok_or(SyncStoreError::CursorNotInitialized)?;
        if cursor.version != commit.expected_cursor_version
            || cursor.token != commit.expected_cursor_token
            || commit.cursor_generation < cursor.generation
        {
            return Err(SyncStoreError::CursorConflict {
                expected_version: commit.expected_cursor_version,
                actual_version: cursor.version,
            });
        }

        let mut outcomes = Vec::with_capacity(commit.events.len());
        let mut statement_index = 0;
        for (event, projection) in &commit.events {
            let envelope_json = serde_json::to_string(event)?;
            let projection_json = serde_json::to_string(&projection.json)?;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO google_events(
                    event_id,account_id,stream,dedup_key,event_kind,object_id,object_version,
                    source_timestamp_ms,observed_at_ms,payload_hash,envelope_json,created_at_ms
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    event.id.to_string(),
                    commit.account_id.to_string(),
                    commit.stream.as_str(),
                    event.dedup_key,
                    event.event.kind(),
                    event.object.object_id,
                    event.object.object_version,
                    event.source_timestamp_ms,
                    event.observed_at_ms,
                    event.payload_hash,
                    envelope_json,
                    commit.committed_at_ms
                ],
            )? == 1;
            self.maybe_fail(statement_index)?;
            statement_index += 1;

            let event_id = if inserted {
                event.id
            } else {
                tx.query_row(
                    "SELECT event_id FROM google_events
                     WHERE account_id=?1 AND stream=?2 AND dedup_key=?3",
                    params![
                        commit.account_id.to_string(),
                        commit.stream.as_str(),
                        event.dedup_key
                    ],
                    |row| parse_event_id(row.get::<_, String>(0)?),
                )?
            };
            if inserted {
                upsert_projection(
                    &tx,
                    event,
                    commit.stream,
                    &projection_json,
                    projection.tombstone,
                )?;
                self.maybe_fail(statement_index)?;
                statement_index += 1;
                tx.execute(
                    "INSERT INTO google_event_outbox(
                        outbox_id,event_id,status,created_at_ms,updated_at_ms
                     ) VALUES(?1,?2,'pending',?3,?3)",
                    params![
                        event.id.to_string(),
                        event.id.to_string(),
                        commit.committed_at_ms
                    ],
                )?;
                self.maybe_fail(statement_index)?;
                statement_index += 1;
            }
            outcomes.push(CommitEventOutcome { event_id, inserted });
        }

        let changed = tx.execute(
            "UPDATE google_sync_cursors SET
                cursor_token=?1,generation=?2,last_success_ms=?3,last_error_ms=NULL,
                retry_count=0,retry_after_ms=NULL,health_state='healthy',version=version+1
             WHERE account_id=?4 AND stream=?5 AND version=?6
               AND (cursor_token IS ?7 OR cursor_token=?7)",
            params![
                commit.successor_cursor_token,
                commit.cursor_generation,
                commit.committed_at_ms,
                commit.account_id.to_string(),
                commit.stream.as_str(),
                commit.expected_cursor_version,
                commit.expected_cursor_token
            ],
        )?;
        self.maybe_fail(statement_index)?;
        if changed != 1 {
            return Err(SyncStoreError::CursorConflict {
                expected_version: commit.expected_cursor_version,
                actual_version: cursor.version,
            });
        }
        let next = tx.query_row(
            "SELECT cursor_token,generation,last_success_ms,last_error_ms,retry_count,
                    retry_after_ms,health_state,version
             FROM google_sync_cursors WHERE account_id=?1 AND stream=?2",
            params![commit.account_id.to_string(), commit.stream.as_str()],
            |row| decode_cursor(row, commit.account_id, commit.stream),
        )?;
        tx.commit()?;
        Ok(SyncCommitOutcome {
            events: outcomes,
            cursor: next,
        })
    }

    pub fn event(
        &self,
        id: ExternalEventId,
    ) -> Result<Option<ExternalEventEnvelope>, SyncStoreError> {
        self.db
            .query_row(
                "SELECT envelope_json FROM google_events WHERE event_id=?1",
                [id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    pub fn projection(
        &self,
        account_id: ExternalIdentityId,
        stream: SyncStream,
        object_id: &str,
    ) -> Result<Option<(String, bool, i64)>, SyncStoreError> {
        self.db
            .query_row(
                "SELECT projection_json,tombstone,source_timestamp_ms FROM google_objects
                 WHERE account_id=?1 AND stream=?2 AND object_id=?3",
                params![account_id.to_string(), stream.as_str(), object_id],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0, row.get(2)?)),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn pending_outbox_count(&self) -> Result<u64, SyncStoreError> {
        self.db
            .query_row(
                "SELECT COUNT(*) FROM google_event_outbox WHERE status='pending'",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn acquire_lease(
        &self,
        account_id: ExternalIdentityId,
        stream: SyncStream,
        owner: &str,
        now_ms: i64,
        lease_duration_ms: i64,
    ) -> Result<bool, SyncStoreError> {
        if owner.is_empty()
            || owner.len() > 256
            || now_ms < 0
            || !(1_000..=300_000).contains(&lease_duration_ms)
        {
            return Err(SyncStoreError::InvalidInput);
        }
        let expires = now_ms.saturating_add(lease_duration_ms);
        let changed = self.db.execute(
            "INSERT INTO google_sync_leases(account_id,stream,lease_owner,lease_expires_at_ms,version)
             VALUES(?1,?2,?3,?4,0)
             ON CONFLICT(account_id,stream) DO UPDATE SET
                lease_owner=excluded.lease_owner,
                lease_expires_at_ms=excluded.lease_expires_at_ms,
                version=google_sync_leases.version+1
             WHERE google_sync_leases.lease_owner=excluded.lease_owner
                OR google_sync_leases.lease_expires_at_ms<=?5",
            params![account_id.to_string(), stream.as_str(), owner, expires, now_ms],
        )?;
        Ok(changed == 1)
    }

    pub fn release_lease(
        &self,
        account_id: ExternalIdentityId,
        stream: SyncStream,
        owner: &str,
    ) -> Result<bool, SyncStoreError> {
        Ok(self.db.execute(
            "DELETE FROM google_sync_leases
             WHERE account_id=?1 AND stream=?2 AND lease_owner=?3",
            params![account_id.to_string(), stream.as_str(), owner],
        )? == 1)
    }

    pub fn record_sync_failure(
        &self,
        account_id: ExternalIdentityId,
        stream: SyncStream,
        now_ms: i64,
        retry_after_ms: Option<i64>,
        health_state: &str,
    ) -> Result<GoogleSyncCursor, SyncStoreError> {
        if now_ms < 0
            || retry_after_ms.is_some_and(|value| value < now_ms)
            || !matches!(
                health_state,
                "retrying" | "circuit_open" | "auth_required" | "revoked"
            )
        {
            return Err(SyncStoreError::InvalidInput);
        }
        let changed = self.db.execute(
            "UPDATE google_sync_cursors SET
                last_error_ms=?1,retry_count=retry_count+1,retry_after_ms=?2,
                health_state=?3,version=version+1
             WHERE account_id=?4 AND stream=?5",
            params![
                now_ms,
                retry_after_ms,
                health_state,
                account_id.to_string(),
                stream.as_str()
            ],
        )?;
        if changed != 1 {
            return Err(SyncStoreError::CursorNotInitialized);
        }
        self.cursor(account_id, stream)?
            .ok_or(SyncStoreError::CursorNotInitialized)
    }

    pub fn claim_outbox(
        &self,
        owner: &str,
        now_ms: i64,
        claim_duration_ms: i64,
        limit: usize,
    ) -> Result<Vec<GoogleOutboxClaim>, SyncStoreError> {
        if owner.is_empty()
            || owner.len() > 256
            || now_ms < 0
            || !(1_000..=300_000).contains(&claim_duration_ms)
            || !(1..=100).contains(&limit)
        {
            return Err(SyncStoreError::InvalidInput);
        }
        let tx = self.db.unchecked_transaction()?;
        let expires = now_ms.saturating_add(claim_duration_ms);
        let mut statement = tx.prepare(
            "SELECT outbox_id FROM google_event_outbox
             WHERE status IN ('pending','failed')
                OR (status='claimed' AND claim_expires_at_ms<=?1)
             ORDER BY created_at_ms,outbox_id LIMIT ?2",
        )?;
        let ids = statement
            .query_map(params![now_ms, limit as i64], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut claims = Vec::with_capacity(ids.len());
        for id in ids {
            let changed = tx.execute(
                "UPDATE google_event_outbox SET
                    status='claimed',claim_owner=?1,claim_expires_at_ms=?2,
                    attempt_count=attempt_count+1,updated_at_ms=?3
                 WHERE outbox_id=?4 AND (
                    status IN ('pending','failed')
                    OR (status='claimed' AND claim_expires_at_ms<=?3)
                 )",
                params![owner, expires, now_ms, id],
            )?;
            if changed == 1 {
                let (json, attempts) = tx.query_row(
                    "SELECT e.envelope_json,o.attempt_count
                     FROM google_event_outbox o JOIN google_events e ON e.event_id=o.event_id
                     WHERE o.outbox_id=?1",
                    [&id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
                )?;
                claims.push(GoogleOutboxClaim {
                    outbox_id: id,
                    event: serde_json::from_str(&json)?,
                    attempt_count: attempts,
                });
            }
        }
        tx.commit()?;
        Ok(claims)
    }

    pub fn acknowledge_outbox(
        &self,
        outbox_id: &str,
        owner: &str,
        now_ms: i64,
    ) -> Result<bool, SyncStoreError> {
        Ok(self.db.execute(
            "UPDATE google_event_outbox SET
                status='delivered',claim_owner=NULL,claim_expires_at_ms=NULL,
                last_error_code=NULL,updated_at_ms=?1,delivered_at_ms=?1
             WHERE outbox_id=?2 AND status='claimed' AND claim_owner=?3",
            params![now_ms, outbox_id, owner],
        )? == 1)
    }

    pub fn fail_outbox(
        &self,
        outbox_id: &str,
        owner: &str,
        error_code: &str,
        now_ms: i64,
    ) -> Result<bool, SyncStoreError> {
        if error_code.is_empty() || error_code.len() > 256 {
            return Err(SyncStoreError::InvalidInput);
        }
        Ok(self.db.execute(
            "UPDATE google_event_outbox SET
                status='failed',claim_owner=NULL,claim_expires_at_ms=NULL,
                last_error_code=?1,updated_at_ms=?2
             WHERE outbox_id=?3 AND status='claimed' AND claim_owner=?4",
            params![error_code, now_ms, outbox_id, owner],
        )? == 1)
    }

    pub fn put_subscription(
        &self,
        subscription: &GoogleSubscription,
        expected_version: Option<u64>,
    ) -> Result<GoogleSubscription, SyncStoreError> {
        validate_subscription(subscription)?;
        let event_kinds = serde_json::to_string(&subscription.event_kinds)?;
        let query = serde_json::to_string(&subscription.query)?;
        match expected_version {
            None => {
                self.db.execute(
                    "INSERT INTO google_subscriptions(
                        subscription_id,principal_id,account_id,stream,event_kinds_json,
                        query_json,cursor_generation,state,version,created_at_ms,updated_at_ms
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,0,?9,?10)",
                    params![
                        subscription.subscription_id,
                        subscription.principal_id.0,
                        subscription.account_id.to_string(),
                        subscription.stream.as_str(),
                        event_kinds,
                        query,
                        subscription.cursor_generation,
                        subscription.state,
                        subscription.created_at_ms,
                        subscription.updated_at_ms
                    ],
                )?;
            }
            Some(expected) => {
                let changed = self.db.execute(
                    "UPDATE google_subscriptions SET
                        event_kinds_json=?1,query_json=?2,cursor_generation=?3,state=?4,
                        version=version+1,updated_at_ms=?5
                     WHERE subscription_id=?6 AND version=?7",
                    params![
                        event_kinds,
                        query,
                        subscription.cursor_generation,
                        subscription.state,
                        subscription.updated_at_ms,
                        subscription.subscription_id,
                        expected
                    ],
                )?;
                if changed != 1 {
                    return Err(SyncStoreError::CursorConflict {
                        expected_version: expected,
                        actual_version: expected.saturating_add(1),
                    });
                }
            }
        }
        self.subscription(&subscription.subscription_id)?
            .ok_or_else(|| SyncStoreError::Storage("subscription write disappeared".into()))
    }

    pub fn subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<GoogleSubscription>, SyncStoreError> {
        self.db
            .query_row(
                "SELECT principal_id,account_id,stream,event_kinds_json,query_json,
                        cursor_generation,state,version,created_at_ms,updated_at_ms
                 FROM google_subscriptions WHERE subscription_id=?1",
                [subscription_id],
                |row| decode_subscription(row, subscription_id),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn matching_subscriptions(
        &self,
        event: &ExternalEventEnvelope,
        stream: SyncStream,
        cursor_generation: u64,
    ) -> Result<Vec<GoogleSubscription>, SyncStoreError> {
        let mut statement = self.db.prepare(
            "SELECT subscription_id,principal_id,account_id,stream,event_kinds_json,query_json,
                    cursor_generation,state,version,created_at_ms,updated_at_ms
             FROM google_subscriptions
             WHERE account_id=?1 AND stream=?2 AND state='active' AND cursor_generation=?3
             ORDER BY subscription_id LIMIT 100",
        )?;
        let subscriptions = statement
            .query_map(
                params![
                    event.account_id.to_string(),
                    stream.as_str(),
                    cursor_generation
                ],
                |row| {
                    let id = row.get::<_, String>(0)?;
                    decode_subscription_offset(row, &id, 1)
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(subscriptions
            .into_iter()
            .filter(|subscription| subscription_matches(subscription, event))
            .collect())
    }

    pub fn account_is_active(
        &self,
        account_id: ExternalIdentityId,
    ) -> Result<bool, SyncStoreError> {
        self.db
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM external_identities i
                    JOIN capability_grants g USING(identity_id)
                    WHERE i.identity_id=?1 AND i.state='active' AND g.state='active'
                )",
                [account_id.to_string()],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    #[cfg(test)]
    fn fail_after(&self, statement_index: usize) {
        self.fail_after_statement.set(Some(statement_index));
    }

    #[cfg(test)]
    fn maybe_fail(&self, statement_index: usize) -> Result<(), SyncStoreError> {
        if self.fail_after_statement.get() == Some(statement_index) {
            self.fail_after_statement.set(None);
            return Err(SyncStoreError::InjectedFailure(statement_index));
        }
        Ok(())
    }

    #[cfg(not(test))]
    fn maybe_fail(&self, _statement_index: usize) -> Result<(), SyncStoreError> {
        Ok(())
    }
}

fn upsert_projection(
    tx: &Transaction<'_>,
    event: &ExternalEventEnvelope,
    stream: SyncStream,
    projection_json: &str,
    tombstone: bool,
) -> Result<(), SyncStoreError> {
    tx.execute(
        "INSERT INTO google_objects(
            account_id,stream,object_id,object_version,latest_event_id,source_timestamp_ms,
            projection_json,tombstone,updated_at_ms
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(account_id,stream,object_id) DO UPDATE SET
            object_version=excluded.object_version,
            latest_event_id=excluded.latest_event_id,
            source_timestamp_ms=excluded.source_timestamp_ms,
            projection_json=excluded.projection_json,
            tombstone=excluded.tombstone,
            updated_at_ms=excluded.updated_at_ms
         WHERE excluded.source_timestamp_ms >= google_objects.source_timestamp_ms",
        params![
            event.account_id.to_string(),
            stream.as_str(),
            event.object.object_id,
            event.object.object_version,
            event.id.to_string(),
            event.source_timestamp_ms,
            projection_json,
            i64::from(tombstone),
            event.observed_at_ms
        ],
    )?;
    Ok(())
}

fn validate_commit(commit: &SyncCommit) -> Result<(), SyncStoreError> {
    validate_cursor(commit.expected_cursor_token.as_deref())?;
    validate_cursor(Some(&commit.successor_cursor_token))?;
    if commit.committed_at_ms < 0 || commit.events.len() > 1_000 {
        return Err(SyncStoreError::InvalidInput);
    }
    for (event, projection) in &commit.events {
        event.validate().map_err(|_| SyncStoreError::InvalidInput)?;
        if event.account_id != commit.account_id {
            return Err(SyncStoreError::InvalidInput);
        }
        let envelope_bytes = serde_json::to_vec(event)?;
        let projection_bytes = serde_json::to_vec(&projection.json)?;
        if contains_forbidden_sensitive_field(&projection.json) {
            return Err(SyncStoreError::InvalidInput);
        }
        if envelope_bytes.len() > MAX_EVENT_JSON_BYTES
            || projection_bytes.len() > MAX_PROJECTION_JSON_BYTES
        {
            return Err(SyncStoreError::PayloadTooLarge);
        }
    }
    Ok(())
}

fn contains_forbidden_sensitive_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "access_token"
                    | "refresh_token"
                    | "authorization"
                    | "body_text"
                    | "raw_body"
                    | "file_content"
            ) || contains_forbidden_sensitive_field(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_forbidden_sensitive_field),
        _ => false,
    }
}

fn validate_subscription(subscription: &GoogleSubscription) -> Result<(), SyncStoreError> {
    let query = &subscription.query;
    if subscription.subscription_id.is_empty()
        || subscription.subscription_id.len() > 1_024
        || subscription.principal_id.0.is_empty()
        || subscription.event_kinds.is_empty()
        || subscription.event_kinds.len() > 32
        || subscription
            .event_kinds
            .iter()
            .any(|kind| kind.is_empty() || kind.len() > 128)
        || !matches!(subscription.state.as_str(), "active" | "paused" | "revoked")
        || subscription.created_at_ms < 0
        || subscription.updated_at_ms < subscription.created_at_ms
        || query
            .object_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 1_024)
        || query
            .telegram_conversation_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        || query
            .current_task_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        || query.source_after_ms.is_some_and(|value| value < 0)
        || query.source_before_ms.is_some_and(|value| value < 0)
        || matches!((query.source_after_ms, query.source_before_ms), (Some(after), Some(before)) if after > before)
    {
        Err(SyncStoreError::InvalidInput)
    } else {
        Ok(())
    }
}

fn decode_subscription(
    row: &rusqlite::Row<'_>,
    subscription_id: &str,
) -> rusqlite::Result<GoogleSubscription> {
    decode_subscription_offset(row, subscription_id, 0)
}

fn decode_subscription_offset(
    row: &rusqlite::Row<'_>,
    subscription_id: &str,
    offset: usize,
) -> rusqlite::Result<GoogleSubscription> {
    let account: String = row.get(offset + 1)?;
    let stream: String = row.get(offset + 2)?;
    let event_kinds: String = row.get(offset + 3)?;
    let query: String = row.get(offset + 4)?;
    Ok(GoogleSubscription {
        subscription_id: subscription_id.to_owned(),
        principal_id: PrincipalId(row.get(offset)?),
        account_id: ExternalIdentityId(uuid::Uuid::parse_str(&account).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                offset + 1,
                rusqlite::types::Type::Text,
                error.into(),
            )
        })?),
        stream: SyncStream::parse(&stream)?,
        event_kinds: serde_json::from_str(&event_kinds).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                offset + 3,
                rusqlite::types::Type::Text,
                error.into(),
            )
        })?,
        query: serde_json::from_str(&query).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                offset + 4,
                rusqlite::types::Type::Text,
                error.into(),
            )
        })?,
        cursor_generation: row.get(offset + 5)?,
        state: row.get(offset + 6)?,
        version: row.get(offset + 7)?,
        created_at_ms: row.get(offset + 8)?,
        updated_at_ms: row.get(offset + 9)?,
    })
}

fn subscription_matches(subscription: &GoogleSubscription, event: &ExternalEventEnvelope) -> bool {
    if !subscription
        .event_kinds
        .iter()
        .any(|kind| kind == event.event.kind())
    {
        return false;
    }
    let query = &subscription.query;
    if query
        .object_id
        .as_deref()
        .is_some_and(|id| id != event.object.object_id)
    {
        return false;
    }
    if query
        .source_after_ms
        .is_some_and(|after| event.source_timestamp_ms < after)
        || query
            .source_before_ms
            .is_some_and(|before| event.source_timestamp_ms > before)
    {
        return false;
    }
    if query.important_only {
        return matches!(
            &event.event,
            fabric::GoogleEvent::MailReceived(change) | fabric::GoogleEvent::MailUpdated(change)
                if change.message.important
        );
    }
    true
}

fn validate_cursor(token: Option<&str>) -> Result<(), SyncStoreError> {
    if token.is_some_and(|token| token.is_empty() || token.len() > MAX_CURSOR_BYTES) {
        Err(SyncStoreError::InvalidInput)
    } else {
        Ok(())
    }
}

fn decode_cursor(
    row: &rusqlite::Row<'_>,
    account_id: ExternalIdentityId,
    stream: SyncStream,
) -> rusqlite::Result<GoogleSyncCursor> {
    Ok(GoogleSyncCursor {
        account_id,
        stream,
        token: row.get(0)?,
        generation: row.get(1)?,
        last_success_ms: row.get(2)?,
        last_error_ms: row.get(3)?,
        retry_count: row.get(4)?,
        retry_after_ms: row.get(5)?,
        health_state: row.get(6)?,
        version: row.get(7)?,
    })
}

fn parse_event_id(value: String) -> rusqlite::Result<ExternalEventId> {
    uuid::Uuid::parse_str(&value)
        .map(ExternalEventId)
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
}

#[derive(Debug)]
pub enum SyncStoreError {
    Storage(String),
    CursorNotInitialized,
    CursorConflict {
        expected_version: u64,
        actual_version: u64,
    },
    InvalidInput,
    PayloadTooLarge,
    InjectedFailure(usize),
}

impl fmt::Display for SyncStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(_) => f.write_str("google sync storage failed"),
            Self::CursorNotInitialized => f.write_str("google sync cursor is not initialized"),
            Self::CursorConflict { .. } => f.write_str("google sync cursor conflict"),
            Self::InvalidInput => f.write_str("google sync input is invalid"),
            Self::PayloadTooLarge => f.write_str("google sync payload exceeds bounds"),
            Self::InjectedFailure(_) => f.write_str("google sync injected failure"),
        }
    }
}

impl std::error::Error for SyncStoreError {}

impl From<rusqlite::Error> for SyncStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<serde_json::Error> for SyncStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::external::ExternalIdentityRepository;
    use corpus::tools::google::oauth::GoogleBinding;
    use fabric::{
        ExternalEventDraft, ExternalObjectRef, ExternalScope, GmailMessageSummary, GoogleEvent,
        IdentityProvider, MailChange, PrincipalId, ProviderRecordRef,
    };

    struct Fixture {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
        account: ExternalIdentityId,
        store: GoogleSyncStore,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("objectives.db");
            let account = ExternalIdentityId::new();
            let repository = ExternalIdentityRepository::open(&path).unwrap();
            repository
                .bind_google(
                    &PrincipalId("owner".into()),
                    GoogleBinding {
                        identity_id: account,
                        provider_subject: "subject".into(),
                        email: "owner@example.com".into(),
                        scopes: vec![ExternalScope::GmailReadonly],
                    },
                    Some("work".into()),
                    1,
                )
                .unwrap();
            drop(repository);
            let store = GoogleSyncStore::open(&path).unwrap();
            store
                .initialize_cursor(account, SyncStream::GmailHistory, Some("h1"), 1)
                .unwrap();
            Self {
                _dir: dir,
                path,
                account,
                store,
            }
        }

        fn event(&self, version: &str, source_timestamp_ms: i64) -> ExternalEventEnvelope {
            let object = ExternalObjectRef {
                provider: IdentityProvider::Google,
                account_id: self.account,
                object_id: "message-1".into(),
                object_version: version.into(),
            };
            let provenance = ProviderRecordRef {
                account_id: self.account,
                provider_object_id: "message-1".into(),
                fetched_at_ms: source_timestamp_ms + 10,
                source_timestamp_ms,
                etag_or_history: Some(version.into()),
            };
            ExternalEventEnvelope::from_draft(ExternalEventDraft {
                provider: IdentityProvider::Google,
                account_id: self.account,
                provider_event_id: Some(format!("history-{version}")),
                object,
                observed_at_ms: source_timestamp_ms + 10,
                source_timestamp_ms,
                provenance: provenance.clone(),
                event: GoogleEvent::MailUpdated(MailChange {
                    message: GmailMessageSummary {
                        source: provenance,
                        thread_id: "thread".into(),
                        subject: "subject".into(),
                        from: "sender@example.com".into(),
                        snippet: "snippet".into(),
                        unread: true,
                        important: true,
                    },
                    content: None,
                }),
            })
            .unwrap()
        }

        fn commit(
            &self,
            event: ExternalEventEnvelope,
            input: &str,
            output: &str,
            version: u64,
        ) -> SyncCommit {
            SyncCommit {
                account_id: self.account,
                stream: SyncStream::GmailHistory,
                expected_cursor_token: Some(input.into()),
                expected_cursor_version: version,
                successor_cursor_token: output.into(),
                cursor_generation: 1,
                events: vec![(
                    event,
                    ProjectionWrite {
                        json: serde_json::json!({"subject":"bounded"}),
                        tombstone: false,
                    },
                )],
                committed_at_ms: 100,
            }
        }
    }

    #[test]
    fn commit_is_atomic_and_restart_durable() {
        let fixture = Fixture::new();
        let event = fixture.event("v1", 50);
        let result = fixture
            .store
            .commit(fixture.commit(event.clone(), "h1", "h2", 0))
            .unwrap();
        assert!(result.events[0].inserted);
        assert_eq!(result.cursor.token.as_deref(), Some("h2"));
        assert_eq!(fixture.store.pending_outbox_count().unwrap(), 1);
        drop(fixture.store);
        let restarted = GoogleSyncStore::open(&fixture.path).unwrap();
        assert_eq!(
            restarted
                .cursor(fixture.account, SyncStream::GmailHistory)
                .unwrap()
                .unwrap()
                .token
                .as_deref(),
            Some("h2")
        );
        assert_eq!(restarted.event(event.id).unwrap(), Some(event));
    }

    #[test]
    fn every_statement_failure_rolls_back_event_projection_outbox_and_cursor() {
        for failpoint in 0..=3 {
            let fixture = Fixture::new();
            fixture.store.fail_after(failpoint);
            let result =
                fixture
                    .store
                    .commit(fixture.commit(fixture.event("v1", 50), "h1", "h2", 0));
            assert!(matches!(result, Err(SyncStoreError::InjectedFailure(_))));
            assert_eq!(
                fixture
                    .store
                    .cursor(fixture.account, SyncStream::GmailHistory)
                    .unwrap()
                    .unwrap()
                    .token
                    .as_deref(),
                Some("h1")
            );
            assert_eq!(fixture.store.pending_outbox_count().unwrap(), 0);
            assert!(fixture
                .store
                .projection(fixture.account, SyncStream::GmailHistory, "message-1")
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn duplicate_delivery_returns_existing_event_and_advances_proven_successor() {
        let fixture = Fixture::new();
        let event = fixture.event("v1", 50);
        fixture
            .store
            .commit(fixture.commit(event.clone(), "h1", "h2", 0))
            .unwrap();
        let duplicate = fixture
            .store
            .commit(fixture.commit(event.clone(), "h2", "h3", 1))
            .unwrap();
        assert!(!duplicate.events[0].inserted);
        assert_eq!(duplicate.events[0].event_id, event.id);
        assert_eq!(duplicate.cursor.token.as_deref(), Some("h3"));
        assert_eq!(fixture.store.pending_outbox_count().unwrap(), 1);
    }

    #[test]
    fn cursor_cas_rejects_concurrent_poller_and_preserves_winner() {
        let fixture = Fixture::new();
        let competing_poller = GoogleSyncStore::open(&fixture.path).unwrap();
        let first = fixture
            .store
            .commit(fixture.commit(fixture.event("v1", 50), "h1", "h2", 0))
            .unwrap();
        assert_eq!(first.cursor.version, 1);
        let loser =
            competing_poller.commit(fixture.commit(fixture.event("v2", 60), "h1", "h-lost", 0));
        assert!(matches!(loser, Err(SyncStoreError::CursorConflict { .. })));
        assert_eq!(
            fixture
                .store
                .cursor(fixture.account, SyncStream::GmailHistory)
                .unwrap()
                .unwrap()
                .token
                .as_deref(),
            Some("h2")
        );
    }

    #[test]
    fn out_of_order_event_is_retained_without_rewriting_newer_projection() {
        let fixture = Fixture::new();
        fixture
            .store
            .commit(fixture.commit(fixture.event("v2", 80), "h1", "h2", 0))
            .unwrap();
        fixture
            .store
            .commit(fixture.commit(fixture.event("v1", 40), "h2", "h3", 1))
            .unwrap();
        let projection = fixture
            .store
            .projection(fixture.account, SyncStream::GmailHistory, "message-1")
            .unwrap()
            .unwrap();
        assert_eq!(projection.2, 80);
        assert_eq!(fixture.store.pending_outbox_count().unwrap(), 2);
    }

    #[test]
    fn tombstone_replaces_current_projection_and_is_durable() {
        let fixture = Fixture::new();
        fixture
            .store
            .commit(fixture.commit(fixture.event("v1", 50), "h1", "h2", 0))
            .unwrap();
        let mut deletion = fixture.event("v2", 80);
        deletion.event = GoogleEvent::MailDeleted(deletion.object.clone());
        deletion = ExternalEventEnvelope::from_draft(ExternalEventDraft {
            provider: deletion.provider,
            account_id: deletion.account_id,
            provider_event_id: deletion.provider_event_id,
            object: deletion.object,
            observed_at_ms: deletion.observed_at_ms,
            source_timestamp_ms: deletion.source_timestamp_ms,
            provenance: deletion.provenance,
            event: deletion.event,
        })
        .unwrap();
        let mut commit = fixture.commit(deletion, "h2", "h3", 1);
        commit.events[0].1.tombstone = true;
        commit.events[0].1.json = serde_json::json!({"deleted":true});
        fixture.store.commit(commit).unwrap();
        assert!(
            fixture
                .store
                .projection(fixture.account, SyncStream::GmailHistory, "message-1")
                .unwrap()
                .unwrap()
                .1
        );
    }

    #[test]
    fn raw_content_and_credentials_are_rejected_from_external_projection() {
        let fixture = Fixture::new();
        for forbidden in ["body_text", "file_content", "access_token", "Authorization"] {
            let mut commit = fixture.commit(fixture.event("v1", 50), "h1", "h2", 0);
            commit.events[0].1.json = serde_json::json!({(forbidden):"secret"});
            assert!(matches!(
                fixture.store.commit(commit),
                Err(SyncStoreError::InvalidInput)
            ));
        }
    }
}
