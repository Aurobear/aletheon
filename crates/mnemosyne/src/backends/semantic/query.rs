//! SemanticMemory query helpers (row_to_entry only).
//!
//! The MemoryBackend trait impl (recall, list, stats, store, forget, compact)
//! lives in `storage.rs` to avoid conflicting impl blocks.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use fabric::{wall_to_datetime, MemoryEntry, MemoryType, WallTime};
use uuid::Uuid;

/// Convert a rusqlite Row into a MemoryEntry.
pub(super) fn row_to_entry(
    row: &rusqlite::Row,
    clock: &Arc<dyn fabric::Clock>,
) -> rusqlite::Result<MemoryEntry> {
    let id_str: String = row.get("id")?;
    let tags_str: String = row.get("tags")?;
    let assoc_str: String = row.get("associations")?;
    let created_at_str: String = row.get("created_at")?;

    Ok(MemoryEntry {
        id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::nil()),
        memory_type: MemoryType::Semantic,
        content: row.get("content")?,
        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
        created_at: created_at_str
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| wall_to_datetime(clock.wall_now())),
        access_count: row.get::<_, i64>("access_count")? as u64,
        importance: row.get("importance")?,
        decay_rate: row.get("decay_rate")?,
        associations: serde_json::from_str(&assoc_str).unwrap_or_default(),
    })
}
