//! Server-bound Agent/Task memory isolation.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use fabric::{AgentId, AgentTaskId, ProcessId};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    MemoryAuthority, MemoryKind, MemoryMetadata, MemoryProjection, MemoryRecord, MemoryRecordId,
    MemoryScope, MemorySensitivity, MemoryStatus, ProjectedMemory,
};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_memory_bindings(
 process_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, task_id TEXT NOT NULL,
 parent_projection_receipt TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS agent_memory_records(
 record_id TEXT PRIMARY KEY, process_id TEXT NOT NULL, agent_id TEXT NOT NULL,
 task_id TEXT NOT NULL, record_json TEXT NOT NULL,
 FOREIGN KEY(process_id) REFERENCES agent_memory_bindings(process_id));
CREATE TABLE IF NOT EXISTS agent_memory_projection(
 process_id TEXT NOT NULL, record_id TEXT NOT NULL, projected_json TEXT NOT NULL,
 PRIMARY KEY(process_id,record_id),
 FOREIGN KEY(process_id) REFERENCES agent_memory_bindings(process_id));
"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMemoryContext {
    pub process_id: ProcessId,
    pub agent_id: AgentId,
    pub task_id: AgentTaskId,
    pub agent_scope: MemoryScope,
    pub task_scope: MemoryScope,
    pub parent_projection_receipt: String,
}

impl AgentMemoryContext {
    /// Constructed by AgentControl after Kernel process creation. Scope is
    /// derived here; no tool-provided scope participates in authorization.
    pub fn verified(
        process_id: ProcessId,
        agent_id: AgentId,
        task_id: AgentTaskId,
        parent_projection_receipt: impl Into<String>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(!task_id.0.trim().is_empty(), "Agent task ID is required");
        let receipt = parent_projection_receipt.into();
        anyhow::ensure!(
            !receipt.trim().is_empty(),
            "parent memory projection receipt is required"
        );
        Ok(Self {
            process_id,
            agent_scope: MemoryScope::Agent(agent_id.0.to_string()),
            task_scope: MemoryScope::Task(task_id.0.clone()),
            agent_id,
            task_id,
            parent_projection_receipt: receipt,
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.agent_scope == MemoryScope::Agent(self.agent_id.0.to_string()),
            "Agent memory scope is not derived from Agent identity"
        );
        anyhow::ensure!(
            self.task_scope == MemoryScope::Task(self.task_id.0.clone()),
            "task memory scope is not derived from task identity"
        );
        anyhow::ensure!(
            !self.parent_projection_receipt.trim().is_empty(),
            "parent memory projection receipt is required"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildMemoryDraft {
    pub kind: MemoryKind,
    pub content: String,
    pub authority: MemoryAuthority,
    pub source_event_ids: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Clone)]
pub struct AgentMemoryVault {
    connection: Arc<Mutex<Connection>>,
}

impl AgentMemoryVault {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> anyhow::Result<Self> {
        connection.execute_batch("PRAGMA foreign_keys=ON;")?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Register the trusted process binding before exposing memory operations.
    pub fn register(&self, context: &AgentMemoryContext) -> anyhow::Result<()> {
        context.validate()?;
        let connection = self.connection.lock();
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO agent_memory_bindings(process_id,agent_id,task_id,parent_projection_receipt) VALUES(?1,?2,?3,?4)",
            params![context.process_id.0.to_string(), context.agent_id.0.to_string(), context.task_id.0, context.parent_projection_receipt],
        )?;
        if inserted == 0 {
            self.verify_binding(&connection, context)?;
        }
        Ok(())
    }

    pub fn projection_receipt(projection: &MemoryProjection) -> anyhow::Result<String> {
        Ok(format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(projection)?)
        ))
    }

    pub fn attach_parent_projection(
        &self,
        context: &AgentMemoryContext,
        projection: &MemoryProjection,
    ) -> anyhow::Result<()> {
        let mut connection = self.connection.lock();
        self.verify_binding(&connection, context)?;
        anyhow::ensure!(
            Self::projection_receipt(projection)? == context.parent_projection_receipt,
            "parent projection does not match its trusted receipt"
        );
        let transaction = connection.transaction()?;
        for projected in &projection.records {
            projected.metadata.validate()?;
            transaction.execute(
                "INSERT OR IGNORE INTO agent_memory_projection(process_id,record_id,projected_json) VALUES(?1,?2,?3)",
                params![context.process_id.0.to_string(), projected.record_id.0, serde_json::to_string(projected)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Record a child experience in Task scope with Agent, task and process
    /// lineage embedded in immutable provenance/tags.
    pub fn record_child(
        &self,
        context: &AgentMemoryContext,
        draft: ChildMemoryDraft,
    ) -> anyhow::Result<MemoryRecord> {
        let connection = self.connection.lock();
        self.verify_binding(&connection, context)?;
        anyhow::ensure!(
            draft.authority != MemoryAuthority::ApprovedCore,
            "child Agent cannot create approved Core memory"
        );
        let mut hash = Sha256::new();
        hash.update(context.process_id.0.as_bytes());
        hash.update(context.agent_id.0.as_bytes());
        hash.update(context.task_id.0.as_bytes());
        hash.update(serde_json::to_vec(&draft)?);
        let id = format!("agent-memory:{:x}", hash.finalize());
        let now = Utc::now();
        let mut tags = draft.tags;
        tags.extend([
            format!("process:{}", context.process_id.0),
            format!("agent:{}", context.agent_id.0),
            format!("task:{}", context.task_id.0),
        ]);
        tags.sort();
        tags.dedup();
        let record = MemoryRecord {
            id: MemoryRecordId(id.clone()),
            kind: draft.kind,
            scope: context.task_scope.clone(),
            content: draft.content,
            metadata: MemoryMetadata {
                record_id: id,
                provenance: crate::MemoryProvenance {
                    source: "child-agent".into(),
                    source_id: context.process_id.0.to_string(),
                    principal: Some(context.agent_id.0.to_string()),
                    source_commit: None,
                },
                source_time: Some(now),
                observed_time: now,
                valid_from: Some(now),
                valid_until: None,
                supersedes: None,
                superseded_by: None,
                confidence: 1.0,
                sensitivity: MemorySensitivity::Internal,
            },
            status: MemoryStatus::Current,
            authority: draft.authority,
            source_event_ids: draft.source_event_ids,
            tags,
        };
        record.validate()?;
        connection.execute(
            "INSERT OR IGNORE INTO agent_memory_records(record_id,process_id,agent_id,task_id,record_json) VALUES(?1,?2,?3,?4,?5)",
            params![record.id.0, context.process_id.0.to_string(), context.agent_id.0.to_string(), context.task_id.0, serde_json::to_string(&record)?],
        )?;
        Ok(record)
    }

    pub fn recall(&self, context: &AgentMemoryContext) -> anyhow::Result<Vec<MemoryRecord>> {
        let connection = self.connection.lock();
        self.verify_binding(&connection, context)?;
        let mut statement = connection.prepare(
            "SELECT record_json FROM agent_memory_records WHERE process_id=?1 AND agent_id=?2 AND task_id=?3 ORDER BY record_id",
        )?;
        let rows = statement.query_map(
            params![
                context.process_id.0.to_string(),
                context.agent_id.0.to_string(),
                context.task_id.0
            ],
            |row| row.get::<_, String>(0),
        )?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn projected_for_child(
        &self,
        context: &AgentMemoryContext,
    ) -> anyhow::Result<Vec<ProjectedMemory>> {
        let connection = self.connection.lock();
        self.verify_binding(&connection, context)?;
        let mut statement = connection.prepare(
            "SELECT projected_json FROM agent_memory_projection WHERE process_id=?1 ORDER BY record_id",
        )?;
        let rows = statement.query_map([context.process_id.0.to_string()], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| {
            let json = row?;
            Ok(serde_json::from_str::<ProjectedMemory>(&json)?)
        })
        .collect()
    }

    pub fn get_record(&self, id: &MemoryRecordId) -> anyhow::Result<Option<MemoryRecord>> {
        let connection = self.connection.lock();
        let json: Option<String> = connection
            .query_row(
                "SELECT record_json FROM agent_memory_records WHERE record_id=?1",
                [&id.0],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn connection(&self) -> Arc<Mutex<Connection>> {
        self.connection.clone()
    }

    fn verify_binding(
        &self,
        connection: &Connection,
        context: &AgentMemoryContext,
    ) -> anyhow::Result<()> {
        context.validate()?;
        let binding: Option<(String, String, String)> = connection
            .query_row(
                "SELECT agent_id,task_id,parent_projection_receipt FROM agent_memory_bindings WHERE process_id=?1",
                [context.process_id.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        anyhow::ensure!(
            binding
                == Some((
                    context.agent_id.0.to_string(),
                    context.task_id.0.clone(),
                    context.parent_projection_receipt.clone(),
                )),
            "Agent memory process binding is missing or mismatched"
        );
        Ok(())
    }
}
