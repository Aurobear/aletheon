//! IdentityLayer — current identity model + mutation history.
//!
//! The identity is the agent's self-model. Every mutation preserves
//! the previous state in a history chain.

use anyhow::Result;
use chrono::Utc;
use fabric::Identity;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Record of a past identity state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityRecord {
    pub identity: Identity,
    pub mutated_at: chrono::DateTime<chrono::Utc>,
    pub reason: String,
}

/// IdentityLayer — holds current identity and a history of past identities.
pub struct IdentityLayer {
    current: RwLock<Identity>,
    history: RwLock<Vec<IdentityRecord>>,
    clock: Arc<dyn fabric::Clock>,
}

impl IdentityLayer {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        version: impl Into<String>,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        let identity = Identity {
            name: name.into(),
            description: description.into(),
            version: version.into(),
            created_at: fabric::wall_to_datetime(clock.wall_now()),
            last_mutation: None,
        };
        Self {
            current: RwLock::new(identity),
            history: RwLock::new(Vec::new()),
            clock,
        }
    }

    /// Get the current identity.
    pub fn current(&self) -> Identity {
        self.current.read().clone()
    }

    /// Apply a mutation. The previous identity is pushed to history.
    pub fn mutate(
        &self,
        new_name: Option<String>,
        new_description: Option<String>,
        new_version: Option<String>,
        reason: impl Into<String>,
    ) -> Identity {
        let mut current = self.current.write();
        let old = current.clone();

        // Push old to history
        self.history.write().push(IdentityRecord {
            identity: old,
            mutated_at: fabric::wall_to_datetime(self.clock.wall_now()),
            reason: reason.into(),
        });

        // Apply changes
        if let Some(name) = new_name {
            current.name = name;
        }
        if let Some(desc) = new_description {
            current.description = desc;
        }
        if let Some(ver) = new_version {
            current.version = ver;
        }
        current.last_mutation = Some(fabric::wall_to_datetime(self.clock.wall_now()));

        current.clone()
    }

    /// Get mutation history (oldest first).
    pub fn history(&self) -> Vec<IdentityRecord> {
        self.history.read().clone()
    }

    /// Number of mutations applied.
    pub fn mutation_count(&self) -> usize {
        self.history.read().len()
    }

    /// Persist current identity and history to the SQLite store.
    pub fn save_to_store(&self, store: &crate::core::store::SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        let current = self.current.read();
        let history = self.history.read();

        // Clear both tables
        conn.execute("DELETE FROM identity_current", [])?;
        conn.execute("DELETE FROM identity_history", [])?;

        // Save current identity
        conn.execute(
            "INSERT INTO identity_current (name, description, version, created_at, last_mutation) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                current.name,
                current.description,
                current.version,
                current.created_at.to_rfc3339(),
                current.last_mutation.map(|t| t.to_rfc3339()),
            ],
        )?;

        // Save history
        let mut stmt = conn.prepare(
            "INSERT INTO identity_history (name, description, version, created_at, last_mutation, mutated_at, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for record in history.iter() {
            stmt.execute(rusqlite::params![
                record.identity.name,
                record.identity.description,
                record.identity.version,
                record.identity.created_at.to_rfc3339(),
                record.identity.last_mutation.map(|t| t.to_rfc3339()),
                record.mutated_at.to_rfc3339(),
                record.reason,
            ])?;
        }
        Ok(())
    }

    /// Load identity and history from the SQLite store, replacing current state.
    pub fn load_from_store(&mut self, store: &crate::core::store::SelfFieldStore) -> Result<()> {
        let conn = store.conn();

        // Load current identity
        {
            let mut stmt = conn.prepare(
                "SELECT name, description, version, created_at, last_mutation FROM identity_current LIMIT 1",
            )?;
            let mut rows = stmt.query([])?;
            if let Some(row) = rows.next()? {
                let created_at: String = row.get(3)?;
                let last_mutation: Option<String> = row.get(4)?;
                let identity = Identity {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    version: row.get(2)?,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| fabric::wall_to_datetime(self.clock.wall_now())),
                    last_mutation: last_mutation
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                };
                *self.current.write() = identity;
            }
        }

        // Load history
        {
            let mut stmt = conn.prepare(
                "SELECT name, description, version, created_at, last_mutation, mutated_at, reason FROM identity_history ORDER BY id ASC",
            )?;
            let loaded: Vec<IdentityRecord> = stmt
                .query_map([], |row| {
                    let created_at: String = row.get(3)?;
                    let last_mutation: Option<String> = row.get(4)?;
                    let mutated_at: String = row.get(5)?;
                    Ok(IdentityRecord {
                        identity: Identity {
                            name: row.get(0)?,
                            description: row.get(1)?,
                            version: row.get(2)?,
                            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                                .map(|dt| dt.with_timezone(&Utc))
                                .unwrap_or_else(|_| fabric::wall_to_datetime(self.clock.wall_now())),
                            last_mutation: last_mutation
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                                .map(|dt| dt.with_timezone(&Utc)),
                        },
                        mutated_at: chrono::DateTime::parse_from_rfc3339(&mutated_at)
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| fabric::wall_to_datetime(self.clock.wall_now())),
                        reason: row.get(6)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            *self.history.write() = loaded;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn test_layer(
        name: &str,
        desc: &str,
        ver: &str,
    ) -> IdentityLayer {
        IdentityLayer::new(name, desc, ver, test_clock())
    }

    #[test]
    fn new_identity() {
        let layer = test_layer("aurb", "An AI agent", "0.1.0");
        let id = layer.current();
        assert_eq!(id.name, "aurb");
        assert_eq!(id.description, "An AI agent");
        assert_eq!(id.version, "0.1.0");
        assert!(id.last_mutation.is_none());
        assert_eq!(layer.mutation_count(), 0);
    }

    #[test]
    fn mutate_preserves_history() {
        let layer = test_layer("aurb", "desc", "0.1.0");
        let updated = layer.mutate(
            Some("aurb-v2".to_string()),
            None,
            Some("0.2.0".to_string()),
            "upgraded",
        );

        assert_eq!(updated.name, "aurb-v2");
        assert_eq!(updated.version, "0.2.0");
        assert!(updated.last_mutation.is_some());

        let history = layer.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].identity.name, "aurb");
        assert_eq!(history[0].identity.version, "0.1.0");
        assert_eq!(history[0].reason, "upgraded");
    }

    #[test]
    fn multiple_mutations_chain() {
        let layer = test_layer("v0", "desc", "0.0.1");

        layer.mutate(Some("v1".to_string()), None, None, "step1");
        layer.mutate(Some("v2".to_string()), None, None, "step2");
        layer.mutate(Some("v3".to_string()), None, None, "step3");

        assert_eq!(layer.current().name, "v3");
        assert_eq!(layer.mutation_count(), 3);

        let history = layer.history();
        assert_eq!(history[0].identity.name, "v0");
        assert_eq!(history[1].identity.name, "v1");
        assert_eq!(history[2].identity.name, "v2");
    }
}
