//! Debug infrastructure — tracepoints, events, and sinks.
//!
//! Inspired by Linux kernel's tracepoint/ftrace/debugfs architecture.
//! This module defines the ABI-level types; implementation lives in aletheon-comm.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Debug level — like Linux kernel's printk levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DebugLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

/// Static tracepoint definition — registered at compile time.
#[derive(Debug, Clone)]
pub struct Tracepoint {
    pub name: &'static str,
    pub module: &'static str,
    pub level: DebugLevel,
    pub description: &'static str,
}

/// Unified debug event format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugEvent {
    pub ts: u64,
    pub tracepoint: String,
    pub module: String,
    pub level: DebugLevel,
    pub data: serde_json::Value,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
}

/// Receives debug events — implemented by bus hook, CLI subscriber, recorder.
#[async_trait]
pub trait DebugSink: Send + Sync {
    async fn emit(&self, event: DebugEvent);
    fn should_trace(&self, tp: &Tracepoint) -> bool;
    /// Unique identifier for this sink (used for removal).
    fn sink_id(&self) -> &str { "" }
    /// Per-sink event filter. If None, accepts all events.
    fn sink_filter(&self) -> Option<&crate::kernel::debug_bus::EventFilter> { None }
}

/// Macro for declaring static tracepoints.
#[macro_export]
macro_rules! tracepoint {
    ($module:ident, $level:ident, $name:expr, $desc:expr) => {
        static __TRACEPOINT: $crate::kernel::debug::Tracepoint = $crate::kernel::debug::Tracepoint {
            name: $name,
            module: stringify!($module),
            level: $crate::kernel::debug::DebugLevel::$level,
            description: $desc,
        };
    };
}

/// Macro for emitting trace events (no-op when no sink is registered).
#[macro_export]
macro_rules! trace {
    ($sink:expr, $tp:expr, $data:expr) => {
        if $sink.should_trace($tp) {
            $sink.emit($crate::kernel::debug::DebugEvent {
                ts: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                tracepoint: $tp.name.to_string(),
                module: $tp.module.to_string(),
                level: $tp.level,
                data: $data,
                session_id: None,
                agent_id: None,
            }).await;
        }
    };
}
