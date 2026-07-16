//! Versioned, transport-neutral session history contracts.

use anyhow::Result;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AuditEventId, OperationId, PermitId, SessionId, TurnStop};

pub const SESSION_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TurnId(pub Uuid);

impl TurnId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TurnId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ItemId(pub Uuid);

impl ItemId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ItemId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionRecord {
    pub schema_version: u16,
    #[schemars(with = "String")]
    pub id: SessionId,
    pub parent: Option<SessionFork>,
    pub created_at_ms: u64,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionFork {
    #[schemars(with = "String")]
    pub session_id: SessionId,
    pub through_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Interrupted,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TurnRecord {
    pub schema_version: u16,
    pub id: TurnId,
    #[schemars(with = "String")]
    pub session_id: SessionId,
    #[schemars(with = "Uuid")]
    pub operation_id: OperationId,
    pub started_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub stop: Option<TurnStop>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ItemRecord {
    pub schema_version: u16,
    pub id: ItemId,
    #[schemars(with = "String")]
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub sequence: u64,
    pub created_at_ms: u64,
    pub payload: ItemPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ItemPayload {
    UserMessage {
        content: String,
    },
    AssistantMessage {
        content: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
        #[schemars(with = "Option<Uuid>")]
        permit_id: Option<PermitId>,
        #[schemars(with = "Option<Uuid>")]
        audit_id: Option<AuditEventId>,
    },
    ContextProjection {
        space: String,
        broadcast_epoch: Option<u64>,
        workspace_version: Option<u64>,
        dasein_version: u64,
        content_ids: Vec<String>,
    },
    SystemNotice {
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SessionNotification {
    ItemAppended {
        schema_version: u16,
        item: ItemRecord,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SessionProtocolV1 {
    Session(SessionRecord),
    Turn(TurnRecord),
    Item(ItemRecord),
    Notification(SessionNotification),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppendOutcome {
    Appended,
    AlreadyPresent,
}

#[async_trait]
pub trait SessionAppendStore: Send + Sync {
    async fn create(&self, session: SessionRecord) -> Result<()>;
    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: ItemRecord,
    ) -> Result<AppendOutcome>;
    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: SessionRecord,
    ) -> Result<()>;
    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>>;
    async fn load_items(&self, session: &SessionId, after: Option<u64>) -> Result<Vec<ItemRecord>>;
}
