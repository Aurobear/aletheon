//! FUSE filesystem for exposing agent state to external tools.
//!
//! This module provides a virtual filesystem that exposes agent state
//! (sensors, context, logs, controls) via FUSE. When the `fuse` feature
//! is enabled, it mounts a real FUSE filesystem using fuse3. Without the
//! feature, it operates with in-memory virtual files only.
//!
//! # Architecture
//!
//! - [`filesystem`] — In-memory virtual FS with `AgentFs` (read/write/readdir)
//! - [`provider`] — `StateProvider` trait for feeding real state into the FS
//! - [`controls`] — Write validation for control commands (pause/resume/config)
//! - [`mount`] — Real FUSE mounting via fuse3 (`FuseMount`)

pub mod controls;
pub mod filesystem;
pub mod mount;
pub mod provider;

pub use controls::{ControlVerdict, ControlsValidator};
pub use filesystem::AgentFs;
pub use mount::FuseMount;
pub use provider::{LiveStateProvider, MockStateProvider, StateProvider};
