use super::bewandtnis::*;
use super::care_structure::*;
use super::negativity::*;
use super::self_model::*;
use super::temporality::*;
use aletheon_kernel::chronos::SystemTimer;
use fabric::dasein::{DaseinEvent, Stimmung};
use fabric::Timer;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

#[async_trait::async_trait]
pub trait SorgeTimer: Send + Sync {
    async fn sleep(&self, duration: Duration);
}

#[derive(Debug, Default)]
pub struct SystemSorgeTimer;

#[async_trait::async_trait]
impl SorgeTimer for SystemSorgeTimer {
    async fn sleep(&self, duration: Duration) {
        SystemTimer.sleep(duration).await;
    }
}

/// The sorge loop — the continuous heartbeat of Dasein.
/// Not an event loop, but an existence loop:
/// perceive -> attune -> care -> act -> reflect -> repeat.
pub struct SorgeLoop {
    running: Arc<AtomicBool>,
    /// Parked roadmap item: internal sender for self-event loop (T3).
    #[allow(dead_code)]
    event_tx: mpsc::Sender<DaseinEvent>,
    event_rx: Arc<Mutex<Option<mpsc::Receiver<DaseinEvent>>>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    stop_tx: watch::Sender<u64>,
    timer: Arc<dyn SorgeTimer>,
    #[allow(dead_code)]
    clock: Arc<dyn fabric::Clock>,
}

impl SorgeLoop {
    pub fn new(
        buffer_size: usize,
        clock: Arc<dyn fabric::Clock>,
        timer: Arc<dyn SorgeTimer>,
    ) -> (Self, mpsc::Sender<DaseinEvent>) {
        let (event_tx, event_rx) = mpsc::channel(buffer_size);
        let (stop_tx, _) = watch::channel(0);
        let external_tx = event_tx.clone();

        (
            Self {
                running: Arc::new(AtomicBool::new(false)),
                event_tx,
                event_rx: Arc::new(Mutex::new(Some(event_rx))),
                task: Mutex::new(None),
                stop_tx,
                timer,
                clock,
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
    ) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        let mut rx_guard = self.event_rx.lock().unwrap();
        let Some(event_rx) = rx_guard.take() else {
            self.running.store(false, Ordering::SeqCst);
            return false;
        };
        drop(rx_guard);

        let running = self.running.clone();
        let shared_mood = shared_mood.clone();
        let mut event_rx = event_rx;
        let receiver_slot = self.event_rx.clone();
        let timer = self.timer.clone();
        let mut stop_rx = self.stop_tx.subscribe();

        let handle = tokio::spawn(async move {
            let mut tick_count: u64 = 0;
            let mut mood = Stimmung::Gelassenheit;

            while running.load(Ordering::Relaxed) {
                // 1. React to an event or a scheduled reflection. Incoming events are
                // never delayed behind a periodic sleep.
                let mut events = Vec::new();
                let reflection_interval = care.rhythm.read().next_interval();
                tokio::select! {
                    Some(event) = event_rx.recv() => {
                        events.push(event);
                    }
                    _ = timer.sleep(reflection_interval) => {}
                    result = stop_rx.changed() => {
                        let _ = result;
                        break;
                    },
                }

                // Drain any remaining events
                while let Ok(event) = event_rx.try_recv() {
                    events.push(event);
                }

                // 2. Ingest events into temporal stream
                for event in &events {
                    let content = match event {
                        DaseinEvent::UserInput { content } => ExperientialContent {
                            semantic: content.clone(),
                            action: Some("user_interaction".to_string()),
                            perception: None,
                            negation: None,
                        },
                        DaseinEvent::SystemEvent { source, content } => ExperientialContent {
                            semantic: format!("[{}] {}", source, content),
                            action: None,
                            perception: Some(content.clone()),
                            negation: None,
                        },
                        DaseinEvent::TimerTick => ExperientialContent {
                            semantic: "tick".to_string(),
                            action: None,
                            perception: None,
                            negation: None,
                        },
                        _ => continue,
                    };
                    temporality.ingest(content, mood.clone());
                }

                // 3. Update mood from all sources
                let world_mood = world.determine_mood();
                let temporal_mood = temporality.determine_mood();
                let care_mood = care.determine_mood();

                let new_mood = Stimmung::synthesize(world_mood, temporal_mood, care_mood, &mood);
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
            }
            running.store(false, Ordering::SeqCst);
            *receiver_slot.lock().unwrap() = Some(event_rx);
        });
        *self.task.lock().unwrap() = Some(handle);
        true
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.stop_tx.send_modify(|generation| *generation += 1);
        let handle = self.task.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}
