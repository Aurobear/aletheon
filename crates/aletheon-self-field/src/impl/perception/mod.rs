//! Perception and FUSE layer.
//!
//! Provides system perception events from various sources (proc, inotify,
//! journald, eBPF), event aggregation with dedup and rate limiting,
//! and a FUSE virtual filesystem.

pub mod event;
pub mod aggregator;
pub mod sources;
pub mod manager;
pub mod fuse;
pub mod bridge;

// Re-export key types at module root
pub use event::{PerceptionEvent, EventSource, EventCategory, Priority, EventData};
