//! Emergency stop authority — independent high-priority local path.
//! E-stop is not Cancel/SafeStop — it is a separate latched authority.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EStopState {
    /// Normal operation, E-stop circuit is armed.
    Armed,
    /// E-stop has been triggered — executing latched stop.
    Triggered,
    /// E-stop is latched — all motion blocked, awaiting local reset.
    Latched,
    /// Awaiting operator to physically reset. Only a local trusted adapter
    /// may transition ResetRequired → Armed. Remote RPC cannot reset.
    ResetRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EStopEvent {
    pub device_id: String,
    pub state: EStopState,
    pub triggered_at_ms: i64,
    pub reason: String,
    pub reset_at_ms: Option<i64>,
    pub operator_id: Option<String>,
}
