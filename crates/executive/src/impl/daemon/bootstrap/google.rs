//! Google identity, OAuth, tools, and synchronizer construction.

use std::sync::Arc;

use anyhow::Context;
use corpus::tools::google::{
    CalendarSyncConfig, CalendarSynchronizer, DriveSyncConfig, DriveSynchronizer,
    GmailHistorySyncConfig, GmailHistorySynchronizer, GoogleApiClient, GoogleApiEndpoints,
    GoogleCalendarAdapter, GoogleDriveAdapter, GoogleGmailAdapter,
};
use corpus::tools::tools::ToolRegistry;
use fabric::Clock;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::r#impl::channel::gmail::{load_gmail_ingress_policies, GmailGoalEventIngress};
use crate::r#impl::external::{
    ExecutiveGoogleAccountResolver, ExecutiveGoogleCredentialSource, ExternalIdentityRepository,
    GoogleIntegration,
};

pub(super) type ConfiguredGoogleReadTools = (
    Arc<GoogleIntegration>,
    crate::r#impl::google::GoogleSyncHandle,
    Arc<std::sync::Mutex<crate::r#impl::google::GoogleSyncStore>>,
    Option<Arc<GmailGoalEventIngress>>,
);

pub(super) fn register_configured_google_read_tools(
    tools: &mut ToolRegistry,
    objective_db_path: &std::path::Path,
    clock: Arc<dyn Clock>,
    cancel: &CancellationToken,
    artifact_root: &std::path::Path,
    storage_quota: Option<crate::r#impl::storage_quota::StorageQuota>,
) -> anyhow::Result<Option<ConfiguredGoogleReadTools>> {
    let client_id = match std::env::var("ALETHEON_GOOGLE_CLIENT_ID") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };
    let redirect_uri = std::env::var("ALETHEON_GOOGLE_REDIRECT_URI")
        .context("ALETHEON_GOOGLE_REDIRECT_URI is required when Google is configured")?;
    let client_secret = std::env::var("ALETHEON_GOOGLE_CLIENT_SECRET")
        .ok()
        .filter(|value| !value.is_empty());

    let repository = ExternalIdentityRepository::open(objective_db_path)
        .context("opening external identity repository")?;
    let active_bindings = repository.list_active()?;
    let gmail_enabled = repository.has_active_scope(fabric::ExternalScope::GmailReadonly)?;
    let calendar_enabled = repository.has_active_scope(fabric::ExternalScope::CalendarReadonly)?;
    let mut scopes = vec![
        fabric::ExternalScope::OpenId,
        fabric::ExternalScope::UserInfoEmail,
        fabric::ExternalScope::GmailReadonly,
        fabric::ExternalScope::CalendarReadonly,
    ];
    if std::env::var("ALETHEON_GOOGLE_DRIVE_SYNC_ENABLED")
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
    {
        scopes.push(fabric::ExternalScope::DriveReadonly);
    }
    let tokens = corpus::tools::mcp::token_store::TokenStore::open_default()
        .context("opening encrypted Google credential vault")?;
    let oauth = corpus::tools::google::oauth::GoogleOAuthProvider::new(
        client_id,
        client_secret,
        redirect_uri,
        scopes,
        tokens,
        clock.clone(),
    )?;
    let repository = Arc::new(std::sync::Mutex::new(repository));
    let oauth = Arc::new(Mutex::new(oauth));
    let integration = Arc::new(GoogleIntegration::new(repository.clone(), oauth.clone()));
    let credentials = Arc::new(ExecutiveGoogleCredentialSource::new(
        repository.clone(),
        oauth,
    ));
    let accounts = Arc::new(ExecutiveGoogleAccountResolver::new(repository));
    let client = GoogleApiClient::new(credentials, GoogleApiEndpoints::default())?;
    let gmail_adapter = gmail_enabled.then(|| Arc::new(GoogleGmailAdapter::new(client.clone())));
    let gmail = gmail_adapter
        .clone()
        .map(|adapter| adapter as Arc<dyn corpus::tools::google::GmailCapability>);
    let calendar = calendar_enabled.then(|| {
        Arc::new(GoogleCalendarAdapter::new(client.clone()))
            as Arc<dyn corpus::tools::google::CalendarCapability>
    });
    tools.register_google_read_tools(gmail, calendar, accounts)?;

    let gmail_ingress = match (
        gmail_adapter,
        std::env::var_os("ALETHEON_GMAIL_INGRESS_POLICY_FILE"),
    ) {
        (Some(adapter), Some(path)) => {
            let owners = active_bindings
                .iter()
                .map(|(identity, _)| (identity.id, identity.principal_id.clone()))
                .collect::<std::collections::HashMap<_, _>>();
            let policies = load_gmail_ingress_policies(std::path::Path::new(&path), &owners)
                .context("loading Gmail ingress policies")?;
            let ingress =
                GmailGoalEventIngress::new(adapter, objective_db_path, artifact_root, policies)?;
            Some(Arc::new(match storage_quota.clone() {
                Some(quota) => ingress.with_storage_quota(quota),
                None => ingress,
            }))
        }
        _ => None,
    };

    let store = Arc::new(std::sync::Mutex::new(
        crate::r#impl::google::GoogleSyncStore::open(objective_db_path)?,
    ));
    let mut manager = crate::r#impl::google::GoogleSyncManager::new(
        store.clone(),
        format!("daemon-{}", uuid::Uuid::new_v4()),
        clock.clone(),
        crate::r#impl::google::GoogleSyncManagerConfig::default(),
    )?;
    let drive_enabled = std::env::var("ALETHEON_GOOGLE_DRIVE_SYNC_ENABLED")
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"));
    let selected_drive_files = std::env::var("ALETHEON_GOOGLE_DRIVE_FILE_IDS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<std::collections::HashSet<_>>();
    let now_ms = clock.wall_now().0.max(0);
    for (identity, grant) in active_bindings {
        if grant.scopes.contains(&fabric::ExternalScope::GmailReadonly) {
            manager.register(crate::r#impl::google::GoogleSyncRegistration {
                principal: identity.principal_id.clone(),
                account_id: identity.id,
                stream: crate::r#impl::google::SyncStream::GmailHistory,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::r#impl::google::GmailHistoryPoller(
                    GmailHistorySynchronizer::new(
                        GoogleGmailAdapter::new(client.clone()),
                        GmailHistorySyncConfig::default(),
                    )?,
                )),
            })?;
        }
        if grant
            .scopes
            .contains(&fabric::ExternalScope::CalendarReadonly)
        {
            manager.register(crate::r#impl::google::GoogleSyncRegistration {
                principal: identity.principal_id.clone(),
                account_id: identity.id,
                stream: crate::r#impl::google::SyncStream::Calendar,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::r#impl::google::CalendarDeltaPoller(
                    CalendarSynchronizer::new(
                        GoogleCalendarAdapter::new(client.clone()),
                        CalendarSyncConfig {
                            window_start_ms: now_ms.saturating_sub(30 * 86_400_000),
                            window_end_ms: now_ms.saturating_add(365 * 86_400_000),
                            timezone: "UTC".into(),
                            max_pages: 20,
                            page_size: 250,
                        },
                    )?,
                )),
            })?;
        }

        if drive_enabled && grant.scopes.contains(&fabric::ExternalScope::DriveReadonly) {
            manager.register(crate::r#impl::google::GoogleSyncRegistration {
                principal: identity.principal_id,
                account_id: identity.id,
                stream: crate::r#impl::google::SyncStream::DriveChanges,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::r#impl::google::DriveChangesPoller(
                    DriveSynchronizer::new(
                        GoogleDriveAdapter::new(client.clone()),
                        DriveSyncConfig {
                            selected_file_ids: selected_drive_files.clone(),
                            content_mime_allowlist: std::collections::HashSet::new(),
                            download_content: false,
                            max_content_bytes: 8 * 1_048_576,
                            max_pages: 20,
                            max_changes: 2_000,
                            page_size: 100,
                        },
                    )?,
                )),
            })?;
        }
    }
    let sync = manager.start(cancel);
    Ok(Some((integration, sync, store, gmail_ingress)))
}
