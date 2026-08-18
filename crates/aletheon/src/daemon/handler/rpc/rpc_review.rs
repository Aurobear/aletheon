use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::host::governed_review::{ReviewServiceError, ReviewStoreError};

use super::RequestHandler;

const INVALID_PARAMS: i64 = -32602;
const SERVICE_DISABLED: i64 = -32043;
const INCOMPATIBLE_SCHEMA: i64 = -32044;
const IDEMPOTENCY_CONFLICT: i64 = -32045;
const WAIT_TIMEOUT: i64 = -32046;
const NOT_FOUND: i64 = -32054;
const SERVICE_ERROR: i64 = -32055;
const MAX_WAIT_MS: u64 = 120_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitParams {
    job: application::governed_review::GovernedReviewJob,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobParams {
    job_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitParams {
    job_id: String,
    timeout_ms: u64,
}

impl RequestHandler {
    pub(super) async fn handle_transaction_settlement_latest(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let params = match serde_json::from_value::<::contracts::TransactionSettlementGetParams>(
            request["params"].clone(),
        ) {
            Ok(params)
                if !params.session_id.trim().is_empty()
                    && !params.transaction_id.trim().is_empty() =>
            {
                params
            }
            Ok(_) => {
                return rpc_error(
                    id,
                    INVALID_PARAMS,
                    "session_id and transaction_id are required",
                )
            }
            Err(error) => {
                return rpc_error(
                    id,
                    INVALID_PARAMS,
                    format!("invalid settlement query: {error}"),
                )
            }
        };
        let authority_key = application::thread_authority::ThreadAuthorityKey::new(
            connection.principal_id.clone(),
            ::contracts::ThreadId(params.session_id.clone()),
        );
        match self.thread_authority.get(&authority_key) {
            Ok(Some(_)) => {}
            Ok(None) => return rpc_error(id, NOT_FOUND, "session workspace authority not found"),
            Err(error) => return rpc_error(id, SERVICE_ERROR, error.to_string()),
        }
        match self
            .ports
            .transaction_review
            .latest(&params.session_id, &params.transaction_id)
            .await
        {
            Ok(Some(receipt)) => json!({"jsonrpc":"2.0", "id":id, "result":{"receipt":receipt}}),
            Ok(None) => rpc_error(id, NOT_FOUND, "transaction settlement not found"),
            Err(error) => rpc_error(id, SERVICE_ERROR, error.to_string()),
        }
    }

    pub(super) async fn handle_transaction_review(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let params = match serde_json::from_value::<::contracts::TransactionReviewParams>(
            request["params"].clone(),
        ) {
            Ok(params)
                if !params.session_id.trim().is_empty()
                    && !params.transaction_id.trim().is_empty() =>
            {
                params
            }
            Ok(_) => {
                return rpc_error(
                    id,
                    INVALID_PARAMS,
                    "session_id and transaction_id are required",
                )
            }
            Err(error) => {
                return rpc_error(
                    id,
                    INVALID_PARAMS,
                    format!("invalid transaction review request: {error}"),
                )
            }
        };
        let transaction_id = match uuid::Uuid::parse_str(&params.transaction_id) {
            Ok(id) => ::contracts::change_transaction::ChangeTransactionId(id),
            Err(error) => {
                return rpc_error(
                    id,
                    INVALID_PARAMS,
                    format!("invalid transaction_id: {error}"),
                )
            }
        };
        let authority_key = application::thread_authority::ThreadAuthorityKey::new(
            connection.principal_id.clone(),
            ::contracts::ThreadId(params.session_id.clone()),
        );
        let settings = match self.thread_authority.get(&authority_key) {
            Ok(Some(settings)) => settings,
            Ok(None) => return rpc_error(id, NOT_FOUND, "session workspace authority not found"),
            Err(error) => return rpc_error(id, SERVICE_ERROR, error.to_string()),
        };
        let action = match params.action {
            ::contracts::TransactionReviewAction::Accept => {
                application::settlement::TransactionReviewAction::Accept
            }
            ::contracts::TransactionReviewAction::Repair => {
                application::settlement::TransactionReviewAction::Repair
            }
            ::contracts::TransactionReviewAction::Rollback => {
                application::settlement::TransactionReviewAction::Rollback
            }
        };
        // Review findings come from the canonical daemon-owned Session/Task
        // projection. The client may choose an action, but it cannot omit,
        // resolve, or forge findings to influence Host settlement.
        let findings = match self
            .protocol_read_snapshot(&::contracts::SessionId(params.session_id.clone()))
            .await
        {
            Ok(snapshot) => snapshot
                .tasks
                .into_iter()
                .flat_map(|task| task.review_findings)
                .collect::<Vec<_>>(),
            Err(error) => return rpc_error(id, SERVICE_ERROR, error.to_string()),
        };
        match self
            .ports
            .transaction_review
            .review(
                action,
                transaction_id,
                &params.session_id,
                settings.workspace.cwd(),
                &findings,
                params.risk_acknowledged,
            )
            .await
        {
            Ok(outcome) => json!({"jsonrpc":"2.0", "id":id, "result":outcome}),
            Err(error) => rpc_error(id, SERVICE_ERROR, error.to_string()),
        }
    }

    pub(super) async fn handle_review_capabilities(&self, id: &Value) -> Value {
        let Some(service) = &self.ports.review else {
            return json!({
                "jsonrpc":"2.0", "id":id,
                "result": {"enabled":false, "protocol_version":1, "schema_versions":[2,1]}
            });
        };
        let capabilities = service.capabilities();
        json!({"jsonrpc":"2.0", "id":id, "result": {"enabled":true, "review":capabilities}})
    }

    pub(super) async fn handle_review_submit(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let Some(service) = &self.ports.review else {
            return rpc_error(id, SERVICE_DISABLED, "governed review service is disabled");
        };
        let params: SubmitParams = match serde_json::from_value(request["params"].clone()) {
            Ok(params) => params,
            Err(error) => {
                return rpc_error(id, INVALID_PARAMS, format!("invalid review job: {error}"))
            }
        };
        let principal = connection.principal_id.0.as_str();
        match service.submit(principal, params.job).await {
            Ok(stored) => json!({
                "jsonrpc":"2.0", "id":id,
                "result": {"status":stored.receipt.status, "job_id":stored.job.job_id,
                    "revision":stored.revision}
            }),
            Err(error) => service_error(id, error),
        }
    }

    pub(super) async fn handle_review_status(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let Some(service) = &self.ports.review else {
            return rpc_error(id, SERVICE_DISABLED, "governed review service is disabled");
        };
        let params = match parse_job_params(id, request) {
            Ok(params) => params,
            Err(response) => return response,
        };
        match service
            .status(connection.principal_id.0.as_str(), &params.job_id)
            .await
        {
            Ok(receipt) => json!({"jsonrpc":"2.0", "id":id, "result":{"receipt":receipt}}),
            Err(error) => service_error(id, error),
        }
    }

    pub(super) async fn handle_review_wait(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let Some(service) = &self.ports.review else {
            return rpc_error(id, SERVICE_DISABLED, "governed review service is disabled");
        };
        let params: WaitParams =
            match serde_json::from_value::<WaitParams>(request["params"].clone()) {
                Ok(params)
                    if !params.job_id.trim().is_empty()
                        && params.job_id.len()
                            <= application::governed_review::MAX_REVIEW_ID_BYTES
                        && (1..=MAX_WAIT_MS).contains(&params.timeout_ms) =>
                {
                    params
                }
                Ok(_) => {
                    return rpc_error(
                        id,
                        INVALID_PARAMS,
                        "job_id and timeout_ms are out of bounds",
                    )
                }
                Err(error) => {
                    return rpc_error(id, INVALID_PARAMS, format!("invalid wait params: {error}"))
                }
            };
        match service
            .wait(
                connection.principal_id.0.as_str(),
                &params.job_id,
                Duration::from_millis(params.timeout_ms),
            )
            .await
        {
            Ok(receipt) if receipt.status.is_terminal() => {
                json!({"jsonrpc":"2.0", "id":id, "result":{"receipt":receipt}})
            }
            Ok(_) => rpc_error(
                id,
                SERVICE_ERROR,
                "nonterminal receipt escaped terminal wait",
            ),
            Err(error) => service_error(id, error),
        }
    }

    pub(super) async fn handle_review_cancel(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
    ) -> Value {
        let Some(service) = &self.ports.review else {
            return rpc_error(id, SERVICE_DISABLED, "governed review service is disabled");
        };
        let params = match parse_job_params(id, request) {
            Ok(params) => params,
            Err(response) => return response,
        };
        match service
            .cancel(connection.principal_id.0.as_str(), &params.job_id)
            .await
        {
            Ok(receipt) => json!({"jsonrpc":"2.0", "id":id, "result":{"receipt":receipt}}),
            Err(error) => service_error(id, error),
        }
    }
}

fn parse_job_params(id: &Value, request: &Value) -> Result<JobParams, Value> {
    match serde_json::from_value::<JobParams>(request["params"].clone()) {
        Ok(params)
            if !params.job_id.trim().is_empty()
                && params.job_id.len() <= application::governed_review::MAX_REVIEW_ID_BYTES =>
        {
            Ok(params)
        }
        Ok(_) => Err(rpc_error(id, INVALID_PARAMS, "job_id is out of bounds")),
        Err(error) => Err(rpc_error(
            id,
            INVALID_PARAMS,
            format!("invalid job params: {error}"),
        )),
    }
}

fn service_error(id: &Value, error: ReviewServiceError) -> Value {
    match error {
        ReviewServiceError::Store(ReviewStoreError::InvalidContract(
            application::governed_review::ReviewContractError::UnsupportedSchema(_),
        )) => rpc_error(id, INCOMPATIBLE_SCHEMA, error.to_string()),
        ReviewServiceError::Store(ReviewStoreError::IdempotencyConflict) => {
            rpc_error(id, IDEMPOTENCY_CONFLICT, error.to_string())
        }
        ReviewServiceError::Store(ReviewStoreError::NotFound) => {
            rpc_error(id, NOT_FOUND, "review job not found")
        }
        ReviewServiceError::WaitTimeout => rpc_error(id, WAIT_TIMEOUT, error.to_string()),
        ReviewServiceError::Store(ReviewStoreError::InvalidContract(_))
        | ReviewServiceError::BudgetExceedsLimit(_) => {
            rpc_error(id, INVALID_PARAMS, error.to_string())
        }
        _ => rpc_error(id, SERVICE_ERROR, error.to_string()),
    }
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message.into()}})
}
