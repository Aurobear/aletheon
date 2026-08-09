//! External runtime capability manifests, deterministic selection, and the
//! Runtime owner contract (RA-01).

pub mod agent_supervisor;
pub mod command;
pub mod error;
pub mod event;
pub mod ids;
pub mod journal;
pub mod manifest;
pub mod orchestration;
pub mod per_turn_scope;
pub mod ports;
pub mod query;
pub mod selector;
pub mod session_authority;
pub mod session_head;
pub mod turn_outcome;
pub mod turn_reducer;

pub use agent_supervisor::{
    AgentSupervisorSeam, DelegateBackend, DelegateBackendId, DelegateBackendRegistry,
    DelegateReceipt, DelegateSpawnRequest,
};
pub use command::{
    CancelTurnCommand, CommandReceipt, CreateSessionCommand, ResumeSessionCommand, RuntimeCommand,
    SpawnAgentRunCommand, StartTurnCommand,
};
pub use error::RuntimeError;
pub use event::{RuntimeEvent, TurnTerminal};
pub use ids::{AgentRunId, Generation, SessionId, TurnId};
pub use journal::{RuntimeJournalShadow, ShadowEntry, ShadowMismatch, StreamKind};
pub use manifest::{
    InteractionMode, RuntimeCapability, RuntimeManifest, RuntimeResourceRequirements, TaskEncoding,
    ToolGovernance, WorkspaceMode, MAX_RUNTIME_STORAGE_BYTES, MAX_RUNTIME_STORAGE_ITEMS,
};
pub use orchestration::{
    EvidenceDrivenController, EvidenceGap, OrchestrationStep, Stage, TaskRisk, TransitionReason,
};
pub use per_turn_scope::{PerTurnScope, ScopeExit, ScopeGuard, ScopedResource};
pub use ports::{RuntimeCommandPort, RuntimeEventPort, RuntimeQueryPort};
pub use query::{AgentRunQuery, RuntimeQuery, SessionSnapshotQuery};
pub use selector::{
    RuntimeCandidateRejection, RuntimeSelectionDecision, RuntimeSelectionError,
    RuntimeSelectionRequest, RuntimeSelector,
};
pub use session_authority::{ContextWorkingSet, SessionAuthority, TurnProjection};
pub use session_head::{PendingAppend, SessionHead, SessionHeadIndex};
pub use turn_outcome::{
    BlockReason, CancelReason, StopReason, TurnExecutionResult, TurnFailure, TurnOutcome, TurnUsage,
};
pub use turn_reducer::{TransitionOutcome, TurnReducerSeam, TurnTransition};
