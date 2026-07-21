use executive::service::context_assembler::{
    working_directory_policy_prompt, ContextAssembler, ContextAssemblyError, ContextFragments,
    ContextSource, ProductionContextSource,
};
use fabric::dasein::{SelfVersion, Stimmung};
use fabric::{
    AgoraSpaceId, ConsciousContextProjection, ContextProjectionReceipt, Message, OperationId,
    ProcessId, StructuredSelfView, TurnRequest,
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
impl fabric::LatestConsciousContextPort for UnavailableConsciousContext {
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
        model_policy: None,
        deadline: None,
    }
}
fn text(message: &Message) -> &str {
    match &message.content[0] {
        fabric::ContentBlock::Text { text } => text,
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
    })));
    let assembled = assembler
        .assemble(&request("raw user"), &[Message::assistant("prior")])
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
async fn fragments_and_history_are_bounded_and_utf8_safe() {
    let huge = "界".repeat(200_000);
    let assembler = ContextAssembler::new(Arc::new(FixedSource(ContextFragments {
        system_prefix: huge.clone(),
        skills: huge,
        conscious: Some(projection()),
    })));
    let assembled = assembler
        .assemble(&request("raw"), &[Message::user("x".repeat(200_000))])
        .await
        .unwrap();
    assert!(assembled.effective_user_message.chars().count() < 50_000);
    assert!(text(&assembled.messages[0]).chars().count() <= 128 * 1024);
    assert!(text(&assembled.messages[1]).chars().count() <= 32 * 1024);
    assert!(assembled
        .effective_user_message
        .is_char_boundary(assembled.effective_user_message.len()));
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
fn turn_pipeline_has_one_context_assembly_route() {
    let pipeline = include_str!("../src/service/turn_pipeline.rs");
    assert!(pipeline.contains(".context_assembler"));
    assert!(pipeline.contains(".assemble(&context_request, &existing_messages)"));
    assert!(pipeline.contains(".canonical_sessions"));
    assert!(pipeline.contains(".resume(&fabric::SessionId"));
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
    let daemon_modules = include_str!("../src/service/daemon_turn/mod.rs");
    assert!(!daemon_modules.contains("mod injection"));
}
mod turn_request_support;
