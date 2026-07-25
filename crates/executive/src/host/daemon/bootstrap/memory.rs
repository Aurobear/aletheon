//! Typed construction unit for daemon memory stores.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use fabric::Clock;
use mnemosyne::runtime::{CoreMemory, FactStore, RecallMemory};
use tokio::sync::Mutex;

pub(super) struct MemoryCompositionInput<'a> {
    pub(super) data_dir: &'a Path,
    pub(super) clock: Arc<dyn Clock>,
}

pub(super) struct MemoryComposition {
    pub(super) core: Arc<Mutex<CoreMemory>>,
    pub(super) recall: Arc<Mutex<RecallMemory>>,
    pub(super) facts: Arc<Mutex<FactStore>>,
}

pub(super) fn compose(input: MemoryCompositionInput<'_>) -> anyhow::Result<MemoryComposition> {
    let core_path = input.data_dir.join("core_memory.json");
    let core = Arc::new(Mutex::new(CoreMemory::load_or_default(&core_path)));
    let recall = Arc::new(Mutex::new(RecallMemory::new(
        &input.data_dir.join("recall_memory.db"),
        input.clock,
    )?));
    let fact_root = input.data_dir.join("mnemosyne");
    std::fs::create_dir_all(&fact_root)
        .with_context(|| format!("creating fact store root: {}", fact_root.display()))?;
    let facts = Arc::new(Mutex::new(
        FactStore::open(&fact_root.join("fact_store.db")).context("opening fact store")?,
    ));

    Ok(MemoryComposition {
        core,
        recall,
        facts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_memory_stores_under_injected_data_root() {
        let root = tempfile::tempdir().unwrap();
        let composition = compose(MemoryCompositionInput {
            data_dir: root.path(),
            clock: Arc::new(kernel::chronos::TestClock::new(100, 0)),
        })
        .unwrap();

        assert!(root.path().join("recall_memory.db").exists());
        assert!(root.path().join("mnemosyne/fact_store.db").exists());
        drop(composition);
    }

    #[test]
    fn reports_fact_root_construction_failure() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("mnemosyne"), "occupied").unwrap();

        assert!(compose(MemoryCompositionInput {
            data_dir: root.path(),
            clock: Arc::new(kernel::chronos::TestClock::new(100, 0)),
        })
        .is_err());
    }
}
