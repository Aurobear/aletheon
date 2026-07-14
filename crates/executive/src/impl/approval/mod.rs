//! Durable approvals for protected Goal operations.

mod repository;

pub use repository::{
    ApprovalChannelPolicy, ApprovalCreate, ApprovalDecision, ApprovalRepository,
    ApprovalRepositoryError, ApprovalResolutionContext,
};
