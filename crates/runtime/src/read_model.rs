//! Deterministic reducers and transactional projection checkpoints.

use crate::SpineEvent;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionDescriptor {
    pub name: &'static str,
    pub version: u32,
    pub accepted_schemas: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionCheckpoint {
    pub projection: String,
    pub version: u32,
    pub through_sequence: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionPoison {
    pub projection: String,
    pub event_id: String,
    pub sequence: u64,
    pub error: String,
}

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("projection descriptor is invalid: {0}")]
    InvalidDescriptor(String),
    #[error("projection {projection} requires rebuild from version {stored} to {requested}")]
    VersionMismatch {
        projection: String,
        stored: u32,
        requested: u32,
    },
    #[error("projection event sequence is not strictly ordered: {previous} then {current}")]
    NonMonotonic { previous: u64, current: u64 },
    #[error("one projection advance batch cannot mix event trees")]
    MixedEventTrees,
    #[error("projection {projection} poisoned at event {event_id}: {message}")]
    Poisoned {
        projection: String,
        event_id: String,
        message: String,
    },
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

pub trait EventProjection {
    type State: Default + Clone + Serialize + DeserializeOwned;

    fn descriptor(&self) -> ProjectionDescriptor;
    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectionAdvanceReport {
    pub checkpoints: Vec<ProjectionCheckpoint>,
    pub lags: Vec<ProjectionLag>,
    pub poisons: Vec<ProjectionPoison>,
    pub failures: Vec<ProjectionFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionLag {
    pub projection: String,
    pub input_sequence: u64,
    pub through_sequence: u64,
    pub pending_events: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionFailure {
    pub projection: String,
    pub error: String,
}

pub trait EventProjectionSink: Send + Sync {
    fn project(&self, event: &SpineEvent) -> ProjectionAdvanceReport;
}

#[derive(Debug, Default)]
pub struct NoopEventProjectionSink;

impl EventProjectionSink for NoopEventProjectionSink {
    fn project(&self, _event: &SpineEvent) -> ProjectionAdvanceReport {
        ProjectionAdvanceReport::default()
    }
}
