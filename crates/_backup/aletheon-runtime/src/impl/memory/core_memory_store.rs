use rusqlite::Connection;
use super::core_memory::{CoreMemory, MemoryBlock};

/// SQLite-backed persistence for CoreMemory blocks.
pub struct CoreMemoryStore {
    db: Connection,
}

impl CoreMemoryStore {
    pub fn new(db_path: &std::path::Path) -> anyhow::Result<Self> {
        let db = Connection::open(db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS core_memory_blocks (
                label TEXT PRIMARY KEY,
                content TEXT NOT NULL DEFAULT '',
                char_limit INTEGER NOT NULL DEFAULT 2000,
                read_only INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );",
        )?;
        Ok(Self { db })
    }

    /// Save all blocks from CoreMemory to SQLite.
    pub fn save(&self, memory: &CoreMemory) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let tx = self.db.unchecked_transaction()?;
        for (label, block) in memory.blocks() {
            tx.execute(
                "INSERT OR REPLACE INTO core_memory_blocks (label, content, char_limit, read_only, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![label, block.value, block.char_limit as i64, block.read_only as i64, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Load CoreMemory from SQLite. Returns None if no blocks stored yet.
    pub fn load(&self) -> anyhow::Result<Option<CoreMemory>> {
        let mut stmt = self.db.prepare(
            "SELECT label, content, char_limit, read_only FROM core_memory_blocks",
        )?;

        let blocks: Vec<(String, String, usize, bool)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as usize,
                    row.get::<_, i64>(3)? != 0,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        if blocks.is_empty() {
            return Ok(None);
        }

        let mut memory = CoreMemory::new();
        for (label, content, char_limit, read_only) in blocks {
            let mut block = MemoryBlock::new(&label, &content, char_limit);
            block.read_only = read_only;
            memory.set_block(block);
        }
        Ok(Some(memory))
    }

    /// Clear all stored blocks.
    pub fn clear(&self) -> anyhow::Result<()> {
        self.db.execute("DELETE FROM core_memory_blocks", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn test_store() -> (CoreMemoryStore, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = CoreMemoryStore::new(tmp.path()).unwrap();
        (store, tmp)
    }

    #[test]
    fn save_and_load_round_trip() {
        let (store, _tmp) = test_store();
        let mut mem = CoreMemory::new();
        mem.set_block(MemoryBlock::new("persona", "test persona", 2000));
        mem.set_block(MemoryBlock::new("human", "test human", 2000));

        store.save(&mem).unwrap();
        let loaded = store.load().unwrap().expect("should have blocks");

        assert_eq!(loaded.get("persona"), Some("test persona"));
        assert_eq!(loaded.get("human"), Some("test human"));
    }

    #[test]
    fn load_empty_returns_none() {
        let (store, _tmp) = test_store();
        let loaded = store.load().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn clear_removes_all_blocks() {
        let (store, _tmp) = test_store();
        let mut mem = CoreMemory::new();
        mem.set_block(MemoryBlock::new("test", "data", 500));

        store.save(&mem).unwrap();
        store.clear().unwrap();

        let loaded = store.load().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn save_preserves_read_only_flag() {
        let (store, _tmp) = test_store();
        let mut mem = CoreMemory::new();
        mem.set_block(MemoryBlock::read_only("persona", "read only", 2000));

        store.save(&mem).unwrap();
        let loaded = store.load().unwrap().unwrap();

        assert!(loaded.blocks().get("persona").unwrap().read_only);
    }

    #[test]
    fn save_updates_existing_blocks() {
        let (store, _tmp) = test_store();
        let mut mem = CoreMemory::new();
        mem.set_block(MemoryBlock::new("test", "version1", 500));
        store.save(&mem).unwrap();

        let mut mem2 = CoreMemory::new();
        mem2.set_block(MemoryBlock::new("test", "version2", 500));
        store.save(&mem2).unwrap();

        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.get("test"), Some("version2"));
    }
}
