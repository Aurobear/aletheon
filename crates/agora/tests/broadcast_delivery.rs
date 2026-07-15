use agora::{
    AdmissionOutcome, BroadcastCoordinator, BroadcastHub, BroadcastHubConfig, BroadcastProcessor,
    CandidatePool, CandidatePoolConfig, ProcessorRegistration, SelectionPolicy,
    SqliteBroadcastStore,
};
use async_trait::async_trait;
use fabric::dasein::SelfVersion;
use fabric::{
    AgoraSpaceId, BroadcastAck, BroadcastAckStatus, BroadcastDelivery, CandidateScore, ContentId,
    MonoTime, ProcessId, SalienceVector, SelectionExplanation, SelectionResult, VisibilityScope,
    WallTime, WorkspaceCandidate, WorkspaceContent, WorkspaceObservation, WorkspaceProvenance,
    WORKSPACE_SCHEMA_V1,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;
use uuid::Uuid;

fn candidate(id: u128, visibility: VisibilityScope) -> WorkspaceCandidate {
    let source = ProcessId(Uuid::from_u128(100));
    WorkspaceCandidate {
        schema_version: WORKSPACE_SCHEMA_V1,
        id: ContentId(Uuid::from_u128(id)),
        space: AgoraSpaceId("space".into()),
        source,
        turn: None,
        content: WorkspaceContent::Observation(WorkspaceObservation {
            what: format!("selected-{id}"),
            source: "fixture".into(),
            data: serde_json::json!({"id": id}),
        }),
        confidence: 1.0,
        salience: SalienceVector {
            urgency: 1.0,
            goal_relevance: 0.0,
            self_relevance: 0.0,
            novelty: 0.0,
            confidence: 0.0,
            prediction_error: 0.0,
            affect_intensity: 0.0,
            social_relevance: 0.0,
        },
        provenance: WorkspaceProvenance {
            producer: source,
            operation: None,
            source_refs: vec![format!("fixture://{id}")],
            observed_at: WallTime(1),
        },
        visibility,
        dependencies: Vec::new(),
        created_at: MonoTime(1),
        expires_at: None,
    }
}

fn selection(selected: Vec<WorkspaceCandidate>) -> SelectionResult {
    let selected_ids = selected.iter().map(|value| value.id).collect();
    SelectionResult {
        selected,
        explanation: SelectionExplanation {
            policy_version: 1,
            evaluated: Vec::<CandidateScore>::new(),
            selected_ids,
            rejected_below_ignition: Vec::new(),
        },
    }
}

fn ack(epoch: u64, processor: u128) -> BroadcastAck {
    BroadcastAck {
        schema_version: WORKSPACE_SCHEMA_V1,
        space: AgoraSpaceId("space".into()),
        epoch: fabric::BroadcastEpoch(epoch),
        processor: ProcessId(Uuid::from_u128(processor)),
        response_ids: Vec::new(),
        status: BroadcastAckStatus::Delivered,
        observed_at: WallTime(2),
        detail: None,
    }
}

#[test]
fn store_reopens_and_replays_exact_epoch_response_graph() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("broadcast.sqlite");
    let first_bytes;
    {
        let store = SqliteBroadcastStore::open(&path).unwrap();
        let broadcast = store
            .open_selection(
                selection(vec![candidate(1, VisibilityScope::Session)]),
                SelfVersion(1),
                1,
                WallTime(1),
            )
            .unwrap();
        assert_eq!(broadcast.epoch.0, 1);
        let retried = store
            .open_selection(
                selection(vec![candidate(1, VisibilityScope::Session)]),
                SelfVersion(1),
                1,
                WallTime(99),
            )
            .unwrap();
        assert_eq!(retried.epoch, broadcast.epoch);
        let value = ack(1, 9);
        store.append_ack(&value).unwrap();
        store.append_ack(&value).unwrap();
        store
            .close_epoch(&AgoraSpaceId("space".into()), broadcast.epoch, WallTime(3))
            .unwrap();
        first_bytes =
            serde_json::to_vec(&store.replay(&AgoraSpaceId("space".into())).unwrap()[0].broadcast)
                .unwrap();
    }
    let reopened = SqliteBroadcastStore::open(&path).unwrap();
    let replay = reopened.replay(&AgoraSpaceId("space".into())).unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].acknowledgements.len(), 1);
    assert_eq!(replay[0].closed_at, Some(WallTime(3)));
    assert_eq!(
        serde_json::to_vec(&replay[0].broadcast).unwrap(),
        first_bytes
    );
}

#[test]
fn store_rejects_conflicts_closed_acks_and_checksum_corruption() {
    let store = SqliteBroadcastStore::open_in_memory().unwrap();
    store
        .open_selection(
            selection(vec![candidate(1, VisibilityScope::Session)]),
            SelfVersion(1),
            1,
            WallTime(1),
        )
        .unwrap();
    let value = ack(1, 9);
    store.append_ack(&value).unwrap();
    let mut conflict = value.clone();
    conflict.observed_at = WallTime(99);
    assert!(store.append_ack(&conflict).is_err());
    store
        .close_epoch(
            &AgoraSpaceId("space".into()),
            fabric::BroadcastEpoch(1),
            WallTime(3),
        )
        .unwrap();
    assert!(store.append_ack(&ack(1, 10)).is_err());
    store
        .connection_for_test()
        .execute("UPDATE broadcast_epochs SET checksum = 'bad'", [])
        .unwrap();
    assert!(store.replay(&AgoraSpaceId("space".into())).is_err());
}

#[derive(Clone)]
enum Behavior {
    Respond(ContentId),
    Fail,
    Sleep,
}

struct RecordingProcessor {
    behavior: Behavior,
    deliveries: Arc<Mutex<Vec<Vec<ContentId>>>>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

#[async_trait]
impl BroadcastProcessor for RecordingProcessor {
    async fn receive(&self, delivery: BroadcastDelivery) -> anyhow::Result<Vec<ContentId>> {
        struct ActiveGuard<'a>(&'a AtomicUsize);
        impl Drop for ActiveGuard<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        let _guard = ActiveGuard(&self.active);
        self.peak.fetch_max(active, Ordering::SeqCst);
        self.deliveries.lock().unwrap().push(
            delivery
                .selected
                .iter()
                .map(|candidate| candidate.id)
                .collect(),
        );
        let result = match self.behavior {
            Behavior::Respond(id) => Ok(vec![id]),
            Behavior::Fail => Err(anyhow::anyhow!("processor failed")),
            Behavior::Sleep => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(Vec::new())
            }
        };
        result
    }
}

fn processor(
    behavior: Behavior,
    deliveries: Arc<Mutex<Vec<Vec<ContentId>>>>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
) -> Arc<dyn BroadcastProcessor> {
    Arc::new(RecordingProcessor {
        behavior,
        deliveries,
        active,
        peak,
    })
}

#[tokio::test]
async fn hub_filters_visibility_bounds_capacity_and_records_terminal_edges() {
    let store = Arc::new(SqliteBroadcastStore::open_in_memory().unwrap());
    let hub = BroadcastHub::new(
        BroadcastHubConfig {
            max_processors: 3,
            max_concurrency: 1,
            delivery_timeout: Duration::from_millis(10),
        },
        store.clone(),
    )
    .unwrap();
    let private_process = ProcessId(Uuid::from_u128(1));
    let tree_process = ProcessId(Uuid::from_u128(2));
    let tree_root = ProcessId(Uuid::from_u128(20));
    let third = ProcessId(Uuid::from_u128(3));
    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    for (process, root, behavior) in [
        (
            private_process,
            private_process,
            Behavior::Respond(ContentId(Uuid::from_u128(91))),
        ),
        (tree_process, tree_root, Behavior::Fail),
        (third, third, Behavior::Sleep),
    ] {
        hub.register(ProcessorRegistration {
            process,
            agent_root: root,
            processor: processor(behavior, deliveries.clone(), active.clone(), peak.clone()),
        })
        .await
        .unwrap();
    }
    assert!(hub
        .register(ProcessorRegistration {
            process: ProcessId(Uuid::from_u128(4)),
            agent_root: ProcessId(Uuid::from_u128(4)),
            processor: processor(
                Behavior::Fail,
                deliveries.clone(),
                active.clone(),
                peak.clone()
            ),
        })
        .await
        .is_err());
    let selected = vec![
        candidate(1, VisibilityScope::Session),
        candidate(
            2,
            VisibilityScope::PrivateProcess {
                process: private_process,
            },
        ),
        candidate(3, VisibilityScope::AgentTree { root: tree_root }),
    ];
    let broadcast = store
        .open_selection(selection(selected), SelfVersion(1), 1, WallTime(1))
        .unwrap();
    let acknowledgements = hub.deliver(&broadcast, WallTime(2)).await.unwrap();
    assert_eq!(acknowledgements.len(), 3);
    assert!(acknowledgements
        .iter()
        .any(|value| value.status == BroadcastAckStatus::Responded));
    assert!(acknowledgements
        .iter()
        .any(|value| value.status == BroadcastAckStatus::Failed));
    assert!(acknowledgements
        .iter()
        .any(|value| value.status == BroadcastAckStatus::TimedOut));
    assert_eq!(peak.load(Ordering::SeqCst), 1);
    let delivered = deliveries.lock().unwrap();
    assert!(delivered
        .iter()
        .any(|ids| ids.len() == 2 && ids.contains(&ContentId(Uuid::from_u128(2)))));
    assert!(delivered
        .iter()
        .any(|ids| ids.len() == 2 && ids.contains(&ContentId(Uuid::from_u128(3)))));
    assert!(delivered
        .iter()
        .any(|ids| ids == &vec![ContentId(Uuid::from_u128(1))]));
}

fn pool() -> CandidatePool {
    CandidatePool::new(
        AgoraSpaceId("space".into()),
        CandidatePoolConfig {
            capacity: 8,
            per_source_capacity: 8,
            max_coalition: 4,
            policy: SelectionPolicy {
                ignition_threshold: 0.1,
                ..SelectionPolicy::default()
            },
        },
    )
    .unwrap()
}

#[tokio::test]
async fn coordinator_persists_before_finalize_and_restart_replays_result() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("coordinator.sqlite");
    let store = Arc::new(SqliteBroadcastStore::open(&path).unwrap());
    let hub = Arc::new(BroadcastHub::new(BroadcastHubConfig::default(), store.clone()).unwrap());
    let coordinator = BroadcastCoordinator::new(store.clone(), hub);
    let mut candidates = pool();
    let value = candidate(1, VisibilityScope::Session);
    assert!(matches!(
        candidates.admit(value, MonoTime(1)),
        AdmissionOutcome::Accepted { .. }
    ));
    let selected = candidates.select(MonoTime(1));
    let broadcast = coordinator
        .broadcast_selection(
            &mut candidates,
            selected,
            SelfVersion(1),
            1,
            WallTime(1),
            WallTime(2),
        )
        .await
        .unwrap();
    assert!(candidates.is_empty());
    drop(coordinator);
    drop(store);
    let reopened = SqliteBroadcastStore::open(&path).unwrap();
    let replay = reopened.replay(&AgoraSpaceId("space".into())).unwrap();
    assert_eq!(
        replay[0].broadcast.checksum().unwrap(),
        broadcast.checksum().unwrap()
    );
    assert_eq!(replay[0].closed_at, Some(WallTime(2)));
}

#[tokio::test]
async fn coordinator_open_failure_keeps_candidates_pending() {
    let store = Arc::new(SqliteBroadcastStore::open_in_memory().unwrap());
    let hub = Arc::new(BroadcastHub::new(BroadcastHubConfig::default(), store.clone()).unwrap());
    let coordinator = BroadcastCoordinator::new(store, hub);
    let mut candidates = pool();
    candidates.admit(candidate(1, VisibilityScope::Session), MonoTime(1));
    let selected = candidates.select(MonoTime(1));
    assert!(coordinator
        .broadcast_selection(
            &mut candidates,
            selected,
            SelfVersion(1),
            0,
            WallTime(1),
            WallTime(2),
        )
        .await
        .is_err());
    assert_eq!(candidates.len(), 1);
}
