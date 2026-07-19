use aletheon_kernel::chronos::TestClock;
use async_trait::async_trait;
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::SandboxPreference;
use corpus::tools::google::{
    CalendarCapability, GmailCapability, GoogleAccountResolver, GoogleApiError,
};
use corpus::tools::tools::ToolRegistry;
use fabric::tool::ToolContext;
use fabric::{
    CalendarEventPage, CalendarTimeRange, ExternalIdentityId, GmailMessage, GmailMessagePage,
    GmailMessageSummary, GmailQuery, PrincipalId, ProviderRecordRef, LOCAL_OWNER_PRINCIPAL,
};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

const TOKEN_SENTINEL: &str = "ya29.must-never-leak";

struct Accounts {
    owner: PrincipalId,
    account: ExternalIdentityId,
    revoked: bool,
}

#[async_trait]
impl GoogleAccountResolver for Accounts {
    async fn resolve_account(
        &self,
        principal: &PrincipalId,
        account_reference: &str,
    ) -> Result<ExternalIdentityId, GoogleApiError> {
        if !self.revoked
            && principal == &self.owner
            && (account_reference == "work" || account_reference == self.account.to_string())
        {
            Ok(self.account)
        } else {
            Err(GoogleApiError::UnauthorizedAccount)
        }
    }
}

struct Gmail {
    seen_principals: Arc<Mutex<Vec<PrincipalId>>>,
}

#[async_trait]
impl GmailCapability for Gmail {
    async fn search_messages(
        &self,
        principal: &PrincipalId,
        query: GmailQuery,
        _cancel: &CancellationToken,
    ) -> Result<GmailMessagePage, GoogleApiError> {
        self.seen_principals.lock().unwrap().push(principal.clone());
        Ok(GmailMessagePage {
            account_id: query.account_id,
            messages: vec![summary(query.account_id)],
            next_page_token: None,
        })
    }

    async fn important_unread(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        _page_size: u16,
        cancel: &CancellationToken,
    ) -> Result<GmailMessagePage, GoogleApiError> {
        self.search_messages(
            principal,
            GmailQuery {
                account_id: account,
                query: "is:important is:unread".into(),
                page_size: 20,
                page_token: None,
            },
            cancel,
        )
        .await
    }

    async fn read_message(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        _message_id: &str,
        _cancel: &CancellationToken,
    ) -> Result<GmailMessage, GoogleApiError> {
        self.seen_principals.lock().unwrap().push(principal.clone());
        Ok(GmailMessage {
            summary: summary(account),
            body_text: "bounded body".into(),
        })
    }
}

struct Calendar;

#[async_trait]
impl CalendarCapability for Calendar {
    async fn list_events(
        &self,
        _principal: &PrincipalId,
        range: CalendarTimeRange,
        _cancel: &CancellationToken,
    ) -> Result<CalendarEventPage, GoogleApiError> {
        Ok(CalendarEventPage {
            account_id: range.account_id,
            events: Vec::new(),
            next_page_token: None,
        })
    }
}

fn summary(account: ExternalIdentityId) -> GmailMessageSummary {
    GmailMessageSummary {
        source: ProviderRecordRef {
            account_id: account,
            provider_object_id: "message-1".into(),
            fetched_at_ms: 1,
            source_timestamp_ms: 1,
            etag_or_history: Some("history-1".into()),
        },
        thread_id: "thread-1".into(),
        subject: "status".into(),
        from: "sender@example.com".into(),
        snippet: "safe normalized snippet".into(),
        unread: true,
        important: true,
    }
}

fn context(owner: &PrincipalId, clock: Arc<TestClock>) -> ToolContext {
    ToolContext {
        approval_authority: None,
        agent: None,
        working_dir: std::env::temp_dir(),
        session_id: owner.0.clone(),
        clock,
        turn_event_sender: None,
    }
}

#[tokio::test]
async fn trusted_principal_and_bound_account_flow_through_guard_and_audit() {
    let owner = PrincipalId(LOCAL_OWNER_PRINCIPAL.into());
    let account = ExternalIdentityId::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let gmail: Arc<dyn GmailCapability> = Arc::new(Gmail {
        seen_principals: seen.clone(),
    });
    let accounts: Arc<dyn GoogleAccountResolver> = Arc::new(Accounts {
        owner: owner.clone(),
        account,
        revoked: false,
    });
    let mut registry = ToolRegistry::new();
    registry
        .register_google_read_tools(Some(gmail), Some(Arc::new(Calendar)), accounts)
        .unwrap();
    assert!(registry.get("google_gmail_search").is_some());
    assert!(registry.get("google_gmail_read").is_some());
    assert!(registry.get("google_calendar_list").is_some());

    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.jsonl");
    let clock = Arc::new(TestClock::default());
    let mut runner = ToolRunnerWithGuard::with_sandbox_preference(
        AuditLogger::new(audit_path.clone()).unwrap(),
        SandboxPreference::Forbid,
        clock.clone(),
    );
    let result = runner
        .execute_tool(
            registry.get("google_gmail_search").unwrap().as_ref(),
            serde_json::json!({"account":"work","query":"is:unread","page_size":1}),
            &context(&owner, clock),
            "turn-google-1",
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("provider_object_id"));
    assert!(!result.content.contains(TOKEN_SENTINEL));
    assert_eq!(seen.lock().unwrap().as_slice(), &[owner]);

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let audit = std::fs::read_to_string(audit_path).unwrap();
    assert!(audit.contains("google_gmail_search"));
    assert!(audit.contains("turn-google-1"));
    assert!(!audit.contains(TOKEN_SENTINEL));
    assert!(!audit.to_ascii_lowercase().contains("authorization"));
}

#[tokio::test]
async fn forged_authority_revoked_accounts_and_schema_overflow_fail_closed() {
    let owner = PrincipalId("owner".into());
    let account = ExternalIdentityId::new();
    let gmail: Arc<dyn GmailCapability> = Arc::new(Gmail {
        seen_principals: Arc::new(Mutex::new(Vec::new())),
    });
    let revoked: Arc<dyn GoogleAccountResolver> = Arc::new(Accounts {
        owner: owner.clone(),
        account,
        revoked: true,
    });
    let mut registry = ToolRegistry::new();
    registry
        .register_google_read_tools(Some(gmail), None, revoked)
        .unwrap();
    let tool = registry.get("google_gmail_search").unwrap();
    let clock = Arc::new(TestClock::default());

    let forged = tool
        .execute(
            serde_json::json!({
                "account": account.to_string(),
                "query": "is:unread",
                "principal_id": "attacker"
            }),
            &context(&owner, clock.clone()),
        )
        .await;
    assert!(forged.is_error);
    assert_eq!(forged.content, "google_invalid_request");

    let revoked = tool
        .execute(
            serde_json::json!({"account":"work","query":"is:unread"}),
            &context(&owner, clock.clone()),
        )
        .await;
    assert!(revoked.is_error);
    assert_eq!(revoked.content, "google_unauthorized_account");

    let overflow = tool
        .execute(
            serde_json::json!({"account":"work","query":"x","page_size":101}),
            &context(&owner, clock),
        )
        .await;
    assert!(overflow.is_error);
    assert_eq!(overflow.content, "google_unauthorized_account");
    let schema = tool.input_schema();
    assert_eq!(schema["properties"]["page_size"]["maximum"], 100);
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["properties"].get("principal_id").is_none());
}

#[test]
fn no_google_tools_are_registered_without_active_capabilities() {
    let accounts: Arc<dyn GoogleAccountResolver> = Arc::new(Accounts {
        owner: PrincipalId("owner".into()),
        account: ExternalIdentityId::new(),
        revoked: false,
    });
    let mut registry = ToolRegistry::new();
    let registrations = registry
        .register_google_read_tools(None, None, accounts)
        .unwrap();
    assert!(registrations.is_empty());
    assert!(registry
        .list()
        .iter()
        .all(|name| !name.starts_with("google_")));
}
