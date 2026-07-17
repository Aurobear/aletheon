use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agora::{
    BroadcastCoordinator, BroadcastHub, BroadcastHubConfig, CandidatePoolConfig, SelectionPolicy,
    SqliteBroadcastStore,
};
use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use async_trait::async_trait;
use dasein::dasein::care_structure::Concern;
use dasein::dasein::types::TemporalPosition;
use executive::service::conscious_core_coordinator::{
    ConsciousCoreConfig, ConsciousCoreCoordinator,
};
use executive::service::conscious_core_ports::{
    CandidateAdmissionStatus, CandidateCause, CandidateSubmission, ConsciousCandidatePort,
    LatestConsciousContextPort,
};
use executive::service::dasein_workspace_adapter::DaseinWorkspaceAdapter;
use fabric::{
    AgentId, AgentProfileId, AgoraSpaceId, ConsciousProcessor, ContentId, MonoDeadline, MonoTime,
    NamespaceId, PredictionFrame, ProcessId, ProcessorAck, ProcessorContext, ProcessorHealth,
    ProcessorId, ProcessorResponse, SalienceVector, SpawnSpec, VisibilityScope, WallTime,
    WorkspaceBroadcast, WorkspaceCandidate, WorkspaceContent, WorkspaceObservation,
    WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
};
use uuid::Uuid;

const SPACE: &str = "session:recurrence";

struct ResponseProcessor {
    id: ProcessorId,
    source: ProcessId,
    response_id: Option<ContentId>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ConsciousProcessor for ResponseProcessor {
    fn id(&self) -> ProcessorId {
        self.id.clone()
    }

    async fn on_broadcast(
        &self,
        broadcast: WorkspaceBroadcast,
        context: ProcessorContext,
    ) -> ProcessorResponse {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let candidates = self
            .response_id
            .map(|id| {
                let now = MonoTime(5);
                WorkspaceCandidate {
                    schema_version: WORKSPACE_SCHEMA_V1,
                    id,
                    space: broadcast.space.clone(),
                    source: self.source,
                    turn: None,
                    content: WorkspaceContent::Observation(WorkspaceObservation {
                        what: "processor response affects the next competition".into(),
                        source: self.id.0.clone(),
                        data: serde_json::json!({"source_epoch": broadcast.epoch.0}),
                        attribution: fabric::WorkspaceAttribution::Cognit,
                    }),
                    confidence: 1.0,
                    salience: SalienceVector {
                        urgency: 1.0,
                        goal_relevance: 1.0,
                        self_relevance: 1.0,
                        novelty: 1.0,
                        confidence: 1.0,
                        prediction_error: 1.0,
                        affect_intensity: 1.0,
                        social_relevance: 1.0,
                    },
                    provenance: WorkspaceProvenance {
                        producer: self.source,
                        operation: None,
                        source_refs: vec![
                            format!("broadcast:{}:{}", broadcast.space.0, broadcast.epoch.0),
                            format!("processor:{}", self.id.0),
                        ],
                        observed_at: WallTime(5),
                    },
                    visibility: VisibilityScope::Session,
                    dependencies: broadcast.winner_ids.clone(),
                    created_at: now,
                    expires_at: Some(MonoDeadline::after(now, 5_000)),
                }
            })
            .into_iter()
            .collect();
        ProcessorResponse {
            processor: self.id.clone(),
            source_epoch: context.source_epoch,
            health: ProcessorHealth::Healthy,
            candidates,
            acknowledgements: broadcast
                .winner_ids
                .iter()
                .map(|id| ProcessorAck {
                    content_id: *id,
                    accepted: true,
                    detail: None,
                })
                .collect(),
            detail: None,
        }
    }
}

struct Fixture {
    coordinator: Arc<ConsciousCoreCoordinator>,
    store: Arc<SqliteBroadcastStore>,
    owner: ProcessId,
    first_calls: Arc<AtomicUsize>,
    second_calls: Arc<AtomicUsize>,
}

async fn fixture() -> Fixture {
    let clock = Arc::new(TestClock::new(1_000, 5));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let owner = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId(Uuid::from_u128(1)),
            parent: None,
            profile: AgentProfileId("root".into()),
            namespace: NamespaceId("recurrence".into()),
            initial_operation: None,
            deadline: None,
        })
        .await
        .unwrap()
        .id;
    let dasein = Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0);
    dasein.care().add_concern(Concern {
        id: "safety".into(),
        purpose: "safety review".into(),
        urgency: 0.9,
        involvement_chain: vec![],
        last_attended: TemporalPosition(0),
        mood_tone: fabric::dasein::Stimmung::Gelassenheit,
    });
    let dasein_port = Arc::new(DaseinWorkspaceAdapter::new(dasein, clock));
    let store = Arc::new(SqliteBroadcastStore::open_in_memory().unwrap());
    let hub = Arc::new(BroadcastHub::new(BroadcastHubConfig::default(), store.clone()).unwrap());
    let broadcast = Arc::new(BroadcastCoordinator::new(store.clone(), hub));
    let coordinator = Arc::new(
        ConsciousCoreCoordinator::new(
            AgoraSpaceId(SPACE.into()),
            CandidatePoolConfig {
                capacity: 64,
                per_source_capacity: 16,
                max_coalition: 4,
                policy: SelectionPolicy {
                    ignition_threshold: 0.5,
                    ..SelectionPolicy::default()
                },
            },
            broadcast,
            store.clone(),
            dasein_port,
            ProcessId(Uuid::from_u128(2)),
            kernel,
            ConsciousCoreConfig {
                cycle_timeout: Duration::from_secs(5),
                processor_timeout: Duration::from_millis(100),
                ..ConsciousCoreConfig::default()
            },
        )
        .unwrap(),
    );
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    coordinator
        .register_processor(
            Arc::new(ResponseProcessor {
                id: ProcessorId("cognit".into()),
                source: ProcessId(Uuid::from_u128(3)),
                response_id: Some(ContentId(Uuid::from_u128(300))),
                calls: first_calls.clone(),
            }),
            owner,
            owner,
        )
        .unwrap();
    coordinator
        .register_processor(
            Arc::new(ResponseProcessor {
                id: ProcessorId("mnemosyne".into()),
                source: ProcessId(Uuid::from_u128(4)),
                response_id: None,
                calls: second_calls.clone(),
            }),
            owner,
            owner,
        )
        .unwrap();
    Fixture {
        coordinator,
        store,
        owner,
        first_calls,
        second_calls,
    }
}

fn observation(id: u128, score: f32, expires_at: Option<MonoDeadline>) -> CandidateSubmission {
    let source = ProcessId(Uuid::from_u128(10));
    let event_ref = format!("event:external:{id}");
    CandidateSubmission {
        candidate: WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: ContentId(Uuid::from_u128(id)),
            space: AgoraSpaceId(SPACE.into()),
            source,
            turn: None,
            content: WorkspaceContent::Observation(WorkspaceObservation {
                what: format!("observation-{id}"),
                source: "environment".into(),
                data: serde_json::json!({"event_id": id}),
                attribution: fabric::WorkspaceAttribution::Environment,
            }),
            confidence: 1.0,
            salience: SalienceVector {
                urgency: score,
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
                source_refs: vec![event_ref.clone()],
                observed_at: WallTime(5),
            },
            visibility: VisibilityScope::Session,
            dependencies: vec![],
            created_at: MonoTime(0),
            expires_at,
        },
        cause: CandidateCause::ExternalObservation { event_ref },
    }
}

#[tokio::test]
async fn observation_selection_self_integration_and_processor_response_recur() {
    let fixture = fixture().await;
    let selected = fixture
        .coordinator
        .submit_candidate(observation(100, 1.0, None))
        .await
        .unwrap();
    assert_eq!(selected.status, CandidateAdmissionStatus::Accepted);
    let unselected = fixture
        .coordinator
        .submit_candidate(observation(101, 0.0, None))
        .await
        .unwrap();
    assert_eq!(unselected.status, CandidateAdmissionStatus::Accepted);
    let expired = fixture
        .coordinator
        .submit_candidate(observation(102, 1.0, Some(MonoDeadline(MonoTime(1)))))
        .await
        .unwrap();
    assert_eq!(expired.status, CandidateAdmissionStatus::RejectedInvalid);

    let first = fixture
        .coordinator
        .run_cycle(fixture.owner, 0)
        .await
        .unwrap();
    let first_broadcast = first.broadcast.as_ref().unwrap();
    assert_eq!(
        first_broadcast.winner_ids,
        vec![ContentId(Uuid::from_u128(100))]
    );
    let transition = first.dasein_transition.as_ref().unwrap();
    assert_eq!(transition.previous_version.0, 0);
    assert_eq!(transition.current_version.0, 1);
    assert_eq!(fixture.first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.second_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .store
            .processor_responses(&AgoraSpaceId(SPACE.into()), first_broadcast.epoch)
            .unwrap()
            .len(),
        2
    );
    let integration = fixture
        .store
        .integration(&AgoraSpaceId(SPACE.into()), first_broadcast.epoch)
        .unwrap()
        .unwrap();
    assert_eq!(integration.transition.event_id, transition.event_id);
    assert_eq!(integration.transition.current_version.0, 1);

    let first_context = fixture
        .coordinator
        .latest_context(&AgoraSpaceId(SPACE.into()))
        .await
        .unwrap();
    assert_eq!(
        first_context.receipt.content_ids,
        vec![ContentId(Uuid::from_u128(100))]
    );
    assert!(!first_context
        .receipt
        .content_ids
        .contains(&ContentId(Uuid::from_u128(101))));

    let second = fixture
        .coordinator
        .run_cycle(fixture.owner, 1)
        .await
        .unwrap();
    let second_broadcast = second.broadcast.as_ref().unwrap();
    assert_eq!(
        second_broadcast.winner_ids[0],
        ContentId(Uuid::from_u128(300))
    );
    assert_eq!(second.dasein_transition.unwrap().current_version.0, 2);
    assert_eq!(second_broadcast.epoch.0, first_broadcast.epoch.0 + 1);
}

#[tokio::test]
async fn recorded_fixture_reproduces_winner_and_dasein_event_identity() {
    let first = fixture().await;
    let second = fixture().await;
    for fixture in [&first, &second] {
        fixture
            .coordinator
            .submit_candidate(observation(100, 1.0, None))
            .await
            .unwrap();
    }
    let first_receipt = first.coordinator.run_cycle(first.owner, 0).await.unwrap();
    let second_receipt = second.coordinator.run_cycle(second.owner, 0).await.unwrap();
    assert_eq!(
        first_receipt.broadcast.unwrap().winner_ids,
        second_receipt.broadcast.unwrap().winner_ids
    );
    assert_eq!(
        first_receipt.dasein_transition.unwrap().event_id,
        second_receipt.dasein_transition.unwrap().event_id
    );
}

#[tokio::test]
async fn dasein_concern_modulation_changes_which_candidate_ignites() {
    let fixture = fixture().await;
    let ordinary = observation(110, 0.4, None);
    let mut cared_for = observation(111, 0.2, None);
    let WorkspaceContent::Observation(value) = &mut cared_for.candidate.content else {
        unreachable!()
    };
    value.what = "perform a safety review before acting".into();
    fixture
        .coordinator
        .submit_candidate(ordinary)
        .await
        .unwrap();
    fixture
        .coordinator
        .submit_candidate(cared_for)
        .await
        .unwrap();

    let cycle = fixture
        .coordinator
        .run_cycle(fixture.owner, 0)
        .await
        .unwrap();
    assert_eq!(
        cycle.broadcast.unwrap().winner_ids,
        vec![ContentId(Uuid::from_u128(111))]
    );
}

#[test]
fn typed_prediction_content_remains_distinct_from_observation() {
    let content = WorkspaceContent::Prediction(PredictionFrame {
        statement: "a later selected outcome will test this".into(),
        horizon_ms: 1_000,
    });
    assert!(matches!(content, WorkspaceContent::Prediction(_)));
}
