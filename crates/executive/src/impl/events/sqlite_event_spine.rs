use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use fabric::{
    EventPosition, EventSpine, EventVisibility, ParentEventId, SchemaId, SpineEvent, TreeSequence,
    UnsequencedEvent,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventAppendMetrics {
    pub accepted: u64,
    pub rejected: u64,
    pub backpressure_rejections: u64,
}

#[derive(Debug, Clone, Default)]
pub struct EventReadFilter {
    pub from_sequence: Option<TreeSequence>,
    pub through_sequence: Option<TreeSequence>,
    pub schema: Option<SchemaId>,
    pub visibility: Option<EventVisibility>,
    pub limit: usize,
}

pub struct SqliteEventSpine {
    connection: parking_lot::Mutex<Connection>,
    accepted: AtomicU64,
    rejected: AtomicU64,
    backpressure_rejections: AtomicU64,
}

pub fn default_event_spine_path() -> std::path::PathBuf {
    fabric::paths::xdg_data_dir().join("event-spine-v1.db")
}

impl SqliteEventSpine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).context("open event spine")?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS event_trees(
               tree_id TEXT PRIMARY KEY,
               next_sequence INTEGER NOT NULL CHECK(next_sequence > 0)
             );
             CREATE TABLE IF NOT EXISTS spine_events(
               event_id TEXT PRIMARY KEY,
               tree_id TEXT NOT NULL,
               sequence INTEGER NOT NULL,
               parent_event_id TEXT,
               schema_id TEXT NOT NULL,
               visibility TEXT NOT NULL,
               input_json TEXT NOT NULL,
               event_json TEXT NOT NULL,
               UNIQUE(tree_id, sequence),
               FOREIGN KEY(tree_id) REFERENCES event_trees(tree_id)
             );
             CREATE INDEX IF NOT EXISTS spine_events_tree_read
               ON spine_events(tree_id, sequence, schema_id, visibility);",
        )?;
        Ok(Self {
            connection: parking_lot::Mutex::new(connection),
            accepted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            backpressure_rejections: AtomicU64::new(0),
        })
    }

    pub fn metrics(&self) -> EventAppendMetrics {
        EventAppendMetrics {
            accepted: self.accepted.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            backpressure_rejections: self.backpressure_rejections.load(Ordering::Relaxed),
        }
    }

    pub fn read_tree(
        &self,
        tree_id: fabric::EventTreeId,
        filter: EventReadFilter,
    ) -> Result<Vec<SpineEvent>> {
        let limit = filter.limit.clamp(1, 10_000);
        let from = filter.from_sequence.map_or(1, |value| value.0);
        let through = filter
            .through_sequence
            .map_or(i64::MAX as u64, |value| value.0.min(i64::MAX as u64));
        let schema = filter.schema.map(|value| value.0);
        let visibility = filter.visibility.map(visibility_name);
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT event_json FROM spine_events
             WHERE tree_id=?1 AND sequence>=?2 AND sequence<=?3
               AND (?4 IS NULL OR schema_id=?4)
               AND (?5 IS NULL OR visibility=?5)
             ORDER BY sequence ASC LIMIT ?6",
        )?;
        let rows: Vec<String> = statement
            .query_map(
                params![
                    tree_id.to_string(),
                    from,
                    through,
                    schema,
                    visibility,
                    limit
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|json| serde_json::from_str(&json).context("decode persisted spine event"))
            .collect()
    }
}

impl EventSpine for SqliteEventSpine {
    fn append(&self, event: UnsequencedEvent) -> Result<SpineEvent> {
        let result = self.append_inner(event);
        match &result {
            Ok(_) => self.accepted.fetch_add(1, Ordering::Relaxed),
            Err(_) => self.rejected.fetch_add(1, Ordering::Relaxed),
        };
        result
    }
}

impl SqliteEventSpine {
    fn append_inner(&self, event: UnsequencedEvent) -> Result<SpineEvent> {
        event.validate()?;
        let input_json = serde_json::to_string(&event)?;
        let Some(mut connection) = self.connection.try_lock_for(Duration::from_secs(1)) else {
            self.backpressure_rejections.fetch_add(1, Ordering::Relaxed);
            bail!("event spine overloaded: append admission timed out");
        };
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some((existing_input, existing_event)) = transaction
            .query_row(
                "SELECT input_json,event_json FROM spine_events WHERE event_id=?1",
                params![event.event_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if existing_input != input_json {
                bail!("event id retry conflicts with persisted content");
            }
            return serde_json::from_str(&existing_event).context("decode idempotent spine event");
        }

        transaction.execute(
            "INSERT OR IGNORE INTO event_trees(tree_id,next_sequence) VALUES(?1,1)",
            params![event.tree_id.to_string()],
        )?;
        if let Some(ParentEventId(parent)) = event.parent {
            let parent_tree: Option<String> = transaction
                .query_row(
                    "SELECT tree_id FROM spine_events WHERE event_id=?1",
                    params![parent.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            match parent_tree {
                None => bail!("causal parent does not exist"),
                Some(parent_tree) if parent_tree != event.tree_id.to_string() => {
                    bail!("causal parent belongs to another event tree")
                }
                Some(_) => {}
            }
        }
        let sequence: u64 = transaction.query_row(
            "SELECT next_sequence FROM event_trees WHERE tree_id=?1",
            params![event.tree_id.to_string()],
            |row| row.get(0),
        )?;
        let persisted = SpineEvent {
            position: EventPosition {
                tree_id: event.tree_id,
                event_id: event.event_id,
                parent: event.parent,
                sequence: TreeSequence(sequence),
            },
            identity: event.identity,
            schema: event.envelope.schema.clone(),
            visibility: event.visibility,
            envelope: event.envelope,
            payload: event.payload,
        };
        let event_json = serde_json::to_string(&persisted)?;
        transaction.execute(
            "INSERT INTO spine_events(event_id,tree_id,sequence,parent_event_id,schema_id,visibility,input_json,event_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                persisted.position.event_id.to_string(),
                persisted.position.tree_id.to_string(),
                sequence,
                persisted.position.parent.map(|parent| parent.0.to_string()),
                persisted.schema.0,
                visibility_name(persisted.visibility),
                input_json,
                event_json,
            ],
        )?;
        transaction.execute(
            "UPDATE event_trees SET next_sequence=?2 WHERE tree_id=?1",
            params![persisted.position.tree_id.to_string(), sequence + 1],
        )?;
        transaction.commit()?;
        Ok(persisted)
    }
}

fn visibility_name(visibility: EventVisibility) -> &'static str {
    match visibility {
        EventVisibility::ModelVisible => "model_visible",
        EventVisibility::Control => "control",
        EventVisibility::Sensitive => "sensitive",
    }
}
