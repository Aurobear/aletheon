//! Version-stable embodiment protocol DTOs.
//!
//! This boundary intentionally contains no ROS or vendor-specific types.

use serde::{Deserialize, Serialize};

use crate::{MonoDeadline, MonoTime, OperationId};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SkillId(pub String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: String,
    pub uri: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbodiedObservation {
    pub schema: String,
    pub schema_version: u16,
    pub source: String,
    pub sequence: u64,
    pub source_time: MonoTime,
    pub received_at: MonoTime,
    pub valid_until: Option<MonoDeadline>,
    pub confidence: f32,
    pub frame_ref: Option<String>,
    pub payload: serde_json::Value,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskClass {
    Read,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillDescriptor {
    pub skill: SkillId,
    pub device: DeviceId,
    pub summary: String,
    pub input_schema: serde_json::Value,
    pub risk: RiskClass,
    pub timeout_ms: u64,
    pub cancellable: bool,
    pub preconditions: Vec<String>,
    pub success_criteria: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillRequest {
    pub skill: SkillId,
    pub device: DeviceId,
    pub parameters: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillProgress {
    pub operation_id: OperationId,
    pub skill: SkillId,
    pub fraction: f32,
    pub note: String,
    pub at: MonoTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SkillOutcome {
    Succeeded,
    Failed { reason: String },
    Cancelled,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillResult {
    pub operation_id: OperationId,
    pub skill: SkillId,
    pub device: DeviceId,
    pub outcome: SkillOutcome,
    pub duration_ms: u64,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyEvent {
    LeaseExpired { device: DeviceId },
    ProviderDisconnected { device: DeviceId },
    StopRequested { device: DeviceId },
    FailSafeApplied { device: DeviceId },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SkillDispatchError {
    #[error("no embodiment provider: {0}")]
    NoProvider(String),
    #[error("embodiment request rejected: {0}")]
    Rejected(String),
}

#[async_trait::async_trait]
pub trait EmbodimentExecutionPort: Send + Sync {
    async fn execute_skill(&self, request: SkillRequest)
        -> Result<SkillResult, SkillDispatchError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embodied_observation_roundtrips_json() {
        let obs = EmbodiedObservation {
            schema: "pose".into(),
            schema_version: 1,
            source: "sim:bot".into(),
            sequence: 7,
            source_time: MonoTime(100),
            received_at: MonoTime(105),
            valid_until: Some(MonoDeadline::after(MonoTime(105), 500)),
            confidence: 0.9,
            frame_ref: Some("map".into()),
            payload: serde_json::json!({"x": 1.0, "y": 2.0}),
            evidence: vec![EvidenceRef {
                kind: "rosbag".into(),
                uri: "artifact://b/1".into(),
            }],
        };
        let json = serde_json::to_string(&obs).unwrap();
        let back: EmbodiedObservation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sequence, 7);
        assert_eq!(back.frame_ref.as_deref(), Some("map"));
    }

    #[test]
    fn skill_ids_are_string_newtypes() {
        assert_eq!(DeviceId("bot".into()).0, "bot");
        assert_eq!(SkillId("wave".into()).0, "wave");
    }

    #[test]
    fn operation_id_parser_accepts_uuid_and_rejects_model_text() {
        let id = OperationId::new();
        assert_eq!(id.0.to_string().parse::<OperationId>().unwrap(), id);
        assert!("cancel-latest".parse::<OperationId>().is_err());
    }
}
