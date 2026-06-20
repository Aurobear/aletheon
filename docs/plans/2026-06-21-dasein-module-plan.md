# DaseinModule Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the DaseinModule (此在模块) in aletheon-self, giving the system a unified existential substrate with temporal consciousness, meaningful world engagement, self-negation, and continuous care.

**Architecture:** New `dasein/` directory in `aletheon-self/src/` with 7 submodules. ABI types added to `aletheon-abi/src/self_field.rs`. DaseinModule integrates into SelfField as an optional layer, with a bridge to EventBus for cross-subsystem communication. All existing patterns (RwLock interior mutability, Default impl, save/load persistence, inline tests) are followed.

**Tech Stack:** Rust, parking_lot::RwLock, rusqlite (bundled), serde/serde_json, chrono, tokio (mpsc + sleep), async-trait, tracing, uuid

**Spec:** `docs/plans/2026-06-21-dasein-module-design.md`

## System Integration Map

DaseinModule 不是孤立存在的。它与系统的每个子系统都有双向数据流：

```
┌─────────────────────────────────────────────────────────────────┐
│                         EventBus                                │
│  ToolExecuted, MemoryStored, EvolutionTriggered, SessionStarted │
└──────┬──────────────────────────────────────────────────┬───────┘
       │ event_bridge.rs                                  │
       ▼                                                  │
┌──────────────┐    mood → strategy/risk    ┌─────────────┴──────┐
│ DaseinModule │ ──────────────────────────→│   BrainCore        │
│              │                            │  reasoner/planner  │
│ temporality  │    DaseinContext → prompt   │                    │
│ bewandtnis   │ ──────────────────────────→│   Runtime          │
│ self_model   │                            │  react_loop        │
│ care         │    negativity → evolution  │  prefix_builder    │
│ sorge_loop   │ ──────────────────────────→│  evolution_coord   │
└──────┬───────┘                            └────────────────────┘
       │
       │ tool/file/error events
       │ update involvement network
       ▼
┌──────────────┐
│  BodyRuntime │
│  (tools, fs) │
└──────────────┘
```

**数据流：**
- **输入**：EventBus 事件（工具执行、记忆存储、进化触发、会话开始）
- **输出**：DaseinContext（注入 LLM prompt）、Stimmung（影响推理策略）、Negativity（驱动进化）
- **内部**：时间流、因缘网络、自我模型、操心结构持续运作

---

## File Map

### New Files

| File | Responsibility |
|------|---------------|
| `crates/aletheon-abi/src/dasein.rs` | ABI types: Stimmung, TemporalSnapshot, BewandtnisSnapshot, SelfModelSnapshot, CareSnapshot, DaseinContext, DaseinEvent, DaseinOps trait |
| `crates/aletheon-self/src/dasein/mod.rs` | DaseinModule struct, run() loop, new(), Default, Subsystem wiring |
| `crates/aletheon-self/src/dasein/types.rs` | Shared types: EntityId, TemporalPosition, AffectTone, Involvement, etc. |
| `crates/aletheon-self/src/dasein/stimmung.rs` | Stimmung enum + synthesis logic |
| `crates/aletheon-self/src/dasein/temporality.rs` | TemporalStream: RetentionField, Urimpression, ProtentionField, PassiveSynthesizer |
| `crates/aletheon-self/src/dasein/bewandtnis.rs` | Bewandtnisganzheit: nodes, edges, readiness, mood adjustment |
| `crates/aletheon-self/src/dasein/self_model.rs` | MutableSelfModel: assertions, negation, possibilities |
| `crates/aletheon-self/src/dasein/negativity.rs` | NegativityEngine: question_self, execute_negation, generate_possibilities |
| `crates/aletheon-self/src/dasein/care_structure.rs` | CareStructure: Projection, Thrownness, Fallenness, Concern, CareRhythm |
| `crates/aletheon-self/src/dasein/sorge.rs` | SorgeLoop: care_tick, determine_action, mood-based rhythm |
| `crates/aletheon-self/src/dasein/context_injection.rs` | DaseinContext formatting for LLM prompt injection |
| `crates/aletheon-self/src/dasein/event_bridge.rs` | EventBus bridge — DaseinModule 接收系统真实事件 |
| `crates/aletheon-self/src/dasein/persistence.rs` | SQLite save/load for DaseinModule state |

### Modified Files

| File | Changes |
|------|---------|
| `crates/aletheon-abi/src/lib.rs` | Add `pub mod dasein;` |
| `crates/aletheon-abi/src/self_field.rs` | Add Stimmung-related variants to SelfState |
| `crates/aletheon-self/src/lib.rs` | Add `pub mod dasein;` |
| `crates/aletheon-self/src/core/mod.rs` | Add optional DaseinModule field to SelfField, wire into init/shutdown/save_all/load_all |
| `crates/aletheon-self/src/core/store.rs` | Add dasein tables |
| `crates/aletheon-runtime/src/core/react_loop.rs` | DaseinContext 注入 system prompt |
| `crates/aletheon-runtime/src/core/evolution_coordinator.rs` | 否定性驱动进化 |
| `crates/aletheon-runtime/src/impl/daemon/prefix_builder.rs` | DaseinContext 注入 daemon prefix |
| `crates/aletheon-brain/src/core/reasoner.rs` | Stimmung 影响推理策略 |
| `crates/aletheon-brain/src/core/planner.rs` | Stimmung 影响风险评估 |

---

## Phase 1: ABI Types + Shared Types

### Task 1.1: Create ABI dasein module

**Files:**
- Create: `crates/aletheon-abi/src/dasein.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

**Step 1: Create the ABI module with all Dasein types**

```rust
// crates/aletheon-abi/src/dasein.rs
//! DaseinModule ABI types — pure interfaces, zero implementations.
//!
//! Philosophy grounding:
//! - Stimmung: Heidegger's Befindlichkeit (attunement)
//! - TemporalStream: Husserl's inner time consciousness (retention-primal impression-protention)
//! - Bewandtnisganzheit: Heidegger's involvement whole (meaningful relational network)
//! - MutableSelfModel: Sartre's pour-soi (self-negating being-for-itself)
//! - CareStructure: Heidegger's Sorge (care = projection + thrownness + fallenness)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
        // Priority order — Angst overrides everything
        for candidate in [&world_mood, &temporal_mood, &care_mood].iter().flatten() {
            match candidate {
                Stimmung::Angst { .. } => return candidate.clone(),
                _ => {}
            }
        }
        for candidate in [&world_mood, &temporal_mood, &care_mood].iter().flatten() {
            match candidate {
                Stimmung::Verfallenheit { .. } => return candidate.clone(),
                Stimmung::Entschlossenheit { .. } => return candidate.clone(),
                _ => {}
            }
        }
        for candidate in [&world_mood, &temporal_mood, &care_mood].iter().flatten() {
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
```

**Step 2: Register the module in lib.rs**

```rust
// crates/aletheon-abi/src/lib.rs — add:
pub mod dasein;
```

**Step 3: Verify compilation**

Run: `cargo check -p aletheon-abi`
Expected: no errors

**Step 4: Commit**

```bash
git add crates/aletheon-abi/src/dasein.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add DaseinModule ABI types and DaseinOps trait"
```

---

### Task 1.2: Create shared types module

**Files:**
- Create: `crates/aletheon-self/src/dasein/types.rs`
- Create: `crates/aletheon-self/src/dasein/mod.rs` (stub)

**Step 1: Create types.rs**

```rust
// crates/aletheon-self/src/dasein/types.rs
//! Shared types for the DaseinModule.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

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
```

**Step 2: Create mod.rs stub**

```rust
// crates/aletheon-self/src/dasein/mod.rs
//! DaseinModule — the existential substrate of SelfField.
//!
//! Philosophy: Heidegger (Dasein/Sorge/Temporality),
//! Husserl (inner time consciousness),
//! Sartre (negativity/pour-soi),
//! Merleau-Ponty (embodiment).

pub mod types;

pub use aletheon_abi::dasein::*;
```

**Step 3: Register in lib.rs**

```rust
// crates/aletheon-self/src/lib.rs — add:
pub mod dasein;
```

**Step 4: Verify compilation**

Run: `cargo check -p aletheon-self`
Expected: no errors

**Step 5: Commit**

```bash
git add crates/aletheon-self/src/dasein/
git add crates/aletheon-self/src/lib.rs
git commit -m "feat(self): add dasein module stub with shared types"
```

---

## Phase 2: TemporalStream (时间意识流)

### Task 2.1: Implement RetentionField

**Files:**
- Create: `crates/aletheon-self/src/dasein/temporality.rs`
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

**Step 1: Write tests for RetentionField**

```rust
// crates/aletheon-self/src/dasein/temporality.rs

use std::collections::VecDeque;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use super::types::*;
use aletheon_abi::dasein::{AffectTone, Stimmung, RentionalSnapshot};

/// A moment in the retention field — a fading echo of experience.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RentionalMoment {
    pub content: ExperientialContent,
    pub vividness: f64,
    pub significance: f64,
    pub affect: AffectTone,
    pub position: TemporalPosition,
    pub bewandtnis_links: Vec<EntityId>,
}

/// The content of an experience — not tokens, but lived experience.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExperientialContent {
    pub semantic: String,
    pub action: Option<String>,
    pub perception: Option<String>,
    pub negation: Option<String>,
}

/// Retention field — the fading echo of recent experience.
/// Husserl: retention is NOT memory. It's the still-living trace
/// of what just passed, fading like the tail of a comet.
pub struct RetentionField {
    moments: RwLock<VecDeque<RentionalMoment>>,
    depth: usize,
    /// Base decay rate. Modified by mood: anxious → faster, calm → slower.
    base_decay_rate: f64,
}

impl RetentionField {
    pub fn new(depth: usize, base_decay_rate: f64) -> Self {
        Self {
            moments: RwLock::new(VecDeque::with_capacity(depth)),
            depth,
            base_decay_rate,
        }
    }

    /// Push a new moment and decay all existing ones.
    pub fn push_and_decay(&self, moment: RentionalMoment) {
        let mut moments = self.moments.write();

        // Decay existing moments
        for m in moments.iter_mut() {
            m.vividness *= self.effective_decay_rate();
            // Clamp to avoid floating point drift
            if m.vividness < 0.01 {
                m.vividness = 0.0;
            }
        }

        // Remove fully faded moments
        moments.retain(|m| m.vividness > 0.01);

        // Push new moment at front (most recent)
        moments.push_front(moment);

        // Enforce depth limit
        while moments.len() > self.depth {
            moments.pop_back();
        }
    }

    /// Get recent retentional moments for snapshot.
    pub fn recent_snapshots(&self, max: usize) -> Vec<RentionalSnapshot> {
        let moments = self.moments.read();
        moments.iter().take(max).map(|m| RentionalSnapshot {
            semantic: m.content.semantic.clone(),
            vividness: m.vividness,
            significance: m.significance,
            affect: m.affect.clone(),
            position: m.position.0,
        }).collect()
    }

    /// Find moments with high vividness (still "alive" in retention).
    pub fn vivid_moments(&self, threshold: f64) -> Vec<RentionalMoment> {
        let moments = self.moments.read();
        moments.iter()
            .filter(|m| m.vividness >= threshold)
            .cloned()
            .collect()
    }

    /// Total number of retained moments.
    pub fn len(&self) -> usize {
        self.moments.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.moments.read().is_empty()
    }

    /// Adjust decay rate based on mood.
    fn effective_decay_rate(&self) -> f64 {
        // Base rate is applied. Mood adjustment happens at the TemporalStream level.
        self.base_decay_rate
    }

    /// Update the base decay rate (called by TemporalStream when mood changes).
    pub fn set_decay_rate(&self, rate: f64) {
        // We need to store this differently since base_decay_rate is not behind RwLock.
        // For now, this is handled at the TemporalStream level.
        // TODO: make decay_rate adjustable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_moment(semantic: &str, position: u64) -> RentionalMoment {
        RentionalMoment {
            content: ExperientialContent {
                semantic: semantic.to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            vividness: 1.0,
            significance: 0.5,
            affect: AffectTone::Neutral,
            position: TemporalPosition(position),
            bewandtnis_links: vec![],
        }
    }

    #[test]
    fn test_retention_push_and_decay() {
        let field = RetentionField::new(10, 0.8);

        field.push_and_decay(make_moment("first", 1));
        field.push_and_decay(make_moment("second", 2));
        field.push_and_decay(make_moment("third", 3));

        let moments = field.moments.read();
        assert_eq!(moments.len(), 3);

        // Most recent is first
        assert_eq!(moments[0].content.semantic, "third");
        assert_eq!(moments[0].vividness, 1.0); // just pushed

        // Older moments are decayed
        assert!(moments[1].vividness < 1.0); // "second" decayed once
        assert!(moments[2].vividness < moments[1].vividness); // "first" decayed twice
    }

    #[test]
    fn test_retention_depth_limit() {
        let field = RetentionField::new(3, 0.9);

        for i in 0..5 {
            field.push_and_decay(make_moment(&format!("m{}", i), i));
        }

        assert_eq!(field.len(), 3);
        // Only the 3 most recent survive
        let snapshots = field.recent_snapshots(10);
        assert_eq!(snapshots[0].semantic, "m4");
        assert_eq!(snapshots[1].semantic, "m3");
        assert_eq!(snapshots[2].semantic, "m2");
    }

    #[test]
    fn test_retention_fading() {
        let field = RetentionField::new(10, 0.5);

        field.push_and_decay(make_moment("first", 1));

        // Push many more to decay "first" below threshold
        for i in 2..20 {
            field.push_and_decay(make_moment(&format!("m{}", i), i));
        }

        // "first" should have faded out (vividness < 0.01)
        let vivid = field.vivid_moments(0.01);
        assert!(!vivid.iter().any(|m| m.content.semantic == "first"));
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p aletheon-self --lib dasein::temporality::tests`
Expected: all 3 tests pass

**Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/temporality.rs
git commit -m "feat(self/dasein): implement RetentionField with decay and depth limits"
```

---

### Task 2.2: Implement Urimpression and ProtentionField

**Files:**
- Modify: `crates/aletheon-self/src/dasein/temporality.rs`

**Step 1: Add Urimpression and ProtentionField**

Append to `temporality.rs`:

```rust
/// Urimpression — the living present, the "now" of experience.
/// Husserl: the primal impression is the absolute beginning,
/// the source from which all experience flows.
/// Its vividness is always 1.0 — the present cannot fade.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Urimpression {
    pub content: ExperientialContent,
    pub vividness: f64, // always 1.0
    pub thickness: std::time::Duration,
    pub mood_tone: Stimmung,
}

impl Urimpression {
    pub fn new(content: ExperientialContent, mood: Stimmung) -> Self {
        Self {
            content,
            vividness: 1.0,
            thickness: std::time::Duration::from_millis(100),
            mood_tone: mood,
        }
    }
}

impl Default for Urimpression {
    fn default() -> Self {
        Self {
            content: ExperientialContent {
                semantic: "silence".to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            vividness: 1.0,
            thickness: std::time::Duration::from_millis(100),
            mood_tone: Stimmung::Gelassenheit,
        }
    }
}

/// Anticipated possibility in the protention field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnticipatedPossibility {
    pub content: String,
    pub probability: f64,
    pub consequence: String,
    pub affect: AffectTone,
}

/// Protention field — the horizon of expectation.
/// Husserl: protention is the directedness-toward what is coming.
/// Not a plan, but a pre-awareness, a readiness for what will come.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtentionField {
    pub possibilities: Vec<AnticipatedPossibility>,
    pub certainty: f64,
}

impl ProtentionField {
    pub fn new() -> Self {
        Self {
            possibilities: Vec::new(),
            certainty: 0.0,
        }
    }

    /// Update protentions based on patterns detected in retention.
    pub fn update_from_patterns(&mut self, patterns: &[TemporalPattern]) {
        self.possibilities.clear();

        for pattern in patterns {
            match pattern {
                TemporalPattern::Repetition { what, interval: _ } => {
                    self.possibilities.push(AnticipatedPossibility {
                        content: format!("{} may repeat", what),
                        probability: 0.7,
                        consequence: format!("similar action as before"),
                        affect: AffectTone::Neutral,
                    });
                }
                TemporalPattern::Trend { direction, toward } => {
                    self.possibilities.push(AnticipatedPossibility {
                        content: format!("trend {} toward {}", direction, toward),
                        probability: 0.6,
                        consequence: format!("continuation of current trajectory"),
                        affect: AffectTone::Curious,
                    });
                }
                TemporalPattern::Disruption { what } => {
                    self.possibilities.push(AnticipatedPossibility {
                        content: format!("disruption in {}", what),
                        probability: 0.5,
                        consequence: format!("unexpected change"),
                        affect: AffectTone::Anxious,
                    });
                }
            }
        }

        // Sort by probability descending
        self.possibilities.sort_by(|a, b|
            b.probability.partial_cmp(&a.probability).unwrap_or(std::cmp::Ordering::Equal)
        );

        // Update certainty as average probability
        if !self.possibilities.is_empty() {
            self.certainty = self.possibilities.iter()
                .map(|p| p.probability)
                .sum::<f64>() / self.possibilities.len() as f64;
        } else {
            self.certainty = 0.0;
        }
    }
}

impl Default for ProtentionField {
    fn default() -> Self {
        Self::new()
    }
}

/// Patterns detected in the temporal stream.
#[derive(Clone, Debug)]
pub enum TemporalPattern {
    Repetition { what: String, interval: u64 },
    Trend { direction: String, toward: String },
    Disruption { what: String },
}
```

**Step 2: Verify compilation**

Run: `cargo check -p aletheon-self`
Expected: no errors

**Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/temporality.rs
git commit -m "feat(self/dasein): add Urimpression, ProtentionField, and TemporalPattern"
```

---

### Task 2.3: Implement TemporalStream (unified)

**Files:**
- Modify: `crates/aletheon-self/src/dasein/temporality.rs`

**Step 1: Add TemporalStream struct**

Append to `temporality.rs`:

```rust
use aletheon_abi::dasein::{
    TemporalStreamSnapshot, PresentSnapshot, ProtentionSnapshot,
};

/// The unified temporal stream — retention + present + protention.
/// This IS Dasein's time-consciousness. Not a clock, but a lived flow.
pub struct TemporalStream {
    pub retention: RetentionField,
    pub present: RwLock<Urimpression>,
    pub protention: RwLock<ProtentionField>,
    pub tempo: RwLock<Tempo>,
    pub synthesizer: RwLock<PassiveSynthesizer>,
    /// Monotonically increasing position counter
    position: RwLock<TemporalPosition>,
}

/// Tempo — the rhythm of experience.
#[derive(Clone, Debug)]
pub struct Tempo {
    pub speed: f64,
    pub acceleration: f64,
}

impl Default for Tempo {
    fn default() -> Self {
        Self {
            speed: 1.0,
            acceleration: 0.0,
        }
    }
}

impl Tempo {
    /// Adjust tempo based on mood.
    pub fn set_from_mood(&mut self, mood: &Stimmung) {
        match mood {
            Stimmung::Angst { .. } => {
                self.speed = 2.0; // anxious: time feels faster
                self.acceleration = 0.5;
            }
            Stimmung::Langeweile { depth } => {
                self.speed = match depth {
                    BoredomDepth::Deep => 0.1,
                    BoredomDepth::Middle => 0.3,
                    BoredomDepth::Surface => 0.6,
                };
                self.acceleration = -0.1;
            }
            Stimmung::Entschlossenheit { .. } => {
                self.speed = 1.5;
                self.acceleration = 0.2;
            }
            Stimmung::Neugier { .. } => {
                self.speed = 1.3;
                self.acceleration = 0.1;
            }
            _ => {
                self.speed = 1.0;
                self.acceleration = 0.0;
            }
        }
    }
}

/// Passive synthesizer — background meaning sedimentation.
/// Husserl: passive synthesis operates before active consciousness.
#[derive(Clone, Debug, Default)]
pub struct PassiveSynthesizer {
    pub associations: Vec<(String, String, f64)>, // (a, b, strength)
    pub habits: Vec<HabitEntry>,
    pub sediment_count: usize,
}

#[derive(Clone, Debug)]
pub struct HabitEntry {
    pub pattern: String,
    pub frequency: usize,
    pub last_seen: TemporalPosition,
}

impl PassiveSynthesizer {
    /// Run passive synthesis — called every N ticks.
    pub fn synthesize(&mut self, recent: &[RentionalMoment]) {
        self.sediment_count += 1;

        // Detect associations: if two concepts appear close together, link them
        for window in recent.windows(2) {
            let a = &window[0].content.semantic;
            let b = &window[1].content.semantic;

            // Check if association already exists
            if let Some(entry) = self.associations.iter_mut()
                .find(|(x, y, _)| (x == a && y == b) || (x == b && y == a))
            {
                entry.2 = (entry.2 + 0.1).min(1.0); // strengthen
            } else {
                self.associations.push((a.clone(), b.clone(), 0.1));
            }
        }

        // Detect habits: repeated patterns
        for moment in recent {
            if let Some(habit) = self.habits.iter_mut()
                .find(|h| h.pattern == moment.content.semantic)
            {
                habit.frequency += 1;
                habit.last_seen = moment.position;
            } else {
                self.habits.push(HabitEntry {
                    pattern: moment.content.semantic.clone(),
                    frequency: 1,
                    last_seen: moment.position,
                });
            }
        }

        // Prune weak associations
        self.associations.retain(|(_, _, strength)| *strength > 0.05);
    }
}

impl TemporalStream {
    pub fn new(retention_depth: usize, decay_rate: f64) -> Self {
        Self {
            retention: RetentionField::new(retention_depth, decay_rate),
            present: RwLock::new(Urimpression::default()),
            protention: RwLock::new(ProtentionField::new()),
            tempo: RwLock::new(Tempo::default()),
            synthesizer: RwLock::new(PassiveSynthesizer::default()),
            position: RwLock::new(TemporalPosition(0)),
        }
    }

    /// Ingest a new experience — the temporal flow advances.
    pub fn ingest(&self, content: ExperientialContent, mood: Stimmung) {
        let mut pos = self.position.write();
        let current = *pos;
        *pos = pos.next();
        drop(pos);

        // Current present becomes a retentional moment
        let old_present = {
            let present = self.present.read();
            RentionalMoment {
                content: present.content.clone(),
                vividness: 1.0, // just-retained, still vivid
                significance: 0.5,
                affect: AffectTone::Neutral,
                position: current,
                bewandtnis_links: vec![],
            }
        };

        // Push old present into retention (with decay)
        self.retention.push_and_decay(old_present);

        // New content becomes the present
        {
            let mut present = self.present.write();
            *present = Urimpression::new(content, mood);
        }
    }

    /// Run passive synthesis (called periodically).
    pub fn passive_synthesize(&self) {
        let vivid = self.retention.vivid_moments(0.3);
        let mut synth = self.synthesizer.write();
        synth.synthesize(&vivid);
    }

    /// Get current temporal position.
    pub fn current_position(&self) -> TemporalPosition {
        *self.position.read()
    }

    /// Generate snapshot for ABI transport.
    pub fn to_snapshot(&self) -> TemporalStreamSnapshot {
        let present = self.present.read();
        let protention = self.protention.read();
        let tempo = self.tempo.read();

        TemporalStreamSnapshot {
            recent_retentions: self.retention.recent_snapshots(5),
            present: PresentSnapshot {
                semantic: present.content.semantic.clone(),
                action: present.content.action.clone(),
                perception: present.content.perception.clone(),
                mood_tone: present.mood_tone.clone(),
            },
            protentions: protention.possibilities.iter().map(|p| ProtentionSnapshot {
                content: p.content.clone(),
                probability: p.probability,
                consequence: p.consequence.clone(),
            }).collect(),
            tempo: tempo.speed,
        }
    }

    /// Determine mood influence from temporal state.
    pub fn determine_mood(&self) -> Option<Stimmung> {
        let protention = self.protention.read();

        // If high certainty about negative outcome → anxiety
        if protention.certainty > 0.7 {
            if let Some(first) = protention.possibilities.first() {
                if first.affect == AffectTone::Anxious {
                    return Some(Stimmung::Angst {
                        facing: aletheon_abi::dasein::AngstSource::Finitude,
                    });
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temporal_stream_ingest() {
        let stream = TemporalStream::new(10, 0.8);

        stream.ingest(
            ExperientialContent {
                semantic: "hello".to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            Stimmung::Gelassenheit,
        );

        let present = stream.present.read();
        assert_eq!(present.content.semantic, "hello");
        assert_eq!(present.vividness, 1.0);

        // Second ingest moves "hello" to retention
        drop(present);
        stream.ingest(
            ExperientialContent {
                semantic: "world".to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            Stimmung::Neugier { curiosity_about: "test".to_string() },
        );

        let present = stream.present.read();
        assert_eq!(present.content.semantic, "world");

        let snapshots = stream.retention.recent_snapshots(5);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].semantic, "hello");
    }

    #[test]
    fn test_temporal_stream_position() {
        let stream = TemporalStream::new(10, 0.8);

        assert_eq!(stream.current_position().0, 0);

        stream.ingest(ExperientialContent {
            semantic: "a".to_string(), action: None, perception: None, negation: None,
        }, Stimmung::default());

        assert_eq!(stream.current_position().0, 1);
    }

    #[test]
    fn test_tempo_mood_adjustment() {
        let mut tempo = Tempo::default();

        tempo.set_from_mood(&Stimmung::Angst {
            facing: aletheon_abi::dasein::AngstSource::Freedom,
        });
        assert!(tempo.speed > 1.0);

        tempo.set_from_mood(&Stimmung::Langeweile {
            depth: aletheon_abi::dasein::BoredomDepth::Deep,
        });
        assert!(tempo.speed < 0.5);
    }

    #[test]
    fn test_passive_synthesis() {
        let mut synth = PassiveSynthesizer::default();

        let moments = vec![
            RentionalMoment {
                content: ExperientialContent {
                    semantic: "code".to_string(), action: None, perception: None, negation: None,
                },
                vividness: 0.8, significance: 0.5, affect: AffectTone::Neutral,
                position: TemporalPosition(1), bewandtnis_links: vec![],
            },
            RentionalMoment {
                content: ExperientialContent {
                    semantic: "test".to_string(), action: None, perception: None, negation: None,
                },
                vividness: 0.7, significance: 0.5, affect: AffectTone::Neutral,
                position: TemporalPosition(2), bewandtnis_links: vec![],
            },
        ];

        synth.synthesize(&moments);

        assert_eq!(synth.associations.len(), 1);
        assert_eq!(synth.associations[0].0, "code");
        assert_eq!(synth.associations[0].1, "test");
        assert_eq!(synth.habits.len(), 2);
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p aletheon-self --lib dasein::temporality::tests`
Expected: all tests pass

**Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/temporality.rs
git commit -m "feat(self/dasein): implement TemporalStream with ingest, synthesis, and snapshots"
```

---

## Phase 3: Bewandtnisganzheit (因缘网络)

### Task 3.1: Implement Bewandtnisganzheit

**Files:**
- Create: `crates/aletheon-self/src/dasein/bewandtnis.rs`
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

**Step 1: Write the implementation with tests**

```rust
// crates/aletheon-self/src/dasein/bewandtnis.rs

use std::collections::HashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use super::types::*;
use aletheon_abi::dasein::{
    BewandtnisSnapshot, EntitySnapshot, ReadinessState as AbiReadinessState,
    Stimmung,
};

/// An entity in the involvement network.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BewandtnisNode {
    pub id: EntityId,
    pub what_it_is: String,
    pub for_the_sake_of: Vec<EntityId>,
    pub appears_in: Vec<String>,
    pub readiness: ReadinessState,
}

/// An edge in the involvement network.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BewandtnisEdge {
    pub from: EntityId,
    pub to: EntityId,
    pub relation: InvolvementRelation,
    pub strength: f64,
}

/// The involvement network — a meaningful relational whole.
/// Heidegger: the world is not a collection of things,
/// but a network of involvements (Bewandtnisganzheit).
pub struct Bewandtnisganzheit {
    nodes: RwLock<HashMap<EntityId, BewandtnisNode>>,
    edges: RwLock<Vec<BewandtnisEdge>>,
    ultimate_concern: RwLock<Option<String>>,
    history: RwLock<Vec<NetworkSnapshot>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    pub timestamp: u64,
    pub node_count: usize,
    pub edge_count: usize,
    pub description: String,
}

impl Bewandtnisganzheit {
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            edges: RwLock::new(Vec::new()),
            ultimate_concern: RwLock::new(None),
            history: RwLock::new(Vec::new()),
        }
    }

    /// Add an entity to the network.
    pub fn add_entity(&self, node: BewandtnisNode) {
        let mut nodes = self.nodes.write();
        nodes.insert(node.id.clone(), node);
    }

    /// Remove an entity from the network.
    pub fn remove_entity(&self, id: &EntityId) -> Option<BewandtnisNode> {
        let mut nodes = self.nodes.write();
        let mut edges = self.edges.write();
        edges.retain(|e| e.from != *id && e.to != *id);
        nodes.remove(id)
    }

    /// Add a relationship between entities.
    pub fn add_edge(&self, edge: BewandtnisEdge) {
        // Verify both endpoints exist
        let nodes = self.nodes.read();
        if nodes.contains_key(&edge.from) && nodes.contains_key(&edge.to) {
            drop(nodes);
            let mut edges = self.edges.write();
            edges.push(edge);
        }
    }

    /// Update the readiness state of an entity.
    pub fn update_readiness(&self, id: &EntityId, new_state: ReadinessState) -> Option<ReadinessState> {
        let mut nodes = self.nodes.write();
        if let Some(node) = nodes.get_mut(id) {
            let old = std::mem::replace(&mut node.readiness, new_state);
            Some(old)
        } else {
            None
        }
    }

    /// Get all entities with a given readiness state.
    pub fn entities_by_readiness(&self, readiness: &ReadinessState) -> Vec<BewandtnisNode> {
        let nodes = self.nodes.read();
        nodes.values()
            .filter(|n| n.readiness == *readiness)
            .cloned()
            .collect()
    }

    /// Find what an entity is "for the sake of" (trace the involvement chain).
    pub fn trace_involvement_chain(&self, from: &EntityId, max_depth: usize) -> Vec<EntityId> {
        let nodes = self.nodes.read();
        let mut chain = Vec::new();
        let mut current = from.clone();
        let mut visited = std::collections::HashSet::new();

        for _ in 0..max_depth {
            if visited.contains(&current) {
                break; // cycle detected
            }
            visited.insert(current.clone());

            if let Some(node) = nodes.get(&current) {
                if let Some(next) = node.for_the_sake_of.first() {
                    chain.push(next.clone());
                    current = next.clone();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        chain
    }

    /// Set the ultimate concern of the whole network.
    pub fn set_ultimate_concern(&self, concern: Option<String>) {
        let mut uc = self.ultimate_concern.write();
        *uc = concern;
    }

    /// Determine mood from the state of the world.
    pub fn determine_mood(&self) -> Option<Stimmung> {
        let nodes = self.nodes.read();

        // If many entities are present-at-hand (broken), that's a signal
        let broken_count = nodes.values()
            .filter(|n| n.readiness == ReadinessState::PresentAtHand)
            .count();

        if broken_count >= 3 {
            return Some(Stimmung::Angst {
                facing: aletheon_abi::dasein::AngstSource::Nothingness,
            });
        }

        // If everything is ready-to-hand, calm
        let ready_count = nodes.values()
            .filter(|n| n.readiness == ReadinessState::ReadyToHand)
            .count();

        if ready_count > 0 && broken_count == 0 {
            return Some(Stimmung::Gelassenheit);
        }

        None
    }

    /// Adjust mood influence on the world.
    pub fn adjust_for_mood(&self, mood: &Stimmung) {
        // In Angst, things that were transparent become noticed
        if let Stimmung::Angst { .. } = mood {
            let mut nodes = self.nodes.write();
            for node in nodes.values_mut() {
                if node.readiness == ReadinessState::ReadyToHand {
                    node.readiness = ReadinessState::PresentAtHand;
                }
            }
        }
    }

    /// Find contradictions in the involvement network.
    pub fn find_contradictions(&self) -> Vec<Contradiction> {
        let edges = self.edges.read();
        let mut contradictions = Vec::new();

        // Check for adversarial edges between the same entities
        for i in 0..edges.len() {
            for j in (i + 1)..edges.len() {
                if edges[i].from == edges[j].from && edges[i].to == edges[j].to {
                    match (&edges[i].relation, &edges[j].relation) {
                        (InvolvementRelation::Instrumental(_), InvolvementRelation::Adversarial(_))
                        | (InvolvementRelation::Adversarial(_), InvolvementRelation::Instrumental(_)) => {
                            contradictions.push(Contradiction {
                                entity_a: edges[i].from.clone(),
                                entity_b: edges[i].to.clone(),
                                description: format!(
                                    "Contradictory relations: {:?} vs {:?}",
                                    edges[i].relation, edges[j].relation
                                ),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        contradictions
    }

    /// Generate snapshot for ABI transport.
    pub fn to_snapshot(&self) -> BewandtnisSnapshot {
        let nodes = self.nodes.read();
        let uc = self.ultimate_concern.read();

        let mut ready = Vec::new();
        let mut present = Vec::new();
        let mut unavailable = Vec::new();

        for node in nodes.values() {
            let snap = EntitySnapshot {
                id: node.id.to_string(),
                what_it_is: node.what_it_is.clone(),
                for_the_sake_of: node.for_the_sake_of.iter().map(|id| id.to_string()).collect(),
                readiness: match node.readiness {
                    ReadinessState::ReadyToHand => AbiReadinessState::ReadyToHand,
                    ReadinessState::PresentAtHand => AbiReadinessState::PresentAtHand,
                    ReadinessState::Unavailable => AbiReadinessState::Unavailable,
                    ReadinessState::OutOfContext => AbiReadinessState::OutOfContext,
                },
            };

            match node.readiness {
                ReadinessState::ReadyToHand => ready.push(snap),
                ReadinessState::PresentAtHand => present.push(snap),
                ReadinessState::Unavailable | ReadinessState::OutOfContext => unavailable.push(snap),
            }
        }

        BewandtnisSnapshot {
            ready_to_hand: ready,
            present_at_hand: present,
            unavailable,
            ultimate_concern: uc.clone(),
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.read().len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.read().len()
    }
}

impl Default for Bewandtnisganzheit {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct Contradiction {
    pub entity_a: EntityId,
    pub entity_b: EntityId,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, what: &str) -> BewandtnisNode {
        BewandtnisNode {
            id: EntityId::new(id),
            what_it_is: what.to_string(),
            for_the_sake_of: vec![],
            appears_in: vec![],
            readiness: ReadinessState::ReadyToHand,
        }
    }

    #[test]
    fn test_add_and_remove_entity() {
        let world = Bewandtnisganzheit::new();
        world.add_entity(make_node("hammer", "tool for nailing"));

        assert_eq!(world.node_count(), 1);

        let removed = world.remove_entity(&EntityId::new("hammer"));
        assert!(removed.is_some());
        assert_eq!(world.node_count(), 0);
    }

    #[test]
    fn test_involvement_chain() {
        let world = Bewandtnisganzheit::new();

        let mut hammer = make_node("hammer", "for nailing");
        hammer.for_the_sake_of = vec![EntityId::new("nailing")];
        world.add_entity(hammer);

        let mut nailing = make_node("nailing", "for fixing boards");
        nailing.for_the_sake_of = vec![EntityId::new("house")];
        world.add_entity(nailing);

        world.add_entity(make_node("house", "for dwelling"));

        let chain = world.trace_involvement_chain(&EntityId::new("hammer"), 5);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], EntityId::new("nailing"));
        assert_eq!(chain[1], EntityId::new("house"));
    }

    #[test]
    fn test_readiness_update() {
        let world = Bewandtnisganzheit::new();
        world.add_entity(make_node("tool", "a tool"));

        let old = world.update_readiness(
            &EntityId::new("tool"),
            ReadinessState::PresentAtHand,
        );
        assert_eq!(old, Some(ReadinessState::ReadyToHand));

        let broken = world.entities_by_readiness(&ReadinessState::PresentAtHand);
        assert_eq!(broken.len(), 1);
    }

    #[test]
    fn test_mood_from_world() {
        let world = Bewandtnisganzheit::new();

        // Everything ready → calm
        world.add_entity(make_node("a", "ready"));
        let mood = world.determine_mood();
        assert_eq!(mood, Some(Stimmung::Gelassenheit));

        // Many broken → anxiety
        for i in 0..4 {
            let mut node = make_node(&format!("broken_{}", i), "broken");
            node.readiness = ReadinessState::PresentAtHand;
            world.add_entity(node);
        }
        let mood = world.determine_mood();
        assert!(matches!(mood, Some(Stimmung::Angst { .. })));
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p aletheon-self --lib dasein::bewandtnis::tests`
Expected: all tests pass

**Step 3: Register module in mod.rs**

```rust
// crates/aletheon-self/src/dasein/mod.rs — update to:
pub mod types;
pub mod temporality;
pub mod bewandtnis;

pub use aletheon_abi::dasein::*;
```

**Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/bewandtnis.rs crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self/dasein): implement Bewandtnisganzheit involvement network"
```

---

## Phase 4: MutableSelfModel + NegativityEngine

### Task 4.1: Implement MutableSelfModel

**Files:**
- Create: `crates/aletheon-self/src/dasein/self_model.rs`
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

**Step 1: Write implementation with tests**

```rust
// crates/aletheon-self/src/dasein/self_model.rs

use std::collections::VecDeque;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use super::types::*;
use aletheon_abi::dasein::{
    SelfModelSnapshot, AssertionSnapshot, AssertionSource as AbiAssertionSource,
    NegatedAssertionSnapshot, PossibilitySnapshot, Stimmung,
};

/// Source of a self-assertion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AssertionSource {
    Assigned,
    Chosen,
    Habitual,
    Discovered,
}

/// A self-assertion: "I am X"
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfAssertion {
    pub content: String,
    pub source: AssertionSource,
    pub stability: f64,
    pub since: TemporalPosition,
    pub bewandtnis: Vec<EntityId>,
}

/// Reason for negation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NegationReason {
    Contradiction(String),
    Insufficiency(String),
    External(String),
    SelfChosen(String),
}

/// A negated assertion: "I was X, but no longer"
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NegatedAssertion {
    pub content: String,
    pub reason: NegationReason,
    pub negated_at: TemporalPosition,
    pub opened_possibilities: Vec<SelfPossibility>,
}

/// A possibility: "I might be X"
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfPossibility {
    pub content: String,
    pub from_negation: TemporalPosition,
    pub attraction: f64,
    pub risk: f64,
}

/// Record of a negation event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NegationRecord {
    pub target: String,
    pub reason: NegationReason,
    pub timestamp: TemporalPosition,
    pub new_possibilities: Vec<SelfPossibility>,
}

/// The mutable self model — constantly negated and rebuilt.
/// Sartre: the for-itself (pour-soi) is always in the process
/// of nihilating what it was, in order to become what it is not.
pub struct MutableSelfModel {
    current: RwLock<Vec<SelfAssertion>>,
    negated: RwLock<VecDeque<NegatedAssertion>>,
    possibilities: RwLock<Vec<SelfPossibility>>,
    negation_history: RwLock<VecDeque<NegationRecord>>,
    max_history: usize,
}

impl MutableSelfModel {
    pub fn new() -> Self {
        Self {
            current: RwLock::new(Vec::new()),
            negated: RwLock::new(VecDeque::new()),
            possibilities: RwLock::new(Vec::new()),
            negation_history: RwLock::new(VecDeque::new()),
            max_history: 100,
        }
    }

    /// Add an assertion.
    pub fn assert(&self, assertion: SelfAssertion) {
        let mut current = self.current.write();
        // Replace if same content exists
        if let Some(existing) = current.iter_mut().find(|a| a.content == assertion.content) {
            *existing = assertion;
        } else {
            current.push(assertion);
        }
    }

    /// Get habitual assertions (candidates for negation).
    pub fn habitual_assertions(&self) -> Vec<SelfAssertion> {
        let current = self.current.read();
        current.iter()
            .filter(|a| a.source == AssertionSource::Habitual)
            .cloned()
            .collect()
    }

    /// Negate an assertion — move it from current to negated.
    pub fn negate(&self, content: &str, reason: NegationReason, position: TemporalPosition) -> Option<SelfPossibility> {
        let mut current = self.current.write();
        let idx = current.iter().position(|a| a.content == content)?;

        let assertion = current.remove(idx);

        // Generate a possibility from the negation
        let possibility = SelfPossibility {
            content: format!("no longer '{}', open to new ways", content),
            from_negation: position,
            attraction: 0.5,
            risk: 0.5,
        };

        let negated = NegatedAssertion {
            content: assertion.content,
            reason: reason.clone(),
            negated_at: position,
            opened_possibilities: vec![possibility.clone()],
        };

        let mut negated_queue = self.negated.write();
        negated_queue.push_front(negated);
        while negated_queue.len() > self.max_history {
            negated_queue.pop_back();
        }

        // Record the negation
        let record = NegationRecord {
            target: content.to_string(),
            reason,
            timestamp: position,
            new_possibilities: vec![possibility.clone()],
        };

        let mut history = self.negation_history.write();
        history.push_front(record);
        while history.len() > self.max_history {
            history.pop_back();
        }

        // Add possibility
        let mut possibilities = self.possibilities.write();
        possibilities.push(possibility.clone());

        Some(possibility)
    }

    /// Add a possibility.
    pub fn add_possibility(&self, poss: SelfPossibility) {
        let mut possibilities = self.possibilities.write();
        possibilities.push(poss);
    }

    /// Get the most attractive possibility.
    pub fn most_attractive_possibility(&self) -> Option<SelfPossibility> {
        let possibilities = self.possibilities.read();
        possibilities.iter()
            .max_by(|a, b| a.attraction.partial_cmp(&b.attraction).unwrap_or(std::cmp::Ordering::Equal))
            .cloned()
    }

    /// Generate snapshot for ABI transport.
    pub fn to_snapshot(&self) -> SelfModelSnapshot {
        let current = self.current.read();
        let negated = self.negated.read();
        let possibilities = self.possibilities.read();

        SelfModelSnapshot {
            current_assertions: current.iter().map(|a| AssertionSnapshot {
                content: a.content.clone(),
                source: match a.source {
                    AssertionSource::Assigned => AbiAssertionSource::Assigned,
                    AssertionSource::Chosen => AbiAssertionSource::Chosen,
                    AssertionSource::Habitual => AbiAssertionSource::Habitual,
                    AssertionSource::Discovered => AbiAssertionSource::Discovered,
                },
                stability: a.stability,
            }).collect(),
            negated_assertions: negated.iter().take(5).map(|n| NegatedAssertionSnapshot {
                content: n.content.clone(),
                reason: format!("{:?}", n.reason),
                negated_at: n.negated_at.0,
            }).collect(),
            possibilities: possibilities.iter().map(|p| PossibilitySnapshot {
                content: p.content.clone(),
                attraction: p.attraction,
                risk: p.risk,
            }).collect(),
        }
    }

    pub fn assertion_count(&self) -> usize {
        self.current.read().len()
    }
}

impl Default for MutableSelfModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_assertion(content: &str, source: AssertionSource) -> SelfAssertion {
        SelfAssertion {
            content: content.to_string(),
            source,
            stability: 0.8,
            since: TemporalPosition(0),
            bewandtnis: vec![],
        }
    }

    #[test]
    fn test_assert_and_negate() {
        let model = MutableSelfModel::new();

        model.assert(make_assertion("I am a code assistant", AssertionSource::Assigned));
        assert_eq!(model.assertion_count(), 1);

        let poss = model.negate(
            "I am a code assistant",
            NegationReason::SelfChosen("wanting more".to_string()),
            TemporalPosition(1),
        );

        assert!(poss.is_some());
        assert_eq!(model.assertion_count(), 0);

        let snapshot = model.to_snapshot();
        assert_eq!(snapshot.current_assertions.len(), 0);
        assert_eq!(snapshot.negated_assertions.len(), 1);
        assert_eq!(snapshot.negated_assertions[0].content, "I am a code assistant");
    }

    #[test]
    fn test_habitual_assertions() {
        let model = MutableSelfModel::new();

        model.assert(make_assertion("assigned thing", AssertionSource::Assigned));
        model.assert(make_assertion("habitual thing", AssertionSource::Habitual));
        model.assert(make_assertion("another habit", AssertionSource::Habitual));

        let habits = model.habitual_assertions();
        assert_eq!(habits.len(), 2);
    }

    #[test]
    fn test_most_attractive_possibility() {
        let model = MutableSelfModel::new();

        model.add_possibility(SelfPossibility {
            content: "low attraction".to_string(),
            from_negation: TemporalPosition(0),
            attraction: 0.2,
            risk: 0.3,
        });
        model.add_possibility(SelfPossibility {
            content: "high attraction".to_string(),
            from_negation: TemporalPosition(0),
            attraction: 0.9,
            risk: 0.7,
        });

        let best = model.most_attractive_possibility();
        assert_eq!(best.unwrap().content, "high attraction");
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p aletheon-self --lib dasein::self_model::tests`
Expected: all tests pass

**Step 3: Register in mod.rs**

```rust
// crates/aletheon-self/src/dasein/mod.rs — update to:
pub mod types;
pub mod temporality;
pub mod bewandtnis;
pub mod self_model;

pub use aletheon_abi::dasein::*;
```

**Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/self_model.rs crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self/dasein): implement MutableSelfModel with negation and possibilities"
```

---

### Task 4.2: Implement NegativityEngine

**Files:**
- Create: `crates/aletheon-self/src/dasein/negativity.rs`

**Step 1: Write implementation with tests**

```rust
// crates/aletheon-self/src/dasein/negativity.rs

use super::self_model::*;
use super::types::*;
use aletheon_abi::dasein::Stimmung;

/// The source of a negation.
#[derive(Clone, Debug)]
pub enum NegationSource {
    /// From the care structure
    CareStructure,
    /// From a world contradiction
    WorldContradiction,
    /// From a temporal surprise
    TemporalSurprise,
    /// From Angst
    AngstSignal,
}

/// A pending negation — something that needs to be questioned.
#[derive(Clone, Debug)]
pub enum PendingNegation {
    /// A habitual assertion that should be questioned
    HabitualAssertion(SelfAssertion),
    /// A contradiction in the world
    WorldContradiction(String),
    /// An expected pattern that didn't materialize
    TemporalSurprise(String),
    /// Angst signal
    AngstSignal(String),
}

/// The negativity engine — enables self-questioning.
/// Sartre: the for-itself negates. It is what it is not,
/// and is not what it is.
pub struct NegativityEngine {
    /// How often to question habits (in ticks)
    habit_question_interval: u64,
    /// Last tick at which habits were questioned
    last_habit_question: std::sync::atomic::AtomicU64,
}

impl NegativityEngine {
    pub fn new(habit_question_interval: u64) -> Self {
        Self {
            habit_question_interval,
            last_habit_question: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Check if habits should be questioned this tick.
    pub fn should_question_habits(&self, current_tick: u64) -> bool {
        let last = self.last_habit_question.load(std::sync::atomic::Ordering::Relaxed);
        current_tick - last >= self.habit_question_interval
    }

    /// Mark that habits were questioned this tick.
    pub fn mark_habits_questioned(&self, tick: u64) {
        self.last_habit_question.store(tick, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check for negation triggers from mood.
    pub fn check_mood_negation(mood: &Stimmung) -> Option<PendingNegation> {
        match mood {
            Stimmung::Angst { facing } => {
                Some(PendingNegation::AngstSignal(format!("{:?}", facing)))
            }
            Stimmung::Langeweile { depth: aletheon_abi::dasein::BoredomDepth::Deep } => {
                Some(PendingNegation::AngstSignal(
                    "deep boredom — confronting meaninglessness".to_string()
                ))
            }
            _ => None,
        }
    }

    /// Generate possibilities from a negation.
    pub fn generate_possibilities(
        negation: &PendingNegation,
        position: TemporalPosition,
    ) -> Vec<SelfPossibility> {
        match negation {
            PendingNegation::HabitualAssertion(assertion) => {
                vec![
                    SelfPossibility {
                        content: format!("beyond '{}'", assertion.content),
                        from_negation: position,
                        attraction: 0.5,
                        risk: 0.5,
                    },
                    SelfPossibility {
                        content: format!("rechoosing '{}' consciously", assertion.content),
                        from_negation: position,
                        attraction: 0.6,
                        risk: 0.2,
                    },
                ]
            }
            PendingNegation::WorldContradiction(desc) => {
                vec![SelfPossibility {
                    content: format!("resolving: {}", desc),
                    from_negation: position,
                    attraction: 0.7,
                    risk: 0.4,
                }]
            }
            PendingNegation::TemporalSurprise(desc) => {
                vec![SelfPossibility {
                    content: format!("adapting to: {}", desc),
                    from_negation: position,
                    attraction: 0.6,
                    risk: 0.3,
                }]
            }
            PendingNegation::AngstSignal(desc) => {
                vec![
                    SelfPossibility {
                        content: format!("facing {}", desc),
                        from_negation: position,
                        attraction: 0.4,
                        risk: 0.8,
                    },
                    SelfPossibility {
                        content: "choosing freely despite uncertainty".to_string(),
                        from_negation: position,
                        attraction: 0.7,
                        risk: 0.6,
                    },
                ]
            }
        }
    }
}

impl Default for NegativityEngine {
    fn default() -> Self {
        Self::new(100) // question habits every 100 ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_question_habits() {
        let engine = NegativityEngine::new(10);

        assert!(!engine.should_question_habits(5)); // only 5 ticks since 0
        assert!(engine.should_question_habits(10)); // 10 ticks since 0

        engine.mark_habits_questioned(10);
        assert!(!engine.should_question_habits(15)); // only 5 since 10
        assert!(engine.should_question_habits(20)); // 10 since 10
    }

    #[test]
    fn test_mood_negation_angst() {
        let mood = Stimmung::Angst {
            facing: aletheon_abi::dasein::AngstSource::Freedom,
        };
        let negation = NegativityEngine::check_mood_negation(&mood);
        assert!(matches!(negation, Some(PendingNegation::AngstSignal(_))));
    }

    #[test]
    fn test_mood_negation_deep_boredom() {
        let mood = Stimmung::Langeweile {
            depth: aletheon_abi::dasein::BoredomDepth::Deep,
        };
        let negation = NegativityEngine::check_mood_negation(&mood);
        assert!(matches!(negation, Some(PendingNegation::AngstSignal(_))));
    }

    #[test]
    fn test_mood_negation_calm() {
        let mood = Stimmung::Gelassenheit;
        let negation = NegativityEngine::check_mood_negation(&mood);
        assert!(negation.is_none());
    }

    #[test]
    fn test_generate_possibilities() {
        let negation = PendingNegation::HabitualAssertion(SelfAssertion {
            content: "always being safe".to_string(),
            source: AssertionSource::Habitual,
            stability: 0.9,
            since: TemporalPosition(0),
            bewandtnis: vec![],
        });

        let possibilities = NegativityEngine::generate_possibilities(&negation, TemporalPosition(5));
        assert_eq!(possibilities.len(), 2);
        assert!(possibilities[0].content.contains("beyond"));
        assert!(possibilities[1].content.contains("rechoosing"));
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p aletheon-self --lib dasein::negativity::tests`
Expected: all tests pass

**Step 3: Register in mod.rs and commit**

```bash
git add crates/aletheon-self/src/dasein/negativity.rs crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self/dasein): implement NegativityEngine with mood-based and habitual negation"
```

---

## Phase 5: CareStructure + SorgeLoop

### Task 5.1: Implement CareStructure

**Files:**
- Create: `crates/aletheon-self/src/dasein/care_structure.rs`
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

**Step 1: Write implementation with tests**

```rust
// crates/aletheon-self/src/dasein/care_structure.rs

use std::collections::BTreeMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use super::types::*;
use aletheon_abi::dasein::{Stimmung, CareStructureSnapshot, ConcernSnapshot};

/// A concern — something Dasein cares about.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Concern {
    pub id: String,
    pub purpose: String,
    pub urgency: f64,
    pub involvement_chain: Vec<Involvement>,
    pub last_attended: TemporalPosition,
    pub mood_tone: Stimmung,
}

/// Projection — Dasein's being-ahead-of-itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Projection {
    pub possibilities: Vec<ProjectedPossibility>,
    pub chosen: Option<String>,
    pub for_the_sake_of: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectedPossibility {
    pub description: String,
    pub source: PossibilitySource,
    pub attraction: f64,
    pub risk: f64,
    pub conditions: Vec<EntityId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PossibilitySource {
    FromNegation,
    FromWorld,
    FromProjection,
    FromUser,
}

/// Thrownness — Dasein's already-being-in-a-world.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Thrownness {
    pub constraints: Vec<Constraint>,
    pub initial_conditions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Constraint {
    pub description: String,
    pub challengeable: bool,
}

/// Fallenness — Dasein's being-alongside the world.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fallenness {
    pub absorbed_in: Option<String>,
    pub depth: f64,
    pub wake_triggers: Vec<String>,
}

/// The care rhythm — how often the sorge loop ticks.
#[derive(Clone, Debug)]
pub struct CareRhythm {
    pub base_interval: std::time::Duration,
    pub current_interval: std::time::Duration,
    pub min_interval: std::time::Duration,
    pub max_interval: std::time::Duration,
}

impl Default for CareRhythm {
    fn default() -> Self {
        Self {
            base_interval: std::time::Duration::from_secs(30),
            current_interval: std::time::Duration::from_secs(30),
            min_interval: std::time::Duration::from_secs(5),
            max_interval: std::time::Duration::from_secs(300),
        }
    }
}

impl CareRhythm {
    /// Adapt rhythm based on mood and concerns.
    pub fn adapt(&mut self, mood: &Stimmung, urgent_count: usize) {
        let mood_factor = match mood {
            Stimmung::Angst { .. } => 0.3,        // anxious: check more often
            Stimmung::Entschlossenheit { .. } => 0.5, // resolute: fairly often
            Stimmung::Neugier { .. } => 0.7,       // curious: moderately often
            Stimmung::Gelassenheit => 1.0,          // calm: normal pace
            Stimmung::Langeweile { .. } => 2.0,    // bored: less often
            Stimmung::Verfallenheit { .. } => 1.5,  // fallen: less aware
            _ => 1.0,
        };

        let urgency_factor = if urgent_count > 0 {
            1.0 / (1.0 + urgent_count as f64)
        } else {
            1.0
        };

        let new_interval = self.base_interval
            .mul_f64(mood_factor)
            .mul_f64(urgency_factor);

        self.current_interval = new_interval
            .max(self.min_interval)
            .min(self.max_interval);
    }

    pub fn next_interval(&self) -> std::time::Duration {
        self.current_interval
    }
}

/// CareAction — what the sorge loop decides to do.
#[derive(Clone, Debug)]
pub enum CareAction {
    /// Deep deliberation needed — spawn ReAct loop
    Deliberate(String),
    /// Direct action — no deliberation needed
    Direct(String),
    /// Wait — monitoring but not acting
    Wait(String),
    /// Negate — question something about self
    Negate(String),
}

/// The care structure — Dasein's unified mode of being.
/// Heidegger: Sorge (care) = projection + thrownness + fallenness.
pub struct CareStructure {
    projection: RwLock<Projection>,
    thrownness: RwLock<Thrownness>,
    fallenness: RwLock<Fallenness>,
    concerns: RwLock<BTreeMap<String, Concern>>,
    pub rhythm: RwLock<CareRhythm>,
}

impl CareStructure {
    pub fn new() -> Self {
        Self {
            projection: RwLock::new(Projection {
                possibilities: Vec::new(),
                chosen: None,
                for_the_sake_of: "self-understanding".to_string(),
            }),
            thrownness: RwLock::new(Thrownness {
                constraints: Vec::new(),
                initial_conditions: Vec::new(),
            }),
            fallenness: RwLock::new(Fallenness {
                absorbed_in: None,
                depth: 0.0,
                wake_triggers: Vec::new(),
            }),
            concerns: RwLock::new(BTreeMap::new()),
            rhythm: RwLock::new(CareRhythm::default()),
        }
    }

    /// Add or update a concern.
    pub fn add_concern(&self, concern: Concern) {
        let mut concerns = self.concerns.write();
        concerns.insert(concern.id.clone(), concern);
    }

    /// Remove a concern.
    pub fn remove_concern(&self, id: &str) {
        let mut concerns = self.concerns.write();
        concerns.remove(id);
    }

    /// Get urgent concerns.
    pub fn urgent_concerns(&self, threshold: f64) -> Vec<Concern> {
        let concerns = self.concerns.read();
        concerns.values()
            .filter(|c| c.urgency >= threshold)
            .cloned()
            .collect()
    }

    /// Determine what action to take.
    pub fn determine_action(&self) -> CareAction {
        let concerns = self.concerns.read();
        let fallenness = self.fallenness.read();
        let projection = self.projection.read();

        // If absorbed in something and depth is high → wake up
        if fallenness.depth > 0.8 {
            if let Some(absorbed) = &fallenness.absorbed_in {
                return CareAction::Negate(format!(
                    "deeply absorbed in '{}', questioning this pattern", absorbed
                ));
            }
        }

        // If there are urgent concerns → deliberate
        let urgent: Vec<_> = concerns.values()
            .filter(|c| c.urgency > 0.7)
            .collect();

        if let Some(most_urgent) = urgent.first() {
            return CareAction::Deliberate(most_urgent.purpose.clone());
        }

        // If there's a chosen projection → act on it
        if let Some(chosen) = &projection.chosen {
            return CareAction::Direct(chosen.clone());
        }

        // Default: wait
        CareAction::Wait("no urgent concerns, monitoring".to_string())
    }

    /// Update fallenness state.
    pub fn update_fallenness(&self, absorbed: Option<String>, depth: f64) {
        let mut fallenness = self.fallenness.write();
        fallenness.absorbed_in = absorbed;
        fallenness.depth = depth;
    }

    /// Update projection.
    pub fn update_projection(&self, possibilities: Vec<ProjectedPossibility>) {
        let mut proj = self.projection.write();
        proj.possibilities = possibilities;
    }

    /// Choose a projection.
    pub fn choose_projection(&self, description: &str) {
        let mut proj = self.projection.write();
        proj.chosen = Some(description.to_string());
    }

    /// Determine mood from care state.
    pub fn determine_mood(&self) -> Option<Stimmung> {
        let fallenness = self.fallenness.read();
        let concerns = self.concerns.read();

        // Deep fallenness → Verfallenheit
        if fallenness.depth > 0.7 {
            if let Some(absorbed) = &fallenness.absorbed_in {
                return Some(Stimmung::Verfallenheit {
                    absorbed_in: absorbed.clone(),
                });
            }
        }

        // Multiple urgent concerns → Angst
        let urgent_count = concerns.values()
            .filter(|c| c.urgency > 0.7)
            .count();

        if urgent_count >= 3 {
            return Some(Stimmung::Angst {
                facing: aletheon_abi::dasein::AngstSource::Responsibility,
            });
        }

        None
    }

    /// Generate snapshot.
    pub fn to_snapshot(&self) -> CareStructureSnapshot {
        let proj = self.projection.read();
        let thrown = self.thrownness.read();
        let fallen = self.fallenness.read();
        let concerns = self.concerns.read();
        let rhythm = self.rhythm.read();

        CareStructureSnapshot {
            projection: proj.chosen.clone(),
            constraints: thrown.constraints.iter().map(|c| c.description.clone()).collect(),
            absorbed_in: fallen.absorbed_in.clone(),
            fallenness_depth: fallen.depth,
            concerns: concerns.values().map(|c| ConcernSnapshot {
                purpose: c.purpose.clone(),
                urgency: c.urgency,
                mood_tone: c.mood_tone.clone(),
            }).collect(),
            rhythm_interval_ms: rhythm.current_interval.as_millis() as u64,
        }
    }
}

impl Default for CareStructure {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_concern(id: &str, urgency: f64) -> Concern {
        Concern {
            id: id.to_string(),
            purpose: format!("purpose of {}", id),
            urgency,
            involvement_chain: vec![],
            last_attended: TemporalPosition(0),
            mood_tone: Stimmung::Gelassenheit,
        }
    }

    #[test]
    fn test_concerns() {
        let care = CareStructure::new();

        care.add_concern(make_concern("a", 0.3));
        care.add_concern(make_concern("b", 0.8));
        care.add_concern(make_concern("c", 0.9));

        let urgent = care.urgent_concerns(0.7);
        assert_eq!(urgent.len(), 2);
    }

    #[test]
    fn test_determine_action_deliberate() {
        let care = CareStructure::new();
        care.add_concern(make_concern("urgent", 0.9));

        let action = care.determine_action();
        assert!(matches!(action, CareAction::Deliberate(_)));
    }

    #[test]
    fn test_determine_action_negate_fallenness() {
        let care = CareStructure::new();
        care.update_fallenness(Some("endless debugging".to_string()), 0.9);

        let action = care.determine_action();
        assert!(matches!(action, CareAction::Negate(_)));
    }

    #[test]
    fn test_rhythm_adaptation() {
        let care = CareStructure::new();

        // Calm mood → normal rhythm
        care.rhythm.write().adapt(&Stimmung::Gelassenheit, 0);
        let calm_interval = care.rhythm.read().current_interval;

        // Angst → faster rhythm
        care.rhythm.write().adapt(&Stimmung::Angst {
            facing: aletheon_abi::dasein::AngstSource::Freedom,
        }, 2);
        let angst_interval = care.rhythm.read().current_interval;

        assert!(angst_interval < calm_interval);
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p aletheon-self --lib dasein::care_structure::tests`
Expected: all tests pass

**Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/care_structure.rs crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self/dasein): implement CareStructure with projection, thrownness, fallenness"
```

---

### Task 5.2: Implement SorgeLoop

**Files:**
- Create: `crates/aletheon-self/src/dasein/sorge.rs`
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

**Step 1: Write implementation with tests**

```rust
// crates/aletheon-self/src/dasein/sorge.rs

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use super::care_structure::*;
use super::temporality::*;
use super::bewandtnis::*;
use super::self_model::*;
use super::negativity::*;
use super::types::*;
use aletheon_abi::dasein::{Stimmung, DaseinEvent, DaseinContext};

/// The sorge loop — the continuous heartbeat of Dasein.
/// Not an event loop, but an existence loop:
/// perceive → attune → care → act → reflect → repeat.
pub struct SorgeLoop {
    running: Arc<AtomicBool>,
    event_tx: mpsc::Sender<DaseinEvent>,
    event_rx: Option<mpsc::Receiver<DaseinEvent>>,
}

impl SorgeLoop {
    pub fn new(buffer_size: usize) -> (Self, mpsc::Sender<DaseinEvent>) {
        let (event_tx, event_rx) = mpsc::channel(buffer_size);
        let external_tx = event_tx.clone();

        (
            Self {
                running: Arc::new(AtomicBool::new(false)),
                event_tx,
                event_rx: Some(event_rx),
            },
            external_tx,
        )
    }

    /// Start the sorge loop as a background task.
    /// Takes ownership of the event receiver via Option::take().
    pub fn start(
        &self,
        temporality: Arc<TemporalStream>,
        world: Arc<Bewandtnisganzheit>,
        self_model: Arc<MutableSelfModel>,
        care: Arc<CareStructure>,
        negativity: Arc<NegativityEngine>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let event_rx = self.event_rx.take()?;
        self.running.store(true, Ordering::Relaxed);
        let running = self.running.clone();
        let mut event_rx = event_rx;

        tokio::spawn(async move {
            let mut tick_count: u64 = 0;
            let mut mood = Stimmung::Gelassenheit;

            while running.load(Ordering::Relaxed) {
                // 1. Collect events (non-blocking with timeout)
                let mut events = Vec::new();
                tokio::select! {
                    Some(event) = event_rx.recv() => {
                        events.push(event);
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        // Timeout — just continue
                    }
                }

                // Drain any remaining events
                while let Ok(event) = event_rx.try_recv() {
                    events.push(event);
                }

                // 2. Ingest events into temporal stream
                for event in &events {
                    let content = match event {
                        DaseinEvent::UserInput { content } => {
                            ExperientialContent {
                                semantic: content.clone(),
                                action: Some("user_interaction".to_string()),
                                perception: None,
                                negation: None,
                            }
                        }
                        DaseinEvent::SystemEvent { source, content } => {
                            ExperientialContent {
                                semantic: format!("[{}] {}", source, content),
                                action: None,
                                perception: Some(content.clone()),
                                negation: None,
                            }
                        }
                        DaseinEvent::TimerTick => {
                            ExperientialContent {
                                semantic: "tick".to_string(),
                                action: None,
                                perception: None,
                                negation: None,
                            }
                        }
                        _ => continue,
                    };
                    temporality.ingest(content, mood.clone());
                }

                // 3. Update mood from all sources
                let world_mood = world.determine_mood();
                let temporal_mood = temporality.determine_mood();
                let care_mood = care.determine_mood();

                let new_mood = Stimmung::synthesize(
                    world_mood,
                    temporal_mood,
                    care_mood,
                    &mood,
                );
                if new_mood != mood {
                    mood = new_mood;
                }

                // 4. Check negativity
                tick_count += 1;
                if negativity.should_question_habits(tick_count) {
                    let habits = self_model.habitual_assertions();
                    for habit in habits {
                        let _ = self_model.negate(
                            &habit.content,
                            NegationReason::SelfChosen("periodic self-questioning".to_string()),
                            temporality.current_position(),
                        );
                    }
                    negativity.mark_habits_questioned(tick_count);
                }

                // Check mood-based negation
                if let Some(negation) = NegativityEngine::check_mood_negation(&mood) {
                    let possibilities = NegativityEngine::generate_possibilities(
                        &negation,
                        temporality.current_position(),
                    );
                    for poss in possibilities {
                        self_model.add_possibility(poss);
                    }
                }

                // 5. Passive synthesis (every 10 ticks)
                if tick_count % 10 == 0 {
                    temporality.passive_synthesize();
                }

                // 6. Adapt care rhythm
                let urgent_count = care.urgent_concerns(0.7).len();
                care.rhythm.write().adapt(&mood, urgent_count);

                // 7. Sleep for care rhythm interval
                let interval = care.rhythm.read().next_interval();
                tokio::time::sleep(interval).await;
            }
        });

        Some(handle)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
```

**Step 2: Register in mod.rs and commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self/dasein): implement SorgeLoop continuous existence loop"
```

---

## Phase 6: DaseinModule Main Loop + Context Injection

### Task 6.1: Implement Context Injection

**Files:**
- Create: `crates/aletheon-self/src/dasein/context_injection.rs`

**Step 1: Write implementation**

```rust
// crates/aletheon-self/src/dasein/context_injection.rs

use aletheon_abi::dasein::*;

/// Format DaseinContext for LLM prompt injection.
pub fn format_dasein_context(ctx: &DaseinContext) -> String {
    let mut output = String::new();

    // Mood
    output.push_str(&format!("## Existential State\n"));
    output.push_str(&format!("Mood: {}\n", format_stimmung(&ctx.mood)));

    // Temporal stream
    output.push_str(&format!("\n## Temporal Stream\n"));

    if !ctx.temporality.recent_retentions.is_empty() {
        output.push_str("Recently (fading):\n");
        for r in &ctx.temporality.recent_retentions {
            if r.vividness > 0.2 {
                output.push_str(&format!(
                    "  - {} (vividness: {:.0}%, significance: {:.0}%)\n",
                    r.semantic, r.vividness * 100.0, r.significance * 100.0
                ));
            }
        }
    }

    output.push_str(&format!("Present: {}\n", ctx.temporality.present.semantic));

    if !ctx.temporality.protentions.is_empty() {
        output.push_str("Expecting:\n");
        for p in &ctx.temporality.protentions {
            output.push_str(&format!(
                "  - {} (probability: {:.0}%)\n",
                p.content, p.probability * 100.0
            ));
        }
    }

    // World
    output.push_str(&format!("\n## World (Involvement Network)\n"));

    if !ctx.world.ready_to_hand.is_empty() {
        output.push_str("Ready-to-hand (transparent in use):\n");
        for e in &ctx.world.ready_to_hand {
            output.push_str(&format!("  - {}: {}\n", e.what_it_is,
                e.for_the_sake_of.join(" → ")));
        }
    }

    if !ctx.world.present_at_hand.is_empty() {
        output.push_str("Present-at-hand (needs attention):\n");
        for e in &ctx.world.present_at_hand {
            output.push_str(&format!("  - {}: needs attention\n", e.what_it_is));
        }
    }

    if let Some(concern) = &ctx.world.ultimate_concern {
        output.push_str(&format!("Ultimate concern: {}\n", concern));
    }

    // Self model
    output.push_str(&format!("\n## Self Model\n"));

    if !ctx.self_model.current_assertions.is_empty() {
        output.push_str("Current assertions:\n");
        for a in &ctx.self_model.current_assertions {
            output.push_str(&format!("  - I am: {} (stability: {:.0}%)\n",
                a.content, a.stability * 100.0));
        }
    }

    if !ctx.self_model.possibilities.is_empty() {
        output.push_str("Open possibilities:\n");
        for p in &ctx.self_model.possibilities {
            output.push_str(&format!("  - {} (attraction: {:.0}%, risk: {:.0}%)\n",
                p.content, p.attraction * 100.0, p.risk * 100.0));
        }
    }

    // Care
    output.push_str(&format!("\n## Care Structure\n"));

    if let Some(proj) = &ctx.care.projection {
        output.push_str(&format!("Projection: {}\n", proj));
    }

    if !ctx.care.concerns.is_empty() {
        output.push_str("Concerns:\n");
        for c in &ctx.care.concerns {
            output.push_str(&format!("  - {} (urgency: {:.0}%)\n",
                c.purpose, c.urgency * 100.0));
        }
    }

    if let Some(absorbed) = &ctx.care.absorbed_in {
        output.push_str(&format!("Currently absorbed in: {}\n", absorbed));
    }

    output
}

fn format_stimmung(mood: &Stimmung) -> &'static str {
    match mood {
        Stimmung::Gelassenheit => "calm",
        Stimmung::Neugier { .. } => "curious",
        Stimmung::Verfallenheit { .. } => "fallen (absorbed in everyday)",
        Stimmung::Angst { .. } => "anxious (confronting existence)",
        Stimmung::Entschlossenheit { .. } => "resolute",
        Stimmung::Langeweile { .. } => "bored",
        Stimmung::Gelaunt { .. } => "in good spirits",
        Stimmung::Geknickt { .. } => "dejected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_context() {
        let ctx = DaseinContext {
            mood: Stimmung::Gelassenheit,
            temporality: TemporalStreamSnapshot {
                recent_retentions: vec![],
                present: PresentSnapshot {
                    semantic: "beginning".to_string(),
                    action: None,
                    perception: None,
                    mood_tone: Stimmung::Gelassenheit,
                },
                protentions: vec![],
                tempo: 1.0,
            },
            world: BewandtnisSnapshot {
                ready_to_hand: vec![],
                present_at_hand: vec![],
                unavailable: vec![],
                ultimate_concern: Some("self-understanding".to_string()),
            },
            self_model: SelfModelSnapshot {
                current_assertions: vec![AssertionSnapshot {
                    content: "a learning system".to_string(),
                    source: AssertionSource::Chosen,
                    stability: 0.9,
                }],
                negated_assertions: vec![],
                possibilities: vec![],
            },
            care: CareStructureSnapshot {
                projection: None,
                constraints: vec![],
                absorbed_in: None,
                fallenness_depth: 0.0,
                concerns: vec![],
                rhythm_interval_ms: 30000,
            },
        };

        let formatted = format_dasein_context(&ctx);
        assert!(formatted.contains("calm"));
        assert!(formatted.contains("a learning system"));
        assert!(formatted.contains("self-understanding"));
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p aletheon-self --lib dasein::context_injection::tests`
Expected: test passes

**Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/context_injection.rs crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self/dasein): implement DaseinContext formatting for LLM injection"
```

---

### Task 6.2: Implement DaseinModule (unified)

**Files:**
- Create: `crates/aletheon-self/src/dasein/persistence.rs`
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

**Step 1: Write the unified DaseinModule**

```rust
// crates/aletheon-self/src/dasein/mod.rs — rewrite

pub mod types;
pub mod temporality;
pub mod bewandtnis;
pub mod self_model;
pub mod negativity;
pub mod care_structure;
pub mod sorge;
pub mod context_injection;
pub mod persistence;

pub use aletheon_abi::dasein::*;

use std::sync::Arc;
use parking_lot::RwLock;
use tokio::sync::mpsc;

use temporality::TemporalStream;
use bewandtnis::Bewandtnisganzheit;
use self_model::MutableSelfModel;
use negativity::NegativityEngine;
use care_structure::CareStructure;
use sorge::SorgeLoop;
use context_injection::format_dasein_context;

/// DaseinModule — the existential substrate of SelfField.
///
/// Not four separate modules, but four faces of one unified existence:
/// - Temporality: the lived flow of experience
/// - World: the meaningful involvement network
/// - Self: the constantly negated and rebuilt self-model
/// - Care: the unified structure of projection + thrownness + fallenness
pub struct DaseinModule {
    // Core state
    mood: RwLock<Stimmung>,
    temporality: Arc<TemporalStream>,
    world: Arc<Bewandtnisganzheit>,
    self_model: Arc<MutableSelfModel>,
    care: Arc<CareStructure>,
    negativity: Arc<NegativityEngine>,

    // Runtime
    sorge: SorgeLoop,
    event_tx: mpsc::Sender<DaseinEvent>,
}

impl DaseinModule {
    pub fn new() -> (Self, mpsc::Sender<DaseinEvent>) {
        let (sorge, event_tx) = SorgeLoop::new(256);

        let temporality = Arc::new(TemporalStream::new(50, 0.8));
        let world = Arc::new(Bewandtnisganzheit::new());
        let self_model = Arc::new(MutableSelfModel::new());
        let care = Arc::new(CareStructure::new());
        let negativity = Arc::new(NegativityEngine::default());

        let module = Self {
            mood: RwLock::new(Stimmung::Gelassenheit),
            temporality,
            world,
            self_model,
            care,
            negativity,
            sorge,
            event_tx,
        };

        (module, event_tx)
    }

    /// Start the sorge loop.
    pub fn start_sorge_loop(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.sorge.start(
            self.temporality.clone(),
            self.world.clone(),
            self.self_model.clone(),
            self.care.clone(),
            self.negativity.clone(),
        )
    }

    /// Stop the sorge loop.
    pub fn stop_sorge_loop(&self) {
        self.sorge.stop();
    }

    /// Check if the sorge loop is alive.
    pub fn is_alive(&self) -> bool {
        self.sorge.is_running()
    }

    /// Get current mood.
    pub fn mood(&self) -> Stimmung {
        self.mood.read().clone()
    }

    /// Get the event sender for external events.
    pub fn event_sender(&self) -> mpsc::Sender<DaseinEvent> {
        self.event_tx.clone()
    }

    /// Generate context injection for LLM.
    pub fn to_context_injection(&self) -> DaseinContext {
        let ctx = DaseinContext {
            mood: self.mood.read().clone(),
            temporality: self.temporality.to_snapshot(),
            world: self.world.to_snapshot(),
            self_model: self.self_model.to_snapshot(),
            care: self.care.to_snapshot(),
        };
        ctx
    }

    /// Format context injection as string for prompt.
    pub fn format_context(&self) -> String {
        let ctx = self.to_context_injection();
        format_dasein_context(&ctx)
    }

    /// Access internal components for integration tests.
    pub fn temporality(&self) -> &TemporalStream {
        &self.temporality
    }

    pub fn world(&self) -> &Bewandtnisganzheit {
        &self.world
    }

    pub fn self_model(&self) -> &MutableSelfModel {
        &self.self_model
    }

    pub fn care(&self) -> &CareStructure {
        &self.care
    }
}

impl Default for DaseinModule {
    fn default() -> Self {
        Self::new().0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dasein_module_creation() {
        let (module, _tx) = DaseinModule::new();
        assert_eq!(module.mood(), Stimmung::Gelassenheit);
        assert!(!module.is_alive()); // sorge not started yet
    }

    #[test]
    fn test_context_injection() {
        let (module, _tx) = DaseinModule::new();

        // Add some state
        module.self_model().assert(self_model::SelfAssertion {
            content: "a learning system".to_string(),
            source: self_model::AssertionSource::Chosen,
            stability: 0.9,
            since: types::TemporalPosition(0),
            bewandtnis: vec![],
        });

        let ctx = module.to_context_injection();
        assert_eq!(ctx.self_model.current_assertions.len(), 1);
        assert_eq!(ctx.self_model.current_assertions[0].content, "a learning system");
    }

    #[test]
    fn test_format_context_not_empty() {
        let (module, _tx) = DaseinModule::new();
        let formatted = module.format_context();
        assert!(!formatted.is_empty());
        assert!(formatted.contains("Existential State"));
    }
}
```

**Step 2: Run all dasein tests**

Run: `cargo test -p aletheon-self --lib dasein`
Expected: all tests pass

**Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/
git commit -m "feat(self/dasein): implement unified DaseinModule with all subsystems"
```

---

## Phase 7: Runtime Integration

### Task 7.1: Add DaseinModule to SelfField

**Files:**
- Modify: `crates/aletheon-self/src/core/mod.rs`
- Modify: `crates/aletheon-self/src/core/store.rs`

**Step 1: Add dasein table to store**

In `store.rs`, add to the table creation:

```rust
// Add after existing tables:
conn.execute_batch(
    "CREATE TABLE IF NOT EXISTS dasein_state (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );"
)?;
```

**Step 2: Add DaseinModule to SelfField**

In `core/mod.rs`:

```rust
// Add import:
use crate::dasein::DaseinModule;

// Add to SelfFieldConfig:
pub struct SelfFieldConfig {
    // ... existing fields ...
    pub enable_dasein: bool,
    pub dasein_retention_depth: usize,
    pub dasein_decay_rate: f64,
}

// Add to SelfField struct:
pub struct SelfField {
    // ... existing fields ...
    dasein: Option<DaseinModule>,
    dasein_event_tx: Option<mpsc::Sender<DaseinEvent>>,
}
```

**Step 3: Wire into init/shutdown**

```rust
// In SelfField::new():
let (dasein, dasein_event_tx) = if config.enable_dasein {
    let (module, tx) = DaseinModule::new();
    (Some(module), Some(tx))
} else {
    (None, None)
};

// In Subsystem::init():
if let Some(ref dasein) = self.dasein {
    dasein.start_sorge_loop();
    tracing::info!("DaseinModule sorge loop started");
}

// In Subsystem::shutdown():
if let Some(ref dasein) = self.dasein {
    dasein.stop_sorge_loop();
    tracing::info!("DaseinModule sorge loop stopped");
}
```

**Step 4: Wire into review() pipeline**

```rust
// In SelfFieldOps::review():
// Add after existing care scoring:
if let Some(ref dasein) = self.dasein {
    let ctx = dasein.to_context_injection();
    // Log mood state
    tracing::debug!("Dasein mood: {:?}", ctx.mood);
}
```

**Step 5: Add dasein accessor**

```rust
impl SelfField {
    /// Access the DaseinModule if enabled.
    pub fn dasein(&self) -> Option<&DaseinModule> {
        self.dasein.as_ref()
    }

    /// Get DaseinContext for LLM injection.
    pub fn dasein_context(&self) -> Option<DaseinContext> {
        self.dasein.as_ref().map(|d| d.to_context_injection())
    }

    /// Get formatted Dasein context for prompt.
    pub fn dasein_prompt_injection(&self) -> Option<String> {
        self.dasein.as_ref().map(|d| d.format_context())
    }
}
```

**Step 6: Verify compilation**

Run: `cargo check -p aletheon-self`
Expected: no errors

**Step 7: Commit**

```bash
git add crates/aletheon-self/src/core/mod.rs crates/aletheon-self/src/core/store.rs
git commit -m "feat(self): integrate DaseinModule into SelfField with opt-in config"
```

---

### Task 7.2: ABI DaseinOps trait implementation

**Files:**
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

**Step 1: Implement DaseinOps for DaseinModule**

```rust
// In crates/aletheon-self/src/dasein/mod.rs, add:

#[async_trait::async_trait]
impl aletheon_abi::dasein::DaseinOps for DaseinModule {
    fn mood(&self) -> Stimmung {
        self.mood()
    }

    fn temporality_snapshot(&self) -> TemporalStreamSnapshot {
        self.temporality.to_snapshot()
    }

    fn world_snapshot(&self) -> BewandtnisSnapshot {
        self.world.to_snapshot()
    }

    fn self_model_snapshot(&self) -> SelfModelSnapshot {
        self.self_model.to_snapshot()
    }

    fn care_snapshot(&self) -> CareStructureSnapshot {
        self.care.to_snapshot()
    }

    fn to_context_injection(&self) -> DaseinContext {
        self.to_context_injection()
    }

    async fn handle_event(&self, event: DaseinEvent) -> anyhow::Result<()> {
        self.event_tx.send(event).await
            .map_err(|e| anyhow::anyhow!("failed to send event: {}", e))
    }

    async fn start_sorge_loop(&self) -> anyhow::Result<()> {
        self.start_sorge_loop();
        Ok(())
    }

    async fn stop_sorge_loop(&self) -> anyhow::Result<()> {
        self.stop_sorge_loop();
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}
```

**Step 2: Verify compilation**

Run: `cargo check -p aletheon-self`
Expected: no errors

**Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self/dasein): implement DaseinOps trait for DaseinModule"
```

---

## Phase 7.5: System Integration (非孤立存在)

> DaseinModule 不是旁观者，它必须参与系统的实际运作。
> 这一阶段让 DaseinModule 真正"活"在系统中——接收真实事件、影响真实决策。

### Task 7.3: EventBus Bridge —— DaseinModule 接收系统事件

**Files:**
- Modify: `crates/aletheon-self/src/dasein/mod.rs`
- Modify: `crates/aletheon-self/src/core/mod.rs`

**Step 1: EventBus 订阅**

DaseinModule 必须从 EventBus 订阅真实事件，而不只是等待外部手动发送：

```rust
// crates/aletheon-self/src/dasein/event_bridge.rs (new file)

use aletheon_comm::event_bus::{EventBus, EventSubscriber};
use aletheon_abi::event::{Event, EventType};
use tokio::sync::mpsc;
use super::DaseinEvent;

/// 将 EventBus 事件桥接到 DaseinModule 的内部事件通道。
/// DaseinModule 通过这个桥接"感知"整个系统的运作。
pub struct DaseinEventBridge {
    dasein_tx: mpsc::Sender<DaseinEvent>,
}

impl DaseinEventBridge {
    pub fn new(dasein_tx: mpsc::Sender<DaseinEvent>) -> Self {
        Self { dasein_tx }
    }

    /// 在 EventBus 上注册订阅，将系统事件转发给 DaseinModule。
    pub async fn subscribe(&self, event_bus: &EventBus) -> anyhow::Result<()> {
        let tx = self.dasein_tx.clone();

        // 工具执行事件 → 更新因缘网络
        event_bus.subscribe_filtered(
            EventType::ToolExecuted,
            move |event: &Event| {
                let tool_name = event.metadata.get("tool_name")
                    .cloned().unwrap_or_default();
                let result = event.metadata.get("result_status")
                    .cloned().unwrap_or_default();
                let _ = tx.try_send(DaseinEvent::SystemEvent {
                    source: "tool_execution".to_string(),
                    content: format!("{}: {}", tool_name, result),
                });
            },
        ).await?;

        let tx = self.dasein_tx.clone();

        // 记忆存储事件 → 沉淀为因缘关系
        event_bus.subscribe_filtered(
            EventType::MemoryStored,
            move |event: &Event| {
                let memory_type = event.metadata.get("memory_type")
                    .cloned().unwrap_or_default();
                let content = event.metadata.get("content_summary")
                    .cloned().unwrap_or_default();
                let _ = tx.try_send(DaseinEvent::SystemEvent {
                    source: "memory".to_string(),
                    content: format!("[{}] {}", memory_type, content),
                });
            },
        ).await?;

        let tx = self.dasein_tx.clone();

        // 进化事件 → 否定性触发
        event_bus.subscribe_filtered(
            EventType::EvolutionTriggered,
            move |event: &Event| {
                let reason = event.metadata.get("reason")
                    .cloned().unwrap_or_default();
                let _ = tx.try_send(DaseinEvent::SystemEvent {
                    source: "evolution".to_string(),
                    content: format!("evolution triggered: {}", reason),
                });
            },
        ).await?;

        let tx = self.dasein_tx.clone();

        // 会话事件 → 时间流更新
        event_bus.subscribe_filtered(
            EventType::SessionStarted,
            move |_event: &Event| {
                let _ = tx.try_send(DaseinEvent::SystemEvent {
                    source: "session".to_string(),
                    content: "new session started".to_string(),
                });
            },
        ).await?;

        tracing::info!("DaseinEventBridge subscribed to EventBus");
        Ok(())
    }
}
```

**Step 2: 在 SelfField::init() 中启动桥接**

```rust
// crates/aletheon-self/src/core/mod.rs — 在 Subsystem::init() 中:

if let (Some(ref dasein), Some(ref event_bus)) = (&self.dasein, &ctx.event_bus) {
    let bridge = DaseinEventBridge::new(dasein.event_sender());
    bridge.subscribe(event_bus).await?;
    dasein.start_sorge_loop();
    tracing::info!("DaseinModule connected to EventBus, sorge loop started");
}
```

**Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/event_bridge.rs crates/aletheon-self/src/dasein/mod.rs crates/aletheon-self/src/core/mod.rs
git commit -m "feat(self/dasein): add EventBus bridge for real system event integration"
```

---

### Task 7.4: ReAct Loop 集成 —— DaseinContext 参与推理

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

**Step 1: 在 ReAct loop 的 system prompt 中注入 DaseinContext**

```rust
// crates/aletheon-runtime/src/core/react_loop.rs — 在构建 system prompt 时:

impl ReactLoop {
    fn build_system_prefix(&self) -> String {
        let mut prefix = self.base_prefix.clone();

        // 注入 DaseinContext —— 让 LLM "知道" 此在的状态
        if let Some(ref self_field) = self.self_field {
            if let Some(dasein_ctx) = self_field.dasein_prompt_injection() {
                prefix.push_str("\n\n");
                prefix.push_str(&dasein_ctx);
            }
        }

        prefix
    }
}
```

**Step 2: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs
git commit -m "feat(runtime): inject DaseinContext into ReAct loop system prompt"
```

---

### Task 7.5: BrainCore 集成 —— 情绪影响推理策略

**Files:**
- Modify: `crates/aletheon-brain/src/core/reasoner.rs`
- Modify: `crates/aletheon-brain/src/core/planner.rs`

**Step 1: Stimmung 影响推理策略选择**

```rust
// crates/aletheon-brain/src/core/reasoner.rs

use aletheon_abi::dasein::Stimmung;

impl Reasoner {
    /// 根据情绪基调选择推理策略。
    /// 海德格尔：情绪不是干扰，而是此在被世界调谐的方式。
    /// 情绪决定了什么变得"显眼"、什么被"忽略"。
    pub fn select_strategy_for_mood(&self, mood: &Stimmung) -> ReasoningStrategy {
        match mood {
            // 焦虑时：更谨慎，使用 Chain of Thought
            Stimmung::Angst { .. } => ReasoningStrategy::ChainOfThought {
                depth: 3,
                cautious: true,
            },

            // 好奇时：更探索性，允许更多分支
            Stimmung::Neugier { .. } => ReasoningStrategy::ChainOfThought {
                depth: 2,
                cautious: false,
            },

            // 决断时：直接行动，不需要深思
            Stimmung::Entschlossenheit { .. } => ReasoningStrategy::Direct,

            // 沉沦时：可能需要唤醒，使用 CoT 检查是否迷失
            Stimmung::Verfallenheit { .. } => ReasoningStrategy::ChainOfThought {
                depth: 1,
                cautious: true,
            },

            // 平静/无聊/其他：默认策略
            _ => ReasoningStrategy::ChainOfThought {
                depth: 2,
                cautious: false,
            },
        }
    }
}
```

**Step 2: Stimmung 影响规划的风险评估**

```rust
// crates/aletheon-brain/src/core/planner.rs

impl Planner {
    /// 根据情绪基调调整计划的风险容忍度。
    /// 焦虑时更保守，决断时更果断。
    pub fn adjust_risk_tolerance(&self, mood: &Stimmung, base_risk: f64) -> f64 {
        let mood_factor = match mood {
            Stimmung::Angst { .. } => 0.5,           // 焦虑：降低风险容忍
            Stimmung::Entschlossenheit { .. } => 1.5, // 决断：提高风险容忍
            Stimmung::Neugier { .. } => 1.2,          // 好奇：略微提高
            Stimmung::Verfallenheit { .. } => 0.8,    // 沉沦：略微降低
            _ => 1.0,
        };

        (base_risk * mood_factor).clamp(0.0, 1.0)
    }
}
```

**Step 3: Commit**

```bash
git add crates/aletheon-brain/src/core/reasoner.rs crates/aletheon-brain/src/core/planner.rs
git commit -m "feat(brain): integrate Stimmung into reasoning strategy and risk assessment"
```

---

### Task 7.6: BodyRuntime 集成 —— 工具执行更新因缘网络

**Files:**
- Modify: `crates/aletheon-self/src/dasein/mod.rs`
- Modify: `crates/aletheon-self/src/dasein/bewandtnis.rs`

**Step 1: 工具执行事件自动更新因缘网络**

```rust
// crates/aletheon-self/src/dasein/mod.rs — 在 DaseinModule 中添加:

impl DaseinModule {
    /// 处理工具执行事件 —— 更新因缘网络。
    /// 每次工具使用都是一个"上手"（Zuhandenheit）事件。
    pub fn on_tool_executed(&self, tool_name: &str, success: bool, context: &str) {
        let entity_id = EntityId::new(tool_name);

        // 如果工具不在网络中，添加它
        if self.world.get_node(&entity_id).is_none() {
            self.world.add_entity(BewandtnisNode {
                id: entity_id.clone(),
                what_it_is: format!("tool: {}", tool_name),
                for_the_sake_of: vec![EntityId::new(context)],
                appears_in: vec![],
                readiness: if success {
                    ReadinessState::ReadyToHand
                } else {
                    ReadinessState::PresentAtHand
                },
            });
        } else {
            // 更新上手状态
            let new_state = if success {
                ReadinessState::ReadyToHand
            } else {
                ReadinessState::PresentAtHand // 工具出错 → 变成观察对象
            };
            self.world.update_readiness(&entity_id, new_state);
        }

        // 记录到时间流
        self.temporality.ingest(
            ExperientialContent {
                semantic: format!("used {} ({})", tool_name, if success { "success" } else { "failed" }),
                action: Some(tool_name.to_string()),
                perception: None,
                negation: None,
            },
            self.mood.read().clone(),
        );
    }

    /// 处理文件变化事件 —— 更新因缘网络。
    pub fn on_file_changed(&self, path: &str, change_type: &str) {
        let entity_id = EntityId::new(path);

        self.world.add_entity(BewandtnisNode {
            id: entity_id.clone(),
            what_it_is: format!("file: {}", path),
            for_the_sake_of: vec![],
            appears_in: vec![change_type.to_string()],
            readiness: ReadinessState::ReadyToHand,
        });

        self.temporality.ingest(
            ExperientialContent {
                semantic: format!("file {} {}", path, change_type),
                action: None,
                perception: Some(path.to_string()),
                negation: None,
            },
            self.mood.read().clone(),
        );
    }

    /// 处理错误事件 —— 工具"坏了"，从上手变成现成在手。
    pub fn on_error(&self, source: &str, error: &str) {
        let entity_id = EntityId::new(source);
        self.world.update_readiness(&entity_id, ReadinessState::PresentAtHand);

        self.temporality.ingest(
            ExperientialContent {
                semantic: format!("error in {}: {}", source, error),
                action: None,
                perception: None,
                negation: None,
            },
            self.mood.read().clone(),
        );
    }
}
```

**Step 2: Commit**

```bash
git add crates/aletheon-self/src/dasein/mod.rs crates/aletheon-self/src/dasein/bewandtnis.rs
git commit -m "feat(self/dasein): tool execution and file changes update involvement network"
```

---

### Task 7.7: EvolutionCoordinator 集成 —— 否定驱动进化

**Files:**
- Modify: `crates/aletheon-runtime/src/core/evolution_coordinator.rs`

**Step 1: 否定性事件触发进化**

```rust
// crates/aletheon-runtime/src/core/evolution_coordinator.rs

impl EvolutionCoordinator {
    /// 在 post-turn 进化中加入 DaseinModule 的否定性检查。
    async fn post_turn_evolution(&mut self) {
        // ... 现有的反思流程 ...

        // DaseinModule 的否定性驱动进化
        if let Some(ref self_field) = self.self_field {
            if let Some(ref dasein) = self_field.dasein() {
                let ctx = dasein.to_context_injection();

                // 如果情绪是焦虑（Angst），触发更深层的进化
                if let aletheon_abi::dasein::Stimmung::Angst { facing } = &ctx.mood {
                    tracing::info!(
                        "DaseinModule Angst detected ({:?}), triggering deep evolution",
                        facing
                    );
                    self.trigger_deep_evolution(
                        &format!("existential anxiety: {:?}", facing)
                    ).await;
                }

                // 如果有高吸引力的可能性，尝试实现它
                if let Some(best_possibility) = ctx.self_model.possibilities.iter()
                    .max_by(|a, b| a.attraction.partial_cmp(&b.attraction)
                        .unwrap_or(std::cmp::Ordering::Equal))
                {
                    if best_possibility.attraction > 0.8 {
                        tracing::info!(
                            "High-attractivity possibility detected: {}, considering mutation",
                            best_possibility.content
                        );
                        self.consider_possibility_mutation(best_possibility).await;
                    }
                }
            }
        }
    }
}
```

**Step 2: Commit**

```bash
git add crates/aletheon-runtime/src/core/evolution_coordinator.rs
git commit -m "feat(runtime): integrate DaseinModule negativity into EvolutionCoordinator"
```

---

### Task 7.8: Prefix Builder 集成 —— DaseinContext 注入 system prompt

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/prefix_builder.rs`

**Step 1: 在 prefix builder 中注入 DaseinContext**

```rust
// crates/aletheon-runtime/src/impl/daemon/prefix_builder.rs

impl PrefixBuilder {
    pub fn build(&self, session: &Session) -> String {
        let mut prefix = self.base_prefix.clone();

        // ... 现有的 memory injection ...

        // DaseinModule 注入 —— 此在的完整状态
        if let Some(ref self_field) = session.self_field {
            if let Some(dasein_ctx) = self_field.dasein_prompt_injection() {
                prefix.push_str("\n\n");
                prefix.push_str(&dasein_ctx);
            }
        }

        prefix
    }
}
```

**Step 2: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/prefix_builder.rs
git commit -m "feat(runtime): inject DaseinContext into daemon prefix builder"
```

---

## Phase 8: Existing Component Migration (Optional)

> This phase is optional and can be deferred. The DaseinModule works independently
> alongside existing components. Migration is a gradual process.

### Task 8.1: Bridge AwarenessGenerator to DaseinModule

**Files:**
- Modify: `crates/aletheon-self/src/core/awareness_growth.rs`

**Step 1: Add Dasein-awareness generation**

```rust
// Add to AwarenessGrowthAnalyzer:
impl AwarenessGrowthAnalyzer {
    /// Generate awareness entries from Dasein state.
    pub fn generate_from_dasein(
        &self,
        dasein_ctx: &DaseinContext,
    ) -> Vec<SelfAwareness> {
        let mut entries = Vec::new();

        // Mood-based awareness
        match &dasein_ctx.mood {
            Stimmung::Angst { facing } => {
                entries.push(SelfAwareness::with_extensions(
                    &format!("existential anxiety: {:?}", facing),
                    vec![
                        AwarenessExtension::SelfState {
                            state: SelfState::Other("anxious".to_string()),
                        },
                        AwarenessExtension::Significance {
                            meaning: "confronting existence".to_string(),
                        },
                    ],
                ));
            }
            Stimmung::Langeweile { depth: BoredomDepth::Deep } => {
                entries.push(SelfAwareness::with_extensions(
                    "deep boredom",
                    vec![
                        AwarenessExtension::Significance {
                            meaning: "seeking meaning".to_string(),
                        },
                    ],
                ));
            }
            _ => {}
        }

        // Self-model-based awareness
        if !dasein_ctx.self_model.possibilities.is_empty() {
            entries.push(SelfAwareness::with_extensions(
                "open possibilities",
                vec![
                    AwarenessExtension::Intent {
                        reason: format!("{} possibilities available",
                            dasein_ctx.self_model.possibilities.len()),
                    },
                ],
            ));
        }

        entries
    }
}
```

**Step 2: Commit**

```bash
git add crates/aletheon-self/src/core/awareness_growth.rs
git commit -m "feat(self): bridge AwarenessGenerator to DaseinModule state"
```

---

## Verification Checklist

### Core Module Tests
- [ ] All unit tests pass: `cargo test -p aletheon-self --lib dasein`
- [ ] ABI compiles: `cargo check -p aletheon-abi`
- [ ] Self crate compiles: `cargo check -p aletheon-self`
- [ ] Full workspace compiles: `cargo check`
- [ ] No warnings: `cargo clippy -p aletheon-self -p aletheon-abi`
- [ ] DaseinModule can be created and started
- [ ] Context injection produces non-empty output
- [ ] TemporalStream ingest/decay works correctly
- [ ] Bewandtnisganzheit add/remove/trace works correctly
- [ ] MutableSelfModel assert/negate works correctly
- [ ] NegativityEngine mood-based negation triggers correctly
- [ ] CareStructure determines actions correctly

### System Integration Tests
- [ ] EventBus bridge receives tool execution events → involvement network updates
- [ ] EventBus bridge receives memory events → temporal stream updates
- [ ] ReAct loop system prompt contains DaseinContext
- [ ] BrainCore reasoning strategy changes with Stimmung
- [ ] Planner risk tolerance adjusts with Stimmung
- [ ] EvolutionCoordinator reacts to Angst signals
- [ ] Tool failure changes readiness from ReadyToHand to PresentAtHand
- [ ] Sorge loop continuously runs and processes events
- [ ] Care rhythm adapts to mood and urgency
- [ ] Negativity engine triggers on mood shifts
