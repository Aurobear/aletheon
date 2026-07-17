use super::bewandtnis::Bewandtnisganzheit;
use super::care_structure::{CareAction, CareStructure, Concern};
use super::ledger::SelfLedger;
use super::self_model::{
    AssertionSource, MutableSelfModel, NegationReason, SelfAssertion, SelfPossibility,
};
use super::temporality::{ExperientialContent, TemporalStream};
use super::types::BewandtnisNode;
use super::types::{EntityId, ReadinessState as InternalReadinessState};
use fabric::dasein::{
    CareActionKind, DaseinEvent, ExperienceProvenance, ExperienceSource, InterpretedExperience,
    NarrativeEntryId, OutcomeStatus, SelfEventId, SelfSignal, SelfTransitionReceipt,
    SelfTransitionRequest, SelfVersion, Stimmung, TemporalEventKind,
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
    ledger: Option<Arc<SelfLedger>>,
}

impl DaseinStateEngine {
    pub fn new(
        temporality: Arc<TemporalStream>,
        world: Arc<Bewandtnisganzheit>,
        self_model: Arc<MutableSelfModel>,
        care: Arc<CareStructure>,
        mood: Arc<RwLock<Stimmung>>,
        clock: Arc<dyn fabric::Clock>,
        ledger: Option<Arc<SelfLedger>>,
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
            ledger,
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
        self.transition_locked(&mut state, request, true)
    }

    fn transition_locked(
        &self,
        state: &mut ReducerState,
        request: SelfTransitionRequest,
        persist: bool,
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

        self.preflight(&request.content)?;

        let previous_version = state.version;
        let current_version = SelfVersion(previous_version.0 + 1);
        if persist {
            if let Some(ledger) = &self.ledger {
                let durable = ledger.append(&request)?;
                anyhow::ensure!(
                    durable.previous_version == previous_version
                        && durable.current_version == current_version,
                    "self ledger and reducer version diverged"
                );
            }
        }
        let emitted = self.reduce(&request.content);
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
        self.transition_locked(&mut state, request, true)
    }

    pub async fn replay(&self) -> anyhow::Result<usize> {
        let Some(ledger) = &self.ledger else {
            return Ok(0);
        };
        let events = ledger.load_replay_plan()?;
        let mut state = self.state.lock().await;
        anyhow::ensure!(
            state.version == SelfVersion(0) && state.receipts.is_empty(),
            "Dasein replay requires a pristine state engine"
        );
        for event in &events {
            let receipt = self.transition_locked(&mut state, event.request.clone(), false)?;
            anyhow::ensure!(
                receipt.previous_version == event.previous_version
                    && receipt.current_version == event.current_version,
                "Dasein replay receipt diverged at sequence {}",
                event.sequence
            );
        }
        Ok(events.len())
    }

    pub fn checkpoint(&self) -> anyhow::Result<()> {
        let Some(ledger) = &self.ledger else {
            return Ok(());
        };
        let events = ledger.load_verified()?;
        ledger.save_checkpoint(&events, self.clock.wall_now().0)
    }

    pub fn last_durable_observed_at(&self) -> anyhow::Result<Option<fabric::WallTime>> {
        let Some(ledger) = &self.ledger else {
            return Ok(None);
        };
        Ok(ledger
            .load_verified()?
            .last()
            .map(|event| event.request.observed_at))
    }

    fn preflight(&self, content: &InterpretedExperience) -> anyhow::Result<()> {
        if let InterpretedExperience::ReadinessChanged {
            entity_id,
            old_state,
            ..
        } = content
        {
            self.world
                .validate_readiness(&EntityId::new(entity_id), &to_internal_readiness(old_state))?;
        }
        Ok(())
    }

    fn reduce(&self, content: &InterpretedExperience) -> Vec<SelfSignal> {
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
            InterpretedExperience::ConcernObserved {
                id,
                purpose,
                urgency,
            } => {
                self.care.add_concern(Concern {
                    id: id.clone(),
                    purpose: purpose.clone(),
                    urgency: *urgency,
                    involvement_chain: Vec::new(),
                    last_attended: self.temporality.current_position(),
                    mood_tone: self.mood.read().clone(),
                });
                self.synthesize_mood(&mut emitted);
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
            InterpretedExperience::WorldEntityObserved {
                entity_id,
                what_it_is,
                for_the_sake_of,
                readiness,
            } => {
                self.world.add_entity(BewandtnisNode {
                    id: EntityId::new(entity_id),
                    what_it_is: what_it_is.clone(),
                    for_the_sake_of: for_the_sake_of.iter().map(EntityId::new).collect(),
                    appears_in: Vec::new(),
                    readiness: to_internal_readiness(readiness),
                });
                emitted.push(SelfSignal::WorldEntityIntegrated {
                    entity_id: entity_id.clone(),
                });
                self.synthesize_mood(&mut emitted);
            }
            InterpretedExperience::ReadinessChanged {
                entity_id,
                old_state,
                new_state,
            } => {
                let entity_id = EntityId::new(entity_id);
                let expected_old = to_internal_readiness(old_state);
                self.world
                    .update_readiness_if(
                        &entity_id,
                        &expected_old,
                        to_internal_readiness(new_state),
                    )
                    .expect("readiness was verified by reducer preflight");
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
            InterpretedExperience::ResumedAfterInterval { elapsed_ms } => {
                self.temporality.ingest(
                    ExperientialContent {
                        semantic: format!("resumed after {elapsed_ms} ms"),
                        action: Some("runtime_resumption".into()),
                        perception: None,
                        negation: None,
                    },
                    self.mood.read().clone(),
                );
                self.synthesize_mood(&mut emitted);
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
                // R1 (conscious-core plan): consume the care structure's
                // decision so it has a behavioral effect. The decision flows
                // through `emitted` -> WorkspaceContent::Concern -> Agora
                // competition, instead of being computed and discarded.
                let (action, rationale) = match self.care.determine_action() {
                    CareAction::Deliberate(r) => (CareActionKind::Deliberate, r),
                    CareAction::Direct(r) => (CareActionKind::Direct, r),
                    CareAction::Wait(r) => (CareActionKind::Wait, r),
                    CareAction::Negate(r) => (CareActionKind::Negate, r),
                };
                emitted.push(SelfSignal::CareDecision { action, rationale });
            }
        }
        emitted
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
