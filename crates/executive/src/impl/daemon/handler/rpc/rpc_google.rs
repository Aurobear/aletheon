//! Authenticated local Google account control-plane RPC.

use super::RequestHandler;
use crate::service::request_use_cases::GoogleUseCaseError;
use serde_json::{json, Value};

const GOOGLE_INVALID_PARAMS: i64 = -32602;

impl RequestHandler {
    pub(super) async fn handle_google_authorization_start(
        &self,
        id: &Value,
        _request: &Value,
    ) -> Value {
        match self.ports.google.authorization_start().await {
            Ok(result) => rpc_result(id, result),
            Err(error) => google_error(id, error),
        }
    }

    pub(super) async fn handle_google_authorization_callback(
        &self,
        id: &Value,
        request: &Value,
    ) -> Value {
        let params = &request["params"];
        let Some(code) = bounded_string(params, "code", 16 * 1024) else {
            return rpc_error(id, GOOGLE_INVALID_PARAMS, "invalid_google_code");
        };
        let Some(state) = bounded_string(params, "state", 1024) else {
            return rpc_error(id, GOOGLE_INVALID_PARAMS, "invalid_google_state");
        };
        let alias = match params.get("alias") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if !value.is_empty() && value.len() <= 128 => {
                Some(value.clone())
            }
            _ => return rpc_error(id, GOOGLE_INVALID_PARAMS, "invalid_google_alias"),
        };
        match self
            .ports
            .google
            .authorization_callback(code.into(), state.into(), alias)
            .await
        {
            Ok(account) => rpc_result(id, json!({"account":account})),
            Err(error) => google_error(id, error),
        }
    }

    pub(super) async fn handle_google_accounts_list(&self, id: &Value, _request: &Value) -> Value {
        match self.ports.google.accounts().await {
            Ok(accounts) => rpc_result(id, json!({"accounts":accounts})),
            Err(error) => google_error(id, error),
        }
    }

    pub(super) async fn handle_google_account_revoke(&self, id: &Value, request: &Value) -> Value {
        let Some(account) = bounded_string(&request["params"], "account", 128) else {
            return rpc_error(id, GOOGLE_INVALID_PARAMS, "invalid_google_account");
        };
        match self.ports.google.revoke(account.into()).await {
            Ok((_, provider_revoked)) => rpc_result(
                id,
                json!({"status":"revoked","provider_revoked":provider_revoked}),
            ),
            Err(error) => google_error(id, error),
        }
    }

    pub(super) async fn handle_google_token_refresh(&self, id: &Value, request: &Value) -> Value {
        let Some(account) = bounded_string(&request["params"], "account", 128) else {
            return rpc_error(id, GOOGLE_INVALID_PARAMS, "invalid_google_account");
        };
        match self.ports.google.refresh(account.into()).await {
            Ok(refresh) => rpc_result(id, json!(refresh)),
            Err(error) => google_error(id, error),
        }
    }
}

fn google_error(id: &Value, error: GoogleUseCaseError) -> Value {
    match error {
        GoogleUseCaseError::Unavailable => rpc_error(id, -32070, "google_not_configured"),
        GoogleUseCaseError::NotFound => rpc_error(id, -32073, "google_account_not_found"),
        GoogleUseCaseError::Forbidden => {
            rpc_error(id, -32073, "google_account_revoked_or_scope_denied")
        }
        GoogleUseCaseError::Provider => rpc_error(id, -32074, "google_provider_operation_failed"),
    }
}

fn bounded_string<'a>(params: &'a Value, field: &str, max: usize) -> Option<&'a str> {
    params
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= max)
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

#[cfg(test)]
mod tests {
    use fabric::{PrincipalId, LOCAL_OWNER_PRINCIPAL};

    #[test]
    fn local_google_principal_is_stable_across_sessions() {
        let first = PrincipalId(LOCAL_OWNER_PRINCIPAL.into());
        let second = PrincipalId(LOCAL_OWNER_PRINCIPAL.into());
        assert_eq!(first, second);
        assert_eq!(first.0, "local-owner");
    }
}
