//! Operation receipts — every side-effecting host operation returns one.

use serde::{Deserialize, Serialize};

/// Outcome of a single host operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostReceipt {
    pub operation: String,
    pub success: bool,
    pub elapsed_us: u64,
    pub detail: Option<String>,
}

impl HostReceipt {
    pub fn ok(operation: impl Into<String>, elapsed_us: u64) -> Self {
        Self {
            operation: operation.into(),
            success: true,
            elapsed_us,
            detail: None,
        }
    }

    pub fn err(operation: impl Into<String>, elapsed_us: u64, detail: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            success: false,
            elapsed_us,
            detail: Some(detail.into()),
        }
    }
}
