use ::contracts::dasein::{SelfVersion, Stimmung};
use ::contracts::{
    AgoraSpaceId, ConsciousContextProjection, ContextProjectionReceipt, Message, OperationId,
    ProcessId, StructuredSelfView, TurnRequest,
};
use aletheon::wiring::application::context_assembler::{
    working_directory_policy_prompt, ContextAssembler, ContextAssemblyError, ContextFragments,
    ContextSource, ProductionContextSource,
};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

struct FixedSource(ContextFragments);
#[async_trait::async_trait]
impl ContextSource for FixedSource {
    async fn load(&self, _: &TurnRequest) -> Result<ContextFragments, ContextAssemblyError> {
        Ok(self.0.clone())
    }
}

struct UnavailableConsciousContext;

#[async_trait::async_trait]
impl ::contracts::LatestConsciousContextPort for UnavailableConsciousContext {
    async fn latest_context(&self, _: &AgoraSpaceId) -> anyhow::Result<ConsciousContextProjection> {
        anyhow::bail!("conscious workspace has not observed a turn")
    }
}

fn request(input: &str) -> TurnRequest {
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        context: turn_request_support::context("session", PathBuf::from("/workspace")),
        input: input.into(),
        execution_target: ::contracts::ExecutionTargetSelection::default(),
        model_policy: None,
        deadline: None,
        requirements: Vec::new(),
        requested_task_kind: None,
        evaluation_contract: None,
    }
}
fn text(message: &Message) -> &str {
    match &message.content[0] {
        ::contracts::ContentBlock::Text { text } => text,
        other => panic!("expected text, got {other:?}"),
    }
}

fn projection() -> ConsciousContextProjection {
    ConsciousContextProjection {
        latest_broadcast: None,
        self_view: StructuredSelfView {
            version: SelfVersion(3),
            mood: Stimmung::Gelassenheit,
            concerns: vec!["finish the current task".into()],
            care_concerns: vec![],
            projection: Some("verify the implementation".into()),
            protentions: vec!["tests remain green".into()],
        },
        receipt: ContextProjectionReceipt {
            space: AgoraSpaceId("session".into()),
            broadcast_epoch: None,
            workspace_version: None,
            dasein_version: SelfVersion(3),
            content_ids: vec![],
        },
    }
}

#[tokio::test]
async fn production_source_allows_first_turn_without_conscious_projection() {
    let skills = tempfile::tempdir().unwrap();
    let source = ProductionContextSource {
        cached_prefix: Arc::new(Mutex::new("system".into())),
        skill_loader: Arc::new(Mutex::new(corpus::SkillLoader::new(
            skills.path().to_path_buf(),
        ))),
        skill_router: Arc::new(Mutex::new(corpus::SkillRouter::new())),
        conscious: Arc::new(UnavailableConsciousContext),
        memory_service: None,
        recall_enabled: false,
        recall_max_items: 0,
        recall_max_bytes: 0,
        recall_timeout_ms: 0,
    };

    let fragments = source.load(&request("first turn")).await.unwrap();

    assert!(fragments.conscious.is_none());
    assert!(fragments.system_prefix.contains("system"));
    assert!(fragments
        .system_prefix
        .contains("Current working directory: /workspace"));
}

#[tokio::test]
async fn fragments_have_one_deterministic_order_before_raw_input() {
    let assembler = ContextAssembler::new(Arc::new(FixedSource(ContextFragments {
        system_prefix: "system".into(),
        skills: "S".into(),
        conscious: Some(projection()),
        memory_context: String::new(),
    })));
    let assembled = assembler
        .assemble(
            &request("raw user"),
            &[Message::assistant("prior")],
            1_000.into(),
            &[],
        )
        .await
        .unwrap();
    let positions: Vec<_> = ["<conscious-context>", "<skills>", "raw user"]
        .into_iter()
        .map(|part| assembled.effective_user_message.find(part).unwrap())
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(text(&assembled.messages[0]), "system");
    assert_eq!(text(&assembled.messages[1]), "prior");
    assert_eq!(assembled.projection_receipt, Some(projection().receipt));
    assert_eq!(
        text(assembled.messages.last().unwrap()),
        assembled.effective_user_message
    );
}

#[tokio::test]
async fn prepared_budget_costs_include_dynamic_context_without_relabeling_user_input() {
    let assembler = ContextAssembler::new(Arc::new(FixedSource(ContextFragments {
        system_prefix: "stable system".into(),
        skills: "selected skill body".into(),
        conscious: None,
        memory_context: "recalled memory evidence".into(),
    })));
    let request = request("raw user input");
    let prepared = assembler.prepare(&request).await.unwrap();
    let costs = prepared.budget_costs(&request.input).unwrap();

    assert_eq!(
        costs.pending_input_tokens.get(),
        u64::try_from(Message::user("raw user input").estimate_tokens()).unwrap()
    );
    assert!(
        costs.system_and_skill_tokens.get()
            > u64::try_from(Message::system("stable system").estimate_tokens()).unwrap(),
        "selected skills and recalled context must consume non-history capacity"
    );
}

#[tokio::test]
async fn partition_profile_covers_tool_memory_and_compacted_history_without_raw_content() {
    use runtime::prompt_partition::PromptRegion;

    let assembler = ContextAssembler::new(Arc::new(FixedSource(ContextFragments {
        system_prefix: "stable-secret-prefix".into(),
        skills: String::new(),
        conscious: None,
        memory_context: "recalled-secret-memory".into(),
    })));
    let history = vec![
        Message::assistant("compacted-secret-summary"),
        Message::tool_result("tool-1", "tool-secret-result", false),
    ];
    let assembled = assembler
        .assemble(
            &request("current-secret-input"),
            &history,
            1_000.into(),
            &[],
        )
        .await
        .unwrap();

    for region in [
        PromptRegion::StablePrefix,
        PromptRegion::Conversation,
        PromptRegion::DynamicContext,
        PromptRegion::CurrentInput,
    ] {
        assert!(assembled.profile.region_bytes(region) > 0);
    }
    let debug = format!("{:?}", assembled.profile);
    for raw in [
        "stable-secret-prefix",
        "recalled-secret-memory",
        "compacted-secret-summary",
        "tool-secret-result",
        "current-secret-input",
    ] {
        assert!(!debug.contains(raw), "profile leaked raw content: {raw}");
    }
    assert!(debug.contains("sha256:"));
}

#[tokio::test]
async fn fragments_and_history_are_bounded_and_utf8_safe() {
    let huge = "界".repeat(200_000);
    let assembler = ContextAssembler::new(Arc::new(FixedSource(ContextFragments {
        system_prefix: huge.clone(),
        skills: huge,
        conscious: Some(projection()),
        memory_context: String::new(),
    })));
    let assembled = assembler
        .assemble(
            &request("raw"),
            &[Message::user("x".repeat(200_000))],
            8_000.into(),
            &[],
        )
        .await
        .unwrap();
    assert!(assembled.effective_user_message.chars().count() < 50_000);
    assert!(text(&assembled.messages[0]).chars().count() <= 128 * 1024);
    assert!(text(&assembled.messages[1]).chars().count() <= 32 * 1024);
    assert!(assembled
        .effective_user_message
        .is_char_boundary(assembled.effective_user_message.len()));
}

#[tokio::test]
async fn host_projected_fragments_rescrub_legacy_secrets_without_rewriting_user_input() {
    let mut conscious = projection();
    conscious.self_view.concerns = vec!["old sk-consciousSecret123".into()];
    let assembler = ContextAssembler::new(Arc::new(FixedSource(ContextFragments {
        system_prefix: "system".into(),
        skills: "credential key-skillSecret123".into(),
        conscious: Some(conscious),
        memory_context: "API 密钥 memorySecret123".into(),
    })));

    let assembled = assembler
        .assemble(
            &request("current user sk-userSecret123"),
            &[],
            1_000.into(),
            &[],
        )
        .await
        .unwrap();

    assert!(!assembled
        .effective_user_message
        .contains("sk-consciousSecret123"));
    assert!(!assembled
        .effective_user_message
        .contains("key-skillSecret123"));
    assert!(!assembled.effective_user_message.contains("memorySecret123"));
    assert!(assembled
        .effective_user_message
        .contains("current user sk-userSecret123"));
    assert_eq!(
        assembled
            .effective_user_message
            .matches("[REDACTED]")
            .count(),
        3
    );
}

#[test]
fn working_directory_prompt_distinguishes_policy_from_host_mounts() {
    let prompt = working_directory_policy_prompt(PathBuf::from("/workspace/project").as_path());
    let lower = prompt.to_lowercase();

    assert!(prompt.contains("Current working directory: /workspace/project"));
    assert!(prompt.contains("configured sandbox/working-directory policy"));
    assert!(lower.contains("host mount state was not checked"));
    assert!(lower.contains("do not change host mounts"));
    assert!(lower.contains("relaunch from the intended working directory"));
    assert!(lower.contains("choose a path inside this directory"));
    assert!(!lower.contains("sudo mount"));
    assert!(!lower.contains("mount -o"));
}

#[test]
fn turn_pipeline_prepares_and_assembles_context_once() {
    let pipeline = include_str!("../src/wiring/application/turn_pipeline.rs");
    assert!(pipeline.contains(".context_assembler"));
    assert_eq!(
        pipeline.matches(".prepare(&context_request)").count(),
        1,
        "the exact projected context must be prepared once before budget planning"
    );
    assert_eq!(
        pipeline.matches(".assemble_prepared(").count(),
        1,
        "the prepared context must be assembled once after history budgeting"
    );
    assert!(pipeline.contains("prepared_context.budget_costs(&context_request.input)"));
    assert!(pipeline.contains("\"prompt_construction_profile\": prompt_profile"));
    assert!(pipeline.contains("\"tool_count\": tool_defs.len()"));
    assert!(!pipeline.contains("serde_json::json!({\"tool_count\": 0})"));
    assert_eq!(
        pipeline.matches("self.active_profile.snapshot()").count(),
        1,
        "one immutable profile snapshot must govern the entire turn"
    );
    assert!(pipeline.contains(".canonical_sessions"));
    assert!(pipeline.contains("let canonical_session = ::contracts::SessionId"));
    assert!(pipeline.contains(".resume(&canonical_session)"));
    for removed in [
        "inject_keyword_skills",
        "inject_composite_recall",
        "inject_core_memory",
        "inject_skill_suggestion",
        "build_request_messages(system_prompt",
        "sm.history()",
    ] {
        assert!(
            !pipeline.contains(removed),
            "duplicate context route: {removed}"
        );
    }
    let daemon_modules = include_str!("../src/wiring/application/daemon_turn/mod.rs");
    assert!(!daemon_modules.contains("mod injection"));
}
mod turn_request_support;

#[tokio::test]
async fn assemble_records_the_real_tool_count_in_the_prompt_profile() {
    let assembler = ContextAssembler::new(Arc::new(FixedSource(ContextFragments {
        system_prefix: "system".into(),
        skills: String::new(),
        conscious: None,
        memory_context: String::new(),
    })));
    let tools = (0..23)
        .map(|index| ::contracts::ToolDefinition {
            name: format!("tool_{index}"),
            description: format!("tool {index}"),
            input_schema: serde_json::json!({"type": "object"}),
        })
        .collect::<Vec<_>>();
    let assembled = assembler
        .assemble(&request("tool profile"), &[], 1_000.into(), &tools)
        .await
        .unwrap();
    let tool_region = assembled
        .profile
        .partitions
        .iter()
        .find(|partition| partition.region == runtime::prompt_partition::PromptRegion::StableTools)
        .expect("stable tools partition must exist");
    assert_eq!(
        tool_region.chars, 23,
        "the real tool count must be recorded in the prompt profile"
    );
    assert!(tool_region.serialized_bytes > 0);
    assert!(tool_region
        .content_digest
        .as_deref()
        .is_some_and(|digest| digest.starts_with("sha256:")));
}
