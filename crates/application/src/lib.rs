//! Aletheon minimal Application use-case facade (APX-01).
//!
//! Pure use-case types, typed errors and the Session/Turn/Delegate facade.
//! The Application does **not** hold a Runtime repository and does **not** mint
//! a core ID — it forwards typed `runtime` commands/queries to the Runtime
//! port and translates results.  Depends only on `contracts` (fabric ownerless
//! primitives) and `runtime`.  No concrete adapter, no Executive.

pub mod error;
pub mod use_case;

pub use error::ApplicationError;
pub use use_case::{
    ApplicationFacade, CreateSession, DeleteSession, ForkSession, GetSession, ListSessions,
    ResumeSession,
};
