use crate::core::store::SelfFieldStore;
use fabric::dasein::{
    SelfEventV1, SelfTransitionRequest, SelfVersion, SELF_EVENT_SCHEMA_V1, SELF_REDUCER_V1,
};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone)]
pub struct SelfLedger {
    store: Arc<SelfFieldStore>,
}

#[derive(Serialize)]
struct EventChecksumMaterial<'a> {
    schema_version: u16,
    reducer_version: u16,
    sequence: u64,
    request: &'a SelfTransitionRequest,
    previous_version: SelfVersion,
    current_version: SelfVersion,
    previous_checksum: &'a str,
}

impl SelfLedger {
    pub fn new(store: Arc<SelfFieldStore>) -> Self {
        Self { store }
    }

    pub fn append(&self, request: &SelfTransitionRequest) -> anyhow::Result<SelfEventV1> {
        let mut conn = self.store.conn();
        let tx = conn.transaction()?;

        if let Some(existing_json) = tx
            .query_row(
                "SELECT request_json FROM self_events WHERE event_id = ?1",
                params![request.event_id.0.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let existing_request: SelfTransitionRequest = serde_json::from_str(&existing_json)?;
            anyhow::ensure!(
                existing_request == *request,
                "self ledger event ID collision with different payload"
            );
            drop(tx);
            drop(conn);
            return self
                .load_verified()?
                .into_iter()
                .find(|event| event.request.event_id == request.event_id)
                .ok_or_else(|| anyhow::anyhow!("existing self event disappeared"));
        }

        let latest = tx
            .query_row(
                "SELECT seq, next_version, checksum FROM self_events ORDER BY seq DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let (sequence, durable_version, previous_checksum) = match latest {
            Some((seq, version, checksum)) => (seq + 1, SelfVersion(version), checksum),
            None => (1, SelfVersion(0), String::new()),
        };
        anyhow::ensure!(
            request.expected_version == durable_version,
            "self ledger version conflict: expected {}, durable {}",
            request.expected_version.0,
            durable_version.0
        );
        let current_version = SelfVersion(durable_version.0 + 1);
        let checksum = event_checksum(
            sequence,
            request,
            durable_version,
            current_version,
            &previous_checksum,
        )?;
        let request_json = serde_json::to_string(request)?;
        tx.execute(
            "INSERT INTO self_events
             (seq, event_id, previous_version, next_version, request_json,
              previous_checksum, checksum, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                sequence,
                request.event_id.0.to_string(),
                durable_version.0,
                current_version.0,
                request_json,
                previous_checksum,
                checksum,
                request.observed_at.0,
            ],
        )?;
        tx.commit()?;

        Ok(SelfEventV1 {
            schema_version: SELF_EVENT_SCHEMA_V1,
            reducer_version: SELF_REDUCER_V1,
            sequence,
            request: request.clone(),
            previous_version: durable_version,
            current_version,
            previous_checksum,
            checksum,
        })
    }

    pub fn load_verified(&self) -> anyhow::Result<Vec<SelfEventV1>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT seq, previous_version, next_version, request_json,
                    previous_checksum, checksum
             FROM self_events ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let mut events = Vec::new();
        let mut expected_checksum = String::new();
        for row in rows {
            let (sequence, previous, current, request_json, previous_checksum, checksum) = row?;
            let request: SelfTransitionRequest = serde_json::from_str(&request_json)?;
            let event = SelfEventV1 {
                schema_version: SELF_EVENT_SCHEMA_V1,
                reducer_version: SELF_REDUCER_V1,
                sequence,
                request,
                previous_version: SelfVersion(previous),
                current_version: SelfVersion(current),
                previous_checksum,
                checksum,
            };
            event.validate_versions()?;
            let expected_sequence = events.len() as u64 + 1;
            anyhow::ensure!(
                event.sequence == expected_sequence,
                "self ledger sequence gap at {}",
                event.sequence
            );
            anyhow::ensure!(
                event.previous_version.0 + 1 == event.current_version.0
                    && event.previous_version.0 == expected_sequence - 1,
                "self ledger version gap at sequence {}",
                event.sequence
            );
            anyhow::ensure!(
                event.previous_checksum == expected_checksum,
                "self ledger checksum chain break at sequence {}",
                event.sequence
            );
            let calculated = event_checksum(
                event.sequence,
                &event.request,
                event.previous_version,
                event.current_version,
                &event.previous_checksum,
            )?;
            anyhow::ensure!(
                event.checksum == calculated,
                "self ledger checksum mismatch at sequence {}",
                event.sequence
            );
            expected_checksum = event.checksum.clone();
            events.push(event);
        }
        Ok(events)
    }

    pub fn save_checkpoint(
        &self,
        events: &[SelfEventV1],
        created_at_ms: i64,
    ) -> anyhow::Result<()> {
        let Some(last) = events.last() else {
            return Ok(());
        };
        let event_prefix_json = serde_json::to_string(events)?;
        let checksum = bytes_checksum(event_prefix_json.as_bytes());
        self.store.conn().execute(
            "INSERT OR REPLACE INTO self_snapshots
             (version, last_event_seq, event_prefix_json, checksum, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                last.current_version.0,
                last.sequence,
                event_prefix_json,
                checksum,
                created_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn load_replay_plan(&self) -> anyhow::Result<Vec<SelfEventV1>> {
        let events = self.load_verified()?;
        let conn = self.store.conn();
        let checkpoint = conn
            .query_row(
                "SELECT last_event_seq, event_prefix_json, checksum
                 FROM self_snapshots ORDER BY version DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((last_seq, prefix_json, checksum)) = checkpoint {
            anyhow::ensure!(
                bytes_checksum(prefix_json.as_bytes()) == checksum,
                "self snapshot checksum mismatch"
            );
            let prefix: Vec<SelfEventV1> = serde_json::from_str(&prefix_json)?;
            anyhow::ensure!(
                prefix.len() as u64 == last_seq && events.starts_with(&prefix),
                "self snapshot does not match verified ledger prefix"
            );
        }
        Ok(events)
    }

    pub fn store(&self) -> &SelfFieldStore {
        &self.store
    }
}

fn event_checksum(
    sequence: u64,
    request: &SelfTransitionRequest,
    previous_version: SelfVersion,
    current_version: SelfVersion,
    previous_checksum: &str,
) -> anyhow::Result<String> {
    let material = EventChecksumMaterial {
        schema_version: SELF_EVENT_SCHEMA_V1,
        reducer_version: SELF_REDUCER_V1,
        sequence,
        request,
        previous_version,
        current_version,
        previous_checksum,
    };
    Ok(bytes_checksum(&serde_json::to_vec(&material)?))
}

fn bytes_checksum(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
