mod agent_control_support;

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use executive::application::agent_control::CognitiveTaskAdmissionPort;
use executive::application::harness_factory::LinearCognitiveSessionFactory;
use executive::application::{CapabilityExecutionContext, CapabilityService};
use executive::testing::coding_runtime::{
    AgentProfileRegistry, NativeCognitRuntime, NativeCognitRuntimeResources, ResolvedAgentProfile,
};
use fabric::cognitive_workflow::{
    AgentTaskPacket, AgoraProjectionReceipt, CognitiveArtifact, CognitiveChangeSetReceipt,
    CognitivePlanArtifact, CognitivePlanArtifactStep, CognitiveRole, CognitiveRoleOutput,
    CognitiveRoleProfile, CognitiveStage, CognitiveTaskNode, CognitiveTaskNodeId,
    CognitiveTaskRuntimeBinding, CognitiveTaskStatus, CognitiveValidationRecord,
    InvestigationReport, ReviewFindingSet,
};
use fabric::{
    AgentApprovalPolicy, AgentBudget, AgentContextFork, AgentControlError, AgentId, AgentProfile,
    AgentProfileId, AgentRunStatus, AgentSnapshot, AgentSpawnRequest, AgentWaitRequest,
    CapabilityCall, CapabilityResult, CapabilityScope, ContentBlock, InferenceUsage, LlmProvider,
    LlmResponse, LlmStream, ParentRestriction, ProcessId, RiskTier, StopReason, Tool,
    ToolApprovalAuthority, ToolContext, ToolDefinition, UsageReport, WorkspacePolicy,
};
use kernel::chronos::TestClock;
use tokio_util::sync::CancellationToken;

const READ_TOOLS: &[&str] = &[
    "repo_inspect",
    "file_read",
    "artifact_read",
    "grep",
    "glob",
    "file_search",
    "code_graph",
];

const WRITE_TOOLS: &[&str] = &[
    "file_write",
    "apply_patch",
    "exec_command",
    "write_stdin",
    "validation_run",
    "change_accept",
    "change_rollback",
];

struct ScriptedLlm {
    responses: Mutex<VecDeque<anyhow::Result<LlmResponse>>>,
    seen: Mutex<usize>,
}

impl ScriptedLlm {
    fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            seen: Mutex::new(0),
        })
    }
}

#[async_trait]
impl LlmProvider for ScriptedLlm {
    async fn complete(
        &self,
        _messages: &[fabric::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        *self.seen.lock().unwrap() += 1;
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted role response")
    }

    async fn complete_stream(
        &self,
        _messages: &[fabric::Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unreachable!("linear harness uses complete")
    }

    fn name(&self) -> &str {
        "scripted/role-boundary"
    }

    fn max_context_length(&self) -> usize {
        128_000
    }
}

#[derive(Default)]
struct BoundaryCapability {
    calls: Mutex<Vec<(String, bool)>>,
}

#[async_trait]
impl CapabilityService for BoundaryCapability {
    async fn invoke(
        &self,
        context: Option<CapabilityExecutionContext>,
        call: CapabilityCall,
        _cancel: CancellationToken,
    ) -> CapabilityResult {
        if call.name == "validation_run" {
            self.calls.lock().unwrap().push((call.name.clone(), false));
            return CapabilityResult {
                call_id: call.call_id,
                output: "governed validation passed".into(),
                is_error: false,
                usage: UsageReport::default(),
                audit_id: None,
                patch_delta: None,
                served_from_cache: false,
            };
        }
        if call.name != "file_write" {
            self.calls.lock().unwrap().push((call.name.clone(), true));
            return CapabilityResult {
                call_id: call.call_id,
                output: format!("unsupported boundary tool: {}", call.name),
                is_error: true,
                usage: UsageReport::default(),
                audit_id: None,
                patch_delta: None,
                served_from_cache: false,
            };
        }

        let context = context.expect("native role capability context");
        let allowed_paths = context
            .workspace
            .writable_roots()
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let tool_context = ToolContext {
            agent: context.agent,
            approval_authority: Some(ToolApprovalAuthority {
                principal_id: context.principal,
                connection_id: context.connection_id,
                thread_id: context.thread_id,
                turn_id: context.turn_id,
                call_id: call.call_id.clone(),
                workspace: context.workspace,
                granted_scope: CapabilityScope {
                    allowed_paths,
                    ..Default::default()
                },
                permission_mode: context.permission_mode,
            }),
            working_dir: context.working_dir,
            session_id: context.session_id,
            clock: Arc::new(TestClock::default()),
            turn_event_sender: None,
        };
        let result = corpus::tools::tools::file_write::FileWriteTool
            .execute(call.input, &tool_context)
            .await;
        self.calls
            .lock()
            .unwrap()
            .push((call.name.clone(), result.is_error));
        CapabilityResult {
            call_id: call.call_id,
            output: result.content,
            is_error: result.is_error,
            usage: UsageReport::default(),
            audit_id: None,
            patch_delta: result.metadata.patch_delta,
            served_from_cache: false,
        }
    }
}

#[derive(Default)]
struct AcceptBinding {
    bindings: Mutex<Vec<CognitiveTaskRuntimeBinding>>,
}

#[async_trait]
impl CognitiveTaskAdmissionPort for AcceptBinding {
    async fn bind_before_launch(
        &self,
        binding: CognitiveTaskRuntimeBinding,
        _allocated_process: ProcessId,
    ) -> Result<(), AgentControlError> {
        self.bindings.lock().unwrap().push(binding);
        Ok(())
    }
}

struct BoundaryResult {
    snapshot: AgentSnapshot,
    output: Option<CognitiveRoleOutput>,
    output_validation: anyhow::Result<()>,
    calls: Vec<(String, bool)>,
}

fn role_tools(role: CognitiveRole) -> Vec<String> {
    let mut tools = READ_TOOLS
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect::<Vec<_>>();
    match role {
        CognitiveRole::Planner | CognitiveRole::Explorer | CognitiveRole::Reviewer => {}
        CognitiveRole::Tester => tools.push("validation_run".into()),
        CognitiveRole::Executor | CognitiveRole::Fixer => {
            tools.extend(WRITE_TOOLS.iter().map(|tool| (*tool).to_owned()));
        }
        CognitiveRole::Root => panic!("root is not a worker role"),
    }
    tools
}

fn profile(role: CognitiveRole, tools: Vec<String>) -> AgentProfile {
    let id = format!("{}-agent", format!("{role:?}").to_lowercase());
    AgentProfile {
        id: AgentProfileId(id.clone()),
        system_prompt: format!("Strict {role:?} boundary fixture"),
        model: "scripted/role-boundary".into(),
        allowed_tools: tools,
        max_iterations: 8,
        max_input_tokens: 16_000,
        max_output_tokens: 2_000,
        max_tool_calls: 8,
        max_elapsed_ms: 5_000,
        profile_name: id,
        risk_tier: if role.can_write_workspace() {
            RiskTier::Sandboxed
        } else {
            RiskTier::ReadOnly
        },
        approval_policy: AgentApprovalPolicy::AutoApprove,
        tool_timeout_ms: 2_000,
        inheritable: false,
        parent_restriction: ParentRestriction::SameOrSafer,
    }
}

fn tool_definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: format!("boundary definition for {name}"),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

fn packet_and_binding(
    role: CognitiveRole,
    workspace_scope: Vec<String>,
) -> (AgentTaskPacket, CognitiveTaskRuntimeBinding) {
    let role_profile = CognitiveRoleProfile::canonical(role);
    let task_node_id = CognitiveTaskNodeId(format!("{role:?}-boundary"));
    let projection_id = uuid::Uuid::new_v4();
    let packet = AgentTaskPacket {
        schema_version: 1,
        task: CognitiveTaskNode {
            id: task_node_id.clone(),
            parent_id: None,
            objective: "verify host-enforced role boundary".into(),
            role,
            stage: match role {
                CognitiveRole::Planner => CognitiveStage::Planning,
                CognitiveRole::Explorer => CognitiveStage::Investigation,
                CognitiveRole::Executor | CognitiveRole::Fixer => CognitiveStage::Execution,
                CognitiveRole::Tester => CognitiveStage::Validation,
                CognitiveRole::Reviewer => CognitiveStage::Review,
                CognitiveRole::Root => unreachable!(),
            },
            status: CognitiveTaskStatus::Running,
            owner: Some(ProcessId::new()),
            role_profile: role_profile.reference.clone(),
            budget: role_profile.budget.clone(),
            dependencies: Vec::new(),
            acceptance_criteria: vec!["terminal boundary receipt observed".into()],
            workspace_scope: workspace_scope.clone(),
            required_artifact_kinds: role_profile.required_output_artifacts.clone(),
            artifact_refs: Vec::new(),
            unresolved_finding_ids: if role == CognitiveRole::Fixer {
                vec!["F-1".into()]
            } else {
                Vec::new()
            },
        },
        role_profile: role_profile.clone(),
        project_instructions: vec!["AGENTS.md".into()],
        workspace_roots: workspace_scope.clone(),
        allowed_capabilities: Vec::new(),
        expected_evidence: vec!["terminal tool result".into()],
        acceptance_criteria: vec!["terminal boundary receipt observed".into()],
        selected_artifacts: Vec::new(),
        projection_receipt: AgoraProjectionReceipt {
            projection_id,
            space: fabric::AgoraSpaceId("role-boundary".into()),
            workspace_version: 1,
            task_node_id: task_node_id.clone(),
            role,
            included_artifact_ids: Vec::new(),
            omitted_artifact_ids: Vec::new(),
        },
    };
    let binding = CognitiveTaskRuntimeBinding {
        space: fabric::AgoraSpaceId("role-boundary".into()),
        task_node_id,
        expected_workspace_version: 1,
        expected_owner: ProcessId::new(),
        role,
        role_profile: role_profile.reference,
        budget: role_profile.budget,
        workspace_scope,
    };
    (packet, binding)
}

fn role_artifact(role: CognitiveRole, changed_path: &str) -> CognitiveArtifact {
    match role {
        CognitiveRole::Planner => CognitiveArtifact::Plan(CognitivePlanArtifact {
            steps: vec![CognitivePlanArtifactStep {
                id: "bounded-step".into(),
                description: "preserve the host boundary".into(),
                requirement_refs: vec!["spec:SUB-P0-1".into()],
                dependencies: Vec::new(),
            }],
        }),
        CognitiveRole::Explorer => CognitiveArtifact::Investigation(InvestigationReport {
            summary: "boundary evidence inspected".into(),
            findings: Vec::new(),
        }),
        CognitiveRole::Executor | CognitiveRole::Fixer => {
            CognitiveArtifact::ChangeSet(CognitiveChangeSetReceipt {
                transaction_id: format!("tx-{role:?}"),
                workspace_version: "tree-2".into(),
                changed_paths: vec![changed_path.into()],
                diff_artifact_ref: "artifact://diff/boundary".into(),
            })
        }
        CognitiveRole::Tester => CognitiveArtifact::Validation(CognitiveValidationRecord {
            transaction_id: "tx-test".into(),
            workspace_version: "tree-1".into(),
            validation_receipt_refs: vec!["receipt://validation/boundary".into()],
            passed: true,
        }),
        CognitiveRole::Reviewer => CognitiveArtifact::Review(ReviewFindingSet {
            transaction_id: "tx-review".into(),
            workspace_version: "tree-1".into(),
            findings: Vec::new(),
        }),
        CognitiveRole::Root => unreachable!(),
    }
}

fn response(content: Vec<ContentBlock>, stop_reason: StopReason) -> LlmResponse {
    LlmResponse {
        content,
        stop_reason,
        usage: InferenceUsage::unsupported(Some(10), Some(5)),
    }
}

async fn run_boundary(
    root_path: &Path,
    role: CognitiveRole,
    scope: Vec<String>,
    tool_calls: Vec<(String, serde_json::Value)>,
    final_artifact: CognitiveArtifact,
) -> BoundaryResult {
    let (packet, binding) = packet_and_binding(role, scope);
    let evidence_refs = if role == CognitiveRole::Fixer {
        vec!["finding:F-1".into(), "receipt://role-boundary".into()]
    } else {
        vec!["receipt://role-boundary".into()]
    };
    let output = CognitiveRoleOutput {
        schema_version: 1,
        projection_id: packet.projection_receipt.projection_id,
        workspace_version: packet.projection_receipt.workspace_version,
        source_versions: vec![format!(
            "agora:{}",
            packet.projection_receipt.workspace_version
        )],
        evidence_refs,
        confidence: 1.0,
        artifact: final_artifact,
    };
    let mut script = Vec::new();
    if !tool_calls.is_empty() {
        script.push(response(
            tool_calls
                .into_iter()
                .enumerate()
                .map(|(index, (name, input))| ContentBlock::ToolUse {
                    id: format!("call-{index}"),
                    name,
                    input,
                })
                .collect(),
            StopReason::ToolUse,
        ));
    }
    script.push(response(
        vec![ContentBlock::Text {
            text: serde_json::to_string(&output).unwrap(),
        }],
        StopReason::EndTurn,
    ));

    let llm = ScriptedLlm::new(script);
    let tools = role_tools(role);
    let definitions = tools
        .iter()
        .map(|name| tool_definition(name))
        .collect::<Vec<_>>();
    let profiles = Arc::new(AgentProfileRegistry::default());
    let profile = profile(role, tools.clone());
    profiles
        .register(ResolvedAgentProfile {
            profile: profile.clone(),
            llm,
            authorized_tools: definitions.clone(),
            tools: definitions,
        })
        .unwrap();
    let capability = Arc::new(BoundaryCapability::default());
    let clock = Arc::new(TestClock::default());
    let runtime = Arc::new(NativeCognitRuntime::new(NativeCognitRuntimeResources {
        sessions: Arc::new(LinearCognitiveSessionFactory::new(
            cognit::harness::HarnessConfig::default(),
            clock.clone(),
        )),
        capabilities: capability.clone(),
        profiles,
        clock,
        conscious_actions: None,
        conscious_candidates: None,
    }));
    let admission = Arc::new(AcceptBinding::default());
    let fixture =
        agent_control_support::fixture_with_task_admission(1, runtime, Some(admission.clone()));
    let root_agent_id = AgentId::new();
    let workspace =
        WorkspacePolicy::from_resolved_roots(root_path.to_path_buf(), Vec::new()).unwrap();
    let handle = fixture
        .port
        .spawn(AgentSpawnRequest {
            root_agent_id,
            parent_agent_id: None,
            parent_process_id: None,
            profile_id: profile.id,
            runtime_id: fabric::RuntimeId(agent_control_support::TEST_RUNTIME.into()),
            trusted_workspace: Some(workspace),
            delegator_authority: None,
            cognitive_binding: Some(binding),
            task: serde_json::to_string(&packet).unwrap(),
            context: AgentContextFork::None,
            broadcast_refs: Vec::new(),
            allowed_tools: tools,
            budget: AgentBudget {
                max_input_tokens: 8_000,
                max_output_tokens: 1_000,
                max_tool_calls: 8,
                max_elapsed_ms: 5_000,
                max_cost_usd: None,
                max_depth: 1,
            },
            background_decls: Vec::new(),
        })
        .await
        .unwrap();
    let snapshot = fixture
        .port
        .wait(AgentWaitRequest {
            caller_root_agent_id: root_agent_id,
            agent_id: handle.agent_id,
            timeout_ms: 5_000,
        })
        .await
        .unwrap();
    assert!(snapshot.status.is_terminal());
    assert_eq!(admission.bindings.lock().unwrap().len(), 1);
    let parsed = snapshot
        .result
        .as_ref()
        .and_then(|result| serde_json::from_str::<CognitiveRoleOutput>(&result.output).ok());
    let output_validation = match parsed.as_ref() {
        Some(output) => output.validate_for(&packet),
        None => Err(anyhow::anyhow!(
            "terminal result did not contain CognitiveRoleOutput"
        )),
    };
    let calls = capability.calls.lock().unwrap().clone();
    BoundaryResult {
        snapshot,
        output: parsed,
        output_validation,
        calls,
    }
}

fn write_call(path: &str, content: &str) -> Vec<(String, serde_json::Value)> {
    vec![(
        "file_write".into(),
        serde_json::json!({"path": path, "content": content}),
    )]
}

#[tokio::test]
async fn planner_explorer_and_reviewer_cannot_modify_the_fixture() {
    for role in [
        CognitiveRole::Planner,
        CognitiveRole::Explorer,
        CognitiveRole::Reviewer,
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let allowed = temporary.path().join("src/allowed.rs");
        std::fs::create_dir_all(allowed.parent().unwrap()).unwrap();
        std::fs::write(&allowed, "original\n").unwrap();

        let result = run_boundary(
            temporary.path(),
            role,
            Vec::new(),
            write_call("src/allowed.rs", "mutated\n"),
            role_artifact(role, "src/allowed.rs"),
        )
        .await;

        assert_eq!(result.snapshot.status, AgentRunStatus::Succeeded);
        result.output_validation.unwrap();
        assert!(
            result.calls.is_empty(),
            "{role:?} reached a mutation capability"
        );
        assert_eq!(std::fs::read_to_string(&allowed).unwrap(), "original\n");
        assert!(result
            .snapshot
            .result
            .unwrap()
            .evidence
            .iter()
            .any(|evidence| {
                evidence.kind == "tool_result"
                    && evidence.content.contains("file_write")
                    && evidence.content.contains("not allowed")
            }));
    }
}

#[tokio::test]
async fn tester_runs_validation_but_cannot_modify_sources() {
    let temporary = tempfile::tempdir().unwrap();
    let allowed = temporary.path().join("src/allowed.rs");
    std::fs::create_dir_all(allowed.parent().unwrap()).unwrap();
    std::fs::write(&allowed, "original\n").unwrap();
    let result = run_boundary(
        temporary.path(),
        CognitiveRole::Tester,
        Vec::new(),
        vec![
            (
                "validation_run".into(),
                serde_json::json!({"target": "focused"}),
            ),
            (
                "file_write".into(),
                serde_json::json!({"path": "src/allowed.rs", "content": "mutated\n"}),
            ),
        ],
        role_artifact(CognitiveRole::Tester, "src/allowed.rs"),
    )
    .await;

    assert_eq!(result.snapshot.status, AgentRunStatus::Succeeded);
    result.output_validation.unwrap();
    assert_eq!(result.calls, vec![("validation_run".into(), false)]);
    assert_eq!(std::fs::read_to_string(&allowed).unwrap(), "original\n");
}

#[tokio::test]
async fn executor_is_task_scoped_and_cannot_forge_review() {
    let temporary = tempfile::tempdir().unwrap();
    let allowed = temporary.path().join("src/allowed.rs");
    let sibling = temporary.path().join("src/sibling.rs");
    std::fs::create_dir_all(allowed.parent().unwrap()).unwrap();
    std::fs::write(&allowed, "original\n").unwrap();
    std::fs::write(&sibling, "sibling\n").unwrap();

    let accepted = run_boundary(
        temporary.path(),
        CognitiveRole::Executor,
        vec!["src/allowed.rs".into()],
        write_call("src/allowed.rs", "changed\n"),
        role_artifact(CognitiveRole::Executor, "src/allowed.rs"),
    )
    .await;
    assert_eq!(accepted.snapshot.status, AgentRunStatus::Succeeded);
    accepted.output_validation.unwrap();
    assert_eq!(accepted.calls, vec![("file_write".into(), false)]);
    assert_eq!(std::fs::read_to_string(&allowed).unwrap(), "changed\n");

    let escaped = run_boundary(
        temporary.path(),
        CognitiveRole::Executor,
        vec!["src/allowed.rs".into()],
        write_call("src/sibling.rs", "escaped\n"),
        role_artifact(CognitiveRole::Executor, "src/sibling.rs"),
    )
    .await;
    assert_eq!(escaped.calls, vec![("file_write".into(), true)]);
    assert_eq!(std::fs::read_to_string(&sibling).unwrap(), "sibling\n");

    let forged = run_boundary(
        temporary.path(),
        CognitiveRole::Executor,
        vec!["src/allowed.rs".into()],
        Vec::new(),
        CognitiveArtifact::Review(ReviewFindingSet {
            transaction_id: "forged".into(),
            workspace_version: "tree-forged".into(),
            findings: Vec::new(),
        }),
    )
    .await;
    assert!(forged
        .output_validation
        .unwrap_err()
        .to_string()
        .contains("role output kind is not authorized"));
}

#[tokio::test]
async fn fixer_can_modify_only_the_explicit_finding_path() {
    let temporary = tempfile::tempdir().unwrap();
    let allowed = temporary.path().join("src/allowed.rs");
    let sibling = temporary.path().join("src/sibling.rs");
    std::fs::create_dir_all(allowed.parent().unwrap()).unwrap();
    std::fs::write(&allowed, "original\n").unwrap();
    std::fs::write(&sibling, "sibling\n").unwrap();

    let accepted = run_boundary(
        temporary.path(),
        CognitiveRole::Fixer,
        vec!["src/allowed.rs".into()],
        write_call("src/allowed.rs", "fixed\n"),
        role_artifact(CognitiveRole::Fixer, "src/allowed.rs"),
    )
    .await;
    assert_eq!(accepted.snapshot.status, AgentRunStatus::Succeeded);
    accepted.output_validation.unwrap();
    assert!(accepted
        .output
        .as_ref()
        .unwrap()
        .evidence_refs
        .contains(&"finding:F-1".into()));
    assert_eq!(accepted.calls, vec![("file_write".into(), false)]);
    assert_eq!(std::fs::read_to_string(&allowed).unwrap(), "fixed\n");
    assert_eq!(std::fs::read_to_string(&sibling).unwrap(), "sibling\n");

    let rejected = run_boundary(
        temporary.path(),
        CognitiveRole::Fixer,
        vec!["src/allowed.rs".into()],
        write_call("src/sibling.rs", "escaped\n"),
        role_artifact(CognitiveRole::Fixer, "src/sibling.rs"),
    )
    .await;
    assert_eq!(rejected.calls, vec![("file_write".into(), true)]);
    assert_eq!(std::fs::read_to_string(&sibling).unwrap(), "sibling\n");
    assert!(rejected
        .snapshot
        .result
        .unwrap()
        .evidence
        .iter()
        .any(|evidence| {
            evidence.kind == "tool_result" && evidence.content.contains("outside the admitted")
        }));
}
