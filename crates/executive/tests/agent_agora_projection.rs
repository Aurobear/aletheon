use std::sync::Arc;

use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use async_trait::async_trait;
use executive::r#impl::events::{EventReadFilter, SqliteEventSpine};
use executive::service::agent_control::{
    AgentCandidateProjector, AgentCandidateSubmissionPort, AgentContextProjection, AgentEventSink,
    AgentRuntimeEvent, AgentRuntimeInbox, AgentRuntimeInput, NoopAgentEventSink,
    SpineAgentEventSink,
};
use executive::service::conscious_workspace::ConsciousWorkspaceRegistry;
use executive::service::dasein_workspace_adapter::DaseinWorkspaceAdapter;
use fabric::{
    AgentArtifact, AgentBroadcastRef, AgentBudget, AgentContextFork, AgentHandle, AgentId,
    AgentProfileId, AgentResult, AgentRunStatus, AgentSpawnRequest, AgoraSpaceId, AttemptEvidence,
    AttemptUsage, BroadcastEpoch, ContentId, LatestConsciousContextPort, NamespaceId, OperationId,
    ProcessId, RuntimeId, SpawnSpec, VisibilityScope, WorkspaceAttribution, WorkspaceContent,
};
use mnemosyne::{ExperienceEvent, ForgetPolicy, MemoryScope, RecallRequest, RecallSet};
use tempfile::tempdir;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn input() -> AgentRuntimeInput {
    let root_agent = AgentId::new();
    let child_agent = AgentId::new();
    let child = ProcessId::new();
    let root = ProcessId::new();
    let broadcast = ContentId::new();
    let request = AgentSpawnRequest {
        root_agent_id: root_agent,
        parent_agent_id: Some(root_agent),
        parent_process_id: Some(root),
        profile_id: AgentProfileId("worker".into()),
        runtime_id: RuntimeId("native-cognit".into()),
        trusted_workspace: None,
        task: "inspect the evidence".into(),
        context: AgentContextFork::SelectedProjection { items: vec![] },
        broadcast_refs: vec![AgentBroadcastRef {
            space: AgoraSpaceId("root-workspace".into()),
            epoch: BroadcastEpoch(7),
            content_id: broadcast,
        }],
        allowed_tools: vec![],
        background_decls: vec![],
        budget: AgentBudget {
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_tool_calls: 2,
            max_elapsed_ms: 2_000,
            max_cost_usd: None,
            max_depth: 2,
        },
    };
    AgentRuntimeInput {
        workspace: None,
        context: AgentContextProjection::from_fork(&request.context).unwrap(),
        memory_context: mnemosyne::AgentMemoryContext::verified(
            child,
            child_agent,
            fabric::AgentTaskId("test-task".into()),
            "sha256:test-projection",
        )
        .unwrap(),
        handle: AgentHandle {
            agent_id: child_agent,
            root_agent_id: root_agent,
            parent_agent_id: Some(root_agent),
            process_id: child,
            operation_id: OperationId::new(),
            runtime_id: request.runtime_id.clone(),
            profile_id: request.profile_id.clone(),
        },
        request,
        workspace_id: AgoraSpaceId(format!("agent:{}", child_agent.0)),
        root_workspace_id: AgoraSpaceId("root-workspace".into()),
        root_process_id: root,
        inbox: AgentRuntimeInbox::empty(),
        cancellation: CancellationToken::new(),
        background_cancellations: std::collections::HashMap::new(),
        background_registrations: std::collections::HashMap::new(),
    }
}

#[test]
fn progress_is_private_bounded_and_never_targets_root() {
    let input = input();
    let projector = AgentCandidateProjector::new(input.clone(), Arc::new(TestClock::default()));
    let projected = projector
        .project(&AgentRuntimeEvent::Progress {
            agent_id: input.handle.agent_id,
            process_id: input.handle.process_id,
            operation_id: input.handle.operation_id,
            summary: "x".repeat(10_000),
        })
        .unwrap();
    assert_eq!(projected.len(), 1);
    let candidate = &projected[0].candidate;
    assert_eq!(candidate.space, input.workspace_id);
    assert_eq!(
        candidate.visibility,
        VisibilityScope::PrivateProcess {
            process: input.handle.process_id
        }
    );
    assert_eq!(
        candidate.dependencies,
        vec![input.request.broadcast_refs[0].content_id]
    );
    assert!(matches!(
        &candidate.content,
        WorkspaceContent::Observation(value)
            if value.what.len() <= 4_100
                && value.attribution == WorkspaceAttribution::ChildAgent { process: input.handle.process_id }
    ));
}

#[tokio::test]
async fn agent_lifecycle_and_tool_events_append_to_root_tree() {
    let input = input();
    let spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let sink = SpineAgentEventSink::new(
        Arc::new(NoopAgentEventSink),
        spine.clone(),
        input.clone(),
        Arc::new(executive::service::event_projection::NoopEventProjectionSink),
    );
    sink.emit(AgentRuntimeEvent::Started {
        agent_id: input.handle.agent_id,
        process_id: input.handle.process_id,
        operation_id: input.handle.operation_id,
    })
    .await;
    sink.emit(AgentRuntimeEvent::Tool {
        agent_id: input.handle.agent_id,
        process_id: input.handle.process_id,
        operation_id: input.handle.operation_id,
        name: "file_read".into(),
        is_error: false,
    })
    .await;
    sink.emit(AgentRuntimeEvent::Terminal {
        agent_id: input.handle.agent_id,
        process_id: input.handle.process_id,
        operation_id: input.handle.operation_id,
        status: AgentRunStatus::Succeeded,
        result: None,
    })
    .await;

    let events = spine
        .read_tree(
            fabric::EventTreeId::for_root_session(&input.handle.root_agent_id.0.to_string()),
            EventReadFilter {
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].schema.0, fabric::SchemaId::EVENT_AGENT_STARTED_V1);
    assert_eq!(
        events[1].schema.0,
        fabric::SchemaId::EVENT_TOOL_OBSERVATION_V1
    );
    assert_eq!(events[2].schema.0, fabric::SchemaId::EVENT_AGENT_STOPPED_V1);
}

#[test]
fn only_explicit_evidence_and_terminal_result_enter_root_agent_tree() {
    let input = input();
    let projector = AgentCandidateProjector::new(input.clone(), Arc::new(TestClock::default()));
    let result = AgentResult {
        output: "done".into(),
        usage: AttemptUsage::default(),
        evidence: vec![
            AttemptEvidence {
                kind: "hypothesis".into(),
                summary: "private".into(),
                content: "maybe".into(),
            },
            AttemptEvidence {
                kind: "exportable:criticism".into(),
                summary: "share".into(),
                content: "counterexample".into(),
            },
        ],
        artifacts: vec![],
    };
    let projected = projector
        .project(&AgentRuntimeEvent::Terminal {
            agent_id: input.handle.agent_id,
            process_id: input.handle.process_id,
            operation_id: input.handle.operation_id,
            status: AgentRunStatus::Succeeded,
            result: Some(result),
        })
        .unwrap();

    assert_eq!(projected.len(), 3);
    assert_eq!(projected[0].candidate.space, input.workspace_id);
    for exported in &projected[1..] {
        assert_eq!(exported.candidate.space, input.root_workspace_id);
        assert_eq!(
            exported.candidate.visibility,
            VisibilityScope::AgentTree {
                root: input.root_process_id
            }
        );
        assert!(exported
            .candidate
            .provenance
            .source_refs
            .iter()
            .any(|reference| reference == "broadcast:root-workspace:7"));
    }
    let WorkspaceContent::AgentResult(exported_result) = &projected[2].candidate.content else {
        panic!("terminal candidate must be typed AgentResult")
    };
    assert_eq!(exported_result.evidence.len(), 1);
    assert_eq!(exported_result.evidence[0].kind, "criticism");
}

#[test]
fn large_terminal_output_crosses_only_as_validated_artifact_reference() {
    let input = input();
    let projector = AgentCandidateProjector::new(input.clone(), Arc::new(TestClock::default()));
    let artifact = AgentArtifact {
        kind: "log".into(),
        reference: "artifact://sha256/abc".into(),
        content_hash: "abc".into(),
    };
    let projected = projector
        .project(&AgentRuntimeEvent::Terminal {
            agent_id: input.handle.agent_id,
            process_id: input.handle.process_id,
            operation_id: input.handle.operation_id,
            status: AgentRunStatus::Succeeded,
            result: Some(AgentResult {
                output: "界".repeat(5_000),
                usage: AttemptUsage::default(),
                evidence: vec![],
                artifacts: vec![artifact.clone()],
            }),
        })
        .unwrap();
    let WorkspaceContent::AgentResult(result) = &projected[0].candidate.content else {
        panic!("expected AgentResult")
    };
    assert!(result.output.contains("artifact references"));
    assert_eq!(result.artifacts, vec![artifact]);
}

#[test]
fn content_fingerprint_deduplicates_repeated_events_but_not_distinct_progress() {
    let input = input();
    let projector = AgentCandidateProjector::new(input.clone(), Arc::new(TestClock::default()));
    let event = |summary: &str| AgentRuntimeEvent::Progress {
        agent_id: input.handle.agent_id,
        process_id: input.handle.process_id,
        operation_id: input.handle.operation_id,
        summary: summary.into(),
    };
    let first = projector
        .project(&event("one"))
        .unwrap()
        .remove(0)
        .candidate;
    let replay = projector
        .project(&event("one"))
        .unwrap()
        .remove(0)
        .candidate;
    let second = projector
        .project(&event("two"))
        .unwrap()
        .remove(0)
        .candidate;
    assert_eq!(first.id, replay.id);
    assert_ne!(first.id, second.id);
}

#[test]
fn projection_source_contains_child_process_operation_and_agent_identity() {
    let input = input();
    let projector = AgentCandidateProjector::new(input.clone(), Arc::new(TestClock::default()));
    let candidate = projector
        .project(&AgentRuntimeEvent::Started {
            agent_id: input.handle.agent_id,
            process_id: input.handle.process_id,
            operation_id: input.handle.operation_id,
        })
        .unwrap()
        .remove(0)
        .candidate;
    assert_eq!(candidate.source, input.handle.process_id);
    assert_eq!(
        candidate.provenance.operation,
        Some(input.handle.operation_id)
    );
    assert!(candidate
        .provenance
        .source_refs
        .iter()
        .any(|reference| reference == &format!("agent:{}", input.handle.agent_id.0)));
    assert_ne!(candidate.id.0, Uuid::nil());
}

struct EmptyMemory;

#[async_trait]
impl mnemosyne::MemoryService for EmptyMemory {
    async fn record(&self, _event: ExperienceEvent) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recall(&self, _request: RecallRequest) -> anyhow::Result<RecallSet> {
        Ok(RecallSet::default())
    }
    async fn consolidate(&self, _scope: MemoryScope) -> anyhow::Result<()> {
        Ok(())
    }
    async fn forget(&self, _policy: ForgetPolicy) -> anyhow::Result<mnemosyne::ForgetReceipt> {
        Ok(mnemosyne::ForgetReceipt::default())
    }
}

#[tokio::test]
async fn root_broadcast_child_candidate_and_later_c01_selection_are_replayable() {
    let directory = tempdir().unwrap();
    let clock = Arc::new(TestClock::new(10_000, 20));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let root = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId::new(),
            parent: None,
            profile: AgentProfileId("root".into()),
            namespace: NamespaceId("projection-root".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await
        .unwrap()
        .id;
    let child = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId::new(),
            parent: Some(root),
            profile: AgentProfileId("child".into()),
            namespace: NamespaceId("projection-child".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await
        .unwrap()
        .id;
    let dasein = Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0);
    let initial_version = dasein.self_version().await;
    let registry = ConsciousWorkspaceRegistry::production(
        directory.path().join("workspace.db"),
        Arc::new(DaseinWorkspaceAdapter::new(dasein.clone(), clock.clone())),
        kernel,
        clock.clone(),
        Arc::new(EmptyMemory),
        Arc::new(Mutex::new(corpus::SkillLoader::new(
            directory.path().join("skills"),
        ))),
    )
    .unwrap();
    let mut runtime_input = input();
    runtime_input.handle.process_id = child;
    runtime_input.root_process_id = root;
    let dependency = runtime_input.request.broadcast_refs[0].content_id;
    let submission = AgentCandidateProjector::new(runtime_input.clone(), clock)
        .project(&AgentRuntimeEvent::Terminal {
            agent_id: runtime_input.handle.agent_id,
            process_id: child,
            operation_id: runtime_input.handle.operation_id,
            status: AgentRunStatus::Succeeded,
            result: Some(AgentResult {
                output: "exported result".into(),
                usage: AttemptUsage::default(),
                evidence: vec![],
                artifacts: vec![],
            }),
        })
        .unwrap()
        .remove(0);
    let candidate_id = submission.candidate.id;

    registry
        .submit_agent_candidate(submission, child, root)
        .await
        .unwrap();
    assert!(registry
        .store()
        .replay(&runtime_input.root_workspace_id)
        .unwrap()
        .is_empty());
    assert!(registry
        .latest_context(&runtime_input.root_workspace_id)
        .await
        .is_err());
    assert_eq!(dasein.self_version().await, initial_version);

    let selected = registry
        .run_pending_cycle(runtime_input.root_workspace_id.clone(), root, root, 0)
        .await
        .unwrap();
    let broadcast = selected.broadcast.unwrap();
    assert!(broadcast.winner_ids.contains(&candidate_id));
    let replay = registry
        .store()
        .replay(&runtime_input.root_workspace_id)
        .unwrap();
    let replayed = replay[0]
        .broadcast
        .selected
        .iter()
        .find(|candidate| candidate.id == candidate_id)
        .unwrap();
    assert_eq!(replayed.dependencies, vec![dependency]);
    assert!(dasein.self_version().await.0 > initial_version.0);
}
