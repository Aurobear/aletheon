use crate::{DeviceId, MonotonicInstant, OperationId, SafetyState};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TelemetryEnvelope {
    pub device: DeviceId,
    pub operation: Option<OperationId>,
    pub stream: String,
    pub sequence: u64,
    pub source_time: MonotonicInstant,
    pub safety: SafetyState,
    pub payload: serde_json::Value,
}
