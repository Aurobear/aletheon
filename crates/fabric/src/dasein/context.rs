use serde::{Deserialize, Serialize};

use crate::dasein::types::{AffectTone, ReadinessState, Stimmung};

// ═══ Temporal Stream Snapshot ═══

/// Snapshot of the temporal stream for ABI transport.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemporalStreamSnapshot {
    /// Recent retentional moments (most recent first), max 5
    pub recent_retentions: Vec<RentionalSnapshot>,
    /// Current present impression
    pub present: PresentSnapshot,
    /// Anticipated possibilities
    pub protentions: Vec<ProtentionSnapshot>,
    /// Current tempo (speed of experience flow)
    pub tempo: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RentionalSnapshot {
    pub semantic: String,
    pub vividness: f64,
    pub significance: f64,
    pub affect: AffectTone,
    pub position: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PresentSnapshot {
    pub semantic: String,
    pub action: Option<String>,
    pub perception: Option<String>,
    pub mood_tone: Stimmung,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtentionSnapshot {
    pub content: String,
    pub probability: f64,
    pub consequence: String,
}

// ═══ Bewandtnisganzheit Snapshot ═══

/// Snapshot of the involvement network for ABI transport.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BewandtnisSnapshot {
    /// Entities currently ready-to-hand (transparent in use)
    pub ready_to_hand: Vec<EntitySnapshot>,
    /// Entities currently present-at-hand (broken, noticed)
    pub present_at_hand: Vec<EntitySnapshot>,
    /// Entities that are unavailable
    pub unavailable: Vec<EntitySnapshot>,
    /// The ultimate concern of the whole network
    pub ultimate_concern: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntitySnapshot {
    pub id: String,
    pub what_it_is: String,
    pub for_the_sake_of: Vec<String>,
    pub readiness: ReadinessState,
}

// ═══ Self Model Snapshot ═══

/// Snapshot of the mutable self model for ABI transport.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfModelSnapshot {
    /// Current assertions: "I am X"
    pub current_assertions: Vec<AssertionSnapshot>,
    /// Recently negated assertions: "I was X"
    pub negated_assertions: Vec<NegatedAssertionSnapshot>,
    /// Open possibilities: "I might be X"
    pub possibilities: Vec<PossibilitySnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssertionSnapshot {
    pub content: String,
    pub source: AssertionSource,
    pub stability: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AssertionSource {
    Assigned,
    Chosen,
    Habitual,
    Discovered,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NegatedAssertionSnapshot {
    pub content: String,
    pub reason: String,
    pub negated_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PossibilitySnapshot {
    pub content: String,
    pub attraction: f64,
    pub risk: f64,
}

// ═══ Care Structure Snapshot ═══

/// Snapshot of the care structure for ABI transport.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CareStructureSnapshot {
    /// Current projection (what Dasein is aiming at)
    pub projection: Option<String>,
    /// Thrownness constraints (what cannot be changed)
    pub constraints: Vec<String>,
    /// Fallenness state
    pub absorbed_in: Option<String>,
    pub fallenness_depth: f64,
    /// Active concerns sorted by urgency
    pub concerns: Vec<ConcernSnapshot>,
    /// Care rhythm interval (ms)
    pub rhythm_interval_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConcernSnapshot {
    pub purpose: String,
    pub urgency: f64,
    pub mood_tone: Stimmung,
}

// ═══ Dasein Context (for LLM injection) ═══

/// The complete Dasein state formatted for LLM prompt injection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaseinContext {
    pub mood: Stimmung,
    pub temporality: TemporalStreamSnapshot,
    pub world: BewandtnisSnapshot,
    pub self_model: SelfModelSnapshot,
    pub care: CareStructureSnapshot,
}
