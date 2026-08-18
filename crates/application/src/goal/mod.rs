//! Goal vertical slice: command/query service and consumer-owned repository port.

pub mod admission;
pub mod advance;
pub mod attempt;
mod attempt_coordinator;
pub mod attempt_persistence;
pub mod budget;
pub mod coding;
pub mod coordinator;
pub mod repository;
pub mod service;
pub mod stimulus;

pub use admission::{GoalId, GoalStorageAdmissionError, GoalStorageAdmissionPort};
pub use advance::{
    GoalAdvanceError, GoalAdvanceRepository, GoalAdvanceService, GoalAttemptUseCase,
};
pub use attempt::{
    AttemptCoordinationOutcome, AttemptCoordinatorError, AttemptRequest, CodingVerifier,
    CodingWorktreePort, GoalProgressPort,
};
pub use attempt_coordinator::AttemptCoordinator;
pub use attempt_persistence::{
    AttemptContext, BeginAttemptCommand, BegunAttempt, GoalAttemptPersistencePort,
};
pub use budget::{GoalBudgetRequest, GoalBudgetReservation};
pub use coding::{PersistedCodingJob, PersistedVerificationReport};
pub use coordinator::{
    GoalCoordinationError, GoalCoordinator, GoalCoordinatorRepository, GoalTickOutcome,
};
pub use repository::{GoalRepository, GoalRepositoryError, LegacyObjectiveDetail, LegacyResume};
pub use service::{GoalAction, GoalService, GoalServiceError, GoalUseCases};
pub use stimulus::{ExternalEventWaitCondition, GoalExternalEvent};
