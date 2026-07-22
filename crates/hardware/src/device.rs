use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub use fabric::types::embodiment::DeviceId;
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrincipalId(pub String);
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub String);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MonotonicInstant(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommandSequence(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceClass {
    Robot,
    Actuator,
    Sensor,
    Camera,
    Bus,
    Composite,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceNamespace {
    Simulation,
    Lab,
    Hil,
    Production,
}

impl Default for DeviceNamespace {
    fn default() -> Self {
        Self::Simulation
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceManifest {
    pub id: DeviceId,
    pub class: DeviceClass,
    pub namespace: DeviceNamespace,
    pub model: String,
    pub capabilities: BTreeSet<String>,
    pub firmware: Option<String>,
}
