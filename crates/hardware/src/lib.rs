//! Governed device control contracts and deterministic simulation.
//!
//! Real actuators are deliberately unsupported. The single `hardware` crate
//! owns device-domain validation while Kernel remains permit authority and
//! Executive remains orchestration/settlement authority.

pub mod clock;
pub mod command;
pub mod device;
pub mod lease;
pub mod provider;
pub mod safety;
pub mod simulator;
pub mod telemetry;

pub use clock::{ManualClock, MonotonicClock};
pub use command::{CommandReceipt, TypedCommand};
pub use device::{
    CommandSequence, DeviceClass, DeviceId, DeviceManifest, DeviceNamespace, MonotonicInstant,
    OperationId, PrincipalId,
};
pub use lease::{ControlLease, ControlPermit};
pub use provider::{DeviceProvider, ValidatedCommand};
pub use safety::{CommandDecision, RejectionReason, SafetyState};
pub use simulator::SimulatedDevice;
pub use telemetry::TelemetryEnvelope;
