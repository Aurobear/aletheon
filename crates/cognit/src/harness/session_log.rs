//! Append-only Harness session log and deterministic model-history projection.
//!
//! This schema is private to the cognitive Harness lifecycle. It is not the
//! public Session schema, Runtime progress stream, or daemon diagnostic payload.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use contracts::{ContentBlock, InferenceUsage, Message, Role, StreamChunk};
use serde::{Deserialize, Serialize};

pub const HARNESS_SESSION_EVENT_SCHEMA_VERSION: u32 = 1;

/// Optional append/read seam for the Harness-private cognitive event log.
///
/// This is not the Runtime Session/Turn authority and implementing it does not
/// by itself provide crash recovery. Runtime writers and their durable adapters
/// remain authoritative for public lifecycle and recovery. Implementations must
/// reject duplicate/non-contiguous writes for the same Harness session.
pub trait HarnessSessionPersistence: Send + Sync + Debug {
    fn load(&self, id: &HarnessSessionId) -> Result<Vec<HarnessSessionEvent>, String>;
    fn append(&self, id: &HarnessSessionId, event: &HarnessSessionEvent) -> Result<(), String>;
}

#[derive(Debug, Default)]
/// Volatile implementation for tests and explicitly non-authoritative
/// observation. Its contents disappear with the process.
pub struct InMemoryHarnessSessionPersistence(
    Mutex<std::collections::BTreeMap<String, Vec<HarnessSessionEvent>>>,
);

impl HarnessSessionPersistence for InMemoryHarnessSessionPersistence {
    fn load(&self, id: &HarnessSessionId) -> Result<Vec<HarnessSessionEvent>, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "harness persistence lock poisoned".to_owned())?
            .get(&id.0)
            .cloned()
            .unwrap_or_default())
    }

    fn append(&self, id: &HarnessSessionId, event: &HarnessSessionEvent) -> Result<(), String> {
        let mut sessions = self
            .0
            .lock()
            .map_err(|_| "harness persistence lock poisoned".to_owned())?;
        let events = sessions.entry(id.0.clone()).or_default();
        if event.seq != events.len() as u64 {
            return Err(format!(
                "non-contiguous harness event: expected {}, got {}",
                events.len(),
                event.seq
            ));
        }
        events.push(event.clone());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessSessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxTarget {
    NextTurn,
    NextStep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxSpliceOutcome {
    Claimed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessInboxMessage {
    pub id: String,
    pub message: Message,
    pub source: UserMessageSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserMessageSource {
    Human,
    Steering,
    Injected,
    Compaction,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndReason {
    Completed,
    MaxTokens,
    Blocked,
    Aborted { cause: String },
    Error { code: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestHeaderReason {
    Initial,
    Resume,
    Change,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessRequestHeader {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SurfaceOp {
    Append,
    Replace { start: u64, end: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HarnessSessionEventKind {
    InboxSpliced {
        target: InboxTarget,
        start: usize,
        removed_count: usize,
        inserted: Vec<HarnessInboxMessage>,
        outcome: Option<InboxSpliceOutcome>,
    },
    TurnStart {
        turn: u64,
    },
    TurnEnd {
        turn: u64,
        reason: TurnEndReason,
    },
    StepStart {
        turn: u64,
        step: u32,
    },
    StepEnd {
        turn: u64,
        step: u32,
    },
    UserMessage {
        turn: u64,
        step: u32,
        source: UserMessageSource,
        message: Message,
    },
    RequestHeader {
        turn: u64,
        step: u32,
        header: HarnessRequestHeader,
        reason: RequestHeaderReason,
    },
    /// Exact model-visible request material for deterministic reconstruction.
    RequestPrepared {
        turn: u64,
        step: u32,
        system_prompt: String,
        tool_definitions: Vec<contracts::ToolDefinition>,
    },
    RequestError {
        turn: u64,
        step: u32,
        attempt: u32,
        code: String,
        message: String,
        will_retry: bool,
    },
    AssistantChunk {
        turn: u64,
        step: u32,
        chunk: StreamChunk,
    },
    AssistantMessage {
        turn: u64,
        step: u32,
        message: Message,
        #[serde(default)]
        usage: InferenceUsage,
    },
    ToolCall {
        turn: u64,
        step: u32,
        call_id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        turn: u64,
        step: u32,
        call_id: String,
        content: String,
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_code: Option<String>,
    },
}

impl HarnessSessionEventKind {
    fn coordinates(&self) -> Option<(u64, u32)> {
        match self {
            Self::StepStart { turn, step }
            | Self::StepEnd { turn, step }
            | Self::UserMessage { turn, step, .. }
            | Self::RequestHeader { turn, step, .. }
            | Self::RequestPrepared { turn, step, .. }
            | Self::RequestError { turn, step, .. }
            | Self::AssistantChunk { turn, step, .. }
            | Self::AssistantMessage { turn, step, .. }
            | Self::ToolCall { turn, step, .. }
            | Self::ToolResult { turn, step, .. } => Some((*turn, *step)),
            Self::InboxSpliced { .. } | Self::TurnStart { .. } | Self::TurnEnd { .. } => None,
        }
    }

    fn is_surface(&self) -> bool {
        matches!(
            self,
            Self::UserMessage { .. } | Self::AssistantMessage { .. } | Self::ToolResult { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessSessionEvent {
    pub schema_version: u32,
    pub seq: u64,
    pub time_ms: i64,
    pub kind: HarnessSessionEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<SurfaceOp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_event_seqs: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HarnessSessionLogError {
    #[error("session id must not be empty")]
    EmptySessionId,
    #[error("event sequence {actual} is not contiguous; expected {expected}")]
    NonContiguousSequence { expected: u64, actual: u64 },
    #[error("event schema version {0} is unsupported")]
    UnsupportedSchema(u32),
    #[error("surface metadata is required exactly for model-visible events")]
    InvalidSurfaceMetadata,
    #[error("source event references must be unique, earlier, and in range")]
    InvalidSourceReferences,
    #[error("surface replacement range is invalid or incompletely cited")]
    InvalidSurfaceReplacement,
    #[error("{0}")]
    InvalidTransition(String),
    #[error("session persistence failed: {0}")]
    Persistence(String),
    #[error("message role is invalid for {event}: expected {expected:?}, got {actual:?}")]
    InvalidMessageRole {
        event: &'static str,
        expected: Role,
        actual: Role,
    },
}

#[derive(Debug, Clone)]
struct SessionTrace {
    open_turn: Option<u64>,
    open_step: Option<u32>,
    next_turn: u64,
    next_step: u32,
    pending_calls: BTreeSet<String>,
}

impl Default for SessionTrace {
    fn default() -> Self {
        Self {
            open_turn: None,
            open_step: None,
            next_turn: 1,
            next_step: 0,
            pending_calls: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HarnessSessionLog {
    id: HarnessSessionId,
    events: Vec<HarnessSessionEvent>,
    surface: Vec<u64>,
    trace: SessionTrace,
    persistence: Option<Arc<dyn HarnessSessionPersistence>>,
}

impl HarnessSessionLog {
    pub fn new(id: HarnessSessionId) -> Result<Self, HarnessSessionLogError> {
        if id.0.trim().is_empty() {
            return Err(HarnessSessionLogError::EmptySessionId);
        }
        Ok(Self {
            id,
            events: Vec::new(),
            surface: Vec::new(),
            trace: SessionTrace::default(),
            persistence: None,
        })
    }

    pub fn restore(
        id: HarnessSessionId,
        events: Vec<HarnessSessionEvent>,
    ) -> Result<Self, HarnessSessionLogError> {
        let mut log = Self::new(id)?;
        for event in events {
            log.accept(event)?;
        }
        Ok(log)
    }

    pub fn new_persistent(
        id: HarnessSessionId,
        persistence: Arc<dyn HarnessSessionPersistence>,
    ) -> Result<Self, HarnessSessionLogError> {
        let events = persistence
            .load(&id)
            .map_err(HarnessSessionLogError::Persistence)?;
        let mut log = Self::restore(id, events)?;
        log.persistence = Some(persistence);
        Ok(log)
    }

    pub fn id(&self) -> &HarnessSessionId {
        &self.id
    }

    pub fn events(&self) -> &[HarnessSessionEvent] {
        &self.events
    }

    pub fn next_seq(&self) -> u64 {
        self.events.len() as u64
    }

    pub fn next_turn_number(&self) -> u64 {
        self.trace.next_turn
    }

    pub fn next_step_number(&self) -> u32 {
        self.trace.next_step
    }

    pub fn surface_seqs(&self) -> &[u64] {
        &self.surface
    }

    pub fn append(
        &mut self,
        time_ms: i64,
        kind: HarnessSessionEventKind,
        surface: Option<SurfaceOp>,
        source_event_seqs: Vec<u64>,
    ) -> Result<&HarnessSessionEvent, HarnessSessionLogError> {
        let event = HarnessSessionEvent {
            schema_version: HARNESS_SESSION_EVENT_SCHEMA_VERSION,
            seq: self.next_seq(),
            time_ms,
            kind,
            surface,
            source_event_seqs,
        };
        if let Some(persistence) = self.persistence.clone() {
            let mut candidate = self.clone();
            candidate.persistence = None;
            candidate.accept(event.clone())?;
            persistence
                .append(&self.id, &event)
                .map_err(HarnessSessionLogError::Persistence)?;
            candidate.persistence = Some(persistence);
            *self = candidate;
        } else {
            self.accept(event)?;
        }
        Ok(self.events.last().expect("accepted event was appended"))
    }

    pub fn derive_messages(&self) -> Vec<Message> {
        self.surface
            .iter()
            .filter_map(|seq| match &self.events[*seq as usize].kind {
                HarnessSessionEventKind::UserMessage { message, .. }
                | HarnessSessionEventKind::AssistantMessage { message, .. } => {
                    (!message.content.is_empty()).then(|| message.clone())
                }
                HarnessSessionEventKind::ToolResult {
                    call_id,
                    content,
                    is_error,
                    ..
                } => Some(Message::tool_result(call_id, content, *is_error)),
                _ => None,
            })
            .collect()
    }

    fn accept(&mut self, event: HarnessSessionEvent) -> Result<(), HarnessSessionLogError> {
        let expected = self.next_seq();
        if event.seq != expected {
            return Err(HarnessSessionLogError::NonContiguousSequence {
                expected,
                actual: event.seq,
            });
        }
        if event.schema_version != HARNESS_SESSION_EVENT_SCHEMA_VERSION {
            return Err(HarnessSessionLogError::UnsupportedSchema(
                event.schema_version,
            ));
        }
        if event.kind.is_surface() != event.surface.is_some() {
            return Err(HarnessSessionLogError::InvalidSurfaceMetadata);
        }
        self.validate_sources(&event)?;
        if matches!(event.kind, HarnessSessionEventKind::RequestPrepared { .. })
            && event.source_event_seqs != self.surface
        {
            return Err(HarnessSessionLogError::InvalidSourceReferences);
        }
        if let HarnessSessionEventKind::ToolResult {
            turn,
            step,
            call_id,
            ..
        } = &event.kind
        {
            if event.source_event_seqs.len() != 1
                || !matches!(
                    &self.events[event.source_event_seqs[0] as usize].kind,
                    HarnessSessionEventKind::ToolCall {
                        turn: call_turn,
                        step: call_step,
                        call_id: source_call_id,
                        ..
                    } if call_turn == turn && call_step == step && source_call_id == call_id
                )
            {
                return Err(HarnessSessionLogError::InvalidSourceReferences);
            }
        }
        if let HarnessSessionEventKind::ToolCall {
            turn,
            step,
            call_id,
            name,
            ..
        } = &event.kind
        {
            let declared = self.events.iter().rev().any(|candidate| {
                matches!(
                    &candidate.kind,
                    HarnessSessionEventKind::AssistantMessage {
                        turn: message_turn,
                        step: message_step,
                        message,
                        ..
                    } if message_turn == turn
                        && message_step == step
                        && assistant_declares_tool_call(message, call_id, name)
                )
            });
            if !declared {
                return Err(HarnessSessionLogError::InvalidTransition(format!(
                    "tool/call {call_id} was not declared by the assistant message"
                )));
            }
        }

        let mut next_trace = self.trace.clone();
        validate_transition(&mut next_trace, &event.kind)?;
        let mut next_surface = self.surface.clone();
        apply_surface(&mut next_surface, &event)?;

        self.trace = next_trace;
        self.surface = next_surface;
        self.events.push(event);
        Ok(())
    }

    fn validate_sources(&self, event: &HarnessSessionEvent) -> Result<(), HarnessSessionLogError> {
        let unique = event
            .source_event_seqs
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if unique.len() != event.source_event_seqs.len()
            || unique.iter().any(|source| *source >= event.seq)
        {
            return Err(HarnessSessionLogError::InvalidSourceReferences);
        }
        if matches!(event.kind, HarnessSessionEventKind::AssistantMessage { .. }) {
            let coordinates = event.kind.coordinates();
            if event.source_event_seqs.iter().any(|source| {
                !matches!(
                    &self.events[*source as usize].kind,
                    HarnessSessionEventKind::AssistantChunk { turn, step, .. }
                        if Some((*turn, *step)) == coordinates
                )
            }) {
                return Err(HarnessSessionLogError::InvalidSourceReferences);
            }
        }
        Ok(())
    }
}

fn require_open_step(
    trace: &SessionTrace,
    event: &'static str,
    turn: u64,
    step: u32,
) -> Result<(), HarnessSessionLogError> {
    if trace.open_turn != Some(turn) || trace.open_step != Some(step) {
        return Err(HarnessSessionLogError::InvalidTransition(format!(
            "{event} names turn {turn}/step {step}, but the open coordinates are {:?}/{:?}",
            trace.open_turn, trace.open_step
        )));
    }
    Ok(())
}

fn validate_transition(
    trace: &mut SessionTrace,
    kind: &HarnessSessionEventKind,
) -> Result<(), HarnessSessionLogError> {
    match kind {
        HarnessSessionEventKind::InboxSpliced { .. } => {}
        HarnessSessionEventKind::TurnStart { turn } => {
            if trace.open_turn.is_some() || *turn != trace.next_turn {
                return Err(HarnessSessionLogError::InvalidTransition(format!(
                    "turn/start expected {}, got {turn}",
                    trace.next_turn
                )));
            }
            trace.open_turn = Some(*turn);
            trace.open_step = None;
            trace.next_step = 0;
        }
        HarnessSessionEventKind::TurnEnd { turn, .. } => {
            if trace.open_turn != Some(*turn)
                || trace.open_step.is_some()
                || !trace.pending_calls.is_empty()
            {
                return Err(HarnessSessionLogError::InvalidTransition(format!(
                    "turn/end {turn} requires its open turn with no open step or pending tool call"
                )));
            }
            trace.open_turn = None;
            trace.next_turn += 1;
        }
        HarnessSessionEventKind::StepStart { turn, step } => {
            if trace.open_turn != Some(*turn)
                || trace.open_step.is_some()
                || *step != trace.next_step
            {
                return Err(HarnessSessionLogError::InvalidTransition(format!(
                    "step/start expected turn {:?}/step {}, got {turn}/{step}",
                    trace.open_turn, trace.next_step
                )));
            }
            trace.open_step = Some(*step);
        }
        HarnessSessionEventKind::StepEnd { turn, step } => {
            require_open_step(trace, "step/end", *turn, *step)?;
            if !trace.pending_calls.is_empty() {
                return Err(HarnessSessionLogError::InvalidTransition(
                    "step/end has pending tool calls".into(),
                ));
            }
            trace.open_step = None;
            trace.next_step += 1;
        }
        HarnessSessionEventKind::UserMessage {
            turn,
            step,
            message,
            ..
        } => {
            require_open_step(trace, "user/message", *turn, *step)?;
            require_role("user/message", Role::User, message.role)?;
        }
        HarnessSessionEventKind::AssistantMessage {
            turn,
            step,
            message,
            ..
        } => {
            require_open_step(trace, "assistant/message", *turn, *step)?;
            require_role("assistant/message", Role::Assistant, message.role)?;
        }
        HarnessSessionEventKind::RequestHeader {
            turn, step, header, ..
        } => {
            require_open_step(trace, "request/header", *turn, *step)?;
            if header.provider.trim().is_empty() || header.model.trim().is_empty() {
                return Err(HarnessSessionLogError::InvalidTransition(
                    "request/header requires provider and model".into(),
                ));
            }
        }
        HarnessSessionEventKind::RequestError {
            turn,
            step,
            attempt,
            code,
            ..
        } => {
            require_open_step(trace, "request/error", *turn, *step)?;
            if *attempt == 0 || code.trim().is_empty() {
                return Err(HarnessSessionLogError::InvalidTransition(
                    "request/error requires a positive attempt and error code".into(),
                ));
            }
        }
        HarnessSessionEventKind::RequestPrepared {
            turn,
            step,
            system_prompt: _,
            tool_definitions,
        } => {
            require_open_step(trace, "request/prepared", *turn, *step)?;
            contracts::canonicalize_tool_definitions(tool_definitions).map_err(|error| {
                HarnessSessionLogError::InvalidTransition(format!(
                    "request/prepared has invalid tool definitions: {error}"
                ))
            })?;
        }
        HarnessSessionEventKind::AssistantChunk { turn, step, .. } => {
            require_open_step(trace, "assistant/chunk", *turn, *step)?;
        }
        HarnessSessionEventKind::ToolCall {
            turn,
            step,
            call_id,
            name,
            arguments,
        } => {
            require_open_step(trace, "tool/call", *turn, *step)?;
            if call_id.trim().is_empty()
                || name.trim().is_empty()
                || serde_json::from_str::<serde_json::Value>(arguments).is_err()
                || !trace.pending_calls.insert(call_id.clone())
            {
                return Err(HarnessSessionLogError::InvalidTransition(
                    "tool/call requires unique id, name, and valid raw JSON arguments".into(),
                ));
            }
        }
        HarnessSessionEventKind::ToolResult {
            turn,
            step,
            call_id,
            ..
        } => {
            require_open_step(trace, "tool/result", *turn, *step)?;
            if !trace.pending_calls.remove(call_id) {
                return Err(HarnessSessionLogError::InvalidTransition(format!(
                    "tool/result has no pending call {call_id}"
                )));
            }
        }
    }
    Ok(())
}

fn require_role(
    event: &'static str,
    expected: Role,
    actual: Role,
) -> Result<(), HarnessSessionLogError> {
    if actual != expected {
        return Err(HarnessSessionLogError::InvalidMessageRole {
            event,
            expected,
            actual,
        });
    }
    Ok(())
}

fn apply_surface(
    surface: &mut Vec<u64>,
    event: &HarnessSessionEvent,
) -> Result<(), HarnessSessionLogError> {
    match &event.surface {
        None => Ok(()),
        Some(SurfaceOp::Append) => {
            surface.push(event.seq);
            Ok(())
        }
        Some(SurfaceOp::Replace { start, end }) => {
            let start_index = surface.iter().position(|seq| seq == start);
            let end_index = surface.iter().position(|seq| seq == end);
            let (Some(start_index), Some(end_index)) = (start_index, end_index) else {
                return Err(HarnessSessionLogError::InvalidSurfaceReplacement);
            };
            if start_index > end_index {
                return Err(HarnessSessionLogError::InvalidSurfaceReplacement);
            }
            let replaced = &surface[start_index..=end_index];
            if !replaced
                .iter()
                .all(|seq| event.source_event_seqs.contains(seq))
            {
                return Err(HarnessSessionLogError::InvalidSurfaceReplacement);
            }
            surface.splice(start_index..=end_index, [event.seq]);
            Ok(())
        }
    }
}

/// Confirm that an assistant message contains the tool-use block represented
/// by a following `tool/call` event.
pub fn assistant_declares_tool_call(message: &Message, call_id: &str, name: &str) -> bool {
    message.content.iter().any(|block| {
        matches!(
            block,
            ContentBlock::ToolUse { id, name: tool_name, .. }
                if id == call_id && tool_name == name
        )
    })
}
