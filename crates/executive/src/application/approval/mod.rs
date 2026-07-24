//! Durable approvals for protected Goal operations.

mod apply_coordinator;
mod repository;

pub use apply_coordinator::{
    ApplyCoordinationError, ApplyCoordinationOutcome, ApplyCoordinator, ApplyCoordinatorConfig,
    GitManagedWorktreeCleaner, ManagedWorktreeCleaner,
};

pub use repository::{
    ApprovalApplyClaim, ApprovalApplyOperation, ApprovalApplyReceipt, ApprovalChannelPolicy,
    ApprovalCreate, ApprovalDecision, ApprovalDelivery, ApprovalRepository,
    ApprovalRepositoryError, ApprovalResolutionContext,
};
