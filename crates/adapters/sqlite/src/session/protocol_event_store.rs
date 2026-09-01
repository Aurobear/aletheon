//! SQLite implementation of Runtime's session protocol journal port.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use contracts::protocol::client::{ClientEvent, EventCursor};
use contracts::SessionId;
use runtime::session_protocol::{
    ProtocolApprovalWrite, ProtocolItemWrite, SessionProtocolEventStore,
};
use rusqlite::{params, Connection, OptionalExtension};

pub struct SqliteSessionProtocolEventStore {
    connection: Mutex<Connection>,
}

impl SqliteSessionProtocolEventStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS protocol_events(
               session_id TEXT NOT NULL,
               sequence INTEGER NOT NULL,
               event_id TEXT NOT NULL,
               item_id TEXT NOT NULL,
               phase TEXT NOT NULL,
               dedupe_key TEXT,
               event_json TEXT NOT NULL,
               PRIMARY KEY(session_id,sequence),
               UNIQUE(session_id,event_id),
               UNIQUE(session_id,dedupe_key)
             );
             CREATE TABLE IF NOT EXISTS protocol_sync_watermarks(
               session_id TEXT PRIMARY KEY,
               canonical_sequence INTEGER NOT NULL
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

impl SessionProtocolEventStore for SqliteSessionProtocolEventStore {
    fn append_item(&self, write: ProtocolItemWrite) -> Result<ClientEvent> {
        let phase_name = format!("{:?}", write.phase).to_ascii_lowercase();
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let tx = connection.transaction()?;
        if let Some(key) = write.dedupe_key.as_deref() {
            if let Some((sequence, json)) = tx.query_row(
                "SELECT sequence,event_json FROM protocol_events WHERE session_id=?1 AND dedupe_key=?2",
                params![write.session_id.0, key],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
            ).optional()? {
                if write.item.is_some() {
                    let mut event: ClientEvent = serde_json::from_str(&json)?;
                    if let ClientEvent::Item(existing) = &mut event {
                        existing.item = write.item;
                        existing.error = write.error;
                    }
                    tx.execute(
                        "UPDATE protocol_events SET event_json=?3 WHERE session_id=?1 AND sequence=?2",
                        params![write.session_id.0, sequence, serde_json::to_string(&event)?],
                    )?;
                    advance_canonical_watermark(
                        &tx,
                        &write.session_id,
                        write.canonical_sequence,
                    )?;
                    tx.commit()?;
                    return Ok(event);
                }
                advance_canonical_watermark(&tx, &write.session_id, write.canonical_sequence)?;
                tx.commit()?;
                return Ok(serde_json::from_str(&json)?);
            }
        }
        let sequence: u64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM protocol_events WHERE session_id=?1",
            params![write.session_id.0],
            |row| row.get(0),
        )?;
        let cursor = EventCursor {
            sequence,
            event_id: Some(uuid::Uuid::new_v4().to_string()),
        };
        let event = ClientEvent::Item(contracts::protocol::client::ItemEvent {
            cursor: cursor.clone(),
            item_id: write.item_id.clone(),
            phase: write.phase,
            delta: write.delta,
            item: write.item,
            error: write.error,
        });
        tx.execute(
            "INSERT INTO protocol_events(session_id,sequence,event_id,item_id,phase,dedupe_key,event_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![write.session_id.0, sequence, cursor.event_id.as_deref().unwrap_or_default(),
                write.item_id, phase_name, write.dedupe_key, serde_json::to_string(&event)?],
        )?;
        advance_canonical_watermark(&tx, &write.session_id, write.canonical_sequence)?;
        tx.commit()?;
        Ok(event)
    }

    fn append_approval(&self, write: ProtocolApprovalWrite) -> Result<ClientEvent> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let tx = connection.transaction()?;
        let dedupe_key = format!("approval-request:{}", write.approval_id);
        if let Some(json) = tx
            .query_row(
                "SELECT event_json FROM protocol_events WHERE session_id=?1 AND dedupe_key=?2",
                params![write.session_id.0, dedupe_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            tx.commit()?;
            return Ok(serde_json::from_str(&json)?);
        }
        let sequence: u64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM protocol_events WHERE session_id=?1",
            params![write.session_id.0],
            |row| row.get(0),
        )?;
        let cursor = EventCursor {
            sequence,
            event_id: Some(uuid::Uuid::new_v4().to_string()),
        };
        let event = ClientEvent::ApprovalRequested {
            cursor: cursor.clone(),
            session_id: write.session_id.clone(),
            turn_id: write.turn_id,
            approval_id: write.approval_id.clone(),
            tool: write.tool,
            action_summary: write.action_summary,
            risk_level: write.risk_level,
            detail: write.detail,
            scope_subject: write.scope_subject,
        };
        tx.execute(
            "INSERT INTO protocol_events(session_id,sequence,event_id,item_id,phase,dedupe_key,event_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![write.session_id.0, sequence, cursor.event_id.as_deref().unwrap_or_default(),
                write.approval_id, "approval_requested", dedupe_key, serde_json::to_string(&event)?],
        )?;
        tx.commit()?;
        Ok(event)
    }

    fn event_id_at(&self, session_id: &SessionId, sequence: u64) -> Result<Option<String>> {
        Ok(self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .query_row(
                "SELECT event_id FROM protocol_events WHERE session_id=?1 AND sequence=?2",
                params![session_id.0, sequence],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn events_after(
        &self,
        session_id: &SessionId,
        sequence: u64,
        limit: usize,
    ) -> Result<Vec<ClientEvent>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut statement = connection.prepare(
            "SELECT event_json FROM protocol_events WHERE session_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
        )?;
        let json = statement
            .query_map(params![session_id.0, sequence, limit], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        json.into_iter()
            .map(|value| Ok(serde_json::from_str(&value)?))
            .collect()
    }

    fn tail_cursor(&self, session_id: &SessionId) -> Result<EventCursor> {
        let row: Option<(u64, String)> = self.connection.lock().unwrap_or_else(|error| error.into_inner()).query_row(
            "SELECT sequence,event_id FROM protocol_events WHERE session_id=?1 ORDER BY sequence DESC LIMIT 1",
            params![session_id.0], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        Ok(
            row.map_or_else(EventCursor::origin, |(sequence, event_id)| EventCursor {
                sequence,
                event_id: Some(event_id),
            }),
        )
    }

    fn canonical_watermark(&self, session_id: &SessionId) -> Result<u64> {
        Ok(self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .query_row(
                "SELECT canonical_sequence FROM protocol_sync_watermarks WHERE session_id=?1",
                params![session_id.0],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }
}

fn advance_canonical_watermark(
    transaction: &rusqlite::Transaction<'_>,
    session_id: &SessionId,
    canonical_sequence: Option<u64>,
) -> Result<()> {
    let Some(canonical_sequence) = canonical_sequence else {
        return Ok(());
    };
    transaction.execute(
        "INSERT INTO protocol_sync_watermarks(session_id,canonical_sequence) VALUES(?1,?2)
         ON CONFLICT(session_id) DO UPDATE SET canonical_sequence=
           MAX(protocol_sync_watermarks.canonical_sequence,excluded.canonical_sequence)",
        params![session_id.0, canonical_sequence],
    )?;
    Ok(())
}
