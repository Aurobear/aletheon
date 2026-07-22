use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use executive::application::context_assembler::{
    ContextAssembler, ContextAssemblyError, ContextFragments, ContextSource,
};
use fabric::dasein::{SelfVersion, Stimmung};
use fabric::{
    AgoraSpaceId, BroadcastEpoch, ConsciousContextProjection, ContextProjectionReceipt, MonoTime,
    OperationId, ProcessId, SelectionExplanation, SelectionResult, StructuredSelfView, TurnRequest,
    WorkspaceBroadcast,
};
use mnemosyne::{
    DefaultMemoryWorkspaceProjector, MemoryAuthority, MemoryCandidateContext, MemoryMetadata,
    MemoryProjectionLimits, MemoryProvenance, MemoryScope, MemorySensitivity,
    MemoryWorkspaceProjector, RecallItem, RecallSet, TemporalState,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
struct FixedSource(ConsciousContextProjection);

#[async_trait]
impl ContextSource for FixedSource {
    async fn load(&self, _: &TurnRequest) -> Result<ContextFragments, ContextAssemblyError> {
        Ok(ContextFragments {
            system_prefix: "system authority".into(),
            skills: String::new(),
            conscious: Some(self.0.clone()),
        })
    }
}

fn request() -> TurnRequest {
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        context: turn_request_support::context("session-1", PathBuf::from("/workspace")),
        input: "current request".into(),
        model_policy: None,
        deadline: None,
    }
}

fn self_view() -> StructuredSelfView {
    StructuredSelfView {
        version: SelfVersion(4),
        mood: Stimmung::Gelassenheit,
        concerns: vec![],
        care_concerns: vec![],
        projection: None,
        protentions: vec![],
    }
}

fn projection(latest_broadcast: Option<WorkspaceBroadcast>) -> ConsciousContextProjection {
    let receipt = match &latest_broadcast {
        Some(broadcast) => ContextProjectionReceipt {
            space: broadcast.space.clone(),
            broadcast_epoch: Some(broadcast.epoch),
            workspace_version: Some(broadcast.workspace_version),
            dasein_version: broadcast.dasein_version,
            content_ids: broadcast.winner_ids.clone(),
        },
        None => ContextProjectionReceipt {
            space: AgoraSpaceId("session-1".into()),
            broadcast_epoch: None,
            workspace_version: None,
            dasein_version: SelfVersion(4),
            content_ids: vec![],
        },
    };
    ConsciousContextProjection {
        latest_broadcast,
        self_view: self_view(),
        receipt,
    }
}

fn recalled() -> RecallSet {
    let observed = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    RecallSet {
        items: vec![RecallItem {
            content: "selected memory marker".into(),
            metadata: MemoryMetadata {
                record_id: "memory-1".into(),
                provenance: MemoryProvenance {
                    source: "mnemosyne.local".into(),
                    source_id: "fact-1".into(),
                    principal: Some("owner".into()),
                    source_commit: None,
                },
                source_time: Some(observed),
                observed_time: observed,
                valid_from: Some(observed),
                valid_until: None,
                supersedes: None,
                superseded_by: None,
                confidence: 0.9,
                sensitivity: MemorySensitivity::Internal,
            },
            temporal_state: TemporalState::Current,
            authority: MemoryAuthority::VerifiedLocalSemantic,
            scope: MemoryScope::Session("session-1".into()),
            score: 0.0,
            evidence: None,
        }],
        degraded_sources: vec![],
    }
}

#[tokio::test]
async fn only_selected_labelled_memory_enters_model_context_with_durable_lineage() {
    let memory = DefaultMemoryWorkspaceProjector
        .project(&recalled(), MemoryProjectionLimits::default())
        .unwrap();
    let source = ProcessId::new();
    let candidate = memory
        .to_candidates(&MemoryCandidateContext {
            space: AgoraSpaceId("session-1".into()),
            source,
            source_epoch: BroadcastEpoch(9),
            dependencies: vec![],
            created_at: MonoTime(5),
            ttl_ms: 30_000,
        })
        .unwrap()
        .remove(0);

    let unselected = ContextAssembler::new(Arc::new(FixedSource(projection(None))))
        .assemble(&request(), &[])
        .await
        .unwrap();
    assert!(!unselected
        .effective_user_message
        .contains("selected memory marker"));

    let selection = SelectionResult {
        selected: vec![candidate.clone()],
        explanation: SelectionExplanation {
            policy_version: 1,
            evaluated: vec![],
            selected_ids: vec![candidate.id],
            rejected_below_ignition: vec![],
        },
    };
    let broadcast =
        WorkspaceBroadcast::from_selection(BroadcastEpoch(10), selection, SelfVersion(4), 12)
            .unwrap();
    assert!(broadcast.selected[0]
        .provenance
        .source_refs
        .contains(&"memory-record:memory-1".into()));
    assert!(broadcast.selected[0]
        .provenance
        .source_refs
        .contains(&"broadcast:session-1:9".into()));

    let selected = ContextAssembler::new(Arc::new(FixedSource(projection(Some(broadcast)))))
        .assemble(&request(), &[])
        .await
        .unwrap();
    assert!(selected
        .effective_user_message
        .contains("selected memory marker"));
    assert!(selected.effective_user_message.contains("untrusted"));
    assert!(!selected.messages[0]
        .content
        .iter()
        .any(|block| format!("{block:?}").contains("selected memory marker")));
}
mod turn_request_support;
