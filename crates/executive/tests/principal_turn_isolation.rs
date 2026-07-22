use std::sync::Arc;

use executive::runtime::session::canonical_store::CanonicalSessionStore;
use executive::service::turn_coordinator::{ActiveTurnKey, TurnCoordinator, TurnExecution};
use executive::service::turn_policy::TurnPolicy;
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, OperationId, PermissionProfileId,
    PrincipalContext, PrincipalId, SessionAppendStore, ThreadId, TurnMetrics, TurnRequest,
    TurnResult, TurnStop, WorkspacePolicy,
};
use kernel::KernelRuntime;
use tokio::sync::{mpsc, Barrier, Mutex, Semaphore};

fn context(uid: u32, thread: &str, cwd: &str) -> PrincipalContext {
    PrincipalContext::new(
        PrincipalId::local_uid(uid),
        LocalOsPrincipal { uid, gid: uid },
        ConnectionId::new(),
        ThreadId(thread.to_owned()),
        WorkspacePolicy::from_resolved_roots(cwd.into(), Vec::new()).unwrap(),
        PermissionProfileId::workspace_write(),
        ApprovalPolicy::OnRequest,
    )
}

#[tokio::test]
async fn concurrent_principals_keep_distinct_thread_authority() {
    let kernel = Arc::new(KernelRuntime::new());
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = Arc::new(TurnCoordinator::new(kernel.clone(), store));
    let alice_process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let bob_process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let alice = context(1001, "alice-thread", "/tmp/alice");
    let bob = context(1002, "bob-thread", "/tmp/bob");
    let alice_key = ActiveTurnKey::from_context(&alice);
    let bob_key = ActiveTurnKey::from_context(&bob);
    assert_ne!(alice_key, bob_key);

    let captured = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(Semaphore::new(0));
    let (started_tx, mut started_rx) = mpsc::channel(2);

    let spawn_turn = |context: PrincipalContext, process_id| {
        let coordinator = coordinator.clone();
        let captured = captured.clone();
        let release = release.clone();
        let started_tx = started_tx.clone();
        tokio::spawn(async move {
            coordinator
                .submit_with(
                    TurnRequest {
                        operation_id: OperationId::default(),
                        process_id,
                        context,
                        input: "hello".into(),
                        model_policy: Some("test-policy".into()),
                        deadline: None,
                    },
                    &TurnPolicy::daemon(),
                    move |request, _cancel| async move {
                        captured.lock().await.push(request.context.clone());
                        started_tx.send(()).await.unwrap();
                        let _permit = release.acquire().await.unwrap();
                        Ok(TurnExecution {
                            result: TurnResult {
                                output: "ok".into(),
                                stop: TurnStop::Completed,
                                metrics: TurnMetrics {
                                    completed_normally: true,
                                    ..Default::default()
                                },
                            },
                            items: Vec::new(),
                            projection: None,
                            context_projection: None,
                        })
                    },
                )
                .await
        })
    };

    let alice_task = spawn_turn(alice.clone(), alice_process.id);
    let bob_task = spawn_turn(bob.clone(), bob_process.id);
    started_rx.recv().await.unwrap();
    started_rx.recv().await.unwrap();

    let active = coordinator.active_index();
    let active = active.lock().await;
    assert!(active.contains_key(&alice_key));
    assert!(active.contains_key(&bob_key));
    let alice_active = active.get(&alice_key).unwrap().clone();
    drop(active);

    assert!(coordinator
        .cancel_operation_by_key(
            &alice.principal_id,
            &alice.thread_id,
            fabric::TurnId::new(),
            alice_active.operation_id,
        )
        .await
        .is_err());
    assert!(coordinator
        .cancel_operation_by_key(
            &bob.principal_id,
            &alice.thread_id,
            alice_active.turn_id,
            alice_active.operation_id,
        )
        .await
        .is_err());
    coordinator
        .cancel_operation_by_key(
            &alice.principal_id,
            &alice.thread_id,
            alice_active.turn_id,
            alice_active.operation_id,
        )
        .await
        .unwrap();
    assert!(alice_active.cancel.is_cancelled());

    release.add_permits(2);
    alice_task.await.unwrap().unwrap();
    bob_task.await.unwrap().unwrap();

    let captured = captured.lock().await;
    assert!(captured.iter().any(|context| {
        context.principal_id == alice.principal_id
            && context.thread_id == alice.thread_id
            && context.workspace == alice.workspace
    }));
    assert!(captured.iter().any(|context| {
        context.principal_id == bob.principal_id
            && context.thread_id == bob.thread_id
            && context.workspace == bob.workspace
    }));
}

fn completed() -> TurnExecution {
    TurnExecution {
        result: TurnResult {
            output: "ok".into(),
            stop: TurnStop::Completed,
            metrics: TurnMetrics {
                completed_normally: true,
                ..Default::default()
            },
        },
        items: Vec::new(),
        projection: None,
        context_projection: None,
    }
}

#[tokio::test]
async fn concurrent_backpressure_admission_never_oversubscribes_capacity() {
    let kernel = Arc::new(KernelRuntime::new());
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = Arc::new(
        TurnCoordinator::new(kernel.clone(), store).with_backpressure(
            executive::core::config::BackpressureConfig {
                max_concurrent_turns: Some(1),
            },
        ),
    );
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let release = Arc::new(Semaphore::new(0));
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for (uid, thread) in [(2001, "capacity-a"), (2002, "capacity-b")] {
        let coordinator = coordinator.clone();
        let barrier = barrier.clone();
        let release = release.clone();
        let started = started.clone();
        let request = TurnRequest {
            operation_id: OperationId::default(),
            process_id: process.id,
            context: context(uid, thread, "/tmp"),
            input: "capacity".into(),
            model_policy: None,
            deadline: None,
        };
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            coordinator
                .submit_with(request, &TurnPolicy::daemon(), move |_, _| async move {
                    started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let _permit = release.acquire().await.unwrap();
                    Ok(completed())
                })
                .await
        }));
    }
    barrier.wait().await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(coordinator.active_turn_count().await, 1);
    assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 1);
    release.add_permits(1);
    let first = tasks.remove(0).await.unwrap();
    let second = tasks.remove(0).await.unwrap();
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(kernel.active_operation_count(process.id).await, 0);
}

#[tokio::test]
async fn duplicate_principal_thread_is_rejected_and_kernel_operation_is_cancelled() {
    let kernel = Arc::new(KernelRuntime::new());
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = Arc::new(TurnCoordinator::new(kernel.clone(), store));
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let shared_context = context(3001, "duplicate", "/tmp");
    let release = Arc::new(Semaphore::new(0));
    let first = {
        let coordinator = coordinator.clone();
        let release = release.clone();
        let request = TurnRequest {
            operation_id: OperationId::default(),
            process_id: process.id,
            context: shared_context.clone(),
            input: "first".into(),
            model_policy: None,
            deadline: None,
        };
        tokio::spawn(async move {
            coordinator
                .submit_with(request, &TurnPolicy::daemon(), move |_, _| async move {
                    let _permit = release.acquire().await.unwrap();
                    Ok(completed())
                })
                .await
        })
    };
    while coordinator.active_turn_count().await == 0 {
        tokio::task::yield_now().await;
    }
    let duplicate = coordinator
        .submit_with(
            TurnRequest {
                operation_id: OperationId::default(),
                process_id: process.id,
                context: shared_context,
                input: "duplicate".into(),
                model_policy: None,
                deadline: None,
            },
            &TurnPolicy::daemon(),
            |_, _| async { panic!("duplicate runner must not start") },
        )
        .await;
    assert!(duplicate
        .unwrap_err()
        .to_string()
        .contains("already has an active turn"));
    assert_eq!(kernel.active_operation_count(process.id).await, 1);
    release.add_permits(1);
    first.await.unwrap().unwrap();
    assert_eq!(kernel.active_operation_count(process.id).await, 0);
}
