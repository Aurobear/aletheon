//! Durable external account bindings and grant lifecycle.

mod repository;

pub use repository::{
    ExternalCredentialRevoker, ExternalIdentityRepository, ExternalRepositoryError,
    ExternalRevocationOutcome,
};
