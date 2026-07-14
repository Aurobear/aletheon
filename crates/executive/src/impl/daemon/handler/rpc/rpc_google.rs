//! Authenticated local Google account control-plane RPC.

use super::RequestHandler;
use fabric::{ExternalIdentityState, ExternalScope, GrantState, PrincipalId};
use serde_json::{json, Value};

const GOOGLE_UNAVAILABLE: i64 = -32070;
const GOOGLE_INVALID_PARAMS: i64 = -32602;
const GOOGLE_FORBIDDEN: i64 = -32073;
const GOOGLE_PROVIDER: i64 = -32074;

impl RequestHandler {
    async fn authenticated_google_principal(&self) -> anyhow::Result<PrincipalId> {
        // The local session gateway selects the principal. Request parameters
        // are deliberately ignored as an authority source.
        Ok(PrincipalId(self.get_or_create_session(None).await?.0))
    }

    pub(super) async fn handle_google_authorization_start(
        &self,
        id: &Value,
        _request: &Value,
    ) -> Value {
        let (google, principal) = match self.google_context(id).await {
            Ok(value) => value,
            Err(response) => return response,
        };
        match google.start_authorization(&principal).await {
            Ok(start) => json!({"jsonrpc":"2.0","id":id,"result":{
                "authorization_url":start.url,
                "state":start.state,
                "expires_at_secs":start.expires_at_secs
            }}),
            Err(_) => rpc_error(id, GOOGLE_PROVIDER, "google_authorization_start_failed"),
        }
    }

    pub(super) async fn handle_google_authorization_callback(
        &self,
        id: &Value,
        request: &Value,
    ) -> Value {
        let (google, principal) = match self.google_context(id).await {
            Ok(value) => value,
            Err(response) => return response,
        };
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
        let now_ms = self.subsystems.ports.clock.wall_now().0;
        match google
            .complete_authorization(&principal, code, state, alias, now_ms)
            .await
        {
            Ok((identity, grant)) => {
                if let Err(error) = self.register_new_google_read_tools(&google).await {
                    tracing::warn!(error = %error, "Google account bound but tool registration failed");
                }
                json!({"jsonrpc":"2.0","id":id,"result":{
                    "account":safe_account(&identity, &grant)
                }})
            }
            Err(_) => rpc_error(id, GOOGLE_PROVIDER, "google_authorization_callback_failed"),
        }
    }

    pub(super) async fn handle_google_accounts_list(&self, id: &Value, _request: &Value) -> Value {
        let (google, principal) = match self.google_context(id).await {
            Ok(value) => value,
            Err(response) => return response,
        };
        let repository = google.repository();
        let result = repository.lock().unwrap().list(&principal);
        match result {
            Ok(bindings) => json!({"jsonrpc":"2.0","id":id,"result":{
                "accounts":bindings.iter().map(|(identity, grant)| safe_account(identity, grant)).collect::<Vec<_>>()
            }}),
            Err(_) => rpc_error(id, GOOGLE_PROVIDER, "google_account_list_failed"),
        }
    }

    pub(super) async fn handle_google_account_revoke(&self, id: &Value, request: &Value) -> Value {
        let (google, principal) = match self.google_context(id).await {
            Ok(value) => value,
            Err(response) => return response,
        };
        let Some(reference) = bounded_string(&request["params"], "account", 128) else {
            return rpc_error(id, GOOGLE_INVALID_PARAMS, "invalid_google_account");
        };
        let repository = google.repository();
        let identity = {
            let repository = repository.lock().unwrap();
            match repository.resolve_account(&principal, reference) {
                Ok(Some(account)) => match repository.get(&principal, account) {
                    Ok(Some((identity, _))) => identity,
                    _ => return rpc_error(id, GOOGLE_FORBIDDEN, "google_account_not_found"),
                },
                _ => return rpc_error(id, GOOGLE_FORBIDDEN, "google_account_not_found"),
            }
        };
        let now_ms = self.subsystems.ports.clock.wall_now().0;
        if repository
            .lock()
            .unwrap()
            .revoke_local(&principal, identity.id, identity.version, now_ms)
            .is_err()
        {
            return rpc_error(id, GOOGLE_PROVIDER, "google_account_revoke_failed");
        }
        let provider_revoked = google
            .oauth()
            .lock()
            .await
            .revoke(identity.id)
            .await
            .is_ok();
        json!({"jsonrpc":"2.0","id":id,"result":{
            "status":"revoked",
            "provider_revoked":provider_revoked
        }})
    }

    pub(super) async fn handle_google_token_refresh(&self, id: &Value, request: &Value) -> Value {
        let (google, principal) = match self.google_context(id).await {
            Ok(value) => value,
            Err(response) => return response,
        };
        let Some(reference) = bounded_string(&request["params"], "account", 128) else {
            return rpc_error(id, GOOGLE_INVALID_PARAMS, "invalid_google_account");
        };
        let account = {
            let repository = google.repository();
            let repository = repository.lock().unwrap();
            let Ok(Some(account)) = repository.resolve_account(&principal, reference) else {
                return rpc_error(id, GOOGLE_FORBIDDEN, "google_account_not_found");
            };
            let Ok(Some((identity, grant))) = repository.get(&principal, account) else {
                return rpc_error(id, GOOGLE_FORBIDDEN, "google_account_not_found");
            };
            let active_read = identity.state == ExternalIdentityState::Active
                && grant.state == GrantState::Active
                && grant.scopes.iter().any(|scope| {
                    matches!(
                        scope,
                        ExternalScope::GmailReadonly | ExternalScope::CalendarReadonly
                    )
                })
                && grant.scopes.iter().all(|scope| !scope.is_write());
            if !active_read {
                return rpc_error(
                    id,
                    GOOGLE_FORBIDDEN,
                    "google_account_revoked_or_scope_denied",
                );
            }
            account
        };
        match google.refresh_singleflight(account).await {
            Ok(_) => json!({"jsonrpc":"2.0","id":id,"result":{"status":"success"}}),
            Err(corpus::tools::google::GoogleApiError::ReauthorizationRequired) => {
                json!({"jsonrpc":"2.0","id":id,"result":{"status":"reauthorization_required"}})
            }
            Err(error) => {
                json!({"jsonrpc":"2.0","id":id,"result":{"status":"error","code":error.to_string()}})
            }
        }
    }

    async fn google_context(
        &self,
        id: &Value,
    ) -> Result<
        (
            std::sync::Arc<crate::r#impl::external::GoogleIntegration>,
            PrincipalId,
        ),
        Value,
    > {
        let Some(google) = self.google.clone() else {
            return Err(rpc_error(id, GOOGLE_UNAVAILABLE, "google_not_configured"));
        };
        let principal = self
            .authenticated_google_principal()
            .await
            .map_err(|_| rpc_error(id, GOOGLE_FORBIDDEN, "google_authentication_failed"))?;
        Ok((google, principal))
    }

    async fn register_new_google_read_tools(
        &self,
        google: &std::sync::Arc<crate::r#impl::external::GoogleIntegration>,
    ) -> anyhow::Result<()> {
        use corpus::tools::google::{
            GoogleApiClient, GoogleApiEndpoints, GoogleCalendarAdapter, GoogleGmailAdapter,
        };
        use fabric::Registry;

        let repository = google.repository();
        let (gmail_enabled, calendar_enabled) = {
            let repository = repository.lock().unwrap();
            (
                repository.has_active_scope(ExternalScope::GmailReadonly)?,
                repository.has_active_scope(ExternalScope::CalendarReadonly)?,
            )
        };
        let credentials = std::sync::Arc::new(
            crate::r#impl::external::ExecutiveGoogleCredentialSource::new(
                repository.clone(),
                google.oauth(),
            ),
        );
        let accounts = std::sync::Arc::new(
            crate::r#impl::external::ExecutiveGoogleAccountResolver::new(repository),
        );
        let client = GoogleApiClient::new(credentials, GoogleApiEndpoints::default())?;
        let mut tools = self.subsystems.corpus.tools.lock().await;
        if gmail_enabled && tools.get("google_gmail_search").is_none() {
            let gmail = std::sync::Arc::new(GoogleGmailAdapter::new(client.clone()));
            tools.register(std::sync::Arc::new(
                corpus::tools::google::GoogleGmailSearchTool::new(gmail.clone(), accounts.clone()),
            ))?;
            tools.register(std::sync::Arc::new(
                corpus::tools::google::GoogleGmailReadTool::new(gmail, accounts.clone()),
            ))?;
        }
        if calendar_enabled && tools.get("google_calendar_list").is_none() {
            tools.register(std::sync::Arc::new(
                corpus::tools::google::GoogleCalendarListTool::new(
                    std::sync::Arc::new(GoogleCalendarAdapter::new(client)),
                    accounts,
                ),
            ))?;
        }
        Ok(())
    }
}

fn safe_account(identity: &fabric::ExternalIdentity, grant: &fabric::CapabilityGrant) -> Value {
    json!({
        "id":identity.id,
        "email":identity.email,
        "alias":identity.alias,
        "state":identity.state,
        "scopes":grant.scopes,
        "grant_state":grant.state,
        "version":identity.version
    })
}

fn bounded_string<'a>(params: &'a Value, field: &str, max: usize) -> Option<&'a str> {
    params
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= max)
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
