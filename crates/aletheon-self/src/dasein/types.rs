//! Shared types for the DaseinModule.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for an entity in the involvement network.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct EntityId(pub String);

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl EntityId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// Position in the temporal stream — not wall clock, but flow position.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct TemporalPosition(pub u64);

impl TemporalPosition {
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

impl Default for TemporalPosition {
    fn default() -> Self {
        Self(0)
    }
}

/// Affect tone of an experience moment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AffectTone {
    Positive,
    Negative,
    Neutral,
    Anxious,
    Curious,
}

impl Default for AffectTone {
    fn default() -> Self {
        AffectTone::Neutral
    }
}

/// Involvement — a "for-the-sake-of" relationship.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Involvement {
    pub entity: EntityId,
    pub for_the_sake_of: EntityId,
    pub context: String,
    pub readiness: ReadinessState,
}

/// Readiness state of an entity in the world.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ReadinessState {
    ReadyToHand,
    PresentAtHand,
    Unavailable,
    OutOfContext,
}

/// Relation type between entities in the involvement network.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum InvolvementRelation {
    Instrumental(String),
    Constitutive(String),
    Conditional(String),
    Adversarial(String),
    Alternative(String),
    Negating(String),
}

/// Edge in the involvement network.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BewandtnisEdge {
    pub from: EntityId,
    pub to: EntityId,
    pub relation: InvolvementRelation,
    pub strength: f64,
}

/// Node in the involvement network.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BewandtnisNode {
    pub id: EntityId,
    pub what_it_is: String,
    pub for_the_sake_of: Vec<EntityId>,
    pub appears_in: Vec<String>,
    pub readiness: ReadinessState,
}

/// Temporal position marker for retention/protention.
pub type TemporalMarker = u64;
