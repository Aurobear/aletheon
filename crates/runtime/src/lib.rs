//! External runtime capability manifests, deterministic selection, and the
//! Runtime owner contract (RA-01).

pub mod command;
pub mod error;
pub mod event;
pub mod ids;
pub mod journal;
pub mod manifest;
pub mod ports;
pub mod query;
pub mod selector;
pub mod session_authority;

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
pub use ports::{RuntimeCommandPort, RuntimeEventPort, RuntimeQueryPort};
pub use query::{AgentRunQuery, RuntimeQuery, SessionSnapshotQuery};
pub use selector::{
    RuntimeCandidateRejection, RuntimeSelectionDecision, RuntimeSelectionError,
    RuntimeSelectionRequest, RuntimeSelector,
};
pub use session_authority::{ContextWorkingSet, SessionAuthority, TurnProjection};
