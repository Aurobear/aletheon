use super::reducer::DaseinStateEngine;
use fabric::dasein::{DaseinEvent, ExperienceSource, InterpretedExperience};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

#[async_trait::async_trait]
pub trait SorgeTimer: Send + Sync {
    async fn sleep(&self, duration: Duration);
}

/// Compatibility wall-clock adapter for callers that do not inject a timer.
///
/// Sorge itself only depends on [`SorgeTimer`]; composition roots can replace
/// this adapter without giving Dasein a dependency on Kernel mechanisms.
#[derive(Debug, Default)]
pub struct SystemSorgeTimer;

#[async_trait::async_trait]
impl SorgeTimer for SystemSorgeTimer {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
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
    pub fn start(&self, engine: Arc<DaseinStateEngine>) -> bool {
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
        let mut event_rx = event_rx;
        let receiver_slot = self.event_rx.clone();
        let timer = self.timer.clone();
        let mut stop_rx = self.stop_tx.subscribe();

        let handle = tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                // 1. React to an event or a scheduled reflection. Incoming events are
                // never delayed behind a periodic sleep.
                let mut events = Vec::new();
                let reflection_interval = engine.reflection_interval();
                let mut scheduled_reflection = false;
                tokio::select! {
                    Some(event) = event_rx.recv() => {
                        events.push(event);
                    }
                    _ = timer.sleep(reflection_interval) => {
                        scheduled_reflection = true;
                    }
                    result = stop_rx.changed() => {
                        let _ = result;
                        break;
                    },
                }

                // Drain any remaining events
                while let Ok(event) = event_rx.try_recv() {
                    events.push(event);
                }

                for event in events {
                    if let Err(error) = engine.apply_compat_event(event, "sorge-event-loop").await {
                        tracing::warn!(%error, "Dasein compatibility event rejected");
                    }
                }
                if scheduled_reflection {
                    let result = engine
                        .transition_current(
                            ExperienceSource::Dasein,
                            "sorge-scheduler",
                            InterpretedExperience::ScheduledReflection,
                        )
                        .await;
                    if let Err(error) = result {
                        tracing::warn!(%error, "Dasein scheduled reflection rejected");
                    }
                }
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
