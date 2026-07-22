//! # Aletheon Memory
//!
//! SQLite-backed implementations of the `MemoryBackend` trait.
//! EpisodicMemory is always available (used by the daemon for reflections).
//! Cognitive backends (MemoryRouter + semantic/procedural/self) are behind the
//! off-by-default `cognitive-memory` feature (M-H Option A).

pub mod agent_scope;
pub mod backends;
pub mod composite_service;
pub mod consolidation;
pub mod credential;
pub mod embodied_episode;
pub mod fact_service;
pub mod r#impl;
pub mod knowledge_graph;
pub mod lifecycle;
pub mod model;
pub mod observability;
pub mod ops;
pub mod projection;
pub mod promotion;
mod recall;
pub mod retention;
pub mod service;

pub use agent_scope::{AgentMemoryContext, AgentMemoryVault, ChildMemoryDraft};
pub use composite_service::{
    CompositeMemoryHealth, CompositeMemoryService, SupplementalMemoryService,
};
pub use fact_service::{
    AddFactRequest, DefaultFactUseCases, FactServiceError, FactUseCases, FactView,
    ListFactsRequest, SearchFactsRequest,
};
pub use promotion::{MemoryPromotionReceipt, MemoryPromotionRequest, PromotionDecision};

// MemoryService facade (docs/arch §11). NOTE: `MemoryScope` from `service` is
// intentionally not re-exported here — it would collide with the existing
// multi-agent `MemoryScope` re-exported below (`r#impl::core_memory::scope`).
// Reach the facade's scope type via `mnemosyne::service::MemoryScope`.
pub use model::{
    MemoryAuthority, MemoryKind, MemoryMetadata, MemoryProvenance, MemoryRecord, MemoryRecordId,
    MemoryScope, MemorySensitivity, MemoryStatus, ScopeAncestry, TemporalState,
};
pub use observability::{
    CandidateDecisionLabel, ConsolidationJobState, SupplementalDegradedCategory, LatencySamples,
    MemoryKindLabel, MemoryMetrics, MemoryMetricsSnapshot, MemoryScopeLabel, RecallOmittedReason,
    RecallSourceLabel, TombstoneDestination,
};
pub use projection::{
    DefaultMemoryWorkspaceProjector, MemoryCandidateContext, MemoryProjection,
    MemoryProjectionLimits, MemoryWorkspaceProjector, ProjectedMemory,
};
pub use recall::pipeline::{
    hybrid_recall, DegradedSource, HybridRecallBackends, LastValidSnapshotBackend,
    RankedRecallItem, RecallPreFilter, RecallSearchBackend, RecallSearchParams, ScopePredicate,
    SearchOutcome,
};
pub use retention::{
    RetentionCompactionPolicy, RetentionCompactionReport, RetentionCompactor, RetentionRepository,
};
pub use service::{
    DefaultMemoryService, ExperienceEvent, ForgetAuthority, ForgetPolicy, ForgetReceipt,
    ForgetSelector, MemoryService, RecallItem, RecallRequest, RecallSet, SynthesisCitation,
    SynthesisContextBlock, SynthesisGap, SynthesisRequest, SynthesisResult,
};

// Wave 1: Recall pipeline enhancements
pub use recall::autocut::{apply_autocut, AutocutDecision};
pub use recall::evidence::{stamp_evidence, CreateSafety, EvidenceLevel};
pub use recall::pipeline::{QueryIntent, RecallMode, RecallModeBundle};

// Wave 2: Zero-LLM knowledge graph
pub use knowledge_graph::{
    extract_entities_from_content, infer_relations, Entity, EntityId, EntityType, KnowledgeGraph,
    Relation, RelationType,
};

// Wave 3: Synthesis model trait (feature-gated)
#[cfg(feature = "llm-synthesis")]
pub use service::SynthesisModel;

// Always-available exports
pub use backends::EpisodicMemory;
pub use ops::{apply_access_boost, compute_strength, should_forget};
pub use ops::{compute_activation, ActivationEntry};

// Cognitive exports (off by default)
#[cfg(feature = "cognitive-memory")]
pub use backends::{ProceduralMemory, SelfMemory, SemanticMemory};
#[cfg(feature = "cognitive-memory")]
pub use ops::{
    ConsolidationConfig, ConsolidationResult, MemoryContext, MemoryRouter, ReflectionSummary,
    SkillSummary,
};

// Sub-module re-exports for direct path access
pub use backends::episodic;
pub use ops::activation;
pub use ops::decay;
pub use ops::schema;

#[cfg(feature = "cognitive-memory")]
pub use backends::procedural;
#[cfg(feature = "cognitive-memory")]
pub use backends::self_memory;
#[cfg(feature = "cognitive-memory")]
pub use backends::semantic;
#[cfg(feature = "cognitive-memory")]
pub use ops::consolidation;
#[cfg(feature = "cognitive-memory")]
pub use ops::router;

// Re-exports from impl (migrated from runtime, Group B Phase 2)
pub use r#impl::auto_memory::AutoMemory;
pub use r#impl::compaction::CompactionManager;
pub use r#impl::compressor::AdvancedCompressor;
pub use r#impl::core_memory::{CoreMemory, MemoryBlock};
pub use r#impl::fact_store::FactStore;
pub use r#impl::recall_memory::RecallMemory;
pub use r#impl::tools as memory_tools;

#[cfg(test)]
pub mod testing;
