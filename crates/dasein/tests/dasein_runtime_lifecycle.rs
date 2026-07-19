use dasein::dasein::sorge::SorgeTimer;
use dasein::dasein::{DaseinModule, DaseinRuntimeConfig};
use fabric::dasein::DaseinEvent;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct ManualTimer {
    sleeps: AtomicUsize,
    wake: Notify,
}

impl ManualTimer {
    fn sleep_count(&self) -> usize {
        self.sleeps.load(Ordering::SeqCst)
    }

    fn advance(&self) {
        self.wake.notify_one();
    }
}

#[async_trait::async_trait]
impl SorgeTimer for ManualTimer {
    async fn sleep(&self, _duration: Duration) {
        self.sleeps.fetch_add(1, Ordering::SeqCst);
        self.wake.notified().await;
    }
}

fn test_clock() -> Arc<dyn fabric::Clock> {
    Arc::new(kernel::chronos::TestClock::default())
}

async fn wait_for_position(module: &DaseinModule, expected: u64) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while module.temporality().current_position().0 < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Dasein did not process the event before the test deadline");
}

#[test]
fn configured_values_are_validated() {
    for invalid in [
        DaseinRuntimeConfig {
            retention_depth: 0,
            ..Default::default()
        },
        DaseinRuntimeConfig {
            decay_rate: f64::NAN,
            ..Default::default()
        },
        DaseinRuntimeConfig {
            event_buffer: 0,
            ..Default::default()
        },
    ] {
        assert!(DaseinModule::with_runtime(
            test_clock(),
            Arc::new(ManualTimer::default()),
            invalid
        )
        .is_err());
    }
}

#[tokio::test]
async fn configured_retention_depth_is_honored() {
    let timer = Arc::new(ManualTimer::default());
    let config = DaseinRuntimeConfig {
        retention_depth: 2,
        decay_rate: 1.0,
        ..Default::default()
    };
    let (module, tx) = DaseinModule::with_runtime(test_clock(), timer, config).unwrap();

    for index in 0..4 {
        tx.send(DaseinEvent::UserInput {
            content: format!("event-{index}"),
        })
        .await
        .unwrap();
    }
    assert!(module.start_sorge_loop());
    wait_for_position(&module, 4).await;

    assert_eq!(
        module.temporality().to_snapshot().recent_retentions.len(),
        2
    );
    module.stop_sorge_loop().await;
}

#[tokio::test]
async fn injected_timer_drives_scheduled_reflection() {
    let timer = Arc::new(ManualTimer::default());
    let (module, _tx) =
        DaseinModule::with_runtime(test_clock(), timer.clone(), DaseinRuntimeConfig::default())
            .unwrap();

    assert!(module.start_sorge_loop());
    tokio::time::timeout(Duration::from_secs(1), async {
        while timer.sleep_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = timer.sleep_count();
    timer.advance();
    tokio::time::timeout(Duration::from_secs(1), async {
        while timer.sleep_count() == before {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    module.stop_sorge_loop().await;
}

#[tokio::test]
async fn start_stop_restart() {
    let timer = Arc::new(ManualTimer::default());
    let (module, tx) =
        DaseinModule::with_runtime(test_clock(), timer, DaseinRuntimeConfig::default()).unwrap();

    assert!(module.start_sorge_loop());
    assert!(
        !module.start_sorge_loop(),
        "duplicate start must be rejected"
    );
    tx.send(DaseinEvent::UserInput {
        content: "first".into(),
    })
    .await
    .unwrap();
    wait_for_position(&module, 1).await;
    module.stop_sorge_loop().await;
    assert!(!module.is_alive());

    assert!(module.start_sorge_loop());
    tx.send(DaseinEvent::UserInput {
        content: "second".into(),
    })
    .await
    .unwrap();
    wait_for_position(&module, 2).await;
    module.stop_sorge_loop().await;

    assert_eq!(module.temporality().current_position().0, 2);
    assert!(!module.is_alive());
}
