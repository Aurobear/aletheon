//! CRUD and query operations for ObjectiveStore.
//!
//! Mirror of `fact_store/query.rs`. Every SELECT uses the shared `COLS` constant
//! to guarantee column order matches `map_objective_row`.

use super::{Objective, ObjectiveStore};
use anyhow::Result;

/// Fixed column order — every SELECT feeding `map_objective_row` MUST use this.
/// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
///          4=session_id, 5=scope, 6=created_at, 7=updated_at
pub(crate) const COLS: &str =
    "objective_id, description, status, parent_id, session_id, scope, created_at, updated_at";

impl ObjectiveStore {
    /// Insert a top-level objective or sub-goal. Returns the new `objective_id`.
    pub fn create(
        &self,
        description: &str,
        parent: Option<i64>,
        session_id: &str,
        scope: &str,
    ) -> Result<i64> {
        self.db.execute(
            "INSERT INTO objectives (description, parent_id, session_id, scope)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![description, parent, session_id, scope],
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
        sql.push_str(&format!(" ORDER BY objective_id DESC LIMIT {}", limit as i64));
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
        let sql = format!(
            "SELECT {COLS} FROM objectives WHERE parent_id = ?1 ORDER BY objective_id ASC"
        );
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

    /// Count objectives matching a status filter (for tests and health checks).
    #[cfg(test)]
    pub(crate) fn count_by_status(&self, status: &str) -> Result<usize> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM objectives WHERE status = ?1",
            rusqlite::params![status],
            |r| r.get(0),
        )?;
        Ok(count as usize)
    }
}
