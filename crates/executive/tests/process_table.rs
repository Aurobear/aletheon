use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use fabric::{ExitReason, ProcessSignal, SpawnSpec};
use std::sync::Arc;

#[tokio::test]
async fn process_table_rejects_illegal_transition() {
    let runtime = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    let handle = runtime.spawn_process(SpawnSpec::default()).await.unwrap();

    let err = runtime
        .signal_process(handle.id, ProcessSignal::Resume)
        .await
        .expect_err("Created -> Running via Resume must be rejected");
    assert!(err.to_string().contains("illegal process transition"));
}

#[tokio::test]
async fn process_table_spawn_wait_reap() {
    let runtime = Arc::new(KernelRuntime::with_clock(Arc::new(TestClock::default())));
    let handle = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    runtime
        .signal_process(handle.id, ProcessSignal::Start)
        .await
        .unwrap();

    let waiter = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.wait_process(handle.id).await.unwrap() })
    };
    runtime
        .exit_process(handle.id, ExitReason::Completed)
        .await
        .unwrap();
    let exit = waiter.await.unwrap();
    assert_eq!(exit.reason, ExitReason::Completed);

    let reaped = runtime.reap_process(handle.id).await.unwrap();
    assert_eq!(reaped.process_id, handle.id);
    assert!(runtime.inspect_process(handle.id).await.is_err());
}

#[tokio::test]
async fn process_panic_converts_to_exit_reason_panic() {
    let runtime = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    let handle = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    runtime
        .signal_process(handle.id, ProcessSignal::Kill)
        .await
        .unwrap();
    let exit = runtime.wait_process(handle.id).await.unwrap();
    assert!(matches!(exit.reason, ExitReason::Panic(_)));
}
