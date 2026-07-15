use executive::service::context_assembler::{
    ContextAssembler, ContextAssemblyError, ContextFragments, ContextSource,
};
use fabric::{Message, OperationId, ProcessId, TurnRequest};
use std::{path::PathBuf, sync::Arc};

struct FixedSource(ContextFragments);
#[async_trait::async_trait]
impl ContextSource for FixedSource {
    async fn load(&self, _: &TurnRequest) -> Result<ContextFragments, ContextAssemblyError> {
        Ok(self.0.clone())
    }
}

fn request(input: &str) -> TurnRequest {
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        session_id: "session".into(),
        input: input.into(),
        working_dir: PathBuf::from("/workspace"),
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

#[tokio::test]
async fn fragments_have_one_deterministic_order_before_raw_input() {
    let assembler = ContextAssembler::new(Arc::new(FixedSource(ContextFragments {
        system_prefix: "system".into(),
        recall: "R".into(),
        core_memory: "C".into(),
        facts: "F".into(),
        skills: "S".into(),
        dasein: "D".into(),
        agora: "A".into(),
    })));
    let assembled = assembler
        .assemble(&request("raw user"), &[Message::assistant("prior")])
        .await
        .unwrap();
    let positions: Vec<_> = [
        "<recall>",
        "<core-memory>",
        "<facts>",
        "<skills>",
        "<dasein>",
        "<agora>",
        "raw user",
    ]
    .into_iter()
    .map(|part| assembled.effective_user_message.find(part).unwrap())
    .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(text(&assembled.messages[0]), "system");
    assert_eq!(text(&assembled.messages[1]), "prior");
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
        recall: huge.clone(),
        core_memory: huge.clone(),
        facts: huge.clone(),
        skills: huge.clone(),
        dasein: huge.clone(),
        agora: huge,
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
