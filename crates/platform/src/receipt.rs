//! Operation receipts — every side-effecting host operation returns one.

use serde::{Deserialize, Serialize};

pub const MAX_RECEIPT_DETAIL_BYTES: usize = 4096;

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
        let detail = detail.into();
        let detail = if detail.len() <= MAX_RECEIPT_DETAIL_BYTES {
            detail
        } else {
            let mut end = MAX_RECEIPT_DETAIL_BYTES;
            while !detail.is_char_boundary(end) {
                end -= 1;
            }
            detail[..end].to_owned()
        };
        Self {
            operation: operation.into(),
            success: false,
            elapsed_us,
            detail: Some(detail),
        }
    }
}
