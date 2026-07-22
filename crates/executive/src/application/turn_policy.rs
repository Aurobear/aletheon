//! Explicit policy differences between turn entry modes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistenceMode {
    Durable,
    Ephemeral,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewerMode {
    SelfField,
    Terminal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDelivery {
    Streaming,
    Silent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentProfile {
    DaemonSandbox,
    LocalExec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnPolicy {
    pub persistence: PersistenceMode,
    pub reviewer: ReviewerMode,
    pub memory_eligible: bool,
    pub agora_available: bool,
    pub event_delivery: EventDelivery,
    pub environment: EnvironmentProfile,
}

impl TurnPolicy {
    pub fn daemon() -> Self {
        Self {
            persistence: PersistenceMode::Durable,
            reviewer: ReviewerMode::SelfField,
            memory_eligible: true,
            agora_available: true,
            event_delivery: EventDelivery::Streaming,
            environment: EnvironmentProfile::DaemonSandbox,
        }
    }
    pub fn exec() -> Self {
        Self {
            persistence: PersistenceMode::Durable,
            reviewer: ReviewerMode::Terminal,
            memory_eligible: false,
            agora_available: false,
            event_delivery: EventDelivery::Silent,
            environment: EnvironmentProfile::LocalExec,
        }
    }
}
