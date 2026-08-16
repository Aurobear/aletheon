//! Durable approvals for protected Goal operations.

mod apply_coordinator;

pub use apply_coordinator::{
    ApplyCoordinationError, ApplyCoordinationOutcome, ApplyCoordinator, ApplyCoordinatorConfig,
    GitManagedWorktreeCleaner, ManagedWorktreeCleaner,
};

pub use adapters_sqlite::approval_repository::{
    ApprovalApplyClaim, ApprovalApplyOperation, ApprovalApplyReceipt, ApprovalChannelPolicy,
    ApprovalCreate, ApprovalDecision, ApprovalDelivery, ApprovalRepository,
    ApprovalRepositoryError, ApprovalResolutionContext,
};
