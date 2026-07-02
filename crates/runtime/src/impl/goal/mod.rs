//! Persistent objective store backed by SQLite.
//!
//! Mirrors `FactStore`'s open/schema idiom (`impl/memory/fact_store/mod.rs`):
//! WAL, `CREATE TABLE IF NOT EXISTS`, positional `map_objective_row`.

mod store;

use anyhow::{Context, Result};
use base::objective::{Objective, ObjectiveStatus};
use rusqlite::Connection;

/// SQLite-backed objective store.
///
/// Held behind `Arc<Mutex<ObjectiveStore>>` in `RequestHandler`, mirroring
/// `fact_store`'s ownership pattern exactly.
pub struct ObjectiveStore {
    pub(crate) db: Connection,
}

impl ObjectiveStore {
    /// Open (or create) an objective store at the given path.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let db = Connection::open(path).context("opening objective store DB")?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::create_schema(&db)?;
        Ok(Self { db })
    }

    fn create_schema(db: &Connection) -> Result<()> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS objectives (
                objective_id INTEGER PRIMARY KEY AUTOINCREMENT,
                description  TEXT NOT NULL,
                status       TEXT NOT NULL DEFAULT 'in_progress'
                             CHECK(status IN ('in_progress','completed','failed','adjusted')),
                parent_id    INTEGER REFERENCES objectives(objective_id) ON DELETE CASCADE,
                session_id   TEXT NOT NULL DEFAULT '',
                scope        TEXT NOT NULL DEFAULT 'session'
                             CHECK(scope IN ('session','project','global')),
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_objectives_status ON objectives(status);
            CREATE INDEX IF NOT EXISTS idx_objectives_parent ON objectives(parent_id);
            CREATE INDEX IF NOT EXISTS idx_objectives_session ON objectives(session_id);
            CREATE INDEX IF NOT EXISTS idx_objectives_scope ON objectives(scope);",
        )?;
        Ok(())
    }

    /// Map a rusqlite Row to an Objective using positional indices.
    ///
    /// Column order MUST match the `COLS` constant in `store.rs`.
    /// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
    ///          4=session_id, 5=scope, 6=created_at, 7=updated_at
    pub(crate) fn map_objective_row(row: &rusqlite::Row) -> rusqlite::Result<Objective> {
        let status_str: String = row.get(2)?;
        let status = ObjectiveStatus::from_str(&status_str)
            .unwrap_or(ObjectiveStatus::InProgress);
        Ok(Objective {
            objective_id: row.get(0)?,
            description: row.get(1)?,
            status,
            parent_id: row.get(3)?,
            session_id: row.get(4)?,
            scope: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn setup() -> (ObjectiveStore, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = ObjectiveStore::open(tmp.path()).unwrap();
        (store, tmp)
    }

    #[test]
    fn create_and_get_roundtrip() {
        let (store, _tmp) = setup();
        let id = store
            .create("Ship the goal layer", None, "sess-1", "project")
            .unwrap();
        assert!(id > 0);
        let row = store.get(id).unwrap().unwrap();
        assert_eq!(row.description, "Ship the goal layer");
        assert_eq!(row.status, ObjectiveStatus::InProgress);
        assert_eq!(row.session_id, "sess-1");
        assert_eq!(row.scope, "project");
        assert!(row.parent_id.is_none());
    }

    #[test]
    fn schema_and_indexes_exist() {
        let (store, _tmp) = setup();
        let names: Vec<String> = store
            .db
            .prepare(
                "SELECT name FROM sqlite_master WHERE type IN ('table','index') ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(names.iter().any(|n| n == "objectives"));
        assert!(names.iter().any(|n| n == "idx_objectives_status"));
        assert!(names.iter().any(|n| n == "idx_objectives_parent"));
        assert!(names.iter().any(|n| n == "idx_objectives_session"));
    }

    #[test]
    fn active_returns_latest_in_progress_top_level() {
        let (store, _tmp) = setup();
        let a = store
            .create("first objective", None, "s", "session")
            .unwrap();
        let b = store
            .create("second objective", None, "s", "session")
            .unwrap();
        // sub-goals of `b`
        store
            .create("sub one", Some(b), "s", "session")
            .unwrap();
        store
            .create("sub two", Some(b), "s", "session")
            .unwrap();
        // finishing `b` makes `a` the active top-level objective
        assert!(store.set_status(b, "completed").unwrap());
        let active = store.active().unwrap().unwrap();
        assert_eq!(active.objective_id, a);
        // sub_goals only returns children of the given parent
        let subs = store.sub_goals(b).unwrap();
        assert_eq!(subs.len(), 2);
        assert!(subs.iter().all(|s| s.parent_id == Some(b)));
    }

    #[test]
    fn resume_reconstructs_active_objective_and_subs() {
        let (store, _tmp) = setup();
        let obj = store
            .create("resume me", None, "s", "project")
            .unwrap();
        store
            .create("child a", Some(obj), "s", "project")
            .unwrap();
        let (active, subs) = store.resume().unwrap().unwrap();
        assert_eq!(active.objective_id, obj);
        assert_eq!(subs.len(), 1);
        assert_eq!(
            store.list(None, 50).unwrap().len(),
            2
        );
        assert_eq!(
            store.list(Some("in_progress"), 50).unwrap().len(),
            2
        );
    }

    #[test]
    fn status_filtering() {
        let (store, _tmp) = setup();
        let id = store
            .create("complete me", None, "s", "session")
            .unwrap();
        store.set_status(id, "completed").unwrap();
        let in_progress = store.list(Some("in_progress"), 50).unwrap();
        let completed = store.list(Some("completed"), 50).unwrap();
        assert_eq!(in_progress.len(), 0);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].objective_id, id);
    }

    #[test]
    fn parent_cascade_deletes_sub_goals() {
        let (store, _tmp) = setup();
        let parent = store
            .create("parent", None, "s", "session")
            .unwrap();
        let child = store
            .create("child", Some(parent), "s", "session")
            .unwrap();
        // Delete parent (using raw SQL since we don't expose delete in MVP API)
        store
            .db
            .execute(
                "DELETE FROM objectives WHERE objective_id = ?1",
                rusqlite::params![parent],
            )
            .unwrap();
        // Sub-goal should be cascade-deleted
        assert!(store.get(child).unwrap().is_none());
    }
}
