//! Consumer-owned admission boundary for Goal attempt storage.

pub use contracts::GoalId;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("goal attempt storage admission failed: {message}")]
pub struct GoalStorageAdmissionError {
    message: String,
}

impl GoalStorageAdmissionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Owns the lifecycle of an opaque host storage reservation.
///
/// Application policy identifies reservations only by Goal. The adapter keeps
/// concrete quota classes, filesystem accounting, and reservation handles.
pub trait GoalStorageAdmissionPort: Send + Sync {
    fn admit(&self, goal_id: GoalId) -> Result<(), GoalStorageAdmissionError>;
    fn release(&self, goal_id: GoalId);
}
