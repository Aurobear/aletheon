//! Perception and FUSE layer.
//!
//! Provides system perception events from various sources (proc, inotify,
//! journald, eBPF), event aggregation with dedup and rate limiting,
//! and a FUSE virtual filesystem.

pub mod aggregator;
pub mod bridge;
pub mod event;
pub mod fuse;
pub mod manager;
pub mod sources;
pub mod visual_aggregator;

// Re-export key types at module root
pub use event::{EventCategory, EventData, EventSource, PerceptionEvent, Priority};
