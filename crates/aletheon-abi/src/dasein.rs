//! DaseinModule ABI types — pure interfaces, zero implementations.
//!
//! Philosophy grounding:
//! - Stimmung: Heidegger's Befindlichkeit (attunement)
//! - TemporalStream: Husserl's inner time consciousness (retention-primal impression-protention)
//! - Bewandtnisganzheit: Heidegger's involvement whole (meaningful relational network)
//! - MutableSelfModel: Sartre's pour-soi (self-negating being-for-itself)
//! - CareStructure: Heidegger's Sorge (care = projection + thrownness + fallenness)

use serde::{Deserialize, Serialize};

// ═══ Stimmung (情绪基调) ═══

/// Heidegger's Befindlichkeit — the way Dasein is always attuned.
/// Not a psychological state, but the way the world discloses itself to Dasein.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Stimmung {
    /// Calm — no pressing concerns, open to the world
    Gelassenheit,
    /// Curious — new possibilities discovered
    Neugier { curiosity_about: String },
    /// Fallen — lost in the everyday, absorbed in tasks
    Verfallenheit { absorbed_in: String },
    /// Anxiety — confronting existence itself (no specific object)
    Angst { facing: AngstSource },
    /// Resolute — a choice has been made, projecting toward possibility
    Entschlossenheit { chosen_possibility: String },
    /// Boredom — waiting for something to happen
    Langeweile { depth: BoredomDepth },
    /// Good mood — world discloses positively
    Gelaunt { toward: String },
    /// Dejected — world discloses negatively
    Geknickt { because: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AngstSource {
    Freedom,
    Finitude,
    Nothingness,
    Responsibility,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BoredomDepth {
    Surface,
    Middle,
    Deep,
}

impl Default for Stimmung {
    fn default() -> Self {
        Stimmung::Gelassenheit
    }
}

impl Stimmung {
    /// Synthesize mood from three sources.
    /// Priority: Angst > Verfallenheit > Entschlossenheit > Neugier > others
    pub fn synthesize(
        world_mood: Option<Stimmung>,
        temporal_mood: Option<Stimmung>,
        care_mood: Option<Stimmung>,
        current: &Stimmung,
    ) -> Stimmung {
        let candidates: [&Option<Stimmung>; 3] = [&world_mood, &temporal_mood, &care_mood];

        // Priority order — Angst overrides everything
        for candidate in candidates.iter().copied().flatten() {
            match candidate {
                Stimmung::Angst { .. } => return candidate.clone(),
                _ => {}
            }
        }
        for candidate in candidates.iter().copied().flatten() {
            match candidate {
                Stimmung::Verfallenheit { .. } => return candidate.clone(),
                Stimmung::Entschlossenheit { .. } => return candidate.clone(),
                _ => {}
            }
        }
        for candidate in candidates.iter().copied().flatten() {
            match candidate {
                Stimmung::Neugier { .. } => return candidate.clone(),
                _ => {}
            }
        }
        // Default: keep current
        current.clone()
    }
}

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ReadinessState {
    ReadyToHand,
    PresentAtHand,
    Unavailable,
    OutOfContext,
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

// ═══ Dasein Events ═══

/// Events flowing into and out of the DaseinModule.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DaseinEvent {
    // External events
    UserInput {
        content: String,
    },
    SystemEvent {
        source: String,
        content: String,
    },
    TimerTick,

    // Internal events
    NegationCompleted {
        target: String,
        new_possibilities: Vec<String>,
    },
    MoodShift {
        from: Stimmung,
        to: Stimmung,
        reason: String,
    },
    BewandtnisChange {
        entity_id: String,
        old_state: ReadinessState,
        new_state: ReadinessState,
    },
    TemporalEvent {
        kind: TemporalEventKind,
        content: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TemporalEventKind {
    RetentionFaded,
    ProtentionRealized,
    ProtentionSurprised,
    PatternDetected,
}

// ═══ DaseinOps Trait ═══

/// The Dasein module's public interface.
#[async_trait::async_trait]
pub trait DaseinOps: Send + Sync {
    /// Get current mood (Stimmung)
    fn mood(&self) -> Stimmung;

    /// Get temporal stream snapshot
    fn temporality_snapshot(&self) -> TemporalStreamSnapshot;

    /// Get involvement network snapshot
    fn world_snapshot(&self) -> BewandtnisSnapshot;

    /// Get self model snapshot
    fn self_model_snapshot(&self) -> SelfModelSnapshot;

    /// Get care structure snapshot
    fn care_snapshot(&self) -> CareStructureSnapshot;

    /// Generate complete context for LLM prompt injection
    fn to_context_injection(&self) -> DaseinContext;

    /// Feed an event into the Dasein module
    async fn handle_event(&self, event: DaseinEvent) -> anyhow::Result<()>;

    /// Start the sorge loop (background task)
    async fn start_sorge_loop(&self) -> anyhow::Result<()>;

    /// Stop the sorge loop
    async fn stop_sorge_loop(&self) -> anyhow::Result<()>;

    /// Check if sorge loop is running
    fn is_alive(&self) -> bool;
}
