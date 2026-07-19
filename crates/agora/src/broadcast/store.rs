use fabric::dasein::SelfVersion;
use fabric::{
    AgoraSpaceId, BroadcastAck, BroadcastEpoch, BroadcastIntegrationReceipt,
    ConsciousContextProjection, ConsciousTraceEvent, ProcessorContext, ProcessorResponse,
    SelectionResult, WallTime, WorkspaceBroadcast,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct BroadcastReplay {
    pub broadcast: WorkspaceBroadcast,
    pub opened_at: WallTime,
    pub closed_at: Option<WallTime>,
    pub acknowledgements: Vec<BroadcastAck>,
}

pub struct SqliteBroadcastStore {
    connection: Mutex<Connection>,
}

impl SqliteBroadcastStore {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS broadcast_epochs (
                space TEXT NOT NULL,
                epoch INTEGER NOT NULL,
                selection_key TEXT NOT NULL,
                workspace_version INTEGER NOT NULL,
                broadcast_json BLOB NOT NULL,
                checksum TEXT NOT NULL,
                opened_at INTEGER NOT NULL,
                closed_at INTEGER,
                PRIMARY KEY(space, epoch),
                UNIQUE(space, selection_key)
             );
             CREATE TABLE IF NOT EXISTS broadcast_acks (
                space TEXT NOT NULL,
                epoch INTEGER NOT NULL,
                processor TEXT NOT NULL,
                ack_json BLOB NOT NULL,
                checksum TEXT NOT NULL,
                PRIMARY KEY(space, epoch, processor),
                FOREIGN KEY(space, epoch) REFERENCES broadcast_epochs(space, epoch)
             );
             CREATE TABLE IF NOT EXISTS conscious_integrations (
                space TEXT NOT NULL,
                epoch INTEGER NOT NULL,
                receipt_json BLOB NOT NULL,
                checksum TEXT NOT NULL,
                PRIMARY KEY(space, epoch),
                FOREIGN KEY(space, epoch) REFERENCES broadcast_epochs(space, epoch)
             );
             CREATE TABLE IF NOT EXISTS conscious_processor_responses (
                space TEXT NOT NULL,
                epoch INTEGER NOT NULL,
                processor TEXT NOT NULL,
                response_json BLOB NOT NULL,
                checksum TEXT NOT NULL,
                PRIMARY KEY(space, epoch, processor),
                FOREIGN KEY(space, epoch) REFERENCES conscious_integrations(space, epoch)
             );
             CREATE TABLE IF NOT EXISTS conscious_context_projections (
                space TEXT NOT NULL,
                epoch INTEGER NOT NULL,
                projection_json BLOB NOT NULL,
                checksum TEXT NOT NULL,
                PRIMARY KEY(space, epoch),
                FOREIGN KEY(space, epoch) REFERENCES conscious_integrations(space, epoch)
             );
             CREATE TABLE IF NOT EXISTS conscious_field_modulations (
                space TEXT NOT NULL,
                operation_id TEXT NOT NULL,
                call_id TEXT NOT NULL,
                event_json BLOB NOT NULL,
                checksum TEXT NOT NULL,
                PRIMARY KEY(space, operation_id, call_id)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::open(":memory:")
    }

    pub fn open_selection(
        &self,
        selection: SelectionResult,
        dasein_version: SelfVersion,
        workspace_version: u64,
        opened_at: WallTime,
    ) -> anyhow::Result<WorkspaceBroadcast> {
        let space = selection
            .selected
            .first()
            .ok_or_else(|| anyhow::anyhow!("cannot open an empty selection"))?
            .space
            .clone();
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let transaction = connection.transaction()?;
        let selection_key = selection_key(&selection, dasein_version, workspace_version)?;
        let existing: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT broadcast_json FROM broadcast_epochs
                 WHERE space = ?1 AND selection_key = ?2",
                params![space.0, selection_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(json) = existing {
            let broadcast: WorkspaceBroadcast = serde_json::from_slice(&json)?;
            broadcast.validate()?;
            transaction.commit()?;
            return Ok(broadcast);
        }
        let last: Option<u64> = transaction
            .query_row(
                "SELECT MAX(epoch) FROM broadcast_epochs WHERE space = ?1",
                params![space.0],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let epoch =
            BroadcastEpoch(last.unwrap_or(0).checked_add(1).ok_or_else(|| {
                anyhow::anyhow!("broadcast epoch exhausted for space {}", space.0)
            })?);
        let broadcast = WorkspaceBroadcast::from_selection(
            epoch,
            selection,
            dasein_version,
            workspace_version,
        )?;
        insert_epoch(&transaction, &broadcast, &selection_key, opened_at)?;
        transaction.commit()?;
        Ok(broadcast)
    }

    pub fn append_ack(&self, ack: &BroadcastAck) -> anyhow::Result<()> {
        ack.validate()?;
        let json = serde_json::to_vec(ack)?;
        let checksum = checksum(&json);
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let closed: Option<Option<i64>> = connection
            .query_row(
                "SELECT closed_at FROM broadcast_epochs WHERE space = ?1 AND epoch = ?2",
                params![ack.space.0, ack.epoch.0],
                |row| row.get(0),
            )
            .optional()?;
        let closed =
            closed.ok_or_else(|| anyhow::anyhow!("acknowledgement epoch does not exist"))?;
        anyhow::ensure!(closed.is_none(), "broadcast epoch is already closed");
        let processor = ack.processor.0.to_string();
        let existing: Option<(Vec<u8>, String)> = connection
            .query_row(
                "SELECT ack_json, checksum FROM broadcast_acks
                 WHERE space = ?1 AND epoch = ?2 AND processor = ?3",
                params![ack.space.0, ack.epoch.0, processor],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((existing_json, existing_checksum)) = existing {
            anyhow::ensure!(
                existing_json == json && existing_checksum == checksum,
                "conflicting acknowledgement for processor"
            );
            return Ok(());
        }
        connection.execute(
            "INSERT INTO broadcast_acks(space, epoch, processor, ack_json, checksum)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ack.space.0, ack.epoch.0, processor, json, checksum],
        )?;
        Ok(())
    }

    pub fn close_epoch(
        &self,
        space: &AgoraSpaceId,
        epoch: BroadcastEpoch,
        closed_at: WallTime,
    ) -> anyhow::Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let current: Option<Option<i64>> = connection
            .query_row(
                "SELECT closed_at FROM broadcast_epochs WHERE space = ?1 AND epoch = ?2",
                params![space.0, epoch.0],
                |row| row.get(0),
            )
            .optional()?;
        match current {
            None => anyhow::bail!("broadcast epoch does not exist"),
            Some(Some(existing)) => anyhow::ensure!(
                existing == closed_at.0,
                "broadcast epoch has a conflicting close time"
            ),
            Some(None) => {
                let opened: i64 = connection.query_row(
                    "SELECT opened_at FROM broadcast_epochs WHERE space = ?1 AND epoch = ?2",
                    params![space.0, epoch.0],
                    |row| row.get(0),
                )?;
                anyhow::ensure!(closed_at.0 >= opened, "broadcast closes before it opens");
                connection.execute(
                    "UPDATE broadcast_epochs SET closed_at = ?3 WHERE space = ?1 AND epoch = ?2",
                    params![space.0, epoch.0, closed_at.0],
                )?;
            }
        }
        Ok(())
    }

    pub fn append_integration(&self, receipt: &BroadcastIntegrationReceipt) -> anyhow::Result<()> {
        receipt.validate()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let stored: Option<(Vec<u8>, String, Option<i64>)> = connection
            .query_row(
                "SELECT broadcast_json, checksum, closed_at FROM broadcast_epochs
                 WHERE space = ?1 AND epoch = ?2",
                params![receipt.space.0, receipt.epoch.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (broadcast_json, broadcast_checksum, closed_at) =
            stored.ok_or_else(|| anyhow::anyhow!("integration broadcast does not exist"))?;
        anyhow::ensure!(closed_at.is_some(), "integration broadcast is not closed");
        anyhow::ensure!(
            broadcast_checksum == receipt.broadcast_checksum,
            "integration checksum differs from durable broadcast"
        );
        let broadcast: WorkspaceBroadcast = serde_json::from_slice(&broadcast_json)?;
        anyhow::ensure!(
            receipt.transition.previous_version == broadcast.dasein_version,
            "integration starts from the wrong Dasein version"
        );
        insert_idempotent_blob(
            &connection,
            "conscious_integrations",
            "receipt_json",
            &receipt.space,
            receipt.epoch,
            None,
            serde_json::to_vec(receipt)?,
        )
    }

    pub fn integration(
        &self,
        space: &AgoraSpaceId,
        epoch: BroadcastEpoch,
    ) -> anyhow::Result<Option<BroadcastIntegrationReceipt>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let json: Option<Vec<u8>> = connection
            .query_row(
                "SELECT receipt_json FROM conscious_integrations WHERE space = ?1 AND epoch = ?2",
                params![space.0, epoch.0],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| {
            let receipt: BroadcastIntegrationReceipt = serde_json::from_slice(&json)?;
            receipt.validate()?;
            Ok(receipt)
        })
        .transpose()
    }

    pub fn append_processor_response(
        &self,
        context: &ProcessorContext,
        response: &ProcessorResponse,
    ) -> anyhow::Result<()> {
        response.validate(context)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        insert_idempotent_blob(
            &connection,
            "conscious_processor_responses",
            "response_json",
            &context.space,
            context.source_epoch,
            Some(&response.processor.0),
            serde_json::to_vec(response)?,
        )
    }

    pub fn processor_responses(
        &self,
        space: &AgoraSpaceId,
        epoch: BroadcastEpoch,
    ) -> anyhow::Result<Vec<ProcessorResponse>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let mut statement = connection.prepare(
            "SELECT response_json, checksum FROM conscious_processor_responses
             WHERE space = ?1 AND epoch = ?2 ORDER BY processor",
        )?;
        let rows = statement.query_map(params![space.0, epoch.0], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut responses = Vec::new();
        for row in rows {
            let (json, stored_checksum) = row?;
            anyhow::ensure!(
                checksum(&json) == stored_checksum,
                "response checksum mismatch"
            );
            let response: ProcessorResponse = serde_json::from_slice(&json)?;
            response.validate_persisted(space, epoch)?;
            responses.push(response);
        }
        Ok(responses)
    }

    pub fn save_context_projection(
        &self,
        projection: &ConsciousContextProjection,
    ) -> anyhow::Result<()> {
        projection.validate()?;
        let epoch = projection
            .receipt
            .broadcast_epoch
            .ok_or_else(|| anyhow::anyhow!("only broadcast-backed projections are durable"))?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        insert_idempotent_blob(
            &connection,
            "conscious_context_projections",
            "projection_json",
            &projection.receipt.space,
            epoch,
            None,
            serde_json::to_vec(projection)?,
        )
    }

    pub fn latest_context_projection(
        &self,
        space: &AgoraSpaceId,
    ) -> anyhow::Result<Option<ConsciousContextProjection>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let stored: Option<(Vec<u8>, String)> = connection
            .query_row(
                "SELECT projection_json, checksum FROM conscious_context_projections
                 WHERE space = ?1 ORDER BY epoch DESC LIMIT 1",
                params![space.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        stored
            .map(|(json, stored_checksum)| {
                anyhow::ensure!(
                    checksum(&json) == stored_checksum,
                    "context projection checksum mismatch"
                );
                let projection: ConsciousContextProjection = serde_json::from_slice(&json)?;
                projection.validate()?;
                Ok(projection)
            })
            .transpose()
    }

    /// Persist one bounded, pre-execution field-modulation event.
    ///
    /// Operation and call identity make retries idempotent. A retry that changes
    /// the evidence is rejected rather than silently replacing audit history.
    pub fn save_field_modulation(
        &self,
        space: &AgoraSpaceId,
        event: &ConsciousTraceEvent,
    ) -> anyhow::Result<()> {
        let ConsciousTraceEvent::FieldModulation {
            operation_id,
            call_id,
            metric_ref,
            ..
        } = event
        else {
            anyhow::bail!("only field-modulation trace events are durable here");
        };
        for (value, label) in [
            (operation_id.as_str(), "modulation operation ID"),
            (call_id.as_str(), "modulation call ID"),
            (metric_ref.as_str(), "modulation metric reference"),
        ] {
            anyhow::ensure!(
                !value.trim().is_empty() && value.len() <= 4096,
                "{label} is empty or exceeds 4096 bytes"
            );
        }
        let json = serde_json::to_vec(event)?;
        let digest = checksum(&json);
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        connection.execute(
            "INSERT INTO conscious_field_modulations
             (space, operation_id, call_id, event_json, checksum)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(space, operation_id, call_id) DO NOTHING",
            params![space.0, operation_id, call_id, json, digest],
        )?;
        let stored: (Vec<u8>, String) = connection.query_row(
            "SELECT event_json, checksum FROM conscious_field_modulations
             WHERE space = ?1 AND operation_id = ?2 AND call_id = ?3",
            params![space.0, operation_id, call_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        anyhow::ensure!(
            checksum(&stored.0) == stored.1 && stored.0 == serde_json::to_vec(event)?,
            "field-modulation evidence changed for an existing call"
        );
        Ok(())
    }

    /// Read checksum-verified modulation evidence in insertion order.
    pub fn field_modulations(
        &self,
        space: &AgoraSpaceId,
    ) -> anyhow::Result<Vec<ConsciousTraceEvent>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let mut statement = connection.prepare(
            "SELECT event_json, checksum FROM conscious_field_modulations
             WHERE space = ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![space.0], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (json, stored_checksum) = row?;
            anyhow::ensure!(
                checksum(&json) == stored_checksum,
                "field-modulation checksum mismatch"
            );
            let event: ConsciousTraceEvent = serde_json::from_slice(&json)?;
            anyhow::ensure!(
                matches!(event, ConsciousTraceEvent::FieldModulation { .. }),
                "stored trace event is not a field modulation"
            );
            events.push(event);
        }
        Ok(events)
    }

    pub fn replay(&self, space: &AgoraSpaceId) -> anyhow::Result<Vec<BroadcastReplay>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("broadcast store lock poisoned"))?;
        let mut epochs = connection.prepare(
            "SELECT epoch, broadcast_json, checksum, opened_at, closed_at
             FROM broadcast_epochs WHERE space = ?1 ORDER BY epoch",
        )?;
        let rows = epochs.query_map(params![space.0], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })?;
        let mut replay = Vec::new();
        for row in rows {
            let (epoch, json, stored_checksum, opened_at, closed_at) = row?;
            anyhow::ensure!(
                epoch == replay.len() as u64 + 1,
                "broadcast epoch gap detected"
            );
            anyhow::ensure!(
                checksum(&json) == stored_checksum,
                "broadcast checksum mismatch"
            );
            let broadcast: WorkspaceBroadcast = serde_json::from_slice(&json)?;
            broadcast.validate()?;
            anyhow::ensure!(
                broadcast.space == *space,
                "broadcast stored under wrong space"
            );
            anyhow::ensure!(broadcast.epoch.0 == epoch, "broadcast epoch key mismatch");
            let mut ack_statement = connection.prepare(
                "SELECT ack_json, checksum FROM broadcast_acks
                 WHERE space = ?1 AND epoch = ?2 ORDER BY processor",
            )?;
            let ack_rows = ack_statement.query_map(params![space.0, epoch], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut acknowledgements = Vec::new();
            for ack_row in ack_rows {
                let (ack_json, ack_checksum) = ack_row?;
                anyhow::ensure!(checksum(&ack_json) == ack_checksum, "ack checksum mismatch");
                let ack: BroadcastAck = serde_json::from_slice(&ack_json)?;
                ack.validate()?;
                anyhow::ensure!(
                    ack.space == *space && ack.epoch.0 == epoch,
                    "ack key mismatch"
                );
                acknowledgements.push(ack);
            }
            replay.push(BroadcastReplay {
                broadcast,
                opened_at: WallTime(opened_at),
                closed_at: closed_at.map(WallTime),
                acknowledgements,
            });
        }
        Ok(replay)
    }

    pub fn replay_epoch(
        &self,
        space: &AgoraSpaceId,
        epoch: BroadcastEpoch,
    ) -> anyhow::Result<BroadcastReplay> {
        self.replay(space)?
            .into_iter()
            .find(|value| value.broadcast.epoch == epoch)
            .ok_or_else(|| anyhow::anyhow!("broadcast epoch does not exist"))
    }

    #[doc(hidden)]
    pub fn connection_for_test(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection.lock().expect("broadcast store lock")
    }
}

fn insert_epoch(
    transaction: &Transaction<'_>,
    broadcast: &WorkspaceBroadcast,
    selection_key: &str,
    opened_at: WallTime,
) -> anyhow::Result<()> {
    broadcast.validate()?;
    let json = serde_json::to_vec(broadcast)?;
    transaction.execute(
        "INSERT INTO broadcast_epochs(
            space, epoch, selection_key, workspace_version, broadcast_json, checksum, opened_at, closed_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![
            broadcast.space.0,
            broadcast.epoch.0,
            selection_key,
            broadcast.workspace_version,
            json,
            checksum(&json),
            opened_at.0
        ],
    )?;
    Ok(())
}

fn selection_key(
    selection: &SelectionResult,
    dasein_version: SelfVersion,
    workspace_version: u64,
) -> anyhow::Result<String> {
    Ok(checksum(&serde_json::to_vec(&(
        selection,
        dasein_version,
        workspace_version,
    ))?))
}

fn checksum(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn insert_idempotent_blob(
    connection: &Connection,
    table: &str,
    value_column: &str,
    space: &AgoraSpaceId,
    epoch: BroadcastEpoch,
    processor: Option<&str>,
    json: Vec<u8>,
) -> anyhow::Result<()> {
    let digest = checksum(&json);
    let (select, insert) = match processor {
        Some(_) => (
            format!(
                "SELECT {value_column}, checksum FROM {table} WHERE space = ?1 AND epoch = ?2 AND processor = ?3"
            ),
            format!(
                "INSERT INTO {table}(space, epoch, processor, {value_column}, checksum) VALUES (?1, ?2, ?3, ?4, ?5)"
            ),
        ),
        None => (
            format!(
                "SELECT {value_column}, checksum FROM {table} WHERE space = ?1 AND epoch = ?2"
            ),
            format!(
                "INSERT INTO {table}(space, epoch, {value_column}, checksum) VALUES (?1, ?2, ?3, ?4)"
            ),
        ),
    };
    let existing: Option<(Vec<u8>, String)> = match processor {
        Some(processor) => connection
            .query_row(&select, params![space.0, epoch.0, processor], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?,
        None => connection
            .query_row(&select, params![space.0, epoch.0], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?,
    };
    if let Some((existing_json, existing_checksum)) = existing {
        anyhow::ensure!(
            existing_json == json && existing_checksum == digest,
            "conflicting recurrent edge"
        );
        return Ok(());
    }
    match processor {
        Some(processor) => {
            connection.execute(&insert, params![space.0, epoch.0, processor, json, digest])?;
        }
        None => {
            connection.execute(&insert, params![space.0, epoch.0, json, digest])?;
        }
    }
    Ok(())
}
