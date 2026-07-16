//! Canonical, versioned events persisted on the Session/Agent event spine.

use std::fmt;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ipc::envelope_v2::{EnvelopeV2, SchemaId};

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

uuid_id!(EventTreeId);
uuid_id!(EventId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParentEventId(pub EventId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TreeSequence(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventIdentity {
    pub root_session_id: String,
    pub session_id: String,
    pub agent_id: Option<String>,
}

impl EventIdentity {
    pub fn validate(&self) -> Result<()> {
        if self.root_session_id.trim().is_empty() || self.session_id.trim().is_empty() {
            bail!("event root/session identity must not be empty");
        }
        if self
            .agent_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty())
        {
            bail!("event agent identity must not be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventVisibility {
    ModelVisible,
    Control,
    Sensitive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "storage", rename_all = "snake_case")]
pub enum EventPayload {
    Inline {
        value: serde_json::Value,
    },
    RawObservationRef {
        uri: String,
        media_type: String,
        sha256: String,
        size_bytes: u64,
    },
}

impl EventPayload {
    pub fn validate(&self, visibility: EventVisibility) -> Result<()> {
        match self {
            Self::Inline { .. } => Ok(()),
            Self::RawObservationRef {
                uri,
                media_type,
                sha256,
                ..
            } => {
                if visibility == EventVisibility::ModelVisible {
                    bail!("raw observations cannot be model-visible");
                }
                if uri.trim().is_empty() || media_type.trim().is_empty() || sha256.len() != 64 {
                    bail!("raw observation reference is incomplete");
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnsequencedEvent {
    pub tree_id: EventTreeId,
    pub event_id: EventId,
    pub parent: Option<ParentEventId>,
    pub identity: EventIdentity,
    pub envelope: EnvelopeV2,
    pub visibility: EventVisibility,
    pub payload: EventPayload,
}

impl UnsequencedEvent {
    pub fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        self.envelope.validate_known_schema()?;
        if self.envelope.target.0.trim().is_empty() {
            bail!("event target must not be empty");
        }
        if self.parent.is_some_and(|parent| parent.0 == self.event_id) {
            bail!("event cannot be its own causal parent");
        }
        self.payload.validate(self.visibility)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPosition {
    pub tree_id: EventTreeId,
    pub event_id: EventId,
    pub parent: Option<ParentEventId>,
    pub sequence: TreeSequence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpineEvent {
    pub position: EventPosition,
    pub identity: EventIdentity,
    pub schema: SchemaId,
    pub visibility: EventVisibility,
    pub envelope: EnvelopeV2,
    pub payload: EventPayload,
}

pub trait EventSpine: Send + Sync {
    fn append(&self, event: UnsequencedEvent) -> Result<SpineEvent>;
}
