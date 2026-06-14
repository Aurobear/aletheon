//! MemoryRouter — dispatches to the correct backend by MemoryType.

use std::collections::HashMap;

use aletheon_abi::{
    CompactResult, CompactStrategy, MemoryBackend, MemoryEntry, MemoryFilter, MemoryHandle,
    MemoryQuery, MemoryStats, MemoryType, Subsystem, SubsystemContext, SubsystemHealth, Version,
};
use anyhow::Result;
use async_trait::async_trait;

use crate::episodic::EpisodicMemory;
use crate::procedural::ProceduralMemory;
use crate::semantic::SemanticMemory;
use crate::self_memory::SelfMemory;

/// Routes memory operations to the correct backend.
pub struct MemoryRouter {
    episodic: EpisodicMemory,
    semantic: SemanticMemory,
    procedural: ProceduralMemory,
    self_mem: SelfMemory,
}

impl MemoryRouter {
    /// Create a new router with DB files in the given directory.
    pub fn new(db_dir: &std::path::Path) -> Self {
        Self {
            episodic: EpisodicMemory::new(db_dir.join("episodic.db")),
            semantic: SemanticMemory::new(db_dir.join("semantic.db")),
            procedural: ProceduralMemory::new(db_dir.join("procedural.db")),
            self_mem: SelfMemory::new(db_dir.join("self.db")),
        }
    }

    fn backend_for(&self, mt: MemoryType) -> &dyn MemoryBackend {
        match mt {
            MemoryType::Episodic => &self.episodic,
            MemoryType::Semantic => &self.semantic,
            MemoryType::Procedural => &self.procedural,
            MemoryType::SelfMemory => &self.self_mem,
        }
    }
}

#[async_trait]
impl Subsystem for MemoryRouter {
    fn name(&self) -> &str {
        "memory_router"
    }

    async fn init(&mut self, ctx: &SubsystemContext) -> Result<()> {
        self.episodic.init(ctx).await?;
        self.semantic.init(ctx).await?;
        self.procedural.init(ctx).await?;
        self.self_mem.init(ctx).await?;
        tracing::info!("MemoryRouter initialized — all 4 backends online");
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        let mut degraded = Vec::new();
        for (name, h) in [
            ("episodic", self.episodic.health().await),
            ("semantic", self.semantic.health().await),
            ("procedural", self.procedural.health().await),
            ("self", self.self_mem.health().await),
        ] {
            if h != SubsystemHealth::Healthy {
                degraded.push(name.to_string());
            }
        }
        if degraded.is_empty() {
            SubsystemHealth::Healthy
        } else {
            SubsystemHealth::Degraded {
                reason: format!("backends degraded: {}", degraded.join(", ")),
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.episodic.shutdown().await?;
        self.semantic.shutdown().await?;
        self.procedural.shutdown().await?;
        self.self_mem.shutdown().await?;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}

#[async_trait]
impl MemoryBackend for MemoryRouter {
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle> {
        self.backend_for(entry.memory_type).store(entry).await
    }

    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        if let Some(mt) = query.memory_type {
            return self.backend_for(mt).recall(query).await;
        }

        // No type filter — query all backends and merge
        let mut all = Vec::new();
        all.extend(self.episodic.recall(query).await?);
        all.extend(self.semantic.recall(query).await?);
        all.extend(self.procedural.recall(query).await?);
        all.extend(self.self_mem.recall(query).await?);

        // Sort by importance descending, then limit
        all.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if query.limit > 0 {
            all.truncate(query.limit);
        }
        Ok(all)
    }

    async fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>> {
        if let Some(mt) = filter.memory_type {
            return self.backend_for(mt).list(filter).await;
        }

        let mut all = Vec::new();
        all.extend(self.episodic.list(filter).await?);
        all.extend(self.semantic.list(filter).await?);
        all.extend(self.procedural.list(filter).await?);
        all.extend(self.self_mem.list(filter).await?);

        all.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
        });
        if filter.limit > 0 {
            all.truncate(filter.limit);
        }
        Ok(all)
    }

    async fn forget(&self, handle: &MemoryHandle) -> Result<()> {
        self.backend_for(handle.memory_type).forget(handle).await
    }

    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult> {
        let mut total = CompactResult {
            entries_before: 0,
            entries_after: 0,
            entries_removed: 0,
            entries_merged: 0,
        };
        for backend in [
            &self.episodic as &dyn MemoryBackend,
            &self.semantic,
            &self.procedural,
            &self.self_mem,
        ] {
            let r = backend.compact(strategy.clone()).await?;
            total.entries_before += r.entries_before;
            total.entries_after += r.entries_after;
            total.entries_removed += r.entries_removed;
            total.entries_merged += r.entries_merged;
        }
        Ok(total)
    }

    async fn stats(&self) -> Result<MemoryStats> {
        let episodic = self.episodic.stats().await?;
        let semantic = self.semantic.stats().await?;
        let procedural = self.procedural.stats().await?;
        let self_mem = self.self_mem.stats().await?;

        let mut by_type = HashMap::new();
        by_type.extend(episodic.by_type);
        by_type.extend(semantic.by_type);
        by_type.extend(procedural.by_type);
        by_type.extend(self_mem.by_type);

        let total_entries = by_type.values().sum();
        let total_size_bytes =
            episodic.total_size_bytes + semantic.total_size_bytes + procedural.total_size_bytes + self_mem.total_size_bytes;

        let oldest = [episodic.oldest_entry, semantic.oldest_entry, procedural.oldest_entry, self_mem.oldest_entry]
            .into_iter()
            .flatten()
            .min();
        let newest = [episodic.newest_entry, semantic.newest_entry, procedural.newest_entry, self_mem.newest_entry]
            .into_iter()
            .flatten()
            .max();

        Ok(MemoryStats {
            total_entries,
            by_type,
            total_size_bytes,
            oldest_entry: oldest,
            newest_entry: newest,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    async fn setup_router() -> (tempfile::TempDir, MemoryRouter) {
        let dir = tempfile::tempdir().unwrap();
        let mut router = MemoryRouter::new(dir.path());
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: std::env::temp_dir(),
            config: serde_json::Value::Null,
        };
        router.init(&ctx).await.unwrap();
        (dir, router)
    }

    fn make_entry(mt: MemoryType, content: &[u8]) -> MemoryEntry {
        MemoryEntry {
            id: Uuid::new_v4(),
            memory_type: mt,
            content: content.to_vec(),
            tags: vec!["test".into()],
            created_at: Utc::now(),
            access_count: 0,
            importance: 0.5,
            decay_rate: 0.0,
            associations: vec![],
        }
    }

    #[tokio::test]
    async fn test_router_store_routes_correctly() {
        let (_dir, router) = setup_router().await;

        let h1 = router
            .store(make_entry(MemoryType::Episodic, b"ep"))
            .await
            .unwrap();
        assert_eq!(h1.memory_type, MemoryType::Episodic);

        let h2 = router
            .store(make_entry(MemoryType::Semantic, b"sem"))
            .await
            .unwrap();
        assert_eq!(h2.memory_type, MemoryType::Semantic);

        let h3 = router
            .store(make_entry(MemoryType::Procedural, b"proc"))
            .await
            .unwrap();
        assert_eq!(h3.memory_type, MemoryType::Procedural);

        let h4 = router
            .store(make_entry(MemoryType::SelfMemory, b"self"))
            .await
            .unwrap();
        assert_eq!(h4.memory_type, MemoryType::SelfMemory);
    }

    #[tokio::test]
    async fn test_router_recall_all_types() {
        let (_dir, router) = setup_router().await;

        router
            .store(make_entry(MemoryType::Episodic, b"event"))
            .await
            .unwrap();
        router
            .store(make_entry(MemoryType::Semantic, b"fact"))
            .await
            .unwrap();

        let query = MemoryQuery {
            limit: 100,
            ..Default::default()
        };
        let results = router.recall(&query).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_router_recall_specific_type() {
        let (_dir, router) = setup_router().await;

        router
            .store(make_entry(MemoryType::Episodic, b"event"))
            .await
            .unwrap();
        router
            .store(make_entry(MemoryType::Semantic, b"fact"))
            .await
            .unwrap();

        let query = MemoryQuery {
            memory_type: Some(MemoryType::Episodic),
            limit: 10,
            ..Default::default()
        };
        let results = router.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].memory_type, MemoryType::Episodic);
    }

    #[tokio::test]
    async fn test_router_stats_aggregation() {
        let (_dir, router) = setup_router().await;

        router
            .store(make_entry(MemoryType::Episodic, b"a"))
            .await
            .unwrap();
        router
            .store(make_entry(MemoryType::Semantic, b"b"))
            .await
            .unwrap();
        router
            .store(make_entry(MemoryType::Procedural, b"c"))
            .await
            .unwrap();

        let stats = router.stats().await.unwrap();
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.by_type.len(), 4); // all 4 types always present (SelfMemory=0)
    }
}
