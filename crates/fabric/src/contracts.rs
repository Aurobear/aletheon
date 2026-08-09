//! D1 contracts seed — ownerless value primitives (Agent Kernel V2).
//!
//! This module is the **gated** ownerless-primitive seed for the future
//! `contracts` crate (AK2-25 mechanical rename).  It re-exposes only the
//! ownerless value semantics that already live in `fabric::types`: ID wrappers
//! and version primitives.  It carries **no** rich aggregate, repository,
//! service, policy, live permit, state machine, UI/wire model, or workflow —
//! any symbol that is not an ownerless value primitive must stay out of here.
//!
//! Constraints honoured (implementation-plan §11.1 / §10.3):
//! - no package rename in this commit;
//! - `contracts` depends on nothing (it only re-exports within `fabric`);
//! - Fabric rich public surface does **not** grow (pure re-export, no new type).

pub use crate::types::admission::{PermitId, PrincipalId};
pub use crate::types::attempt::RuntimeId;
pub use crate::types::channel::MessageId;
pub use crate::types::operation::{OperationId, ProcessId};
pub use crate::types::process::{AgentId, NamespaceId};
pub use crate::types::session::TurnId;
pub use crate::types::space::SessionId;
pub use crate::ipc::envelope_v2::SchemaId;
pub use crate::include::subsystem::Version;
