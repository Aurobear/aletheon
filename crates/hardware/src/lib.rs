//! Governed device control contracts and deterministic simulation.
//!
//! Real actuators are deliberately unsupported. The single `hardware` crate
//! owns device-domain validation while Kernel remains permit authority and
//! Executive remains orchestration/settlement authority.

pub mod broker;
pub mod clock;
pub mod command;
pub mod deployment_gate;
pub mod device;
pub mod emergency_stop;
pub mod grpc;
pub mod lease;
pub mod observation;
pub mod provider;
pub mod registry;
pub mod safety;
pub mod simulator;
pub mod skill;
pub mod telemetry;

pub use broker::{Broker, BrokerError};
pub use clock::{ManualClock, MonotonicClock};
pub use command::{CommandReceipt, TypedCommand};
pub use deployment_gate::{validate_gate, DeploymentGateInput, DeploymentGateResult};
pub use device::{
    CommandSequence, DeviceClass, DeviceId, DeviceManifest, DeviceNamespace, MonotonicInstant,
    OperationId, PrincipalId,
};
pub use emergency_stop::EmergencyStop;
pub use lease::{ControlLease, ControlPermit};
pub use observation::{is_stale, ObservationIngest};
pub use provider::{DeviceProvider, ValidatedCommand};
pub use registry::ProviderRegistry;
pub use safety::{CommandDecision, RejectionReason, SafetyState};
pub use simulator::{SimulatedDevice, SimulatedEmbodiment};
pub use grpc::provider::{GrpcEmbodimentProvider, GrpcProviderConfig};
pub use skill::{
    AuthorizedSkillRequest, CancelAck, EmbodimentProvider, ProviderError, SkillProgressSink,
    StopReceipt, ValidatedSkillCommand,
};
pub use telemetry::TelemetryEnvelope;
