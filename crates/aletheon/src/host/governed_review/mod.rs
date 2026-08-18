//! Durable, bounded governed-review application service.

pub mod ports;
pub mod prompt;
pub mod service;
pub mod store;

pub use ports::{ReviewPort, ReviewPortAdapter};
pub use service::{
    GovernedReviewLimits, GovernedReviewService, ReviewCapabilities, ReviewServiceError,
};
pub use store::{GovernedReviewStore, ReviewStoreError, StoredReview};
