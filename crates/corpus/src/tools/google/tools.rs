//! Principal-aware native Google tools.

use super::{CalendarCapability, GmailCapability, GoogleApiError};
use async_trait::async_trait;
use fabric::tool::{
    ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta,
};
use fabric::{
    CalendarQuery, ExternalIdentityId, MailQuery, OpaqueCursor, PrincipalId, LOCAL_OWNER_PRINCIPAL,
};
use serde::Deserialize;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait GoogleAccountResolver: Send + Sync {
    async fn resolve_account(
        &self,
        principal: &PrincipalId,
        account_reference: &str,
    ) -> Result<ExternalIdentityId, GoogleApiError>;
}

#[derive(Clone)]
pub struct GoogleGmailSearchTool {
    gmail: Arc<dyn GmailCapability>,
    accounts: Arc<dyn GoogleAccountResolver>,
}

impl GoogleGmailSearchTool {
    pub fn new(gmail: Arc<dyn GmailCapability>, accounts: Arc<dyn GoogleAccountResolver>) -> Self {
        Self { gmail, accounts }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GmailSearchInput {
    account: String,
    query: String,
    #[serde(default = "default_page_size")]
    page_size: u16,
    page_token: Option<String>,
}

#[async_trait]
impl Tool for GoogleGmailSearchTool {
    fn name(&self) -> &str {
        "google_gmail_search"
    }
    fn description(&self) -> &str {
        "Search bounded Gmail metadata in an explicitly selected bound account"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type":"object",
            "properties":{
                "account":{"type":"string","minLength":1,"maxLength":128,"description":"Bound account alias or ID"},
                "query":{"type":"string","minLength":1,"maxLength":1024},
                "page_size":{"type":"integer","minimum":1,"maximum":100,"default":20},
                "page_token":{"type":"string","minLength":1,"maxLength":1024}
            },
            "required":["account","query"],
            "additionalProperties":false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let started = ctx.clock.mono_now().0;
        let parsed: GmailSearchInput = match serde_json::from_value(input) {
            Ok(value) => value,
            Err(_) => return error_result(GoogleApiError::InvalidRequest, started, ctx),
        };
        let principal = trusted_principal(ctx);
        let account = match self
            .accounts
            .resolve_account(&principal, &parsed.account)
            .await
        {
            Ok(account) => account,
            Err(error) => return error_result(error, started, ctx),
        };
        let page_token = match parsed.page_token.map(OpaqueCursor::new).transpose() {
            Ok(token) => token,
            Err(_) => return error_result(GoogleApiError::InvalidRequest, started, ctx),
        };
        let result = self
            .gmail
            .search_messages(
                &principal,
                MailQuery {
                    account_id: account,
                    query: parsed.query,
                    page_size: parsed.page_size,
                    page_token,
                },
                &CancellationToken::new(),
            )
            .await;
        result_to_tool(result, started, ctx)
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct GoogleGmailReadTool {
    gmail: Arc<dyn GmailCapability>,
    accounts: Arc<dyn GoogleAccountResolver>,
}

impl GoogleGmailReadTool {
    pub fn new(gmail: Arc<dyn GmailCapability>, accounts: Arc<dyn GoogleAccountResolver>) -> Self {
        Self { gmail, accounts }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GmailReadInput {
    account: String,
    message_id: String,
}

#[async_trait]
impl Tool for GoogleGmailReadTool {
    fn name(&self) -> &str {
        "google_gmail_read"
    }
    fn description(&self) -> &str {
        "Read one bounded Gmail message from an explicitly selected bound account"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type":"object",
            "properties":{
                "account":{"type":"string","minLength":1,"maxLength":128,"description":"Bound account alias or ID"},
                "message_id":{"type":"string","minLength":1,"maxLength":1024,"pattern":"^[A-Za-z0-9_-]+$"}
            },
            "required":["account","message_id"],
            "additionalProperties":false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let started = ctx.clock.mono_now().0;
        let parsed: GmailReadInput = match serde_json::from_value(input) {
            Ok(value) => value,
            Err(_) => return error_result(GoogleApiError::InvalidRequest, started, ctx),
        };
        let principal = trusted_principal(ctx);
        let account = match self
            .accounts
            .resolve_account(&principal, &parsed.account)
            .await
        {
            Ok(account) => account,
            Err(error) => return error_result(error, started, ctx),
        };
        let result = self
            .gmail
            .read_message(
                &principal,
                account,
                &parsed.message_id,
                &CancellationToken::new(),
            )
            .await;
        result_to_tool(result, started, ctx)
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct GoogleCalendarListTool {
    calendar: Arc<dyn CalendarCapability>,
    accounts: Arc<dyn GoogleAccountResolver>,
}

impl GoogleCalendarListTool {
    pub fn new(
        calendar: Arc<dyn CalendarCapability>,
        accounts: Arc<dyn GoogleAccountResolver>,
    ) -> Self {
        Self { calendar, accounts }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CalendarListInput {
    account: String,
    start_ms: i64,
    end_ms: i64,
    timezone: String,
    #[serde(default = "default_page_size")]
    page_size: u16,
    page_token: Option<String>,
}

#[async_trait]
impl Tool for GoogleCalendarListTool {
    fn name(&self) -> &str {
        "google_calendar_list"
    }
    fn description(&self) -> &str {
        "List bounded Calendar events in an explicitly selected bound account"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type":"object",
            "properties":{
                "account":{"type":"string","minLength":1,"maxLength":128,"description":"Bound account alias or ID"},
                "start_ms":{"type":"integer","minimum":0},
                "end_ms":{"type":"integer","minimum":1},
                "timezone":{"type":"string","minLength":1,"maxLength":128},
                "page_size":{"type":"integer","minimum":1,"maximum":100,"default":20},
                "page_token":{"type":"string","minLength":1,"maxLength":1024}
            },
            "required":["account","start_ms","end_ms","timezone"],
            "additionalProperties":false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let started = ctx.clock.mono_now().0;
        let parsed: CalendarListInput = match serde_json::from_value(input) {
            Ok(value) => value,
            Err(_) => return error_result(GoogleApiError::InvalidRequest, started, ctx),
        };
        let principal = trusted_principal(ctx);
        let account = match self
            .accounts
            .resolve_account(&principal, &parsed.account)
            .await
        {
            Ok(account) => account,
            Err(error) => return error_result(error, started, ctx),
        };
        let page_token = match parsed.page_token.map(OpaqueCursor::new).transpose() {
            Ok(token) => token,
            Err(_) => return error_result(GoogleApiError::InvalidRequest, started, ctx),
        };
        let result = self
            .calendar
            .list_events(
                &principal,
                CalendarQuery {
                    account_id: account,
                    start_ms: parsed.start_ms,
                    end_ms: parsed.end_ms,
                    timezone: parsed.timezone,
                    page_size: parsed.page_size,
                    page_token,
                },
                &CancellationToken::new(),
            )
            .await;
        result_to_tool(result, started, ctx)
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }
}

fn trusted_principal(_ctx: &ToolContext) -> PrincipalId {
    // The native daemon's local socket is the credential-checked authority.
    // Session UUIDs rotate on restart and therefore cannot own durable Google
    // bindings. No tool schema accepts a principal field, so model input still
    // cannot select authority.
    PrincipalId(LOCAL_OWNER_PRINCIPAL.into())
}

fn default_page_size() -> u16 {
    20
}

fn result_to_tool<T: serde::Serialize>(
    result: Result<T, GoogleApiError>,
    started: u64,
    ctx: &ToolContext,
) -> ToolResult {
    match result {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(content) => ToolResult {
                content,
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(started),
                    truncated: false,
                    patch_delta: None,
                },
            },
            Err(_) => error_result(GoogleApiError::MalformedResponse, started, ctx),
        },
        Err(error) => error_result(error, started, ctx),
    }
}

fn error_result(error: GoogleApiError, started: u64, ctx: &ToolContext) -> ToolResult {
    ToolResult {
        content: error.to_string(),
        is_error: true,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(started),
            truncated: false,
            patch_delta: None,
        },
    }
}
