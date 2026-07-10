//! RFC-017 primitives — the canonical shared vocabulary.
//!
//! Every subsystem communicates using these primitives instead of concrete
//! implementations. Pure types only; no business logic.

pub mod cognitive;
pub mod comm;

pub use cognitive::{
    Commitment, CommitmentStatus, Decision, Evidence, Experience, Hypothesis, Intent, Narrative,
    Observation, Plan,
};
pub use comm::{Command, Event, Mailbox, Query, Stream};
