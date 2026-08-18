//! Principal- and session-scoped read-only evaluation RPCs.

use std::collections::HashSet;

use ::contracts::{EvaluationReceiptId, EvaluationReceiptRef, ItemPayload, PrincipalId, SessionId};
use serde_json::{json, Value};

use super::RequestHandler;

const INVALID_PARAMS: i64 = -32602;
const FORBIDDEN: i64 = -32047;
const NOT_FOUND: i64 = -32054;
const STORAGE_ERROR: i64 = -32055;
const MAX_LIST_LIMIT: usize = 100;
const DEFAULT_LIST_LIMIT: usize = 20;
const MAX_EVIDENCE_ITEMS: usize = 100;
const MAX_EVIDENCE_BYTES: usize = 256 * 1024;

impl RequestHandler {
    pub(super) async fn handle_evaluation_list(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let session_id = match self.authorized_evaluation_session(connection, id, request) {
            Ok(session_id) => session_id,
            Err(response) => return response,
        };
        let limit = request["params"]
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .clamp(1, MAX_LIST_LIMIT);
        let receipts = match self.session_evaluation_receipts(&session_id, limit).await {
            Ok(receipts) => receipts,
            Err(error) => return rpc_error(id, STORAGE_ERROR, error.to_string()),
        };
        let receipts = receipts
            .iter()
            .map(|receipt| scoped_summary(&connection.principal_id, &session_id, receipt))
            .collect::<Vec<_>>();
        json!({"jsonrpc":"2.0", "id":id, "result":{"receipts":receipts}})
    }

    pub(super) async fn handle_evaluation_latest(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let session_id = match self.authorized_evaluation_session(connection, id, request) {
            Ok(session_id) => session_id,
            Err(response) => return response,
        };
        let include_evidence = match parse_include_evidence(request) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, INVALID_PARAMS, message),
        };
        let receipt = match self.session_evaluation_receipts(&session_id, 1).await {
            Ok(mut receipts) => receipts.pop(),
            Err(error) => return rpc_error(id, STORAGE_ERROR, error.to_string()),
        };
        let Some(receipt) = receipt else {
            return json!({"jsonrpc":"2.0", "id":id, "result":{"receipt":Value::Null}});
        };
        self.evaluation_result(connection, id, &session_id, receipt, include_evidence)
            .await
    }

    pub(super) async fn handle_evaluation_get(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let session_id = match self.authorized_evaluation_session(connection, id, request) {
            Ok(session_id) => session_id,
            Err(response) => return response,
        };
        let receipt_id = match request["params"].get("receipt_id").and_then(Value::as_str) {
            Some(raw) => match uuid::Uuid::parse_str(raw) {
                Ok(id) => EvaluationReceiptId(id),
                Err(_) => return rpc_error(id, INVALID_PARAMS, "receipt_id must be a UUID string"),
            },
            None => return rpc_error(id, INVALID_PARAMS, "receipt_id is required"),
        };
        let include_evidence = match parse_include_evidence(request) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, INVALID_PARAMS, message),
        };
        let receipt = match self
            .session_evaluation_receipt(&session_id, receipt_id)
            .await
        {
            Ok(receipt) => receipt,
            Err(error) => return rpc_error(id, STORAGE_ERROR, error.to_string()),
        };
        let Some(receipt) = receipt else {
            // Do not distinguish a missing receipt from one owned by another
            // principal/session.
            return rpc_error(id, NOT_FOUND, "evaluation receipt not found");
        };
        self.evaluation_result(connection, id, &session_id, receipt, include_evidence)
            .await
    }

    fn authorized_evaluation_session(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Result<SessionId, Value> {
        let Some(session_id) = request["params"]
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Err(rpc_error(id, INVALID_PARAMS, "session_id is required"));
        };
        let session_id = SessionId(session_id.to_owned());
        match has_session_authority(
            self.thread_authority.as_ref(),
            &connection.principal_id,
            &session_id,
        ) {
            Ok(true) => Ok(session_id),
            Ok(false) => Err(rpc_error(
                id,
                FORBIDDEN,
                "session is not visible to authenticated principal",
            )),
            Err(error) => Err(rpc_error(id, STORAGE_ERROR, error.to_string())),
        }
    }

    async fn session_evaluation_receipts(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> anyhow::Result<Vec<EvaluationReceiptRef>> {
        let snapshot = self
            .ports
            .session_gateway
            .protocol_snapshot(session_id)
            .await?;
        Ok(bounded_receipt_refs(snapshot.items, limit))
    }

    async fn session_evaluation_receipt(
        &self,
        session_id: &SessionId,
        receipt_id: EvaluationReceiptId,
    ) -> anyhow::Result<Option<EvaluationReceiptRef>> {
        let snapshot = self
            .ports
            .session_gateway
            .protocol_snapshot(session_id)
            .await?;
        Ok(snapshot
            .items
            .into_iter()
            .rev()
            .find_map(|item| match item.payload {
                ItemPayload::EvaluationReceiptRef { receipt }
                    if receipt.receipt_id == receipt_id =>
                {
                    Some(receipt)
                }
                _ => None,
            }))
    }

    async fn evaluation_result(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        session_id: &SessionId,
        summary: EvaluationReceiptRef,
        include_evidence: bool,
    ) -> Value {
        let scoped = scoped_summary(&connection.principal_id, session_id, &summary);
        if !include_evidence {
            return json!({"jsonrpc":"2.0", "id":id, "result":{"receipt":scoped}});
        }

        let receipt = match self.ports.evaluation.receipt(&summary.receipt_id).await {
            Ok(Some(receipt)) if receipt.reference() == summary => receipt,
            Ok(Some(_)) => {
                return rpc_error(id, STORAGE_ERROR, "evaluation summary integrity mismatch")
            }
            Ok(None) => return rpc_error(id, NOT_FOUND, "evaluation receipt not found"),
            Err(error) => return rpc_error(id, STORAGE_ERROR, error.to_string()),
        };
        let snapshot = match self
            .ports
            .evaluation
            .evidence_snapshot_for_receipt(&summary.receipt_id)
            .await
        {
            Ok(Some(snapshot)) if snapshot.sha256 == receipt.evidence_snapshot_sha256 => snapshot,
            Ok(Some(_)) => {
                return rpc_error(id, STORAGE_ERROR, "evaluation evidence integrity mismatch")
            }
            Ok(None) => return rpc_error(id, NOT_FOUND, "evaluation evidence not found"),
            Err(error) => return rpc_error(id, STORAGE_ERROR, error.to_string()),
        };
        let (evidence, evidence_truncated) = bounded_evidence(snapshot.evidence);
        json!({
            "jsonrpc":"2.0",
            "id":id,
            "result":{
                "receipt":scoped,
                "detail":receipt,
                "evidence":evidence,
                "evidence_truncated":evidence_truncated,
            }
        })
    }
}

fn has_session_authority(
    store: &application::thread_authority::ThreadAuthorityStore,
    principal_id: &PrincipalId,
    session_id: &SessionId,
) -> Result<bool, application::thread_authority::ThreadAuthorityError> {
    let authority = application::thread_authority::ThreadAuthorityKey::new(
        principal_id.clone(),
        ::contracts::ThreadId(session_id.0.clone()),
    );
    store.get(&authority).map(|settings| settings.is_some())
}

fn parse_include_evidence(request: &Value) -> Result<bool, &'static str> {
    match request["params"].get("include_evidence") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err("include_evidence must be a boolean"),
    }
}

fn bounded_receipt_refs(
    items: Vec<::contracts::ItemRecord>,
    requested_limit: usize,
) -> Vec<EvaluationReceiptRef> {
    let limit = requested_limit.clamp(1, MAX_LIST_LIMIT);
    let mut seen = HashSet::new();
    items
        .into_iter()
        .rev()
        .filter_map(|item| match item.payload {
            ItemPayload::EvaluationReceiptRef { receipt } if seen.insert(receipt.receipt_id) => {
                Some(receipt)
            }
            _ => None,
        })
        .take(limit)
        .collect::<Vec<_>>()
}

fn scoped_summary(
    principal_id: &PrincipalId,
    session_id: &SessionId,
    receipt: &EvaluationReceiptRef,
) -> Value {
    let mut value = serde_json::to_value(receipt).expect("evaluation receipt ref serializes");
    let object = value
        .as_object_mut()
        .expect("evaluation receipt ref serializes as an object");
    object.insert("principal_id".into(), json!(principal_id.0));
    object.insert("session_id".into(), json!(session_id.0));
    value
}

fn bounded_evidence(
    evidence: Vec<::contracts::types::metacognition_evidence::EvidenceItem>,
) -> (
    Vec<::contracts::types::metacognition_evidence::EvidenceItem>,
    bool,
) {
    let total_items = evidence.len();
    // Account for the surrounding JSON array and per-item commas so the
    // serialized evidence list itself remains within the wire bound.
    let mut bytes = 2usize;
    let mut bounded = Vec::new();
    for item in evidence.into_iter().take(MAX_EVIDENCE_ITEMS) {
        let item_bytes =
            serde_json::to_vec(&item).map_or(MAX_EVIDENCE_BYTES + 1, |value| value.len());
        let separator = usize::from(!bounded.is_empty());
        if bytes.saturating_add(separator).saturating_add(item_bytes) > MAX_EVIDENCE_BYTES {
            break;
        }
        bytes += separator + item_bytes;
        bounded.push(item);
    }
    let truncated = bounded.len() < total_items;
    (bounded, truncated)
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::{
        EvaluationContractId, EvaluationDecision, EvaluationReceiptId, SessionId,
        EVALUATION_SCHEMA_V1,
    };

    fn receipt(index: u128) -> EvaluationReceiptRef {
        EvaluationReceiptRef {
            schema_version: EVALUATION_SCHEMA_V1,
            receipt_id: EvaluationReceiptId(uuid::Uuid::from_u128(index + 1)),
            contract_id: EvaluationContractId(uuid::Uuid::from_u128(index + 1_000)),
            subject_kind: "turn".into(),
            subject_id: format!("turn-{index}"),
            decision: EvaluationDecision::ObservedPass,
            weighted_total_millis: Some(80_000),
            evidence_coverage_millis: 900,
            confidence_millis: 850,
            failed_gates: Vec::new(),
            created_at_ms: index as i64,
        }
    }

    fn item(index: u128) -> ::contracts::ItemRecord {
        ::contracts::ItemRecord {
            schema_version: ::contracts::SESSION_SCHEMA_VERSION,
            id: ::contracts::ItemId(uuid::Uuid::from_u128(index + 10_000)),
            session_id: SessionId("session-a".into()),
            turn_id: ::contracts::TurnId(uuid::Uuid::from_u128(1)),
            sequence: index as u64,
            created_at_ms: index as u64,
            payload: ItemPayload::EvaluationReceiptRef {
                receipt: receipt(index),
            },
        }
    }

    #[test]
    fn evaluation_list_clamps_limit_and_returns_newest_first() {
        let items = (0..150).map(item).collect();
        let receipts = bounded_receipt_refs(items, 500);
        assert_eq!(receipts.len(), MAX_LIST_LIMIT);
        assert_eq!(receipts[0].subject_id, "turn-149");
        assert_eq!(receipts[99].subject_id, "turn-50");
    }

    #[test]
    fn evaluation_summary_is_scoped_to_authenticated_principal_and_session() {
        let summary = scoped_summary(
            &PrincipalId("p1".into()),
            &SessionId("session-a".into()),
            &receipt(1),
        );
        assert_eq!(summary["principal_id"], "p1");
        assert_eq!(summary["session_id"], "session-a");
        assert_eq!(summary["subject_id"], "turn-1");
    }

    #[test]
    fn evaluation_session_authority_is_principal_scoped() {
        use application::thread_authority::{
            ThreadAuthorityKey, ThreadAuthorityStore, ThreadSettings,
        };

        let store = ThreadAuthorityStore::in_memory();
        let session_id = SessionId("session-a".into());
        let principal = PrincipalId("p1".into());
        store
            .bind_or_verify(
                &ThreadAuthorityKey::new(
                    principal.clone(),
                    ::contracts::ThreadId(session_id.0.clone()),
                ),
                &ThreadSettings {
                    workspace: ::contracts::WorkspacePolicy::from_resolved_roots(
                        "/tmp/project".into(),
                        vec![],
                    )
                    .unwrap(),
                    permission_profile: ::contracts::PermissionProfileId::workspace_write(),
                    approval_policy: ::contracts::ApprovalPolicy::OnRequest,
                    model_policy: None,
                },
            )
            .unwrap();

        assert!(has_session_authority(&store, &principal, &session_id).unwrap());
        assert!(!has_session_authority(&store, &PrincipalId("p2".into()), &session_id).unwrap());
    }

    #[test]
    fn evidence_query_enforces_item_and_byte_bounds() {
        use ::contracts::types::metacognition_evidence::{
            EvidenceId, EvidenceItem, EvidenceKind, EvidenceTrust,
        };
        use ::contracts::types::metacognition_experience::{ExperienceId, METACOGNITION_SCHEMA_V1};
        use sha2::{Digest, Sha256};

        let evidence = (0..150)
            .map(|index| {
                let payload = json!({"index": index, "content": "x".repeat(4_000)});
                let sha256 = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&payload).unwrap())
                );
                EvidenceItem {
                    schema_version: METACOGNITION_SCHEMA_V1,
                    evidence_id: EvidenceId(format!("e-{index:03}")),
                    experience_id: ExperienceId("experience".into()),
                    kind: EvidenceKind::Observation,
                    source: "test".into(),
                    producer: "test".into(),
                    captured_at_ms: 1,
                    payload,
                    sha256,
                    trust: EvidenceTrust::Authoritative,
                    freshness_ms: None,
                    redacted: false,
                }
            })
            .collect();
        let (bounded, truncated) = bounded_evidence(evidence);
        assert!(truncated);
        assert!(bounded.len() <= MAX_EVIDENCE_ITEMS);
        assert!(serde_json::to_vec(&bounded).unwrap().len() <= MAX_EVIDENCE_BYTES);
    }
}
