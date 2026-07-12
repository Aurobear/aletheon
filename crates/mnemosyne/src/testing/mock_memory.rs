use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use fabric::{
    wall_to_datetime, CompactResult, CompactStrategy, MemoryBackend, MemoryEntry, MemoryFilter,
    MemoryHandle, MemoryQuery, MemoryStats, MemoryType, Subsystem, SubsystemContext,
    SubsystemHealth, Version, WallTime,
};
use uuid::Uuid;

/// Mock memory backend with in-memory storage.
///
/// Stores `MemoryEntry` objects in a `Vec` and supports recall/list/forget/compact/stats.
/// Useful for unit-testing memory-dependent code without SQLite.
pub struct MockMemoryBackend {
    name: String,
    memory_type: MemoryType,
    entries: Mutex<Vec<MemoryEntry>>,
    initialized: Mutex<bool>,
    clock: Arc<dyn fabric::Clock>,
}

impl MockMemoryBackend {
    pub fn new(
        name: impl Into<String>,
        memory_type: MemoryType,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        Self {
            name: name.into(),
            memory_type,
            entries: Mutex::new(Vec::new()),
            initialized: Mutex::new(false),
            clock,
        }
    }

    /// Create a mock for episodic memory.
    pub fn episodic(clock: Arc<dyn fabric::Clock>) -> Self {
        Self::new("mock_episodic", MemoryType::Episodic, clock)
    }

    /// Create a mock for semantic memory.
    pub fn semantic(clock: Arc<dyn fabric::Clock>) -> Self {
        Self::new("mock_semantic", MemoryType::Semantic, clock)
    }

    /// Create a mock for procedural memory.
    pub fn procedural(clock: Arc<dyn fabric::Clock>) -> Self {
        Self::new("mock_procedural", MemoryType::Procedural, clock)
    }

    /// Create a mock for self memory.
    pub fn self_memory(clock: Arc<dyn fabric::Clock>) -> Self {
        Self::new("mock_self", MemoryType::SelfMemory, clock)
    }

    /// Number of entries currently stored.
    pub fn entry_count(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Get all entries (for assertion).
    pub fn all_entries(&self) -> Vec<MemoryEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// Create a MemoryEntry with the correct memory type.
    pub fn make_entry(&self, content: &[u8], tags: Vec<String>, importance: f64) -> MemoryEntry {
        MemoryEntry {
            id: Uuid::new_v4(),
            memory_type: self.memory_type,
            content: content.to_vec(),
            tags,
            created_at: wall_to_datetime(self.clock.wall_now()),
            access_count: 0,
            importance,
            decay_rate: 0.0,
            associations: vec![],
        }
    }
}

impl Default for MockMemoryBackend {
    fn default() -> Self {
        Self::episodic(Arc::new(aletheon_kernel::chronos::TestClock::default()))
    }
}

#[async_trait]
impl Subsystem for MockMemoryBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        *self.initialized.lock().unwrap() = true;
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        if *self.initialized.lock().unwrap() {
            SubsystemHealth::Healthy
        } else {
            SubsystemHealth::Degraded {
                reason: "not initialized".into(),
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        *self.initialized.lock().unwrap() = false;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}

#[async_trait]
impl MemoryBackend for MockMemoryBackend {
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle> {
        let handle = MemoryHandle {
            id: entry.id,
            memory_type: entry.memory_type,
        };
        self.entries.lock().unwrap().push(entry);
        Ok(handle)
    }

    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        let entries = self.entries.lock().unwrap();
        let mut results: Vec<MemoryEntry> = entries
            .iter()
            .filter(|e| {
                // Text search
                if let Some(ref text) = query.text {
                    let content_str = String::from_utf8_lossy(&e.content);
                    if !content_str.to_lowercase().contains(&text.to_lowercase()) {
                        return false;
                    }
                }
                // Time range filter
                if let Some((start, end)) = &query.time_range {
                    if e.created_at < *start || e.created_at > *end {
                        return false;
                    }
                }
                // Tag filter
                if let Some(ref tags) = query.tags {
                    if !tags.iter().any(|t| e.tags.contains(t)) {
                        return false;
                    }
                }
                // Importance filter
                if let Some(min_imp) = query.min_importance {
                    if e.importance < min_imp {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        // Sort by created_at descending
        #[allow(clippy::unnecessary_sort_by)]
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        if query.limit > 0 {
            results.truncate(query.limit);
        }
        Ok(results)
    }

    async fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>> {
        let entries = self.entries.lock().unwrap();
        let mut results: Vec<MemoryEntry> = entries
            .iter()
            .filter(|e| {
                if let Some(ref tags) = filter.tags {
                    if !tags.iter().any(|t| e.tags.contains(t)) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        #[allow(clippy::unnecessary_sort_by)]
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if filter.limit > 0 {
            results.truncate(filter.limit);
        }
        Ok(results)
    }

    async fn forget(&self, handle: &MemoryHandle) -> Result<()> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|e| e.id != handle.id);
        Ok(())
    }

    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult> {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();

        match strategy {
            CompactStrategy::PruneBelowImportance { threshold } => {
                entries.retain(|e| e.importance >= threshold);
            }
            CompactStrategy::KeepTopN { n } => {
                entries.sort_by(|a, b| {
                    b.importance
                        .partial_cmp(&a.importance)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                entries.truncate(n);
            }
            CompactStrategy::MergeSimilar { .. } => {
                // No-op in mock
            }
            CompactStrategy::AgeBased {
                max_age,
                min_access_count,
            } => {
                let cutoff = wall_to_datetime(WallTime(
                    self.clock.wall_now().0 - max_age.num_milliseconds(),
                ));
                entries.retain(|e| e.created_at >= cutoff || e.access_count >= min_access_count);
            }
        }

        let after = entries.len();
        Ok(CompactResult {
            entries_before: before,
            entries_after: after,
            entries_removed: before - after,
            entries_merged: 0,
        })
    }

    async fn stats(&self) -> Result<MemoryStats> {
        let entries = self.entries.lock().unwrap();
        let total_size = entries.iter().map(|e| e.content.len() as u64).sum();
        let oldest = entries.iter().map(|e| e.created_at).min();
        let newest = entries.iter().map(|e| e.created_at).max();

        let mut by_type = HashMap::new();
        by_type.insert(self.memory_type, entries.len());

        Ok(MemoryStats {
            total_entries: entries.len(),
            by_type,
            total_size_bytes: total_size,
            oldest_entry: oldest,
            newest_entry: newest,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    async fn setup_backend() -> MockMemoryBackend {
        let mut backend =
            MockMemoryBackend::episodic(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: PathBuf::from("/tmp"),
            config: json!({}),
            bus: None,
        };
        backend.init(&ctx).await.unwrap();
        backend
    }

    #[tokio::test]
    async fn test_mock_backend_store_and_recall() {
        let backend = setup_backend().await;

        let entry = backend.make_entry(b"hello world", vec!["test".into()], 0.7);
        let handle = backend.store(entry).await.unwrap();
        assert_eq!(handle.memory_type, MemoryType::Episodic);

        let query = MemoryQuery {
            text: Some("hello".into()),
            limit: 10,
            ..Default::default()
        };
        let results = backend.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, b"hello world");
    }

    #[tokio::test]
    async fn test_mock_backend_forget() {
        let backend = setup_backend().await;

        let entry = backend.make_entry(b"to forget", vec![], 0.5);
        let handle = backend.store(entry).await.unwrap();
        assert_eq!(backend.entry_count(), 1);

        backend.forget(&handle).await.unwrap();
        assert_eq!(backend.entry_count(), 0);
    }

    #[tokio::test]
    async fn test_mock_backend_compact() {
        let backend = setup_backend().await;

        for i in 0..5 {
            let entry =
                backend.make_entry(format!("entry_{}", i).as_bytes(), vec![], 0.1 * i as f64);
            backend.store(entry).await.unwrap();
        }

        let result = backend
            .compact(CompactStrategy::PruneBelowImportance { threshold: 0.3 })
            .await
            .unwrap();
        assert_eq!(result.entries_before, 5);
        assert!(result.entries_after < 5);
    }

    #[tokio::test]
    async fn test_mock_backend_stats() {
        let backend = setup_backend().await;

        backend
            .store(backend.make_entry(b"a", vec![], 0.5))
            .await
            .unwrap();
        backend
            .store(backend.make_entry(b"bb", vec![], 0.5))
            .await
            .unwrap();

        let stats = backend.stats().await.unwrap();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.total_size_bytes, 3); // 1 + 2
    }

    #[tokio::test]
    async fn test_mock_backend_list() {
        let backend = setup_backend().await;

        backend
            .store(backend.make_entry(b"a", vec!["tag1".into()], 0.5))
            .await
            .unwrap();
        backend
            .store(backend.make_entry(b"b", vec!["tag2".into()], 0.5))
            .await
            .unwrap();

        let filter = MemoryFilter {
            tags: Some(vec!["tag1".into()]),
            limit: 10,
            ..Default::default()
        };
        let results = backend.list(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, b"a");
    }
}
