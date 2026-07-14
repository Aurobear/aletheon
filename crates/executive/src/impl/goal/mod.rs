//! Persistent objective store backed by SQLite.
//!
//! Mirrors `FactStore`'s open/schema idiom (`impl/memory/fact_store/mod.rs`):
//! WAL, versioned migrations, positional row mapping.
//!
//! M2 extends the schema with goal-specific columns, event log, and budget ledger,
//! while keeping legacy Objective CRUD intact.

mod budget;
pub mod coordinator;
mod migrations;
pub(crate) mod store;
mod transition;
pub use self::coordinator::{GoalCoordinator, GoalTickOutcome};

use anyhow::{Context, Result};
use fabric::goal::{GoalBudgetUsage, GoalId, GoalSnapshot, GoalSpec, GoalState, GoalWaitReason};
use fabric::objective::{Objective, ObjectiveStatus};
use fabric::PrincipalId;
use fabric::ProcessId;
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
        migrations::run_migrations(&db)?;
        Ok(Self { db })
    }

    /// Map a rusqlite Row to an Objective using positional indices.
    ///
    /// Column order MUST match the `COLS` constant in `store.rs`.
    /// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
    ///          4=session_id, 5=scope, 6=created_at, 7=updated_at
    pub(crate) fn map_objective_row(row: &rusqlite::Row) -> rusqlite::Result<Objective> {
        let status_str: String = row.get(2)?;
        let status = ObjectiveStatus::from_str(&status_str).unwrap_or(ObjectiveStatus::InProgress);
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

    /// Map a rusqlite Row to a GoalSnapshot using positional indices.
    ///
    /// Column order MUST match the `GOAL_COLS` constant in `store.rs`.
    /// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
    ///          4=session_id, 5=scope, 6=created_at, 7=updated_at,
    ///          8=owner_id, 9=goal_state, 10=spec_json, 11=wait_json,
    ///          12=process_id, 13=version, 14=deadline_ms
    pub(crate) fn map_goal_snapshot_row(row: &rusqlite::Row) -> rusqlite::Result<GoalSnapshot> {
        let spec_json: String = row.get(10)?;
        let spec: GoalSpec = serde_json::from_str(&spec_json).unwrap_or_else(|_| GoalSpec {
            original_intent: row.get::<_, String>(1).unwrap_or_default(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        });

        let wait_json: Option<String> = row.get(11)?;
        let wait_reason: Option<GoalWaitReason> =
            wait_json.and_then(|s| serde_json::from_str(&s).ok());

        let process_id_str: Option<String> = row.get(12)?;
        let process_id: Option<ProcessId> = process_id_str
            .and_then(|s| uuid::Uuid::parse_str(&s).ok())
            .map(ProcessId);

        let goal_state_str: String = row.get(9)?;
        let state = GoalState::from_str(&goal_state_str).unwrap_or(GoalState::Ready);

        Ok(GoalSnapshot {
            id: GoalId(row.get(0)?),
            owner: PrincipalId(row.get(8)?),
            state,
            spec,
            usage: GoalBudgetUsage::default(),
            wait_reason,
            process_id,
            version: row.get(13)?,
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
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','index') ORDER BY name")
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
        store.create("sub one", Some(b), "s", "session").unwrap();
        store.create("sub two", Some(b), "s", "session").unwrap();
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
        let obj = store.create("resume me", None, "s", "project").unwrap();
        store.create("child a", Some(obj), "s", "project").unwrap();
        let (active, subs) = store.resume().unwrap().unwrap();
        assert_eq!(active.objective_id, obj);
        assert_eq!(subs.len(), 1);
        assert_eq!(store.list(None, 50).unwrap().len(), 2);
        assert_eq!(store.list(Some("in_progress"), 50).unwrap().len(), 2);
    }

    #[test]
    fn status_filtering() {
        let (store, _tmp) = setup();
        let id = store.create("complete me", None, "s", "session").unwrap();
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
        let parent = store.create("parent", None, "s", "session").unwrap();
        let child = store.create("child", Some(parent), "s", "session").unwrap();
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

    // -----------------------------------------------------------------------
    // M2 Goal API tests
    // -----------------------------------------------------------------------

    #[test]
    fn create_goal_roundtrip() {
        let (store, _tmp) = setup();
        let spec = GoalSpec {
            original_intent: "ship feature X".into(),
            desired_state: vec!["feature X deployed".into()],
            constraints: vec!["no breaking changes".into()],
            acceptance_criteria: vec!["tests pass".into()],
            budget: Default::default(),
        };
        let owner = PrincipalId("test-owner".into());
        let snapshot = store
            .create_goal(&owner, "sess-1", "project", &spec)
            .unwrap();
        assert_eq!(snapshot.id.0, 1);
        assert_eq!(snapshot.owner.0, "test-owner");
        assert_eq!(snapshot.state, GoalState::Ready);
        assert_eq!(snapshot.spec.original_intent, "ship feature X");
        assert_eq!(snapshot.version, 0);

        // Read back.
        let got = store.get_goal(snapshot.id).unwrap().unwrap();
        assert_eq!(got.spec.original_intent, "ship feature X");
        assert_eq!(got.owner.0, "test-owner");
    }

    #[test]
    fn create_goal_inserts_event() {
        let (store, _tmp) = setup();
        let spec = GoalSpec {
            original_intent: "event test".into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        };
        store
            .create_goal(&PrincipalId("o".into()), "s", "session", &spec)
            .unwrap();

        let count: i64 = store
            .db
            .query_row(
                "SELECT COUNT(*) FROM goal_events WHERE objective_id = 1 AND version = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn legacy_create_preserves_original_intent() {
        let (store, _tmp) = setup();
        let id = store
            .create("legacy objective", None, "sess", "session")
            .unwrap();
        let goal = store.get_goal(GoalId(id)).unwrap().unwrap();
        assert_eq!(goal.spec.original_intent, "legacy objective");
        assert_eq!(goal.state, GoalState::Ready);
    }

    #[test]
    fn list_goals_by_state() {
        let (store, _tmp) = setup();
        let s1 = GoalSpec {
            original_intent: "goal 1".into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        };
        let s2 = GoalSpec {
            original_intent: "goal 2".into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        };
        store
            .create_goal(&PrincipalId("o".into()), "s", "session", &s1)
            .unwrap();
        store
            .create_goal(&PrincipalId("o".into()), "s", "session", &s2)
            .unwrap();

        let ready = store.list_goals(&[GoalState::Ready], 10).unwrap();
        assert_eq!(ready.len(), 2);
        let draft = store.list_goals(&[GoalState::Draft], 10).unwrap();
        assert_eq!(draft.len(), 0);
    }

    #[test]
    fn recoverable_goals_returns_non_terminal() {
        let (store, _tmp) = setup();
        let spec = GoalSpec {
            original_intent: "recoverable".into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        };
        store
            .create_goal(&PrincipalId("o".into()), "s", "session", &spec)
            .unwrap();
        let recoverable = store.recoverable_goals().unwrap();
        assert_eq!(recoverable.len(), 1);
        assert_eq!(recoverable[0].spec.original_intent, "recoverable");

        // Mark it completed; should no longer be recoverable.
        store
            .db
            .execute(
                "UPDATE objectives SET goal_state = 'completed' WHERE objective_id = 1",
                [],
            )
            .unwrap();
        let empty = store.recoverable_goals().unwrap();
        assert!(empty.is_empty());
    }
}
