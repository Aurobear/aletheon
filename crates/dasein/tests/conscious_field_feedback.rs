//! R2 acceptance tests: SelfField conscious field feedback monotonic modulation.
//!
//! Verifies that injecting the Fabric reader into SelfField:
//! - raises attention priority for higher-urgency broadcasts (same intent),
//! - preserves exact fallback when the reader is absent, errors, or empty.

use async_trait::async_trait;
use dasein::core::SelfFieldConfig;
use fabric::conscious_arbitration::LatestConsciousContextPort;
use fabric::dasein::Stimmung;
use fabric::dasein::{CareActionKind, SelfSignal, SelfVersion};
use fabric::workspace::{CareConcernFrame, SelectionExplanation};
use fabric::{
    AgoraSpaceId, BroadcastEpoch, ConsciousContextProjection, ContentId, Context,
    ContextProjectionReceipt, Intent, IntentSource, MonoTime, ProcessId, SalienceVector,
    SelfFieldOps, StructuredSelfView, Verdict, VisibilityScope, WorkspaceBroadcast,
    WorkspaceCandidate, WorkspaceContent, WorkspaceProvenance,
};
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Stub port
// ---------------------------------------------------------------------------

enum StubMode {
    /// Return a valid broadcast with the given CareActionKind and concern urgency.
    Broadcast(CareActionKind, f32),
    /// Return an empty projection (latest_broadcast = None).
    Empty,
    /// Return an error.
    Error,
}

struct StubConsciousContextPort {
    mode: StubMode,
}

impl StubConsciousContextPort {
    fn new(mode: StubMode) -> Self {
        Self { mode }
    }
}

fn make_care_decision_candidate(
    action: CareActionKind,
    concern_urgency: f32,
) -> WorkspaceCandidate {
    let id = ContentId::new();
    let source = ProcessId(uuid::Uuid::new_v4());
    WorkspaceCandidate {
        schema_version: fabric::workspace::WORKSPACE_SCHEMA_V1,
        id,
        space: AgoraSpaceId("test-session".into()),
        source,
        turn: None,
        content: WorkspaceContent::Concern(SelfSignal::CareDecision {
            action,
            rationale: "test rationale".into(),
        }),
        confidence: 0.9,
        salience: SalienceVector {
            urgency: concern_urgency,
            goal_relevance: 0.0,
            self_relevance: concern_urgency,
            novelty: 0.0,
            confidence: 0.9,
            prediction_error: 0.0,
            affect_intensity: 0.0,
            social_relevance: 0.0,
        },
        provenance: WorkspaceProvenance {
            producer: source,
            operation: None,
            source_refs: vec!["dasein:test".into()],
            observed_at: fabric::WallTime(0),
        },
        visibility: VisibilityScope::Session,
        dependencies: vec![],
        created_at: MonoTime(0),
        expires_at: None,
    }
}

fn make_care_concern_candidate(urgency: f32) -> WorkspaceCandidate {
    let id = ContentId::new();
    let source = ProcessId(uuid::Uuid::new_v4());
    WorkspaceCandidate {
        schema_version: fabric::workspace::WORKSPACE_SCHEMA_V1,
        id,
        space: AgoraSpaceId("test-session".into()),
        source,
        turn: None,
        content: WorkspaceContent::CareConcern(CareConcernFrame {
            purpose: "test concern".into(),
            urgency,
        }),
        confidence: 0.8,
        salience: SalienceVector {
            urgency,
            goal_relevance: 0.0,
            self_relevance: urgency,
            novelty: 0.0,
            confidence: 0.8,
            prediction_error: 0.0,
            affect_intensity: 0.0,
            social_relevance: 0.0,
        },
        provenance: WorkspaceProvenance {
            producer: source,
            operation: None,
            source_refs: vec!["dasein:concern".into()],
            observed_at: fabric::WallTime(0),
        },
        visibility: VisibilityScope::Session,
        dependencies: vec![],
        created_at: MonoTime(0),
        expires_at: None,
    }
}

#[async_trait]
impl LatestConsciousContextPort for StubConsciousContextPort {
    async fn latest_context(
        &self,
        _space: &AgoraSpaceId,
    ) -> anyhow::Result<ConsciousContextProjection> {
        match &self.mode {
            StubMode::Error => anyhow::bail!("stub error"),
            StubMode::Empty => Ok(empty_projection()),
            StubMode::Broadcast(action, urgency) => {
                let care = make_care_decision_candidate(action.clone(), *urgency);
                let concern = make_care_concern_candidate(*urgency);
                let ids = vec![care.id, concern.id];
                let broadcast = WorkspaceBroadcast {
                    schema_version: fabric::workspace::WORKSPACE_SCHEMA_V1,
                    epoch: BroadcastEpoch(1),
                    space: AgoraSpaceId("test-session".into()),
                    winner_ids: ids.clone(),
                    contents: vec![care.content.clone(), concern.content.clone()],
                    selected: vec![care, concern],
                    selected_because: SelectionExplanation {
                        policy_version: 1,
                        evaluated: vec![],
                        selected_ids: ids,
                        rejected_below_ignition: vec![],
                    },
                    dasein_version: SelfVersion(1),
                    workspace_version: 1,
                };
                let receipt = ContextProjectionReceipt {
                    space: AgoraSpaceId("test-session".into()),
                    broadcast_epoch: Some(BroadcastEpoch(1)),
                    workspace_version: Some(1),
                    dasein_version: SelfVersion(1),
                    content_ids: broadcast.winner_ids.clone(),
                };
                Ok(ConsciousContextProjection {
                    latest_broadcast: Some(broadcast),
                    self_view: StructuredSelfView {
                        version: SelfVersion(1),
                        mood: Stimmung::Gelassenheit,
                        concerns: vec![],
                        projection: None,
                        protentions: vec![],
                    },
                    receipt,
                })
            }
        }
    }
}

fn empty_projection() -> ConsciousContextProjection {
    ConsciousContextProjection {
        latest_broadcast: None,
        self_view: StructuredSelfView {
            version: SelfVersion(1),
            mood: Stimmung::Gelassenheit,
            concerns: vec![],
            projection: None,
            protentions: vec![],
        },
        receipt: ContextProjectionReceipt {
            space: AgoraSpaceId("test-session".into()),
            broadcast_epoch: None,
            workspace_version: None,
            dasein_version: SelfVersion(1),
            content_ids: vec![],
        },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_config(port_mode: Option<StubMode>) -> SelfFieldConfig {
    SelfFieldConfig {
        clock: Some(Arc::new(aletheon_kernel::chronos::TestClock::default())),
        conscious_context: port_mode.map(|m| {
            Arc::new(StubConsciousContextPort::new(m)) as Arc<dyn LatestConsciousContextPort>
        }),
        ..SelfFieldConfig::default()
    }
}

fn make_intent(action: &str, description: &str) -> Intent {
    Intent {
        action: action.to_string(),
        parameters: serde_json::json!({}),
        source: IntentSource::User,
        description: description.to_string(),
    }
}

fn test_ctx() -> Context {
    Context::new("test-session", PathBuf::from("/tmp"))
}

async fn review_priority(config: SelfFieldConfig) -> f64 {
    let sf = dasein::core::SelfField::new(config);
    let intent = make_intent("write_config", "write important config");
    let ctx = test_ctx();
    let _verdict = sf.review(&intent, &ctx).await.unwrap();
    // Priority is 0.0 when no focus topic exists (e.g., baseline care is zero).
    sf.attention()
        .current_focus()
        .map(|f| f.priority)
        .unwrap_or(0.0)
}

async fn review_priority_without_reader() -> f64 {
    review_priority(make_config(None)).await
}

// ---------------------------------------------------------------------------
// R2 acceptance tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn higher_field_urgency_raises_attention_for_same_intent() {
    let low = review_priority(make_config(Some(StubMode::Broadcast(
        CareActionKind::Direct,
        0.10,
    ))))
    .await;
    let high = review_priority(make_config(Some(StubMode::Broadcast(
        CareActionKind::Negate,
        0.90,
    ))))
    .await;
    assert!(
        high > low,
        "higher field urgency must raise attention priority; high={high} low={low}"
    );
}

#[tokio::test]
async fn empty_and_error_equal_legacy_baseline() {
    let baseline = review_priority_without_reader().await;
    assert_eq!(
        review_priority(make_config(Some(StubMode::Empty))).await,
        baseline,
        "empty projection must equal legacy baseline"
    );
    assert_eq!(
        review_priority(make_config(Some(StubMode::Error))).await,
        baseline,
        "reader error must equal legacy baseline"
    );
}

#[tokio::test]
async fn no_reader_equals_legacy_baseline() {
    let baseline = review_priority_without_reader().await;
    // None reader is the same as review_priority_without_reader
    let also_baseline = review_priority(make_config(None)).await;
    assert_eq!(
        also_baseline, baseline,
        "None reader must produce baseline priority"
    );
}

#[tokio::test]
async fn verdict_still_allow_for_normal_action_with_field() {
    // Field modulation should not change verdict to Deny for a low-risk action.
    let config = make_config(Some(StubMode::Broadcast(CareActionKind::Direct, 0.10)));
    let sf = dasein::core::SelfField::new(config);
    let intent = make_intent("ls", "list files");
    let ctx = test_ctx();
    let verdict = sf.review(&intent, &ctx).await.unwrap();
    assert!(matches!(verdict, Verdict::Allow));
}
