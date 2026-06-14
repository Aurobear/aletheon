pub mod compaction;
pub mod compressor;
pub mod core_memory;
pub mod core_memory_store;
pub mod recall_memory;
pub mod archival_memory;
pub mod memory_pipeline;
pub mod scope;
pub mod budget;
pub mod tools;
pub mod vector_store;
pub mod pipeline;

pub use compaction::CompactionManager;
pub use compressor::AdvancedCompressor;
pub use core_memory::{CoreMemory, MemoryBlock};
pub use memory_pipeline::{
    ExtractionResult as MemoryExtractionResult, ExtractedFact, FactCategory, MemoryPipeline,
    MemoryPipelineConfig,
};
pub use archival_memory::{ArchivalMemory, ArchivalEntry, VectorArchival, InMemoryArchival};
pub use scope::{
    MemoryScope, PendingWrite, PendingWriteType, RecallScopeFilter, RetentionPolicy,
    ScopeFilter, ScopedCoreMemory, ScopedMemoryBlock, ScopedRecallFilter, Scratchpad,
    ScratchpadEntry, WriteOutcome, scope_metadata,
};
