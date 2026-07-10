//! SemanticMemory — knowledge, concepts, facts, with FTS5 keyword search
//! and optional embedding-based vector search.

mod query;
mod schema;
mod storage;

pub use schema::{HashEmbeddingProvider, SemanticMemory};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use fabric::{
        CompactStrategy, EmbeddingProvider, MemoryBackend, MemoryEntry, MemoryFilter, MemoryQuery,
        MemoryType, Subsystem, SubsystemContext,
    };
    use std::sync::Arc;
    use uuid::Uuid;

    use schema::{cosine_similarity, hash_embedding, l2_norm, VectorIndex};

    fn setup() -> (tempfile::NamedTempFile, SemanticMemory) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mem = SemanticMemory::new(tmp.path().to_path_buf());
        (tmp, mem)
    }

    fn setup_with_embedding() -> (tempfile::NamedTempFile, SemanticMemory) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let provider = Arc::new(HashEmbeddingProvider::new(32));
        let mem = SemanticMemory::with_embedding_provider(tmp.path().to_path_buf(), provider);
        (tmp, mem)
    }

    async fn init_mem(mem: &mut SemanticMemory) {
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: std::env::temp_dir(),
            config: serde_json::Value::Null,
            bus: std::sync::Arc::new(fabric::CommunicationBus::new()),
        };
        mem.init(&ctx).await.unwrap();
    }

    fn make_entry(content: &[u8]) -> MemoryEntry {
        MemoryEntry {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            content: content.to_vec(),
            tags: vec!["knowledge".into()],
            created_at: Utc::now(),
            access_count: 0,
            importance: 0.8,
            decay_rate: 0.0,
            associations: vec![],
        }
    }

    #[tokio::test]
    async fn test_semantic_store_and_fts_search() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"Rust is a systems programming language"))
            .await
            .unwrap();
        mem.store(make_entry(b"Python is a scripting language"))
            .await
            .unwrap();

        let query = MemoryQuery {
            text: Some("systems programming".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(String::from_utf8_lossy(&results[0].content).contains("Rust"));
    }

    #[tokio::test]
    async fn test_semantic_search_ranking() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"memory management in Rust is critical"))
            .await
            .unwrap();
        mem.store(make_entry(b"Rust uses ownership for resource management"))
            .await
            .unwrap();

        let query = MemoryQuery {
            text: Some("memory".into()),
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(!results.is_empty());
        assert!(String::from_utf8_lossy(&results[0].content).contains("memory management"));
    }

    #[tokio::test]
    async fn test_semantic_merge_similar() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let mut e1 = make_entry(b"Same Title\nVersion A");
        e1.importance = 0.9;
        let mut e2 = make_entry(b"Same Title\nVersion B");
        e2.importance = 0.5;
        mem.store(e1).await.unwrap();
        mem.store(e2).await.unwrap();

        let result = mem
            .compact(CompactStrategy::MergeSimilar {
                similarity_threshold: 0.9,
            })
            .await
            .unwrap();
        assert_eq!(result.entries_before, 2);
        assert_eq!(result.entries_after, 1);
    }

    #[tokio::test]
    async fn test_semantic_forget() {
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        let entry = make_entry(b"forgettable knowledge");
        let handle = mem.store(entry).await.unwrap();
        mem.forget(&handle).await.unwrap();

        let stats = mem.stats().await.unwrap();
        assert_eq!(stats.total_entries, 0);
    }

    // --- Vector search tests ---

    #[tokio::test]
    async fn test_vector_index_cosine_similarity() {
        let index = VectorIndex::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        // Two similar vectors and one different
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![0.9, 0.1, 0.0]; // similar to v1
        let v3 = vec![0.0, 0.0, 1.0]; // orthogonal to v1

        index.upsert(id1, v1.clone());
        index.upsert(id2, v2);
        index.upsert(id3, v3);

        let results = index.search(&v1, 3);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, id1);
        assert!((results[0].1 - 1.0).abs() < 0.001);
        assert_eq!(results[1].0, id2);
        assert!(results[1].1 > 0.8);
        assert_eq!(results[2].0, id3);
        assert!(results[2].1 < 0.1);
    }

    #[tokio::test]
    async fn test_vector_index_top_k() {
        let index = VectorIndex::new();
        let query = vec![1.0, 0.0, 0.0];

        for _ in 0..10 {
            index.upsert(Uuid::new_v4(), vec![0.5, 0.5, 0.0]);
        }

        let results = index.search(&query, 3);
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_vector_index_upsert_updates() {
        let index = VectorIndex::new();
        let id = Uuid::new_v4();

        index.upsert(id, vec![1.0, 0.0, 0.0]);
        assert_eq!(index.len(), 1);

        // Upsert same ID with different embedding
        index.upsert(id, vec![0.0, 1.0, 0.0]);
        assert_eq!(index.len(), 1);

        let results = index.search(&[0.0, 1.0, 0.0], 1);
        assert!((results[0].1 - 1.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_vector_index_remove() {
        let index = VectorIndex::new();
        let id = Uuid::new_v4();
        index.upsert(id, vec![1.0, 0.0]);
        assert_eq!(index.len(), 1);

        index.remove(&id);
        assert_eq!(index.len(), 0);
    }

    #[tokio::test]
    async fn test_hash_embedding_deterministic() {
        let provider = HashEmbeddingProvider::new(32);
        let e1 = provider.embed("hello world").await.unwrap();
        let e2 = provider.embed("hello world").await.unwrap();
        assert_eq!(e1, e2);
        assert_eq!(e1.len(), 32);
    }

    #[tokio::test]
    async fn test_hash_embedding_different_texts() {
        let provider = HashEmbeddingProvider::new(32);
        let e1 = provider.embed("hello").await.unwrap();
        let e2 = provider.embed("world").await.unwrap();
        // Different texts should produce different embeddings
        assert_ne!(e1, e2);
    }

    #[tokio::test]
    async fn test_semantic_store_and_vector_search() {
        let (_tmp, mut mem) = setup_with_embedding();
        init_mem(&mut mem).await;

        let e1 = make_entry(b"Rust is a systems programming language");
        let e2 = make_entry(b"Python is a scripting language");
        let id1 = e1.id;
        mem.store(e1).await.unwrap();
        mem.store(e2).await.unwrap();

        // Direct vector search should find the stored entries
        let query_text = "Rust is a systems programming language";
        let provider = HashEmbeddingProvider::new(32);
        let query_emb = provider.embed(query_text).await.unwrap();

        let results = mem.search_by_embedding(&query_emb, 10).await.unwrap();
        assert!(!results.is_empty());
        // The exact match should be first
        assert_eq!(results[0].id, id1);
    }

    #[tokio::test]
    async fn test_recall_with_semantic_field() {
        let (_tmp, mut mem) = setup_with_embedding();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"quantum computing basics"))
            .await
            .unwrap();
        mem.store(make_entry(b"classical algorithms overview"))
            .await
            .unwrap();

        // Use recall with semantic field set
        let provider = HashEmbeddingProvider::new(32);
        let query_emb = provider.embed("quantum computing basics").await.unwrap();

        let query = MemoryQuery {
            semantic: Some(query_emb),
            limit: 5,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(!results.is_empty());
        assert!(String::from_utf8_lossy(&results[0].content).contains("quantum"));
    }

    #[tokio::test]
    async fn test_recall_semantic_fallback_to_fts() {
        // Without embedding provider, semantic field is ignored, falls back to FTS
        let (_tmp, mut mem) = setup();
        init_mem(&mut mem).await;

        mem.store(make_entry(b"Rust memory safety guarantees"))
            .await
            .unwrap();

        let query = MemoryQuery {
            text: Some("memory".into()),
            semantic: Some(vec![1.0, 0.0]), // will be ignored (no provider)
            limit: 10,
            ..Default::default()
        };
        let results = mem.recall(&query).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_forget_removes_from_vector_index() {
        let (_tmp, mut mem) = setup_with_embedding();
        init_mem(&mut mem).await;

        let entry = make_entry(b"forgettable with embedding");
        let handle = mem.store(entry).await.unwrap();

        // Verify vector index has the entry
        let results = mem
            .search_by_embedding(&hash_embedding("forgettable with embedding", 32), 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        // Forget
        mem.forget(&handle).await.unwrap();

        // Verify vector index no longer has the entry
        let results = mem
            .search_by_embedding(&hash_embedding("forgettable with embedding", 32), 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_cosine_similarity_basic() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let a_norm = l2_norm(&a);
        assert!((cosine_similarity(&a, a_norm, &b) - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, a_norm, &c).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 0.0];
        assert_eq!(cosine_similarity(&a, 0.0, &b), 0.0);
    }

    #[test]
    fn test_l2_norm() {
        assert!((l2_norm(&[3.0, 4.0]) - 5.0).abs() < 0.001);
        assert_eq!(l2_norm(&[0.0, 0.0]), 0.0);
    }
}
