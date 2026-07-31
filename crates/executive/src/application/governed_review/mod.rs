//! Durable, bounded governed-review application service.

pub mod store;

pub use store::{GovernedReviewStore, ReviewStoreError, StoredReview};
