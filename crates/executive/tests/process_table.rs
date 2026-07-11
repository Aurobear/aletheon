use executive::kernel::chronos::TestClock;
use executive::kernel::process::ProcessTable;
use fabric::{ExitReason, ProcessManager, ProcessSignal, ProcessState, SpawnSpec};
use std::sync::Arc;

#[tokio::test]
async fn process_table_rejects_illegal_transition() {
    let table = ProcessTable::new(Arc::new(TestClock::default()));
    let handle = table.spawn(SpawnSpec::default()).await.unwrap();

    let err = table
        .transition(handle.id, ProcessState::Exited)
        .await
        .expect_err("Created -> Exited must be table-rejected");
    assert!(err.to_string().contains("illegal process transition"));
}

#[tokio::test]
async fn process_table_spawn_wait_reap() {
    let table = Arc::new(ProcessTable::new(Arc::new(TestClock::default())));
    let handle = table.spawn(SpawnSpec::default()).await.unwrap();
    table.signal(handle.id, ProcessSignal::Start).await.unwrap();

    let waiter = {
        let table = table.clone();
        tokio::spawn(async move { table.wait(handle.id).await.unwrap() })
    };
    table
        .mark_exit(handle.id, ExitReason::Completed)
        .await
        .unwrap();
    let exit = waiter.await.unwrap();
    assert_eq!(exit.reason, ExitReason::Completed);

    let reaped = table.reap(handle.id).await.unwrap();
    assert_eq!(reaped.process_id, handle.id);
    assert!(table.inspect(handle.id).await.is_err());
}

#[tokio::test]
async fn process_panic_converts_to_exit_reason_panic() {
    let table = ProcessTable::new(Arc::new(TestClock::default()));
    let handle = table.spawn(SpawnSpec::default()).await.unwrap();
    table.signal(handle.id, ProcessSignal::Kill).await.unwrap();
    let exit = table.wait(handle.id).await.unwrap();
    assert!(matches!(exit.reason, ExitReason::Panic(_)));
}
