use ::contracts::types::embodiment::DeviceId;
use ::contracts::types::world_state::{WorldSnapshot, WorldStatePort, ANY_SCHEMA};
use ::contracts::{MonoDeadline, MonoTime};
use hardware::world_state::EmbodimentWorldState;
use kernel::chronos::TestClock;
use std::sync::Arc;

fn world_state(max_devices: usize) -> EmbodimentWorldState {
    EmbodimentWorldState::new(max_devices, Arc::new(TestClock::default()))
}

fn snapshot(device: &str, seq: u64, x: f64) -> WorldSnapshot {
    WorldSnapshot {
        device: DeviceId(device.into()),
        schema: "test".into(),
        schema_version: 1,
        sequence: seq,
        payload: serde_json::json!({"x": x}),
        observed_at: MonoTime(seq),
        valid_until: None,
        stale: false,
    }
}

#[tokio::test]
async fn per_device_latest_sequence() {
    let ws = world_state(5);
    let a = DeviceId("a".into());
    let b = DeviceId("b".into());
    ws.ingest(a.clone(), snapshot("a", 1, 1.0)).unwrap();
    ws.ingest(b.clone(), snapshot("b", 1, 10.0)).unwrap();
    ws.ingest(a.clone(), snapshot("a", 2, 2.0)).unwrap();
    assert_eq!(ws.latest(&a, ANY_SCHEMA).await.unwrap().sequence, 2);
    assert_eq!(ws.latest(&b, ANY_SCHEMA).await.unwrap().sequence, 1);
}

#[tokio::test]
async fn keep_separate_sequence_per_device() {
    let ws = world_state(5);
    ws.ingest(DeviceId("a".into()), snapshot("a", 100, 1.0))
        .unwrap();
    ws.ingest(DeviceId("b".into()), snapshot("b", 1, 2.0))
        .unwrap();
    // b's low sequence doesn't touch a
    assert_eq!(
        ws.latest(&DeviceId("a".into()), ANY_SCHEMA)
            .await
            .unwrap()
            .sequence,
        100
    );
}

#[tokio::test]
async fn observe_until_uses_injected_kernel_clock_for_expiry() {
    let clock = Arc::new(TestClock::new(0, 100));
    let ws = EmbodimentWorldState::new(5, clock);
    let device = DeviceId("deadline-device".into());
    ws.ingest(device.clone(), snapshot("deadline-device", 1, 1.0))
        .unwrap();

    let observed = tokio::time::timeout(
        std::time::Duration::from_millis(20),
        ws.observe_until(&device, ANY_SCHEMA, 1, MonoDeadline(MonoTime(100))),
    )
    .await
    .expect("expired deadline must not wait");
    assert!(observed.is_none());
}

#[tokio::test]
async fn cached_snapshot_becomes_stale_when_provider_deadline_expires() {
    let clock = Arc::new(TestClock::new(0, 100));
    let ws = EmbodimentWorldState::new(5, clock.clone());
    let device = DeviceId("freshness-device".into());
    let mut sample = snapshot("freshness-device", 1, 1.0);
    sample.observed_at = MonoTime(100);
    sample.valid_until = Some(MonoDeadline(MonoTime(110)));
    ws.ingest(device.clone(), sample).unwrap();
    assert!(!ws.latest(&device, ANY_SCHEMA).await.unwrap().stale);

    clock.advance(11);
    assert!(ws.latest(&device, ANY_SCHEMA).await.unwrap().stale);
}
