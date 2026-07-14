//! CRUD and query operations for ObjectiveStore.
//!
//! Mirror of `fact_store/query.rs`. Every SELECT uses the shared `COLS` constant
//! to guarantee column order matches `map_objective_row`.
//!
//! M2 adds goal-oriented methods (`create_goal`, `get_goal`, etc.) that use
//! `GOAL_COLS` and `map_goal_snapshot_row`. Legacy methods continue to work
//! unchanged.

use super::{GoalId, GoalSnapshot, GoalSpec, GoalState, Objective, ObjectiveStore, PrincipalId};
use anyhow::Result;

/// Fixed column order — every SELECT feeding `map_objective_row` MUST use this.
/// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
///          4=session_id, 5=scope, 6=created_at, 7=updated_at
pub(crate) const COLS: &str =
    "objective_id, description, status, parent_id, session_id, scope, created_at, updated_at";

/// Extended column order for goal snapshots.
/// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
///          4=session_id, 5=scope, 6=created_at, 7=updated_at,
///          8=owner_id, 9=goal_state, 10=spec_json, 11=wait_json,
///          12=process_id, 13=version, 14=deadline_ms
pub(crate) const GOAL_COLS: &str = concat!(
    "objective_id, description, status, parent_id, session_id, scope, ",
    "created_at, updated_at, owner_id, goal_state, spec_json, wait_json, ",
    "process_id, version, deadline_ms"
);

impl ObjectiveStore {
    // -----------------------------------------------------------------------
    // Legacy API (preserved for backward compatibility)
    // -----------------------------------------------------------------------

    /// Insert a top-level objective or sub-goal. Returns the new `objective_id`.
    ///
    /// In M2, this also populates `spec_json` so the row is readable as a
    /// GoalSnapshot with `original_intent` equal to `description`.
    pub fn create(
        &self,
        description: &str,
        parent: Option<i64>,
        session_id: &str,
        scope: &str,
    ) -> Result<i64> {
        let spec_json = serde_json::json!({
            "original_intent": description,
            "desired_state": [],
            "constraints": [],
            "acceptance_criteria": [],
            "budget": {
                "max_input_tokens": 1_000_000,
                "max_output_tokens": 500_000,
                "max_cost_usd": null,
                "max_attempts": 10,
                "deadline_ms": null
            }
        })
        .to_string();
        self.db.execute(
            "INSERT INTO objectives (description, parent_id, session_id, scope, spec_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![description, parent, session_id, scope, spec_json],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Fetch one objective by id.
    pub fn get(&self, id: i64) -> Result<Option<Objective>> {
        let sql = format!("SELECT {COLS} FROM objectives WHERE objective_id = ?1");
        let mut stmt = self.db.prepare(&sql)?;
        let mut rows = stmt.query_map(rusqlite::params![id], Self::map_objective_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Update status; bumps `updated_at`. Returns true if a row changed.
    pub fn set_status(&self, id: i64, status: &str) -> Result<bool> {
        let changed = self.db.execute(
            "UPDATE objectives SET status = ?1, updated_at = datetime('now')
             WHERE objective_id = ?2",
            rusqlite::params![status, id],
        )?;
        Ok(changed > 0)
    }

    /// List objectives, optionally filtered by status, newest first.
    pub fn list(&self, status_filter: Option<&str>, limit: usize) -> Result<Vec<Objective>> {
        let mut sql = format!("SELECT {COLS} FROM objectives");
        if status_filter.is_some() {
            sql.push_str(" WHERE status = ?1");
        }
        sql.push_str(&format!(
            " ORDER BY objective_id DESC LIMIT {}",
            limit as i64
        ));
        let mut stmt = self.db.prepare(&sql)?;
        let rows = if let Some(s) = status_filter {
            stmt.query_map(rusqlite::params![s], Self::map_objective_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], Self::map_objective_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    /// The single active top-level objective: newest `in_progress` with no parent.
    /// (MVP is single-objective; see non-goals.)
    pub fn active(&self) -> Result<Option<Objective>> {
        let sql = format!(
            "SELECT {COLS} FROM objectives
             WHERE status = 'in_progress' AND parent_id IS NULL
             ORDER BY objective_id DESC LIMIT 1"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let mut rows = stmt.query_map([], Self::map_objective_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Direct children of an objective, oldest first (milestone order).
    pub fn sub_goals(&self, parent: i64) -> Result<Vec<Objective>> {
        let sql =
            format!("SELECT {COLS} FROM objectives WHERE parent_id = ?1 ORDER BY objective_id ASC");
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![parent], Self::map_objective_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Resume: the active top-level objective plus its sub-goals, if any.
    /// Returns `None` when no active objective exists (fresh start).
    pub fn resume(&self) -> Result<Option<(Objective, Vec<Objective>)>> {
        match self.active()? {
            Some(obj) => {
                let subs = self.sub_goals(obj.objective_id)?;
                Ok(Some((obj, subs)))
            }
            None => Ok(None),
        }
    }

    // -----------------------------------------------------------------------
    // M2 Goal API (new — beside legacy API, not replacing it)
    // -----------------------------------------------------------------------

    /// Create a goal from a full GoalSpec. Inserts an `objectives` row plus a
    /// version-0 `goal_events` entry in a single transaction.
    pub fn create_goal(
        &self,
        owner: &PrincipalId,
        session_id: &str,
        scope: &str,
        spec: &GoalSpec,
    ) -> Result<GoalSnapshot> {
        let spec_json = serde_json::to_string(spec)?;
        let tx = self.db.unchecked_transaction()?;

        tx.execute(
            "INSERT INTO objectives (description, session_id, scope, owner_id, goal_state, spec_json, version)
             VALUES (?1, ?2, ?3, ?4, 'ready', ?5, 0)",
            rusqlite::params![spec.original_intent, session_id, scope, owner.0, spec_json],
        )?;
        let objective_id = tx.last_insert_rowid();

        tx.execute(
            "INSERT INTO goal_events (objective_id, version, event_type, payload_json)
             VALUES (?1, 0, 'created', ?2)",
            rusqlite::params![objective_id, "{}"],
        )?;

        tx.commit()?;

        self.get_goal(GoalId(objective_id))?
            .ok_or_else(|| anyhow::anyhow!("goal {objective_id} disappeared after insert"))
    }

    /// Fetch a goal by id.
    pub fn get_goal(&self, id: GoalId) -> Result<Option<GoalSnapshot>> {
        let sql = format!(
            "SELECT {GOAL_COLS} FROM objectives WHERE objective_id = ?1"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let mut rows =
            stmt.query_map(rusqlite::params![id.0], Self::map_goal_snapshot_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// List goals filtered by state, newest first.
    pub fn list_goals(
        &self,
        states: &[GoalState],
        limit: usize,
    ) -> Result<Vec<GoalSnapshot>> {
        if states.is_empty() {
            let sql = format!(
                "SELECT {GOAL_COLS} FROM objectives ORDER BY objective_id DESC LIMIT ?1"
            );
            let mut stmt = self.db.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params![limit as i64], Self::map_goal_snapshot_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            return Ok(rows);
        }

        let placeholders: Vec<String> = states.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT {GOAL_COLS} FROM objectives WHERE goal_state IN ({}) ORDER BY objective_id DESC LIMIT ?{}",
            placeholders.join(","),
            placeholders.len() + 1
        );
        let mut stmt = self.db.prepare(&sql)?;

        let state_strs: Vec<String> = states.iter().map(|s| s.as_str().to_string()).collect();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = state_strs
            .iter()
            .map(|s| Box::new(s.clone()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        params.push(Box::new(limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), Self::map_goal_snapshot_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Return non-terminal goals that should be considered for recovery.
    pub fn recoverable_goals(&self) -> Result<Vec<GoalSnapshot>> {
        let sql = format!(
            "SELECT {GOAL_COLS} FROM objectives
             WHERE parent_id IS NULL AND goal_state NOT IN ('completed','failed','cancelled')
             ORDER BY objective_id ASC"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt
            .query_map([], Self::map_goal_snapshot_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Recover goals at daemon startup.
    ///
    /// Policy:
    /// - Running goals: clear stale process link, transition to Ready, log event
    /// - Draft, Ready, Suspended, AwaitingHuman, Blocked: leave as-is
    /// - Terminal: not returned by `recoverable_goals()`
    pub fn recover_goals(&self) -> Result<Vec<GoalSnapshot>> {
        let candidates = self.recoverable_goals()?;
        let mut recovered = Vec::with_capacity(candidates.len());

        for goal in candidates {
            match goal.state {
                GoalState::Running => {
                    let tx = self.db.unchecked_transaction()?;
                    let new_version = goal.version + 1;
                    tx.execute(
                        "UPDATE objectives SET goal_state = 'ready', process_id = NULL,
                         version = ?1, updated_at = datetime('now')
                         WHERE objective_id = ?2",
                        rusqlite::params![new_version, goal.id.0],
                    )?;
                    tx.execute(
                        "INSERT INTO goal_events (objective_id, version, event_type, payload_json)
                         VALUES (?1, ?2, 'recovered', ?3)",
                        rusqlite::params![
                            goal.id.0,
                            new_version,
                            serde_json::json!({"action": "recover", "from": "running", "to": "ready"}).to_string(),
                        ],
                    )?;
                    tx.commit()?;
                    let fresh = self
                        .get_goal(goal.id)?
                        .ok_or_else(|| anyhow::anyhow!("goal {} disappeared during recovery", goal.id))?;
                    recovered.push(fresh);
                }
                _ => {
                    recovered.push(goal);
                }
            }
        }
        Ok(recovered)
    }
}
