//! Observable — subsystem status reporting.
//!
//! Like Linux kernel's `/proc` interface: any subsystem can report its
//! current status and metrics without the runtime needing to know the type.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Snapshot of a subsystem's current status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemStatus {
    pub name: String,
    pub running: bool,
    pub status_line: String,
    pub details: HashMap<String, String>,
}

/// Trait for subsystems that can report their status.
pub trait Observable {
    /// Return a status snapshot.
    fn status(&self) -> SubsystemStatus;

    /// Return metrics as key-value pairs. Default: empty.
    fn metrics(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}
