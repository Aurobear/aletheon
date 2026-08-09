//! Aletheon minimal Application use-case facade (APX-01).
//!
//! Pure use-case types, typed errors and the Session/Turn/Delegate facade.
//! The Application does **not** hold a Runtime repository and does **not** mint
//! a core ID — it forwards typed `runtime` commands/queries to the Runtime
//! port and translates results.  Depends only on `contracts` (fabric ownerless
//! primitives) and `runtime`.  No concrete adapter, no Executive.

pub mod approval;
pub mod cache;
pub mod error;
pub mod extension;
pub mod goal_draft;
pub mod use_case;

pub use approval::{
    resolve_decision, ApprovalError, ApprovalRecord, ApprovalScope, ApprovalStore,
    ApprovingPrincipal, DecisionRequestId, OpaqueApprovalGrant,
};
pub use cache::{decide_cache, CacheDecision, CacheKey, CacheLayer, PromptConstructionProfile};
pub use error::ApplicationError;
pub use extension::{
    ExtensionFlags, ExtensionId, ExtensionPort, ExtensionRegistration, InMemoryExtensionRegistry,
};
pub use goal_draft::{create_goal_draft, ingest_external_stimulus, ExternalStimulus, GoalDraft};
pub use use_case::{
    ApplicationFacade, CreateSession, DeleteSession, ForkSession, GetSession, ListSessions,
    ResumeSession,
};
