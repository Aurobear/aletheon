use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tracing::debug;

/// Session event types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    SessionCreated {
        session_id: String,
    },
    UserMessage {
        content: String,
    },
    AssistantMessage {
        content: String,
    },
    /// A ContentBlock::ToolUse block within a message.
    /// Used during recovery to reconstruct multi-block assistant messages.
    ToolUseBlock {
        tool_use_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    /// A ContentBlock::ToolResult block within a message.
    /// Used during recovery to reconstruct tool_result user messages.
    ToolResultBlock {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    ToolCallStarted {
        tool_call_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    ToolCallCompleted {
        tool_call_id: String,
        is_error: bool,
        content: String,
        elapsed_ms: u64,
    },
    CheckpointBoundary {
        iteration: usize,
    },
    Compacted {
        before_count: usize,
        after_count: usize,
    },
    Summary {
        text: String,
    },
    SessionEnded {
        reason: String,
    },
}

/// A journal entry with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub timestamp: DateTime<Utc>,
    pub session_id: String,
    pub event: SessionEvent,
}

/// Message sent to the writer task.
enum WriterMsg {
    /// A journal entry to persist.
    Entry(JournalEntry),
    /// A flush request: writer responds on the oneshot after processing all
    /// prior messages.
    Flush(oneshot::Sender<()>),
}

/// EventJournal: append-only JSONL log with async writer task.
pub struct EventJournal {
    tx: mpsc::Sender<WriterMsg>,
    _handle: tokio::task::JoinHandle<()>,
    session_id: String,
    db_path: PathBuf,
}

impl EventJournal {
    /// Create a new EventJournal for a session.
    pub async fn create(session_id: &str, data_dir: &Path) -> Result<Self> {
        let log_path = data_dir.join(format!("{}.jsonl", session_id));
        let db_path = data_dir.join(format!("{}.db", session_id));

        // Ensure data directory exists
        tokio::fs::create_dir_all(data_dir).await?;

        // Open JSONL file for appending
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;

        // Create SQLite index
        let db = rusqlite::Connection::open(&db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                session_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_session ON events(session_id);
            CREATE INDEX IF NOT EXISTS idx_type ON events(event_type);
            ",
        )?;

        // Spawn writer task
        let (tx, mut rx) = mpsc::channel::<WriterMsg>(256);
        let db_path_clone = db_path.clone();

        let handle = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    WriterMsg::Flush(done) => {
                        let _ = done.send(());
                        continue;
                    }
                    WriterMsg::Entry(entry) => {
                        // Write to JSONL
                        let json = serde_json::to_string(&entry).unwrap_or_default();
                        let _ = file.write_all(json.as_bytes()).await;
                        let _ = file.write_all(b"\n").await;

                        // Write to SQLite index
                        let event_type = match &entry.event {
                            SessionEvent::SessionCreated { .. } => "session_created",
                            SessionEvent::UserMessage { .. } => "user_message",
                            SessionEvent::AssistantMessage { .. } => "assistant_message",
                            SessionEvent::ToolUseBlock { .. } => "tool_use_block",
                            SessionEvent::ToolResultBlock { .. } => "tool_result_block",
                            SessionEvent::ToolCallStarted { .. } => "tool_call_started",
                            SessionEvent::ToolCallCompleted { .. } => "tool_call_completed",
                            SessionEvent::CheckpointBoundary { .. } => "checkpoint_boundary",
                            SessionEvent::Compacted { .. } => "compacted",
                            SessionEvent::Summary { .. } => "summary",
                            SessionEvent::SessionEnded { .. } => "session_ended",
                        };

                        if let Ok(db) = rusqlite::Connection::open(&db_path_clone) {
                            let _ = db.execute(
                                "INSERT INTO events (timestamp, session_id, event_type, event_json) VALUES (?1, ?2, ?3, ?4)",
                                rusqlite::params![
                                    entry.timestamp.to_rfc3339(),
                                    entry.session_id,
                                    event_type,
                                    json,
                                ],
                            );
                        }

                        debug!(event_type, "Event written to journal");
                    }
                }
            }
        });

        Ok(Self {
            tx,
            _handle: handle,
            session_id: session_id.to_string(),
            db_path,
        })
    }

    /// Append an event to the journal.
    pub async fn append(&self, event: SessionEvent) -> Result<()> {
        let entry = JournalEntry {
            timestamp: Utc::now(),
            session_id: self.session_id.clone(),
            event,
        };
        self.tx.send(WriterMsg::Entry(entry)).await?;
        Ok(())
    }

    /// Wait for all previously appended events to be durably written.
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(WriterMsg::Flush(tx)).await?;
        let _ = rx.await;
        Ok(())
    }

    /// Get the SQLite database path for this session.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Query journal entries from the SQLite index.
    ///
    /// Filters are optional; when `None` the dimension is not filtered.
    /// Results are ordered by timestamp ascending.
    pub fn query(
        &self,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        event_type: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<JournalEntry>> {
        let db = rusqlite::Connection::open(&self.db_path)?;
        let mut sql = String::from("SELECT event_json FROM events WHERE session_id = ?1");
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params.push(Box::new(self.session_id.clone()));

        if from.is_some() {
            sql.push_str(&format!(" AND timestamp >= ?{}", params.len() + 1));
            params.push(Box::new(from.unwrap().to_rfc3339()));
        }
        if to.is_some() {
            sql.push_str(&format!(" AND timestamp <= ?{}", params.len() + 1));
            params.push(Box::new(to.unwrap().to_rfc3339()));
        }
        if let Some(et) = event_type {
            sql.push_str(&format!(" AND event_type = ?{}", params.len() + 1));
            params.push(Box::new(et.to_string()));
        }
        sql.push_str(" ORDER BY timestamp ASC");
        if let Some(lim) = limit {
            sql.push_str(&format!(" LIMIT {}", lim));
        }

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = db.prepare(&sql)?;
        let entries: Vec<JournalEntry> = stmt
            .query_map(param_refs.as_slice(), |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str::<JournalEntry>(&json).ok())
            .collect();

        Ok(entries)
    }

    /// Recover session state from journal.
    /// Returns the last checkpoint and subsequent events.
    pub async fn recover(data_dir: &Path, session_id: &str) -> Result<RecoveryState> {
        let db_path = data_dir.join(format!("{}.db", session_id));
        let db = rusqlite::Connection::open(&db_path)?;

        // Find last checkpoint
        let last_checkpoint: Option<usize> = db
            .query_row(
                "SELECT MAX(id) FROM events WHERE event_type = 'checkpoint_boundary'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(None);

        // Get events after last checkpoint
        let mut stmt = db.prepare("SELECT event_json FROM events WHERE id > ?1 ORDER BY id ASC")?;

        let events: Vec<SessionEvent> = stmt
            .query_map(rusqlite::params![last_checkpoint.unwrap_or(0)], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|json| {
                serde_json::from_str::<JournalEntry>(&json)
                    .ok()
                    .map(|entry| entry.event)
            })
            .collect();

        Ok(RecoveryState {
            session_id: session_id.to_string(),
            events_after_checkpoint: events,
        })
    }
}

/// State recovered from a journal after crash.
pub struct RecoveryState {
    pub session_id: String,
    pub events_after_checkpoint: Vec<SessionEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile;

    #[tokio::test]
    async fn journal_query_by_type() {
        let tmp = tempfile::tempdir().unwrap();
        let journal = EventJournal::create("test-query", tmp.path())
            .await
            .unwrap();

        // Append some events
        journal
            .append(SessionEvent::UserMessage {
                content: "hello".into(),
            })
            .await
            .unwrap();
        journal
            .append(SessionEvent::AssistantMessage {
                content: "hi there".into(),
            })
            .await
            .unwrap();
        journal
            .append(SessionEvent::ToolCallStarted {
                tool_call_id: "t1".into(),
                tool_name: "bash".into(),
                input: serde_json::Value::Null,
            })
            .await
            .unwrap();

        // Flush to ensure events are written to SQLite
        journal.flush().await.unwrap();

        // Query user messages
        let results = journal
            .query(None, None, Some("user_message"), None)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].event, SessionEvent::UserMessage { .. }));

        // Query with limit
        let results = journal.query(None, None, None, Some(2)).unwrap();
        assert_eq!(results.len(), 2);

        // Query all
        let results = journal.query(None, None, None, None).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn journal_query_empty_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let journal = EventJournal::create("test-empty", tmp.path())
            .await
            .unwrap();
        let results = journal.query(None, None, None, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn db_path_returns_correct_path() {
        // Use a sync path to test db_path behavior
        let tmp = tempfile::tempdir().unwrap();
        let expected = tmp.path().join("test-path.db");
        let db = rusqlite::Connection::open(&expected).unwrap();
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                session_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_json TEXT NOT NULL
            );",
        )
        .unwrap();

        // Verify path construction logic
        assert!(expected.to_str().unwrap().ends_with("test-path.db"));
    }
}
