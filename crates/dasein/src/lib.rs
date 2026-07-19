//! # Aletheon SelfField
//!
//! The policy facade that reviews intents, enforces constitutional boundaries,
//! and exposes read projections. Versioned lived-self mutation is owned by
//! `DaseinModule::transition`; the legacy policy layers are not a second
//! self-evolution authority.
//!
//! ## First Principle
//!
//! **Everything is interpreted by the Self.** SelfField is not a module -- it is the
//! field through which every event, intent, memory, and action passes. Just as Linux
//! organizes around the process primitive and Unix around the file primitive, Aletheon
//! organizes around the Self primitive. See [docs/design/self/first-principle.md].

#![allow(
    clippy::if_same_then_else,
    clippy::unnecessary_sort_by,
    clippy::manual_clamp,
    clippy::ptr_arg,
    clippy::new_without_default,
    clippy::vec_init_then_push,
    clippy::manual_checked_ops,
    clippy::module_inception,
    clippy::too_many_arguments,
    clippy::wrong_self_convention,
    deprecated
)]
//!
//! ## Architecture
//!
//! Policy/read-model layers wired into a `SelfField` implementation:
//!
//! - **Boundary** — pattern-matching rules engine (fast gate)
//! - **Identity** — current self-model + mutation history
//! - **Care** — weighted concerns that influence action scoring
//! - **Narrative** — ring buffer decision log
//! - **Conflict** — multi-source arbitration
//! - **Attention** — focus tracking with priority and decay
//! - **Continuity** — lineage records for identity continuity
//! - **Mutation** — mutation request tracking and approval
//!
//! ## review() Pipeline
//!
//! ```text
//! Intent arrives
//!   → HookBridge.fire_pre_tool()  [pre-tool hooks can block/modify]
//!      → Block? return Verdict::Deny
//!   → PolicyBridge.check()        [PolicyEngine]
//!      → Deny? return Verdict::Deny
//!      → RequireApproval? return Verdict::RequireConfirmation
//!   → BoundaryLayer.check()       [pattern matching, like SELinux]
//!      → Deny? return Verdict::Deny
//!      → Sandbox? return Verdict::SandboxFirst
//!      → Confirm? return Verdict::RequireConfirmation
//!   → CareLayer.score_action()    [weighted concern scoring]
//!   → Permission check            [Context.permissions vs required level]
//!   → NarrativeLayer.record()     [always record for continuity]
//!   → return Verdict::Allow
//! ```

// Core: policy engine layers (identity, boundary, care, etc.)
pub mod core;

// DaseinModule — existential substrate (temporality, bewandtnis, self-model, care)
pub mod dasein;

// Bridge: adapters from impl subsystems into SelfField Verdict system
pub mod bridge;

// Implementation: concrete subsystem implementations
pub mod r#impl;

// Re-export the main entry point
pub use core::{SelfField, SelfFieldConfig};

#[cfg(test)]
pub mod testing;
