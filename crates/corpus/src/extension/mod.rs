//! Extension platform: manifest parsing, package inspection, store, and validation.
//!
//! This module implements Phase 2 of the Aletheon extension platform:
//! Package inspection, validation, safe extraction, and content-addressed storage.
//! It does NOT activate extensions — activation is handled by the executive crate.

pub mod inspector;
pub mod manifest;
pub mod store;
pub mod validation;

// Legacy import support (stub for Phase 6)
pub mod legacy {
    //! Stub — legacy import will be implemented in Phase 6.
}
