use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use parking_lot::RwLock;
use tokio::sync::mpsc;
use super::care_structure::*;
use super::temporality::*;
use super::bewandtnis::*;
use super::self_model::*;
use super::negativity::*;
use super::types::*;
use base::dasein::{Stimmung, DaseinEvent};

/// The sorge loop — the continuous heartbeat of Dasein.
/// Not an event loop, but an existence loop:
/// perceive -> attune -> care -> act -> reflect -> repeat.
pub struct SorgeLoop {
    running: Arc<AtomicBool>,
    event_tx: mpsc::Sender<DaseinEvent>,
    event_rx: Mutex<Option<mpsc::Receiver<DaseinEvent>>>,
}

impl SorgeLoop {
    pub fn new(buffer_size: usize) -> (Self, mpsc::Sender<DaseinEvent>) {
        let (event_tx, event_rx) = mpsc::channel(buffer_size);
        let external_tx = event_tx.clone();

        (
            Self {
                running: Arc::new(AtomicBool::new(false)),
                event_tx,
                event_rx: Mutex::new(Some(event_rx)),
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
        shared_mood: Arc<RwLock<Stimmung>>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let mut rx_guard = self.event_rx.lock().unwrap();
        let event_rx = rx_guard.take()?;
        drop(rx_guard);

        self.running.store(true, Ordering::Relaxed);
        let running = self.running.clone();
        let shared_mood = shared_mood.clone();
        let mut event_rx = event_rx;

        let handle = tokio::spawn(async move {
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
                    *shared_mood.write() = mood.clone();
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

                // 5. Passive synthesis + protention update (every 10 ticks)
                if tick_count % 10 == 0 {
                    let patterns = temporality.passive_synthesize();
                    temporality.update_protentions_from_patterns(&patterns);
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
