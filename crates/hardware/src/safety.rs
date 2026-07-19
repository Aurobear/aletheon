use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyState {
    Ready,
    Active,
    Stopping,
    SafeStopped,
    Faulted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    MissingPermit,
    ExpiredPermit,
    RevokedPermit,
    PermitOperationMismatch,
    PermitPrincipalMismatch,
    PermitDeviceMismatch,
    PermitOutOfScope,
    MissingLease,
    ExpiredLease,
    WrongHolder,
    WrongDevice,
    LeaseOperationMismatch,
    LeaseOutOfScope,
    ExpiredDeadline,
    ReplayOrOutOfOrder,
    SchemaMismatch,
    UnsafeState,
    InvalidPayload,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", content = "reason", rename_all = "snake_case")]
pub enum CommandDecision {
    Accepted,
    FailSafeApplied,
    Rejected(RejectionReason),
}
