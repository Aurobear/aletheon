//! # Aletheon SelfField
//!
//! The policy engine that reviews intents, enforces boundaries, resolves conflicts,
//! and maintains identity continuity. Like Linux kernel's LSM / SELinux.
//!
//! ## Architecture
//!
//! 8 internal layers wired into a `SelfField` struct that implements `SelfFieldOps`:
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
//!   → PolicyBridge.check()        [argos-security PolicyEngine]
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

// Bridge: adapters from impl subsystems into SelfField Verdict system
pub mod bridge;

// Implementation: concrete subsystem implementations
pub mod r#impl;

// Re-export the main entry point
pub use core::{SelfField, SelfFieldConfig};

#[cfg(test)]
pub mod testing;
