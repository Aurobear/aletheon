mod agent_control_support;

use agent_control_support::{fixture, spawn_request, TestLauncher};
use executive::service::agent_control::{
    AgentContextItemKind, AgentContextProjection, AgentContextProjectionBuilder, AgentRunRepository,
};
use fabric::{
    AgentBroadcastRef, AgentContextFork, AgentControlErrorKind, AgentId, AgoraSpaceId,
    AgoraVersion, BroadcastEpoch, ContentId, ContextBinding, MonoTime, ProcessId, SalienceVector,
    VisibilityScope, WallTime, WorkspaceCandidate, WorkspaceContent, WorkspaceObservation,
    WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
};

#[test]
fn none_last_turns_and_selected_projection_are_labelled_and_deterministic() {
    assert_eq!(
        AgentContextProjection::from_fork(&AgentContextFork::None)
            .unwrap()
            .items
            .len(),
        0
    );
    let recent =
        AgentContextProjection::from_fork(&AgentContextFork::LastTurns { count: 3 }).unwrap();
    assert_eq!(recent.items[0].kind, AgentContextItemKind::RecentTurns);
    assert_eq!(recent.items[0].label, "requested_recent_turn_count");

    let first = AgentContextProjectionBuilder::new()
        .constraint("z constraint")
        .unwrap()
        .constraint("a constraint")
        .unwrap()
        .item(AgentContextItemKind::Evidence, "b", "second")
        .unwrap()
        .item(AgentContextItemKind::Memory, "a", "first")
        .unwrap()
        .build()
        .unwrap();
    let second = AgentContextProjectionBuilder::new()
        .item(AgentContextItemKind::Memory, "a", "first")
        .unwrap()
        .item(AgentContextItemKind::Evidence, "b", "second")
        .unwrap()
        .constraint("a constraint")
        .unwrap()
        .constraint("z constraint")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.constraints, vec!["a constraint", "z constraint"]);
}

#[test]
fn projection_rejects_hidden_reasoning_and_raw_tool_output() {
    for kind in [
        AgentContextItemKind::HiddenReasoning,
        AgentContextItemKind::RawToolOutput,
    ] {
        let error = AgentContextProjectionBuilder::new()
            .item(kind, "private", "secret")
            .unwrap_err();
        assert_eq!(error.kind, AgentControlErrorKind::InvalidRequest);
    }
    let error = AgentContextProjection::from_fork(&AgentContextFork::SelectedProjection {
        items: vec!["chain_of_thought: private tokens".into()],
    })
    .unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::InvalidRequest);
}

#[test]
fn large_tool_content_requires_artifact_reference() {
    let error = AgentContextProjectionBuilder::new()
        .item(
            AgentContextItemKind::ArtifactReference,
            "tool output",
            "raw bytes",
        )
        .unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::InvalidRequest);
    let projection = AgentContextProjectionBuilder::new()
        .item(
            AgentContextItemKind::ArtifactReference,
            "tool output",
            format!("sha256:{}", "a".repeat(64)),
        )
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(projection.items.len(), 1);
}

#[test]
fn limits_truncate_at_utf8_boundaries_and_count_omissions() {
    let multi_byte = "界".repeat(4_000);
    let mut builder = AgentContextProjectionBuilder::new();
    for index in 0..80 {
        builder = builder
            .item(
                AgentContextItemKind::Selected,
                format!("item-{index:03}"),
                multi_byte.clone(),
            )
            .unwrap();
    }
    let projection = builder.build().unwrap();
    assert!(projection.total_bytes() <= 64 * 1024);
    assert!(projection.omitted_count > 0);
    assert!(projection
        .items
        .iter()
        .all(|item| item.content.is_char_boundary(item.content.len())));
}

#[test]
fn broadcast_references_are_deduplicated_and_ordered() {
    let first = ContentId::new();
    let second = ContentId::new();
    let projection = AgentContextProjectionBuilder::new()
        .broadcast_ref(second)
        .broadcast_ref(first)
        .broadcast_ref(second)
        .build()
        .unwrap();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(projection.broadcast_refs, expected);
}

fn candidate(
    space: AgoraSpaceId,
    id: ContentId,
    source: ProcessId,
    visibility: VisibilityScope,
) -> WorkspaceCandidate {
    WorkspaceCandidate {
        schema_version: WORKSPACE_SCHEMA_V1,
        id,
        space,
        source,
        turn: None,
        content: WorkspaceContent::Observation(WorkspaceObservation {
            what: "bounded child input".into(),
            source: "durable broadcast".into(),
            data: serde_json::json!({"trusted_as": "data"}),
            attribution: fabric::WorkspaceAttribution::RootAgent { process: source },
        }),
        confidence: 1.0,
        salience: SalienceVector {
            urgency: 0.0,
            goal_relevance: 1.0,
            self_relevance: 0.0,
            novelty: 0.5,
            confidence: 1.0,
            prediction_error: 0.0,
            affect_intensity: 0.0,
            social_relevance: 0.0,
        },
        provenance: WorkspaceProvenance {
            producer: source,
            operation: None,
            source_refs: vec!["broadcast:test".into()],
            observed_at: WallTime(1),
        },
        visibility,
        dependencies: vec![],
        created_at: MonoTime(1),
        expires_at: None,
    }
}

#[tokio::test]
async fn workspace_identity_is_unique_bound_and_persisted_for_siblings() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(4, launcher.clone());
    let root = AgentId::new();
    let root_handle = fixture.port.spawn(spawn_request(root, None)).await.unwrap();
    launcher.wait_started().await;

    let source_space = AgoraSpaceId("root:broadcast".into());
    let selected = ContentId::new();
    let mut first_request = spawn_request(root, Some((root, root_handle.process_id)));
    first_request.broadcast_refs = vec![AgentBroadcastRef {
        space: source_space,
        epoch: BroadcastEpoch(7),
        content_id: selected,
    }];
    let first = fixture.port.spawn(first_request).await.unwrap();
    let second = fixture
        .port
        .spawn(spawn_request(root, Some((root, root_handle.process_id))))
        .await
        .unwrap();

    let first_run = fixture
        .repository
        .get(first.agent_id)
        .await
        .unwrap()
        .unwrap();
    let second_run = fixture
        .repository
        .get(second.agent_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first_run.workspace_id, second_run.workspace_id);
    assert_eq!(first_run.root_process_id, root_handle.process_id);
    assert_eq!(first_run.broadcast_refs, first_run.request.broadcast_refs);

    for run in [&first_run, &second_run] {
        let process = fixture
            .kernel
            .inspect_process(run.snapshot.handle.process_id)
            .await
            .unwrap();
        let context_space = fixture.kernel.inspect_space(process.space).unwrap();
        let agora = context_space
            .bindings
            .iter()
            .filter_map(|binding| match binding {
                ContextBinding::Agora(space, version) => Some((space, version)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(agora, vec![(&run.workspace_id, &AgoraVersion(0))]);
        assert_eq!(
            context_space.overlay.entries["agent.workspace_receipt"]["workspace_id"],
            run.workspace_id.0
        );
    }

    fixture.port.cancel(root, first.agent_id).await.unwrap();
    fixture.port.cancel(root, second.agent_id).await.unwrap();
    fixture.port.cancel(root, root).await.unwrap();
}

#[tokio::test]
async fn workspace_subscription_requires_receipt_space_epoch_content_and_visibility() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(4, launcher.clone());
    let root = AgentId::new();
    let root_handle = fixture.port.spawn(spawn_request(root, None)).await.unwrap();
    launcher.wait_started().await;
    let source_space = AgoraSpaceId("root:broadcast".into());
    let selected = ContentId::new();
    let receipt = AgentBroadcastRef {
        space: source_space.clone(),
        epoch: BroadcastEpoch(9),
        content_id: selected,
    };
    let mut request = spawn_request(root, Some((root, root_handle.process_id)));
    request.broadcast_refs = vec![receipt.clone()];
    let child = fixture.port.spawn(request).await.unwrap();
    let mut sibling_request = spawn_request(root, Some((root, root_handle.process_id)));
    sibling_request.broadcast_refs = vec![receipt];
    let sibling = fixture.port.spawn(sibling_request).await.unwrap();
    let run = fixture
        .repository
        .get(child.agent_id)
        .await
        .unwrap()
        .unwrap();
    let sibling_run = fixture
        .repository
        .get(sibling.agent_id)
        .await
        .unwrap()
        .unwrap();

    let session = candidate(
        source_space.clone(),
        selected,
        child.process_id,
        VisibilityScope::Session,
    );
    assert!(run.can_observe_broadcast(BroadcastEpoch(9), &session));
    fixture
        .service
        .authorize_broadcast(root, child.agent_id, BroadcastEpoch(9), &session)
        .await
        .unwrap();
    assert!(!run.can_observe_broadcast(BroadcastEpoch(8), &session));
    assert!(!run.can_observe_broadcast(
        BroadcastEpoch(9),
        &candidate(
            AgoraSpaceId("other".into()),
            selected,
            child.process_id,
            VisibilityScope::Session,
        ),
    ));
    assert!(!run.can_observe_broadcast(
        BroadcastEpoch(9),
        &candidate(
            source_space.clone(),
            ContentId::new(),
            child.process_id,
            VisibilityScope::Session,
        ),
    ));

    let private = candidate(
        source_space.clone(),
        selected,
        child.process_id,
        VisibilityScope::PrivateProcess {
            process: child.process_id,
        },
    );
    assert!(run.can_observe_broadcast(BroadcastEpoch(9), &private));
    assert!(!sibling_run.can_observe_broadcast(BroadcastEpoch(9), &private));
    assert_eq!(
        fixture
            .service
            .authorize_broadcast(root, sibling.agent_id, BroadcastEpoch(9), &private)
            .await
            .unwrap_err()
            .kind,
        AgentControlErrorKind::Forbidden
    );

    let tree = candidate(
        source_space,
        selected,
        child.process_id,
        VisibilityScope::AgentTree {
            root: root_handle.process_id,
        },
    );
    assert!(run.can_observe_broadcast(BroadcastEpoch(9), &tree));

    fixture.port.cancel(root, child.agent_id).await.unwrap();
    fixture.port.cancel(root, sibling.agent_id).await.unwrap();
    fixture.port.cancel(root, root).await.unwrap();
}
