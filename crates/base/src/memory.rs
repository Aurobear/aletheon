//! Memory trait — like Linux kernel's VFS.
//!
//! Unified memory abstraction with multiple backends (episodic, semantic,
//! procedural, self). Like VFS provides a single interface over ext4,
//! tmpfs, procfs, etc.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::subsystem::Subsystem;

/// Memory type — the four memory subsystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryType {
    /// Episodic — what happened, when, what was done, outcome.
    Episodic,
    /// Semantic — knowledge, concepts, facts, documentation.
    Semantic,
    /// Procedural — reusable skills, workflows, patterns.
    Procedural,
    /// Self — identity changes, boundary decisions, care evolution, mutation history.
    SelfMemory,
}

/// A memory entry — the fundamental unit of memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique identifier.
    pub id: Uuid,
    /// Memory type (which backend stores this).
    pub memory_type: MemoryType,
    /// Content (serialized bytes — could be text, JSON, binary).
    pub content: Vec<u8>,
    /// Tags for categorization and retrieval.
    pub tags: Vec<String>,
    /// When this memory was created.
    pub created_at: DateTime<Utc>,
    /// How many times this memory has been accessed.
    pub access_count: u64,
    /// Importance score (0.0 to 1.0). Higher = harder to forget.
    pub importance: f64,
    /// Decay rate — how fast this memory fades (0.0 = permanent).
    pub decay_rate: f64,
    /// IDs of related memories.
    pub associations: Vec<Uuid>,
}

/// Handle to a stored memory — used for retrieval and deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHandle {
    pub id: Uuid,
    pub memory_type: MemoryType,
}

/// Query for memory retrieval.
#[derive(Debug, Clone, Default)]
pub struct MemoryQuery {
    /// Text search (keyword matching).
    pub text: Option<String>,
    /// Semantic vector search (embedding similarity).
    pub semantic: Option<Vec<f32>>,
    /// Time range filter.
    pub time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// Tag filter (any match).
    pub tags: Option<Vec<String>>,
    /// Memory type filter.
    pub memory_type: Option<MemoryType>,
    /// Maximum results to return.
    pub limit: usize,
    /// Minimum importance threshold.
    pub min_importance: Option<f64>,
}

/// Filter for listing memories.
#[derive(Debug, Clone, Default)]
pub struct MemoryFilter {
    pub prefix: Option<String>,
    pub memory_type: Option<MemoryType>,
    pub tags: Option<Vec<String>>,
    pub limit: usize,
}

/// Compaction strategy.
#[derive(Debug, Clone)]
pub enum CompactStrategy {
    /// Remove memories below importance threshold.
    PruneBelowImportance { threshold: f64 },
    /// Keep only the N most important memories.
    KeepTopN { n: usize },
    /// Merge similar memories (dedup).
    MergeSimilar { similarity_threshold: f64 },
    /// Age-based: remove memories older than duration with low access count.
    AgeBased {
        max_age: chrono::Duration,
        min_access_count: u64,
    },
}

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactResult {
    pub entries_before: usize,
    pub entries_after: usize,
    pub entries_removed: usize,
    pub entries_merged: usize,
}

/// Memory statistics.
#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub total_entries: usize,
    pub by_type: std::collections::HashMap<MemoryType, usize>,
    pub total_size_bytes: u64,
    pub oldest_entry: Option<DateTime<Utc>>,
    pub newest_entry: Option<DateTime<Utc>>,
}

/// Trait for text embedding models.
///
/// Implementations convert a text string into a dense float vector
/// suitable for cosine similarity search.  The ABI crate defines only
/// the contract; concrete providers (OpenAI, local models, hash-based
/// fallbacks) live in higher-level crates.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a text string into a vector of floats.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Return the dimension of the embedding vectors.
    fn dimension(&self) -> usize;
}

/// Memory backend trait — like VFS super_operations.
///
/// Each memory type (episodic, semantic, procedural, self) implements
/// this trait. The memory subsystem dispatches to the correct backend.
#[async_trait]
pub trait MemoryBackend: Subsystem {
    /// Store a memory entry.
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle>;

    /// Retrieve memories matching a query.
    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>>;

    /// List memories matching a filter.
    async fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>>;

    /// Delete a memory by handle. May require SelfField approval.
    async fn forget(&self, handle: &MemoryHandle) -> Result<()>;

    /// Compact/defragment memory.
    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult>;

    /// Get memory statistics.
    async fn stats(&self) -> Result<MemoryStats>;
}
