//! Hardware Control Platform — Device/Robot domain types (D0).
//! Transport-agnostic, no ROS/CAN/serial dependency here.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceClass { Robot, Actuator, Sensor, Camera, Bus, Composite }

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceNamespace { Simulation, Lab, Production }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceManifest {
    pub id: DeviceId,
    pub class: DeviceClass,
    pub namespace: DeviceNamespace,
    pub model: String,
    pub capabilities: BTreeSet<String>,
    pub firmware: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypedCommand {
    pub command_id: String,
    pub device: DeviceId,
    pub schema: String,
    pub payload: serde_json::Value,
    pub deadline_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControlLease {
    pub lease_id: String,
    pub device: DeviceId,
    pub holder: String,
    pub expires_at_ms: u64,
    pub exclusive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TelemetryEnvelope {
    pub device: DeviceId,
    pub stream: String,
    pub sequence: u64,
    pub source_time_ms: u64,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandReceipt {
    pub command_id: String,
    pub accepted: bool,
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_manifest_round_trip() {
        let m = DeviceManifest {
            id: DeviceId("dev-01".into()),
            class: DeviceClass::Robot,
            namespace: DeviceNamespace::Simulation,
            model: "v1".into(),
            capabilities: BTreeSet::from(["navigate".into()]),
            firmware: None,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: DeviceManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, DeviceId("dev-01".into()));
    }

    #[test]
    fn typed_command_carries_deadline() {
        let cmd = TypedCommand {
            command_id: "c1".into(),
            device: DeviceId("d1".into()),
            schema: "navigate".into(),
            payload: serde_json::json!({"x": 1.0}),
            deadline_ms: 5000,
        };
        assert_eq!(cmd.deadline_ms, 5000);
    }
}
