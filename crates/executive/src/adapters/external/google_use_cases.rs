//! Provider adapter implementing the external-account application port.

use std::sync::Arc;

use async_trait::async_trait;
use corpus::tools::google::is_google_read_capability;
use fabric::{
    Clock, ExternalCapabilityId, ExternalIdentityState, GrantState, PrincipalId,
    LOCAL_OWNER_PRINCIPAL,
};
use serde_json::json;
use tokio::sync::RwLock;

use crate::adapters::external::GoogleIntegration;
use crate::application::request_use_cases::{
    ExternalRefreshStatus, ExternalSourceUseCaseError, ExternalSourceUseCases,
};

pub struct ProductionExternalSourceUseCases {
    integration: Option<Arc<GoogleIntegration>>,
    corpus: Arc<dyn corpus::CorpusService>,
    capabilities: Arc<RwLock<Vec<fabric::CapabilityId>>>,
    clock: Arc<dyn Clock>,
}

impl ProductionExternalSourceUseCases {
    pub fn new(
        integration: Option<Arc<GoogleIntegration>>,
        corpus: Arc<dyn corpus::CorpusService>,
        capabilities: Arc<RwLock<Vec<fabric::CapabilityId>>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            integration,
            corpus,
            capabilities,
            clock,
        }
    }

    fn context(&self) -> Result<(Arc<GoogleIntegration>, PrincipalId), ExternalSourceUseCaseError> {
        Ok((
            self.integration
                .clone()
                .ok_or(ExternalSourceUseCaseError::Unavailable)?,
            PrincipalId(LOCAL_OWNER_PRINCIPAL.into()),
        ))
    }

    async fn register_read_tools(&self, google: &Arc<GoogleIntegration>) -> anyhow::Result<()> {
        use corpus::tools::google::{
            GoogleApiClient, GoogleApiEndpoints, GoogleCalendarAdapter, GoogleGmailAdapter,
        };
        let repository = google.repository();
        let (gmail, calendar) = {
            let repository = repository.lock().unwrap();
            (
                repository.has_active_scope(ExternalCapabilityId::new("mail.read").unwrap())?,
                repository.has_active_scope(ExternalCapabilityId::new("calendar.read").unwrap())?,
            )
        };
        let credentials = Arc::new(
            crate::adapters::external::ExecutiveGoogleCredentialSource::new(
                repository.clone(),
                google.oauth(),
            ),
        );
        let accounts =
            Arc::new(crate::adapters::external::ExecutiveGoogleAccountResolver::new(repository));
        let client = GoogleApiClient::new(credentials, GoogleApiEndpoints::default())?;
        if gmail && !self.tool_registered("google_gmail_search").await? {
            let gmail = Arc::new(GoogleGmailAdapter::new(client.clone()));
            self.corpus
                .register_tool(Arc::new(corpus::tools::google::GoogleGmailSearchTool::new(
                    gmail.clone(),
                    accounts.clone(),
                )))
                .await?;
            self.grant_tool("google_gmail_search").await;
            self.corpus
                .register_tool(Arc::new(corpus::tools::google::GoogleGmailReadTool::new(
                    gmail,
                    accounts.clone(),
                )))
                .await?;
            self.grant_tool("google_gmail_read").await;
        }
        if calendar && !self.tool_registered("google_calendar_list").await? {
            self.corpus
                .register_tool(Arc::new(
                    corpus::tools::google::GoogleCalendarListTool::new(
                        Arc::new(GoogleCalendarAdapter::new(client)),
                        accounts,
                    ),
                ))
                .await?;
            self.grant_tool("google_calendar_list").await;
        }
        Ok(())
    }

    async fn tool_registered(&self, name: &str) -> anyhow::Result<bool> {
        let grant = corpus::ExtensionGrant {
            grant_id: format!("google-tool-check:{name}"),
            principal: PrincipalId(LOCAL_OWNER_PRINCIPAL.into()),
            session_id: "google-admin".into(),
            agent_id: None,
            capabilities: vec![fabric::CapabilityId(name.into())],
            resources: fabric::CapabilityScope::default(),
        };
        Ok(!self.corpus.catalog(&grant).await?.entries.is_empty())
    }

    async fn grant_tool(&self, name: &str) {
        let capability = fabric::CapabilityId(name.into());
        let mut capabilities = self.capabilities.write().await;
        if !capabilities.contains(&capability) {
            capabilities.push(capability);
        }
    }
}

#[async_trait]
impl ExternalSourceUseCases for ProductionExternalSourceUseCases {
    async fn authorization_start(&self) -> Result<serde_json::Value, ExternalSourceUseCaseError> {
        let (google, principal) = self.context()?;
        let start = google
            .start_authorization(&principal)
            .await
            .map_err(|_| ExternalSourceUseCaseError::Provider)?;
        Ok(
            json!({"authorization_url":start.url,"state":start.state,"expires_at_secs":start.expires_at_secs}),
        )
    }
    async fn authorization_callback(
        &self,
        code: String,
        state: String,
        alias: Option<String>,
    ) -> Result<serde_json::Value, ExternalSourceUseCaseError> {
        let (google, principal) = self.context()?;
        let (identity, grant) = google
            .complete_authorization(&principal, &code, &state, alias, self.clock.wall_now().0)
            .await
            .map_err(|_| ExternalSourceUseCaseError::Provider)?;
        if let Err(error) = self.register_read_tools(&google).await {
            tracing::warn!(%error, "Google account bound but tool registration failed");
        }
        Ok(safe_account(&identity, &grant))
    }
    async fn accounts(&self) -> Result<Vec<serde_json::Value>, ExternalSourceUseCaseError> {
        let (google, principal) = self.context()?;
        google
            .repository()
            .lock()
            .unwrap()
            .list(&principal)
            .map(|items| {
                items
                    .iter()
                    .map(|(identity, grant)| safe_account(identity, grant))
                    .collect()
            })
            .map_err(|_| ExternalSourceUseCaseError::Provider)
    }
    async fn revoke(&self, account: String) -> Result<(bool, bool), ExternalSourceUseCaseError> {
        let (google, principal) = self.context()?;
        let repository = google.repository();
        let identity = {
            let repository = repository.lock().unwrap();
            let id = repository
                .resolve_account(&principal, &account)
                .map_err(|_| ExternalSourceUseCaseError::Provider)?
                .ok_or(ExternalSourceUseCaseError::NotFound)?;
            repository
                .get(&principal, id)
                .map_err(|_| ExternalSourceUseCaseError::Provider)?
                .map(|item| item.0)
                .ok_or(ExternalSourceUseCaseError::NotFound)?
        };
        repository
            .lock()
            .unwrap()
            .revoke_local(
                &principal,
                identity.id,
                identity.version,
                self.clock.wall_now().0,
            )
            .map_err(|_| ExternalSourceUseCaseError::Provider)?;
        let provider = google
            .oauth()
            .lock()
            .await
            .revoke(identity.id)
            .await
            .is_ok();
        Ok((true, provider))
    }
    async fn refresh(
        &self,
        account: String,
    ) -> Result<ExternalRefreshStatus, ExternalSourceUseCaseError> {
        let (google, principal) = self.context()?;
        let account_id = {
            let repository = google.repository();
            let repository = repository.lock().unwrap();
            let id = repository
                .resolve_account(&principal, &account)
                .map_err(|_| ExternalSourceUseCaseError::Provider)?
                .ok_or(ExternalSourceUseCaseError::NotFound)?;
            let (identity, grant) = repository
                .get(&principal, id)
                .map_err(|_| ExternalSourceUseCaseError::Provider)?
                .ok_or(ExternalSourceUseCaseError::NotFound)?;
            let active = identity.state == ExternalIdentityState::Active
                && grant.state == GrantState::Active
                && grant.scopes.iter().any(|scope| {
                    [
                        ExternalCapabilityId::new("mail.read").unwrap(),
                        ExternalCapabilityId::new("calendar.read").unwrap(),
                        ExternalCapabilityId::new("file.read").unwrap(),
                    ]
                    .contains(scope)
                })
                && grant.scopes.iter().all(is_google_read_capability);
            if !active {
                return Err(ExternalSourceUseCaseError::Forbidden);
            }
            id
        };
        match google.refresh_singleflight(account_id).await {
            Ok(_) => Ok(ExternalRefreshStatus {
                status: "success".into(),
                code: None,
            }),
            Err(corpus::tools::google::GoogleApiError::ReauthorizationRequired) => {
                Ok(ExternalRefreshStatus {
                    status: "reauthorization_required".into(),
                    code: None,
                })
            }
            Err(error) => Ok(ExternalRefreshStatus {
                status: "error".into(),
                code: Some(error.to_string()),
            }),
        }
    }
}

fn safe_account(
    identity: &fabric::ExternalIdentity,
    grant: &fabric::CapabilityGrant,
) -> serde_json::Value {
    json!({
        "id": identity.id, "email": identity.email, "alias": identity.alias,
        "state": identity.state, "scopes": grant.scopes,
        "grant_state": grant.state, "version": identity.version,
    })
}
