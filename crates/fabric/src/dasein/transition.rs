use crate::dasein::{ReadinessState, Stimmung, TemporalEventKind};
use crate::WallTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_TEXT_BYTES: usize = 32 * 1024;
const MAX_ASSERTIONS: usize = 128;
const MAX_POSSIBILITIES: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SelfVersion(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SelfEventId(pub Uuid);

impl SelfEventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SelfEventId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NarrativeEntryId(pub Uuid);

impl NarrativeEntryId {
    pub fn for_event(event_id: SelfEventId) -> Self {
        Self(Uuid::new_v5(&Uuid::NAMESPACE_OID, event_id.0.as_bytes()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceSource {
    User,
    Runtime,
    Tool,
    Memory,
    Metacog,
    Agora,
    Dasein,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceProvenance {
    pub producer: String,
    pub session_id: Option<Uuid>,
    pub turn_id: Option<Uuid>,
    pub source_ref: Option<String>,
}

impl ExperienceProvenance {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_text("provenance producer", &self.producer, false)?;
        if let Some(source_ref) = &self.source_ref {
            validate_text("provenance source reference", source_ref, false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "experience_kind", rename_all = "snake_case")]
pub enum InterpretedExperience {
    Lived {
        semantic: String,
        action: Option<String>,
        perception: Option<String>,
    },
    Outcome {
        summary: String,
        status: OutcomeStatus,
    },
    KnowledgeAsserted {
        assertions: Vec<String>,
        confidence: f64,
    },
    NegationCompleted {
        target: String,
        new_possibilities: Vec<String>,
    },
    MoodObserved {
        mood: Stimmung,
        reason: String,
    },
    WorldEntityObserved {
        entity_id: String,
        what_it_is: String,
        for_the_sake_of: Vec<String>,
        readiness: ReadinessState,
    },
    ReadinessChanged {
        entity_id: String,
        old_state: ReadinessState,
        new_state: ReadinessState,
    },
    TemporalSignal {
        kind: TemporalEventKind,
        content: String,
    },
    ResumedAfterInterval {
        elapsed_ms: u64,
    },
    ScheduledReflection,
}

impl InterpretedExperience {
    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::Lived {
                semantic,
                action,
                perception,
            } => {
                validate_text("lived semantic", semantic, false)?;
                validate_optional_text("lived action", action)?;
                validate_optional_text("lived perception", perception)?;
            }
            Self::Outcome { summary, .. } => validate_text("outcome summary", summary, false)?,
            Self::KnowledgeAsserted {
                assertions,
                confidence,
            } => {
                anyhow::ensure!(
                    confidence.is_finite() && (0.0..=1.0).contains(confidence),
                    "knowledge confidence must be finite and between 0 and 1"
                );
                validate_list("knowledge assertions", assertions, MAX_ASSERTIONS)?;
            }
            Self::NegationCompleted {
                target,
                new_possibilities,
            } => {
                validate_text("negation target", target, false)?;
                validate_list(
                    "negation possibilities",
                    new_possibilities,
                    MAX_POSSIBILITIES,
                )?;
            }
            Self::MoodObserved { reason, .. } => {
                validate_text("mood observation reason", reason, false)?
            }
            Self::WorldEntityObserved {
                entity_id,
                what_it_is,
                for_the_sake_of,
                ..
            } => {
                validate_text("world entity ID", entity_id, false)?;
                validate_text("world entity description", what_it_is, false)?;
                anyhow::ensure!(
                    for_the_sake_of.len() <= MAX_POSSIBILITIES,
                    "world entity involvement list exceeds item limit"
                );
                for target in for_the_sake_of {
                    validate_text("world entity involvement", target, false)?;
                }
            }
            Self::ReadinessChanged { entity_id, .. } => {
                validate_text("readiness entity", entity_id, false)?
            }
            Self::TemporalSignal { content, .. } => {
                validate_text("temporal signal content", content, false)?
            }
            Self::ResumedAfterInterval { .. } => {}
            Self::ScheduledReflection => {}
        }
        Ok(())
    }

    pub fn is_lived(&self) -> bool {
        matches!(
            self,
            Self::Lived { .. } | Self::Outcome { .. } | Self::ResumedAfterInterval { .. }
        )
    }
}

/// Fabric-level projection of Dasein's `CareStructure::determine_action()`
/// decision. The concrete `CareAction` type lives in the `dasein` crate (which
/// depends on `fabric`, not the reverse), so the reducer maps it into this
/// dependency-free representation when emitting a [`SelfSignal::CareDecision`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CareActionKind {
    /// Deep deliberation needed — spawn a ReAct loop.
    Deliberate,
    /// Direct action — no deliberation needed.
    Direct,
    /// Monitor but do not act.
    Wait,
    /// Question something about the self.
    Negate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SelfSignal {
    MoodChanged { from: Stimmung, to: Stimmung },
    PredictionError { description: String },
    KnowledgeIntegrated { assertion_count: usize },
    PossibilitiesOpened { count: usize },
    WorldReadinessChanged { entity_id: String },
    WorldEntityIntegrated { entity_id: String },
    ReflectionCompleted,
    /// The care structure decided what to do during scheduled reflection.
    /// Flows into Agora as a candidate so "care" has a behavioral effect
    /// (conscious-core plan R1: close the `determine_action` dead-code gap).
    CareDecision { action: CareActionKind, rationale: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfTransitionRequest {
    pub event_id: SelfEventId,
    pub source: ExperienceSource,
    pub observed_at: WallTime,
    pub content: InterpretedExperience,
    pub provenance: ExperienceProvenance,
    pub expected_version: SelfVersion,
}

impl SelfTransitionRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.provenance.validate()?;
        self.content.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfTransitionReceipt {
    pub event_id: SelfEventId,
    pub previous_version: SelfVersion,
    pub current_version: SelfVersion,
    pub narrative_entry_id: NarrativeEntryId,
    pub emitted: Vec<SelfSignal>,
}

pub const SELF_EVENT_SCHEMA_V1: u16 = 1;
pub const SELF_REDUCER_V1: u16 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfEventV1 {
    pub schema_version: u16,
    pub reducer_version: u16,
    pub sequence: u64,
    pub request: SelfTransitionRequest,
    pub previous_version: SelfVersion,
    pub current_version: SelfVersion,
    pub previous_checksum: String,
    pub checksum: String,
}

impl SelfEventV1 {
    pub fn validate_versions(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == SELF_EVENT_SCHEMA_V1,
            "unsupported self event schema {}",
            self.schema_version
        );
        anyhow::ensure!(
            self.reducer_version == SELF_REDUCER_V1,
            "unsupported self reducer version {}",
            self.reducer_version
        );
        anyhow::ensure!(
            self.current_version.0 == self.previous_version.0 + 1,
            "self event version range must advance exactly once"
        );
        self.request.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfLineageV1 {
    pub version: String,
    pub parent_version: Option<String>,
    pub mutation_id: Option<String>,
    pub approval_id: Option<String>,
    pub checksum: String,
}

fn validate_optional_text(field: &str, value: &Option<String>) -> anyhow::Result<()> {
    if let Some(value) = value {
        validate_text(field, value, true)?;
    }
    Ok(())
}

fn validate_list(field: &str, values: &[String], max: usize) -> anyhow::Result<()> {
    anyhow::ensure!(!values.is_empty(), "{field} must not be empty");
    anyhow::ensure!(values.len() <= max, "{field} exceeds item limit {max}");
    for value in values {
        validate_text(field, value, false)?;
    }
    Ok(())
}

fn validate_text(field: &str, value: &str, allow_empty: bool) -> anyhow::Result<()> {
    if !allow_empty {
        anyhow::ensure!(!value.trim().is_empty(), "{field} must not be empty");
    }
    anyhow::ensure!(
        value.len() <= MAX_TEXT_BYTES,
        "{field} exceeds {MAX_TEXT_BYTES} bytes"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(content: InterpretedExperience) -> SelfTransitionRequest {
        SelfTransitionRequest {
            event_id: SelfEventId::new(),
            source: ExperienceSource::Runtime,
            observed_at: WallTime(7),
            content,
            provenance: ExperienceProvenance {
                producer: "transition-test".into(),
                session_id: None,
                turn_id: None,
                source_ref: None,
            },
            expected_version: SelfVersion(0),
        }
    }

    #[test]
    fn dasein_transition_contract_round_trips() {
        let value = request(InterpretedExperience::Outcome {
            summary: "tool completed".into(),
            status: OutcomeStatus::Succeeded,
        });
        value.validate().unwrap();
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            serde_json::from_str::<SelfTransitionRequest>(&json).unwrap(),
            value
        );
    }

    #[test]
    fn dasein_transition_rejects_invalid_bounds() {
        let invalid = request(InterpretedExperience::KnowledgeAsserted {
            assertions: Vec::new(),
            confidence: f64::NAN,
        });
        assert!(invalid.validate().is_err());

        let mut missing_producer = request(InterpretedExperience::ScheduledReflection);
        missing_producer.provenance.producer.clear();
        assert!(missing_producer.validate().is_err());
    }

    #[test]
    fn dasein_transition_narrative_id_is_stable_per_event() {
        let event = SelfEventId::new();
        assert_eq!(
            NarrativeEntryId::for_event(event),
            NarrativeEntryId::for_event(event)
        );
    }

    #[test]
    fn durable_self_event_round_trips_and_validates_versions() {
        let request = request(InterpretedExperience::ResumedAfterInterval { elapsed_ms: 10 });
        let event = SelfEventV1 {
            schema_version: SELF_EVENT_SCHEMA_V1,
            reducer_version: SELF_REDUCER_V1,
            sequence: 1,
            previous_version: SelfVersion(0),
            current_version: SelfVersion(1),
            previous_checksum: String::new(),
            checksum: "abc".into(),
            request,
        };
        event.validate_versions().unwrap();
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<SelfEventV1>(&json).unwrap(), event);
    }

    #[test]
    fn durable_self_event_rejects_unknown_schema_and_reducer() {
        let mut event = SelfEventV1 {
            schema_version: 99,
            reducer_version: SELF_REDUCER_V1,
            sequence: 1,
            previous_version: SelfVersion(0),
            current_version: SelfVersion(1),
            previous_checksum: String::new(),
            checksum: "abc".into(),
            request: request(InterpretedExperience::ScheduledReflection),
        };
        assert!(event.validate_versions().is_err());
        event.schema_version = SELF_EVENT_SCHEMA_V1;
        event.reducer_version = 99;
        assert!(event.validate_versions().is_err());
    }
}
