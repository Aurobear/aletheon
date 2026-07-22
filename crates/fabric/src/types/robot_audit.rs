//! Immutable robot governance audit records.

use serde::{Deserialize, Serialize};

use crate::types::embodiment::DeviceId;
use crate::OperationId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotAuditRecord {
    pub sequence: u64,
    pub goal_id: String,
    pub operation_id: OperationId,
    pub attempt: u32,
    pub device: DeviceId,
    pub device_serial: String,
    pub manifest_digest: String,
    pub limits_digest: String,
    pub skill_id: String,
    pub permit_id: String,
    pub lease_id: String,
    pub decision: String,
    pub verification: Option<String>,
    pub recovery: Option<String>,
    pub safe_stop: bool,
    pub emergency_stop: bool,
    pub operator_arming_id: Option<String>,
    pub at_ms: i64,
}
