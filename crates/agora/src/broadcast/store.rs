use fabric::dasein::SelfVersion;
use fabric::{
    AgoraSpaceId, BroadcastAck, BroadcastEpoch, SelectionResult, WallTime, WorkspaceBroadcast,
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
