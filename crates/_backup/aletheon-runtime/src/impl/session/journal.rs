use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
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

/// EventJournal: append-only JSONL log with async writer task.
pub struct EventJournal {
    tx: mpsc::Sender<JournalEntry>,
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
        let (tx, mut rx) = mpsc::channel::<JournalEntry>(256);
        let db_path_clone = db_path.clone();

        let handle = tokio::spawn(async move {
            while let Some(entry) = rx.recv().await {
                // Write to JSONL
                let json = serde_json::to_string(&entry).unwrap_or_default();
                let _ = file.write_all(json.as_bytes()).await;
                let _ = file.write_all(b"\n").await;

                // Write to SQLite index
                let event_type = match &entry.event {
                    SessionEvent::SessionCreated { .. } => "session_created",
                    SessionEvent::UserMessage { .. } => "user_message",
                    SessionEvent::AssistantMessage { .. } => "assistant_message",
                    SessionEvent::ToolCallStarted { .. } => "tool_call_started",
                    SessionEvent::ToolCallCompleted { .. } => "tool_call_completed",
                    SessionEvent::CheckpointBoundary { .. } => "checkpoint_boundary",
                    SessionEvent::Compacted { .. } => "compacted",
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
        self.tx.send(entry).await?;
        Ok(())
    }

    /// Get the SQLite database path for this session.
    pub fn db_path(&self) -> &Path {
        &self.db_path
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
        let mut stmt =
            db.prepare("SELECT event_json FROM events WHERE id > ?1 ORDER BY id ASC")?;

        let events: Vec<SessionEvent> = stmt
            .query_map(rusqlite::params![last_checkpoint.unwrap_or(0)], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
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
