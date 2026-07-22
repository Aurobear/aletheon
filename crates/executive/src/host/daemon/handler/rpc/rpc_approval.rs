//! Durable approval JSON-RPC handlers.

use super::RequestHandler;
use crate::application::approval_service::{
    ApprovalContext, ApprovalServiceError, ResolveApprovalRequest,
};
use crate::r#impl::approval::ApprovalDecision;
use fabric::ApprovalId;
use serde_json::{json, Value};

const INVALID_PARAMS: i64 = -32602;
const APPROVAL_NOT_FOUND: i64 = -32041;
const APPROVAL_FORBIDDEN: i64 = -32043;
const APPROVAL_CONFLICT: i64 = -32049;
const APPROVAL_STORAGE: i64 = -32040;

impl RequestHandler {
    fn authenticated_approval_context(
        connection: &super::super::super::server::ConnectionContext,
    ) -> ApprovalContext {
        ApprovalContext {
            principal_id: connection.principal_id.clone(),
            channel: "local_rpc".into(),
        }
    }

    pub(super) async fn handle_approval_list(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        _request: &Value,
    ) -> Value {
        let context = Self::authenticated_approval_context(connection);
        match self.ports.approvals.list(context).await {
            Ok(approvals) => json!({"jsonrpc":"2.0", "id":id, "result":{"approvals":approvals}}),
            Err(error) => approval_error(id, error),
        }
    }

    pub(super) async fn handle_approval_show(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let context = Self::authenticated_approval_context(connection);
        let approval_id = match parse_id(request) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, INVALID_PARAMS, message),
        };
        match self.ports.approvals.show(context, approval_id).await {
            Ok(approval) => json!({"jsonrpc":"2.0", "id":id, "result":{"approval":approval}}),
            Err(error) => approval_error(id, error),
        }
    }

    pub(super) async fn handle_approval_approve(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        self.handle_durable_resolution(connection, id, request, ApprovalDecision::Approve)
            .await
    }

    pub(super) async fn handle_approval_reject(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let reason = request["params"]["reason"].as_str().map(str::to_owned);
        self.handle_durable_resolution(connection, id, request, ApprovalDecision::Reject { reason })
            .await
    }

    async fn handle_durable_resolution(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
        decision: ApprovalDecision,
    ) -> Value {
        let context = Self::authenticated_approval_context(connection);
        let approval_id = match parse_id(request) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, INVALID_PARAMS, message),
        };
        let version = match request["params"]["version"].as_u64() {
            Some(value) => value,
            None => return rpc_error(id, INVALID_PARAMS, "version must be an unsigned integer"),
        };
        match self
            .ports
            .approvals
            .resolve(ResolveApprovalRequest {
                context,
                approval_id,
                version,
                decision,
            })
            .await
        {
            Ok(approval) => json!({"jsonrpc":"2.0", "id":id, "result":{"approval":approval}}),
            Err(error) => approval_error(id, error),
        }
    }
}

fn parse_id(request: &Value) -> Result<ApprovalId, &'static str> {
    let raw = request["params"]["approval_id"]
        .as_str()
        .ok_or("approval_id must be a UUID string")?;
    uuid::Uuid::parse_str(raw)
        .map(ApprovalId)
        .map_err(|_| "approval_id must be a UUID string")
}

fn approval_error(id: &Value, error: ApprovalServiceError) -> Value {
    let code = match error {
        ApprovalServiceError::NotFound => APPROVAL_NOT_FOUND,
        ApprovalServiceError::Forbidden(_) => APPROVAL_FORBIDDEN,
        ApprovalServiceError::Conflict(_) => APPROVAL_CONFLICT,
        ApprovalServiceError::RuntimeUnavailable(_) | ApprovalServiceError::Store(_) => {
            APPROVAL_STORAGE
        }
    };
    rpc_error(id, code, error.to_string())
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message.into()}})
}
