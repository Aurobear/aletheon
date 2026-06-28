use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Record of a tool call outcome for learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeRecord {
    pub id: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub result_summary: String,
    pub is_error: bool,
    pub user_feedback: Option<UserFeedback>,
    pub timestamp: DateTime<Utc>,
    pub context: OutcomeContext,
}

/// User feedback on the outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFeedback {
    pub rating: i8, // -1 (bad), 0 (neutral), 1 (good)
    pub comment: Option<String>,
}

/// Context surrounding the outcome.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutcomeContext {
    pub preceding_errors: usize,
    pub iteration_count: usize,
    pub system_state: Option<String>,
}

/// Records outcomes for the learning pipeline.
pub struct OutcomeRecorder {
    db_path: std::path::PathBuf,
}

impl OutcomeRecorder {
    pub fn new(db_path: std::path::PathBuf) -> Self {
        Self { db_path }
    }

    /// Record an outcome.
    pub fn record(&self, outcome: &OutcomeRecord) -> Result<(), anyhow::Error> {
        // Store in SQLite
        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS outcomes (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                turn_id TEXT,
                tool_name TEXT,
                args TEXT,
                result_summary TEXT,
                is_error INTEGER,
                user_feedback TEXT,
                timestamp TEXT,
                context TEXT
            )",
        )?;

        conn.execute(
            "INSERT OR REPLACE INTO outcomes VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                outcome.id,
                outcome.session_id,
                outcome.turn_id,
                outcome.tool_name,
                serde_json::to_string(&outcome.args)?,
                outcome.result_summary,
                outcome.is_error as i32,
                outcome
                    .user_feedback
                    .as_ref()
                    .map(|f| serde_json::to_string(f).ok())
                    .flatten(),
                outcome.timestamp.to_rfc3339(),
                serde_json::to_string(&outcome.context)?,
            ],
        )?;

        Ok(())
    }

    /// Get recent outcomes.
    pub fn get_recent(&self, limit: usize) -> Result<Vec<OutcomeRecord>, anyhow::Error> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare("SELECT * FROM outcomes ORDER BY timestamp DESC LIMIT ?1")?;

        let outcomes = stmt
            .query_map([limit], |row| {
                Ok(OutcomeRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    turn_id: row.get(2)?,
                    tool_name: row.get(3)?,
                    args: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
                    result_summary: row.get(5)?,
                    is_error: row.get::<_, i32>(6)? != 0,
                    user_feedback: row
                        .get::<_, Option<String>>(7)?
                        .and_then(|s| serde_json::from_str(&s).ok()),
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    context: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(outcomes)
    }
}
