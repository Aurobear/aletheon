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
    ReadinessChanged {
        entity_id: String,
        old_state: ReadinessState,
        new_state: ReadinessState,
    },
    TemporalSignal {
        kind: TemporalEventKind,
        content: String,
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
            Self::ReadinessChanged { entity_id, .. } => {
                validate_text("readiness entity", entity_id, false)?
            }
            Self::TemporalSignal { content, .. } => {
                validate_text("temporal signal content", content, false)?
            }
            Self::ScheduledReflection => {}
        }
        Ok(())
    }

    pub fn is_lived(&self) -> bool {
        matches!(self, Self::Lived { .. } | Self::Outcome { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SelfSignal {
    MoodChanged { from: Stimmung, to: Stimmung },
    PredictionError { description: String },
    KnowledgeIntegrated { assertion_count: usize },
    PossibilitiesOpened { count: usize },
    WorldReadinessChanged { entity_id: String },
    ReflectionCompleted,
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
}
