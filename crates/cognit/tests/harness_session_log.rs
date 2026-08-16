use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use cognit::harness::session_log::{
    HarnessRequestHeader, HarnessSessionEvent, HarnessSessionEventKind, HarnessSessionId,
    HarnessSessionLog, HarnessSessionLogError, HarnessSessionPersistence, RequestHeaderReason,
    SurfaceOp, TurnEndReason, UserMessageSource,
};
use contracts::{ContentBlock, Message, Role, StopReason, StreamChunk};

fn assistant_with_tool(id: &str, name: &str) -> Message {
    Message {
        role: Role::Assistant,
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input: serde_json::json!({"path": "README.md"}),
        }],
    }
}

#[derive(Debug, Default)]
struct MemoryPersistence(Mutex<BTreeMap<String, Vec<HarnessSessionEvent>>>);

impl HarnessSessionPersistence for MemoryPersistence {
    fn load(&self, id: &HarnessSessionId) -> Result<Vec<HarnessSessionEvent>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&id.0)
            .cloned()
            .unwrap_or_default())
    }

    fn append(&self, id: &HarnessSessionId, event: &HarnessSessionEvent) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .entry(id.0.clone())
            .or_default()
            .push(event.clone());
        Ok(())
    }
}

#[derive(Debug)]
struct RejectingPersistence;

impl HarnessSessionPersistence for RejectingPersistence {
    fn load(&self, _: &HarnessSessionId) -> Result<Vec<HarnessSessionEvent>, String> {
        Ok(vec![])
    }

    fn append(&self, _: &HarnessSessionId, _: &HarnessSessionEvent) -> Result<(), String> {
        Err("disk unavailable".into())
    }
}

#[test]
fn append_only_log_reconstructs_exact_model_surface() {
    let mut log = HarnessSessionLog::new(HarnessSessionId("s1".into())).unwrap();
    log.append(
        1,
        HarnessSessionEventKind::TurnStart { turn: 1 },
        None,
        vec![],
    )
    .unwrap();
    log.append(
        2,
        HarnessSessionEventKind::StepStart { turn: 1, step: 0 },
        None,
        vec![],
    )
    .unwrap();
    log.append(
        3,
        HarnessSessionEventKind::UserMessage {
            turn: 1,
            step: 0,
            source: UserMessageSource::Human,
            message: Message::user("inspect the repository"),
        },
        Some(SurfaceOp::Append),
        vec![],
    )
    .unwrap();
    log.append(
        4,
        HarnessSessionEventKind::RequestHeader {
            turn: 1,
            step: 0,
            header: HarnessRequestHeader {
                provider: "deepseek".into(),
                model: "deepseek-chat".into(),
                reasoning_effort: None,
                max_output_tokens: Some(4096),
            },
            reason: RequestHeaderReason::Initial,
        },
        None,
        vec![],
    )
    .unwrap();
    let chunk_seq = log
        .append(
            5,
            HarnessSessionEventKind::AssistantChunk {
                turn: 1,
                step: 0,
                chunk: StreamChunk::ToolUseStart {
                    id: "call-1".into(),
                    name: "file_read".into(),
                },
            },
            None,
            vec![],
        )
        .unwrap()
        .seq;
    log.append(
        6,
        HarnessSessionEventKind::AssistantMessage {
            turn: 1,
            step: 0,
            message: assistant_with_tool("call-1", "file_read"),
            usage: Default::default(),
        },
        Some(SurfaceOp::Append),
        vec![chunk_seq],
    )
    .unwrap();
    let call_seq = log
        .append(
            7,
            HarnessSessionEventKind::ToolCall {
                turn: 1,
                step: 0,
                call_id: "call-1".into(),
                name: "file_read".into(),
                arguments: r#"{"path":"README.md"}"#.into(),
            },
            None,
            vec![],
        )
        .unwrap()
        .seq;
    log.append(
        8,
        HarnessSessionEventKind::ToolResult {
            turn: 1,
            step: 0,
            call_id: "call-1".into(),
            content: "repository read".into(),
            is_error: false,
            error_code: None,
        },
        Some(SurfaceOp::Append),
        vec![call_seq],
    )
    .unwrap();
    log.append(
        9,
        HarnessSessionEventKind::StepEnd { turn: 1, step: 0 },
        None,
        vec![],
    )
    .unwrap();
    log.append(
        10,
        HarnessSessionEventKind::TurnEnd {
            turn: 1,
            reason: TurnEndReason::Completed,
        },
        None,
        vec![],
    )
    .unwrap();

    let messages = log.derive_messages();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[1].role, Role::Assistant);
    assert!(matches!(
        messages[2].content.as_slice(),
        [ContentBlock::ToolResult { tool_use_id, .. }] if tool_use_id == "call-1"
    ));

    let restored =
        HarnessSessionLog::restore(HarnessSessionId("s1".into()), log.events().to_vec()).unwrap();
    assert_eq!(restored.events(), log.events());
    assert_eq!(restored.derive_messages().len(), 3);
}

#[test]
fn transition_failure_is_atomic_and_pending_tools_block_step_end() {
    let mut log = HarnessSessionLog::new(HarnessSessionId("s2".into())).unwrap();
    log.append(
        1,
        HarnessSessionEventKind::TurnStart { turn: 1 },
        None,
        vec![],
    )
    .unwrap();
    log.append(
        2,
        HarnessSessionEventKind::StepStart { turn: 1, step: 0 },
        None,
        vec![],
    )
    .unwrap();
    log.append(
        3,
        HarnessSessionEventKind::AssistantMessage {
            turn: 1,
            step: 0,
            message: assistant_with_tool("call-1", "file_read"),
            usage: Default::default(),
        },
        Some(SurfaceOp::Append),
        vec![],
    )
    .unwrap();
    log.append(
        4,
        HarnessSessionEventKind::ToolCall {
            turn: 1,
            step: 0,
            call_id: "call-1".into(),
            name: "file_read".into(),
            arguments: "{}".into(),
        },
        None,
        vec![],
    )
    .unwrap();
    let before = log.events().len();
    let error = log
        .append(
            5,
            HarnessSessionEventKind::StepEnd { turn: 1, step: 0 },
            None,
            vec![],
        )
        .unwrap_err();
    assert!(matches!(
        error,
        HarnessSessionLogError::InvalidTransition(_)
    ));
    assert_eq!(log.events().len(), before);
}

#[test]
fn surface_replacement_requires_complete_citations() {
    let mut log = HarnessSessionLog::new(HarnessSessionId("s3".into())).unwrap();
    log.append(
        0,
        HarnessSessionEventKind::TurnStart { turn: 1 },
        None,
        vec![],
    )
    .unwrap();
    log.append(
        0,
        HarnessSessionEventKind::StepStart { turn: 1, step: 0 },
        None,
        vec![],
    )
    .unwrap();
    let first = log
        .append(
            0,
            HarnessSessionEventKind::UserMessage {
                turn: 1,
                step: 0,
                source: UserMessageSource::Human,
                message: Message::user("first"),
            },
            Some(SurfaceOp::Append),
            vec![],
        )
        .unwrap()
        .seq;
    let second = log
        .append(
            0,
            HarnessSessionEventKind::UserMessage {
                turn: 1,
                step: 0,
                source: UserMessageSource::Injected,
                message: Message::user("second"),
            },
            Some(SurfaceOp::Append),
            vec![],
        )
        .unwrap()
        .seq;
    let error = log
        .append(
            0,
            HarnessSessionEventKind::UserMessage {
                turn: 1,
                step: 0,
                source: UserMessageSource::Compaction,
                message: Message::user("summary"),
            },
            Some(SurfaceOp::Replace {
                start: first,
                end: second,
            }),
            vec![first],
        )
        .unwrap_err();
    assert_eq!(error, HarnessSessionLogError::InvalidSurfaceReplacement);

    log.append(
        0,
        HarnessSessionEventKind::UserMessage {
            turn: 1,
            step: 0,
            source: UserMessageSource::Compaction,
            message: Message::user("summary"),
        },
        Some(SurfaceOp::Replace {
            start: first,
            end: second,
        }),
        vec![first, second],
    )
    .unwrap();
    let messages = log.derive_messages();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        messages[0].content.as_slice(),
        [ContentBlock::Text { text }] if text == "summary"
    ));
}

#[test]
fn assistant_sources_must_be_chunks_from_the_same_step() {
    let mut log = HarnessSessionLog::new(HarnessSessionId("s4".into())).unwrap();
    log.append(
        0,
        HarnessSessionEventKind::TurnStart { turn: 1 },
        None,
        vec![],
    )
    .unwrap();
    log.append(
        0,
        HarnessSessionEventKind::StepStart { turn: 1, step: 0 },
        None,
        vec![],
    )
    .unwrap();
    let unrelated = log
        .append(
            0,
            HarnessSessionEventKind::InboxSpliced {
                target: cognit::harness::session_log::InboxTarget::NextStep,
                start: 0,
                removed_count: 0,
                inserted: vec![],
                outcome: None,
            },
            None,
            vec![],
        )
        .unwrap()
        .seq;
    let error = log
        .append(
            0,
            HarnessSessionEventKind::AssistantMessage {
                turn: 1,
                step: 0,
                message: Message::assistant("done"),
                usage: Default::default(),
            },
            Some(SurfaceOp::Append),
            vec![unrelated],
        )
        .unwrap_err();
    assert_eq!(error, HarnessSessionLogError::InvalidSourceReferences);

    // Keep otherwise-unused imported protocol variants covered as serializable
    // event vocabulary, not terminal inference.
    let _ = StreamChunk::Done {
        stop_reason: StopReason::EndTurn,
    };
}

#[test]
fn observation_store_is_append_before_publish_and_restores_private_harness_log() {
    let rejected = Arc::new(RejectingPersistence);
    let mut failing =
        HarnessSessionLog::new_persistent(HarnessSessionId("persist-fail".into()), rejected)
            .unwrap();
    assert!(matches!(
        failing.append(
            1,
            HarnessSessionEventKind::TurnStart { turn: 1 },
            None,
            vec![]
        ),
        Err(HarnessSessionLogError::Persistence(_))
    ));
    assert!(
        failing.events().is_empty(),
        "failed durable append leaked into live state"
    );

    let store = Arc::new(MemoryPersistence::default());
    let id = HarnessSessionId("persist-ok".into());
    let mut first = HarnessSessionLog::new_persistent(id.clone(), store.clone()).unwrap();
    first
        .append(
            1,
            HarnessSessionEventKind::TurnStart { turn: 1 },
            None,
            vec![],
        )
        .unwrap();
    first
        .append(
            2,
            HarnessSessionEventKind::TurnEnd {
                turn: 1,
                reason: TurnEndReason::Blocked,
            },
            None,
            vec![],
        )
        .unwrap();
    drop(first);

    let restored = HarnessSessionLog::new_persistent(id, store).unwrap();
    assert_eq!(restored.events().len(), 2);
    assert_eq!(restored.next_turn_number(), 2);
}
