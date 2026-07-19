//! Deterministic hardware simulator — executable specification of the safety model (D0).

use hardware_api::{CommandReceipt, ControlLease, DeviceClass, DeviceId, DeviceManifest, DeviceNamespace, TelemetryEnvelope, TypedCommand};
use std::collections::BTreeSet;

pub struct SimulatedDevice {
    pub manifest: DeviceManifest,
    pub position: (f64, f64),
    pub battery: f64,
    pub lease: Option<ControlLease>,
}

impl SimulatedDevice {
    pub fn mobile_robot(id: &str) -> Self {
        Self {
            manifest: DeviceManifest {
                id: DeviceId(id.into()),
                class: DeviceClass::Robot,
                namespace: DeviceNamespace::Simulation,
                model: "sim-mobile-v1".into(),
                capabilities: BTreeSet::from(["navigate".into(), "stop".into()]),
                firmware: Some("sim-0.1.0".into()),
            },
            position: (0.0, 0.0),
            battery: 1.0,
            lease: None,
        }
    }

    pub fn telemetry(&self) -> TelemetryEnvelope {
        TelemetryEnvelope {
            device: self.manifest.id.clone(),
            stream: "pose".into(),
            sequence: 0,
            source_time_ms: 0,
            payload: serde_json::json!({
                "x": self.position.0,
                "y": self.position.1,
                "battery": self.battery,
            }),
        }
    }

    pub fn execute(&mut self, _cmd: &TypedCommand) -> CommandReceipt {
        CommandReceipt {
            command_id: "sim-ack".into(),
            accepted: self.lease.is_some(),
            reason: if self.lease.is_none() {
                Some("no active lease".into())
            } else {
                None
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_robot_refuses_command_without_lease() {
        let mut robot = SimulatedDevice::mobile_robot("test-bot");
        let cmd = TypedCommand {
            command_id: "c1".into(),
            device: DeviceId("test-bot".into()),
            schema: "navigate".into(),
            payload: serde_json::json!({"x": 1.0, "y": 2.0}),
            deadline_ms: 5000,
        };
        let receipt = robot.execute(&cmd);
        assert!(!receipt.accepted);
        assert!(receipt.reason.unwrap().contains("lease"));
    }

    #[test]
    fn telemetry_includes_battery() {
        let robot = SimulatedDevice::mobile_robot("test-bot");
        let t = robot.telemetry();
        let payload = t.payload;
        assert!(payload["battery"].as_f64().unwrap() > 0.0);
    }
}
