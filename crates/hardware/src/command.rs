use crate::{
    CommandDecision, CommandSequence, DeviceId, MonotonicInstant, OperationId, PrincipalId,
    SafetyState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypedCommand {
    pub command_id: String,
    pub operation: OperationId,
    pub principal: PrincipalId,
    pub sequence: CommandSequence,
    pub device: DeviceId,
    pub schema: String,
    pub payload: serde_json::Value,
    pub deadline: MonotonicInstant,
}
impl TypedCommand {
    pub fn is_stop(&self) -> bool {
        self.schema == "stop"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReceipt {
    pub operation: OperationId,
    pub principal: PrincipalId,
    pub device: DeviceId,
    pub command_id: String,
    pub sequence: CommandSequence,
    pub decision: CommandDecision,
    pub safety_before: SafetyState,
    pub safety_after: SafetyState,
    pub observed_at: MonotonicInstant,
}
impl CommandReceipt {
    pub fn accepted(&self) -> bool {
        matches!(
            self.decision,
            CommandDecision::Accepted | CommandDecision::FailSafeApplied
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decision_round_trip_has_no_bool_reason_contradiction() {
        let receipt = CommandReceipt {
            operation: OperationId("o".into()),
            principal: PrincipalId("p".into()),
            device: DeviceId("d".into()),
            command_id: "c".into(),
            sequence: CommandSequence(1),
            decision: CommandDecision::Rejected(crate::RejectionReason::MissingPermit),
            safety_before: SafetyState::Ready,
            safety_after: SafetyState::Ready,
            observed_at: MonotonicInstant(4),
        };
        let encoded = serde_json::to_string(&receipt).unwrap();
        assert_eq!(
            serde_json::from_str::<CommandReceipt>(&encoded).unwrap(),
            receipt
        );
        assert!(!receipt.accepted());
    }
}
