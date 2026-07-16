//! Durable, reviewed promotion of child-scoped memory.

use fabric::{BroadcastEpoch, ContentId, PrincipalId};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AgentMemoryContext, AgentMemoryVault, MemoryAuthority, MemoryProvenance, MemoryRecord,
    MemoryRecordId, MemoryScope,
};

const PROMOTION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_memory_promotions(
 request_hash TEXT PRIMARY KEY, source_record TEXT NOT NULL, target_scope TEXT NOT NULL,
 request_json TEXT NOT NULL, receipt_json TEXT NOT NULL,
 UNIQUE(source_record,target_scope));
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionDecision {
    Promoted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPromotionRequest {
    pub source_record: MemoryRecordId,
    pub child: AgentMemoryContext,
    pub root_content: ContentId,
    pub broadcast: BroadcastEpoch,
    pub selected_candidate: ContentId,
    pub selection_receipt: String,
    pub reviewer: PrincipalId,
    pub review_receipt: String,
    pub target_scope: MemoryScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPromotionReceipt {
    pub request_hash: String,
    pub resulting_record: MemoryRecordId,
    pub resulting_version: u64,
    pub decision: PromotionDecision,
}

impl AgentMemoryVault {
    pub fn promote(
        &self,
        request: &MemoryPromotionRequest,
    ) -> anyhow::Result<MemoryPromotionReceipt> {
        request.child.validate()?;
        anyhow::ensure!(request.broadcast.0 > 0, "root broadcast receipt is missing");
        anyhow::ensure!(
            !request.selection_receipt.trim().is_empty(),
            "root selection receipt is missing"
        );
        anyhow::ensure!(
            !request.reviewer.0.trim().is_empty() && !request.review_receipt.trim().is_empty(),
            "parent/consolidator review receipt is missing"
        );
        anyhow::ensure!(
            matches!(
                request.target_scope,
                MemoryScope::Session(_) | MemoryScope::Global
            ),
            "child memory may only be promoted to Session or Global scope"
        );
        request.target_scope.validate()?;
        let request_hash = format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(request)?));
        let connection = self.connection();
        let mut connection = connection.lock();
        connection.execute_batch(PROMOTION_SCHEMA)?;
        if let Some(json) = connection
            .query_row(
                "SELECT receipt_json FROM agent_memory_promotions WHERE request_hash=?1",
                [&request_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(serde_json::from_str(&json)?);
        }
        let source_json: Option<String> = connection
            .query_row(
                "SELECT record_json FROM agent_memory_records WHERE record_id=?1 AND process_id=?2 AND agent_id=?3 AND task_id=?4",
                params![request.source_record.0, request.child.process_id.0.to_string(), request.child.agent_id.0.to_string(), request.child.task_id.0],
                |row| row.get(0),
            )
            .optional()?;
        let source: MemoryRecord = serde_json::from_str(
            &source_json
                .ok_or_else(|| anyhow::anyhow!("child source record provenance mismatch"))?,
        )?;
        anyhow::ensure!(
            source.authority != MemoryAuthority::ApprovedCore,
            "Core memory cannot be promoted by an Agent"
        );
        let resulting_id = MemoryRecordId(format!(
            "promoted:{:x}",
            Sha256::digest(request_hash.as_bytes())
        ));
        let mut promoted = source.clone();
        promoted.id = resulting_id.clone();
        promoted.scope = request.target_scope.clone();
        promoted.metadata.record_id = resulting_id.0.clone();
        promoted.metadata.provenance = MemoryProvenance {
            source: "reviewed-agent-promotion".into(),
            source_id: request_hash.clone(),
            principal: Some(request.reviewer.0.clone()),
            source_commit: Some(source.id.0.clone()),
        };
        promoted.source_event_ids.extend([
            format!("child-process:{}", request.child.process_id.0),
            format!("child-agent:{}", request.child.agent_id.0),
            format!("child-task:{}", request.child.task_id.0),
            format!("root-content:{}", request.root_content.0),
            format!("root-broadcast:{}", request.broadcast.0),
            format!("selected-candidate:{}", request.selected_candidate.0),
            format!("selection-receipt:{}", request.selection_receipt),
            format!("review-receipt:{}", request.review_receipt),
        ]);
        promoted.source_event_ids.sort();
        promoted.source_event_ids.dedup();
        promoted.validate()?;
        let receipt = MemoryPromotionReceipt {
            request_hash: request_hash.clone(),
            resulting_record: resulting_id,
            resulting_version: 1,
            decision: PromotionDecision::Promoted,
        };
        let transaction = connection.transaction()?;
        let inserted = transaction.execute(
            "INSERT INTO agent_memory_promotions(request_hash,source_record,target_scope,request_json,receipt_json) VALUES(?1,?2,?3,?4,?5)",
            params![request_hash, request.source_record.0, serde_json::to_string(&request.target_scope)?, serde_json::to_string(request)?, serde_json::to_string(&receipt)?],
        );
        if let Err(error) = inserted {
            if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
                anyhow::bail!("conflicting promotion for source record and target scope");
            }
            return Err(error.into());
        }
        transaction.execute(
            "INSERT INTO agent_memory_records(record_id,process_id,agent_id,task_id,record_json) VALUES(?1,?2,?3,?4,?5)",
            params![promoted.id.0, request.child.process_id.0.to_string(), request.child.agent_id.0.to_string(), request.child.task_id.0, serde_json::to_string(&promoted)?],
        )?;
        transaction.commit()?;
        Ok(receipt)
    }
}
