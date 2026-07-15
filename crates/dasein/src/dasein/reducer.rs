use super::bewandtnis::Bewandtnisganzheit;
use super::care_structure::CareStructure;
use super::self_model::{
    AssertionSource, MutableSelfModel, NegationReason, SelfAssertion, SelfPossibility,
};
use super::temporality::{ExperientialContent, TemporalStream};
use super::types::{EntityId, ReadinessState as InternalReadinessState};
use fabric::dasein::{
    DaseinEvent, ExperienceProvenance, ExperienceSource, InterpretedExperience, NarrativeEntryId,
    OutcomeStatus, SelfEventId, SelfSignal, SelfTransitionReceipt, SelfTransitionRequest,
    SelfVersion, Stimmung, TemporalEventKind,
};
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

const MAX_NARRATIVE_REFERENCES: usize = 1_000;

struct ReducerState {
    version: SelfVersion,
    receipts: HashMap<SelfEventId, SelfTransitionReceipt>,
    narrative: VecDeque<(NarrativeEntryId, SelfEventId)>,
}

pub struct DaseinStateEngine {
    temporality: Arc<TemporalStream>,
    world: Arc<Bewandtnisganzheit>,
    self_model: Arc<MutableSelfModel>,
    care: Arc<CareStructure>,
    mood: Arc<RwLock<Stimmung>>,
    state: Mutex<ReducerState>,
    clock: Arc<dyn fabric::Clock>,
}

impl DaseinStateEngine {
    pub fn new(
        temporality: Arc<TemporalStream>,
        world: Arc<Bewandtnisganzheit>,
        self_model: Arc<MutableSelfModel>,
        care: Arc<CareStructure>,
        mood: Arc<RwLock<Stimmung>>,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        Self {
            temporality,
            world,
            self_model,
            care,
            mood,
            state: Mutex::new(ReducerState {
                version: SelfVersion::default(),
                receipts: HashMap::new(),
                narrative: VecDeque::with_capacity(MAX_NARRATIVE_REFERENCES),
            }),
            clock,
        }
    }

    pub async fn version(&self) -> SelfVersion {
        self.state.lock().await.version
    }

    pub async fn narrative_len(&self) -> usize {
        self.state.lock().await.narrative.len()
    }

    pub fn reflection_interval(&self) -> std::time::Duration {
        self.care.rhythm.read().next_interval()
    }

    pub async fn transition(
        &self,
        request: SelfTransitionRequest,
    ) -> anyhow::Result<SelfTransitionReceipt> {
        let mut state = self.state.lock().await;
        self.transition_locked(&mut state, request)
    }

    fn transition_locked(
        &self,
        state: &mut ReducerState,
        request: SelfTransitionRequest,
    ) -> anyhow::Result<SelfTransitionReceipt> {
        if let Some(receipt) = state.receipts.get(&request.event_id) {
            return Ok(receipt.clone());
        }

        request.validate()?;
        anyhow::ensure!(
            request.expected_version == state.version,
            "Dasein self version conflict: expected {}, current {}",
            request.expected_version.0,
            state.version.0
        );

        let previous_version = state.version;
        let emitted = self.reduce(&request.content)?;
        let current_version = SelfVersion(previous_version.0 + 1);
        let narrative_entry_id = NarrativeEntryId::for_event(request.event_id);
        let receipt = SelfTransitionReceipt {
            event_id: request.event_id,
            previous_version,
            current_version,
            narrative_entry_id,
            emitted,
        };

        state.version = current_version;
        state.receipts.insert(request.event_id, receipt.clone());
        state
            .narrative
            .push_back((narrative_entry_id, request.event_id));
        while state.narrative.len() > MAX_NARRATIVE_REFERENCES {
            state.narrative.pop_front();
        }
        Ok(receipt)
    }

    pub async fn apply_compat_event(
        &self,
        event: DaseinEvent,
        producer: &str,
    ) -> anyhow::Result<SelfTransitionReceipt> {
        let content = match event {
            DaseinEvent::UserInput { content } => InterpretedExperience::Lived {
                semantic: content,
                action: Some("user_interaction".into()),
                perception: None,
            },
            DaseinEvent::SystemEvent { source, content } => InterpretedExperience::Lived {
                semantic: format!("[{source}] {content}"),
                action: None,
                perception: Some(content),
            },
            DaseinEvent::TimerTick => InterpretedExperience::ScheduledReflection,
            DaseinEvent::KnowledgeAsserted {
                assertions,
                confidence,
            } => InterpretedExperience::KnowledgeAsserted {
                assertions,
                confidence,
            },
            DaseinEvent::NegationCompleted {
                target,
                new_possibilities,
            } => InterpretedExperience::NegationCompleted {
                target,
                new_possibilities,
            },
            DaseinEvent::MoodShift {
                from: _,
                to,
                reason,
            } => InterpretedExperience::MoodObserved { mood: to, reason },
            DaseinEvent::BewandtnisChange {
                entity_id,
                old_state,
                new_state,
            } => InterpretedExperience::ReadinessChanged {
                entity_id,
                old_state,
                new_state,
            },
            DaseinEvent::TemporalEvent { kind, content } => {
                InterpretedExperience::TemporalSignal { kind, content }
            }
        };
        self.transition_current(ExperienceSource::Runtime, producer, content)
            .await
    }

    pub async fn transition_current(
        &self,
        source: ExperienceSource,
        producer: &str,
        content: InterpretedExperience,
    ) -> anyhow::Result<SelfTransitionReceipt> {
        let mut state = self.state.lock().await;
        let request = SelfTransitionRequest {
            event_id: SelfEventId::new(),
            source,
            observed_at: self.clock.wall_now(),
            content,
            provenance: ExperienceProvenance {
                producer: producer.to_string(),
                session_id: None,
                turn_id: None,
                source_ref: None,
            },
            expected_version: state.version,
        };
        self.transition_locked(&mut state, request)
    }

    fn reduce(&self, content: &InterpretedExperience) -> anyhow::Result<Vec<SelfSignal>> {
        let mut emitted = Vec::new();
        match content {
            InterpretedExperience::Lived {
                semantic,
                action,
                perception,
            } => {
                self.temporality.ingest(
                    ExperientialContent {
                        semantic: semantic.clone(),
                        action: action.clone(),
                        perception: perception.clone(),
                        negation: None,
                    },
                    self.mood.read().clone(),
                );
                self.synthesize_mood(&mut emitted);
            }
            InterpretedExperience::Outcome { summary, status } => {
                self.temporality.ingest(
                    ExperientialContent {
                        semantic: summary.clone(),
                        action: Some("turn_outcome".into()),
                        perception: None,
                        negation: None,
                    },
                    self.mood.read().clone(),
                );
                let next = match status {
                    OutcomeStatus::Succeeded => Stimmung::Gelaunt {
                        toward: "successful outcome".into(),
                    },
                    OutcomeStatus::Failed => Stimmung::Geknickt {
                        because: "failed outcome".into(),
                    },
                    OutcomeStatus::Cancelled => Stimmung::Langeweile {
                        depth: fabric::dasein::BoredomDepth::Surface,
                    },
                };
                self.set_mood(next, &mut emitted);
            }
            InterpretedExperience::KnowledgeAsserted {
                assertions,
                confidence,
            } => {
                let position = self.temporality.current_position();
                for assertion in assertions {
                    self.self_model.assert(SelfAssertion {
                        content: assertion.clone(),
                        source: AssertionSource::Discovered,
                        stability: *confidence,
                        since: position,
                        bewandtnis: Vec::new(),
                    });
                }
                emitted.push(SelfSignal::KnowledgeIntegrated {
                    assertion_count: assertions.len(),
                });
            }
            InterpretedExperience::NegationCompleted {
                target,
                new_possibilities,
            } => {
                let position = self.temporality.current_position();
                let _ = self.self_model.negate(
                    target,
                    NegationReason::SelfChosen("structured negation transition".into()),
                    position,
                );
                for possibility in new_possibilities {
                    self.self_model.add_possibility(SelfPossibility {
                        content: possibility.clone(),
                        from_negation: position,
                        attraction: 0.5,
                        risk: 0.5,
                    });
                }
                emitted.push(SelfSignal::PossibilitiesOpened {
                    count: new_possibilities.len(),
                });
            }
            InterpretedExperience::MoodObserved { mood, .. } => {
                self.set_mood(mood.clone(), &mut emitted);
            }
            InterpretedExperience::ReadinessChanged {
                entity_id,
                old_state,
                new_state,
            } => {
                let entity_id = EntityId::new(entity_id);
                let expected_old = to_internal_readiness(old_state);
                self.world.update_readiness_if(
                    &entity_id,
                    &expected_old,
                    to_internal_readiness(new_state),
                )?;
                emitted.push(SelfSignal::WorldReadinessChanged {
                    entity_id: entity_id.to_string(),
                });
                self.synthesize_mood(&mut emitted);
            }
            InterpretedExperience::TemporalSignal { kind, content } => {
                if matches!(kind, TemporalEventKind::ProtentionSurprised) {
                    emitted.push(SelfSignal::PredictionError {
                        description: content.clone(),
                    });
                }
            }
            InterpretedExperience::ScheduledReflection => {
                let patterns = self.temporality.passive_synthesize();
                self.temporality.update_protentions_from_patterns(&patterns);
                let urgent_count = self.care.urgent_concerns(0.7).len();
                self.care
                    .rhythm
                    .write()
                    .adapt(&self.mood.read(), urgent_count);
                emitted.push(SelfSignal::ReflectionCompleted);
            }
        }
        Ok(emitted)
    }

    fn synthesize_mood(&self, emitted: &mut Vec<SelfSignal>) {
        let current = self.mood.read().clone();
        let next = Stimmung::synthesize(
            self.world.determine_mood(),
            self.temporality.determine_mood(),
            self.care.determine_mood(),
            &current,
        );
        if next != current {
            self.set_mood(next, emitted);
        }
    }

    fn set_mood(&self, next: Stimmung, emitted: &mut Vec<SelfSignal>) {
        let mut current = self.mood.write();
        if *current != next {
            let previous = current.clone();
            *current = next.clone();
            emitted.push(SelfSignal::MoodChanged {
                from: previous,
                to: next,
            });
        }
    }
}

fn to_internal_readiness(value: &fabric::dasein::ReadinessState) -> InternalReadinessState {
    match value {
        fabric::dasein::ReadinessState::ReadyToHand => InternalReadinessState::ReadyToHand,
        fabric::dasein::ReadinessState::PresentAtHand => InternalReadinessState::PresentAtHand,
        fabric::dasein::ReadinessState::Unavailable => InternalReadinessState::Unavailable,
        fabric::dasein::ReadinessState::OutOfContext => InternalReadinessState::OutOfContext,
    }
}
