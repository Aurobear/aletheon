//! # Aletheon Memory
//!
//! SQLite-backed implementations of the `MemoryBackend` trait.
//! EpisodicMemory is always available (used by the daemon for reflections).
//! Cognitive backends (MemoryRouter + semantic/procedural/self) are behind the
//! off-by-default `cognitive-memory` feature (M-H Option A).

mod adapters;
pub mod agent_scope;
mod application;
mod backends;
pub mod composite_service;
pub mod consolidation;
pub mod credential;
mod domain;
pub mod embodied_episode;
pub mod fact_service;
mod host;
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
    CandidateDecisionLabel, ConsolidationJobState, LatencySamples, MemoryKindLabel, MemoryMetrics,
    MemoryMetricsSnapshot, MemoryScopeLabel, RecallOmittedReason, RecallSourceLabel,
    SupplementalDegradedCategory, TombstoneDestination,
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

pub use ops::{apply_access_boost, compute_strength, should_forget};
pub use ops::{compute_activation, ActivationEntry};

// Cognitive exports (off by default)
#[cfg(feature = "cognitive-memory")]
pub use ops::{
    ConsolidationConfig, ConsolidationResult, MemoryContext, MemoryRouter, ReflectionSummary,
    SkillSummary,
};

pub use ops::activation;
pub use ops::decay;
pub use ops::schema;

#[cfg(feature = "cognitive-memory")]
pub use ops::consolidation;
#[cfg(feature = "cognitive-memory")]
pub use ops::router;

/// Composition-only local memory runtime.
///
/// These concrete handles are intentionally separated from request-facing
/// memory contracts. Hosts may construct them and inject `MemoryService` or
/// `FactUseCases`; application code must depend on those stable traits.
pub mod runtime {
    pub use crate::adapters::storage::fact_store::FactStore;
    pub use crate::adapters::storage::recall_memory::RecallMemory;
    pub use crate::application::compressor::budget::{
        BudgetAction, ContextBudgetInput, ContextBudgetPlan, ContextBudgetPlanner,
    };
    pub use crate::application::compressor::AdvancedCompressor;
    pub use crate::backends::EpisodicMemory;
    #[cfg(feature = "cognitive-memory")]
    pub use crate::backends::{ProceduralMemory, SelfMemory, SemanticMemory};
    pub use crate::domain::core_memory::{CoreMemory, MemoryBlock};
}

/// Stable supplemental-memory contracts and durable local outbox facade.
///
/// The remote transport implementation is supplied by the host; Mnemosyne
/// owns the product-neutral contracts, reconciliation rules, and spool.
pub mod supplemental {
    pub use crate::backends::supplemental::*;

    pub mod config {
        pub use crate::backends::supplemental::config::*;
    }

    pub mod page {
        pub use crate::backends::supplemental::page::*;
    }
}

/// Host tool adapters for the local memory runtime.
pub mod memory_tools {
    pub use crate::host::tools::*;
}

#[cfg(test)]
pub mod testing;
