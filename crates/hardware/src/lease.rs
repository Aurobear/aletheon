use crate::{DeviceId, MonotonicInstant, OperationId, PrincipalId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPermit {
    pub permit_id: String,
    pub operation: OperationId,
    pub principal: PrincipalId,
    pub device: DeviceId,
    pub scope: BTreeSet<String>,
    pub expires_at: MonotonicInstant,
    pub revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlLease {
    pub lease_id: String,
    pub operation: OperationId,
    pub device: DeviceId,
    pub holder: PrincipalId,
    pub scope: BTreeSet<String>,
    pub expires_at: MonotonicInstant,
    pub exclusive: bool,
}
