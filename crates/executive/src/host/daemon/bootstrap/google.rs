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

use crate::adapters::channel::gmail::{load_gmail_ingress_policies, GmailGoalEventIngress};
use crate::adapters::external::{
    ExecutiveGoogleAccountResolver, ExecutiveGoogleCredentialSource, ExternalIdentityRepository,
    GoogleIntegration,
};

pub(super) type ConfiguredGoogleReadTools = (
    Arc<GoogleIntegration>,
    crate::adapters::google::GoogleSyncHandle,
    Arc<std::sync::Mutex<crate::adapters::google::GoogleSyncStore>>,
    Option<Arc<GmailGoalEventIngress>>,
);

pub(super) fn register_configured_external_read_tools(
    tools: &mut ToolRegistry,
    objective_db_path: &std::path::Path,
    clock: Arc<dyn Clock>,
    cancel: &CancellationToken,
    artifact_root: &std::path::Path,
    storage_quota: Option<crate::r#impl::storage_quota::StorageQuota>,
    config: Option<&crate::composition::config::ResolvedGoogleIntegration>,
) -> anyhow::Result<Option<ConfiguredGoogleReadTools>> {
    let Some(config) = config else {
        return Ok(None);
    };

    let repository = ExternalIdentityRepository::open(objective_db_path)
        .context("opening external identity repository")?;
    let active_bindings = repository.list_active()?;
    let gmail_enabled =
        repository.has_active_scope(fabric::ExternalCapabilityId::new("mail.read").unwrap())?;
    let calendar_enabled =
        repository.has_active_scope(fabric::ExternalCapabilityId::new("calendar.read").unwrap())?;
    let mut scopes = vec![
        fabric::ExternalCapabilityId::new("identity.openid").unwrap(),
        fabric::ExternalCapabilityId::new("identity.email.read").unwrap(),
        fabric::ExternalCapabilityId::new("mail.read").unwrap(),
        fabric::ExternalCapabilityId::new("calendar.read").unwrap(),
    ];
    if config.drive_sync_enabled {
        scopes.push(fabric::ExternalCapabilityId::new("file.read").unwrap());
    }
    let tokens = corpus::tools::mcp::token_store::TokenStore::open_default()
        .context("opening encrypted Google credential vault")?;
    let oauth = corpus::tools::google::oauth::GoogleOAuthProvider::new(
        config.client_id.clone(),
        config
            .client_secret
            .as_ref()
            .map(|value| value.expose().to_owned()),
        config.redirect_uri.clone(),
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

    let gmail_ingress = match (gmail_adapter, config.gmail_ingress_policy_file.as_ref()) {
        (Some(adapter), Some(path)) => {
            let owners = active_bindings
                .iter()
                .map(|(identity, _)| (identity.id, identity.principal_id.clone()))
                .collect::<std::collections::HashMap<_, _>>();
            let policies = load_gmail_ingress_policies(path, &owners)
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
        crate::adapters::google::GoogleSyncStore::open(objective_db_path)?,
    ));
    let mut manager = crate::adapters::google::GoogleSyncManager::new(
        store.clone(),
        format!("daemon-{}", uuid::Uuid::new_v4()),
        clock.clone(),
        crate::adapters::google::GoogleSyncManagerConfig::default(),
    )?;
    let drive_enabled = config.drive_sync_enabled;
    let selected_drive_files = config
        .drive_file_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let now_ms = clock.wall_now().0.max(0);
    for (identity, grant) in active_bindings {
        if grant
            .scopes
            .contains(&fabric::ExternalCapabilityId::new("mail.read").unwrap())
        {
            manager.register(crate::adapters::google::GoogleSyncRegistration {
                principal: identity.principal_id.clone(),
                account_id: identity.id,
                stream: crate::adapters::google::SyncStream::GmailHistory,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::adapters::google::GmailHistoryPoller(
                    GmailHistorySynchronizer::new(
                        GoogleGmailAdapter::new(client.clone()),
                        GmailHistorySyncConfig::default(),
                    )?,
                )),
            })?;
        }
        if grant
            .scopes
            .contains(&fabric::ExternalCapabilityId::new("calendar.read").unwrap())
        {
            manager.register(crate::adapters::google::GoogleSyncRegistration {
                principal: identity.principal_id.clone(),
                account_id: identity.id,
                stream: crate::adapters::google::SyncStream::Calendar,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::adapters::google::CalendarDeltaPoller(
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

        if drive_enabled
            && grant
                .scopes
                .contains(&fabric::ExternalCapabilityId::new("file.read").unwrap())
        {
            manager.register(crate::adapters::google::GoogleSyncRegistration {
                principal: identity.principal_id,
                account_id: identity.id,
                stream: crate::adapters::google::SyncStream::DriveChanges,
                initial_cursor: None,
                cursor_generation: 1,
                poller: Arc::new(crate::adapters::google::DriveChangesPoller(
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
