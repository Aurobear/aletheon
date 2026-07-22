//! Durable external account bindings and grant lifecycle.

pub mod google_use_cases;

mod repository;

use async_trait::async_trait;
use corpus::tools::google::oauth::GoogleOAuthProvider;
use corpus::tools::google::{
    is_google_read_capability, is_google_write_capability, GoogleAccessToken,
    GoogleAccountResolver, GoogleApiError, GoogleCredentialSource,
};
use fabric::{
    ExternalCapabilityId, ExternalIdentityId, ExternalIdentityState, GrantState, PrincipalId,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub use repository::{
    ExternalCredentialRevoker, ExternalIdentityRepository, ExternalRepositoryError,
    ExternalRevocationOutcome,
};

pub struct ExecutiveGoogleCredentialSource {
    repository: Arc<std::sync::Mutex<ExternalIdentityRepository>>,
    oauth: Arc<Mutex<GoogleOAuthProvider>>,
}

pub struct ExecutiveGoogleAccountResolver {
    repository: Arc<std::sync::Mutex<ExternalIdentityRepository>>,
}

pub struct GoogleIntegration {
    repository: Arc<std::sync::Mutex<ExternalIdentityRepository>>,
    oauth: Arc<Mutex<GoogleOAuthProvider>>,
    pending_owners: Mutex<HashMap<String, PrincipalId>>,
    refresh_locks: std::sync::Mutex<HashMap<ExternalIdentityId, Arc<Mutex<()>>>>,
    refresh_versions: Mutex<HashMap<ExternalIdentityId, u64>>,
}

impl GoogleIntegration {
    pub fn new(
        repository: Arc<std::sync::Mutex<ExternalIdentityRepository>>,
        oauth: Arc<Mutex<GoogleOAuthProvider>>,
    ) -> Self {
        Self {
            repository,
            oauth,
            pending_owners: Mutex::new(HashMap::new()),
            refresh_locks: std::sync::Mutex::new(HashMap::new()),
            refresh_versions: Mutex::new(HashMap::new()),
        }
    }

    pub fn repository(&self) -> Arc<std::sync::Mutex<ExternalIdentityRepository>> {
        self.repository.clone()
    }

    pub fn oauth(&self) -> Arc<Mutex<GoogleOAuthProvider>> {
        self.oauth.clone()
    }

    pub async fn start_authorization(
        &self,
        principal: &PrincipalId,
    ) -> anyhow::Result<corpus::tools::google::oauth::AuthorizationStart> {
        let start = self.oauth.lock().await.start_authorization()?;
        self.pending_owners
            .lock()
            .await
            .insert(start.state.clone(), principal.clone());
        Ok(start)
    }

    pub async fn complete_authorization(
        &self,
        principal: &PrincipalId,
        code: &str,
        state: &str,
        alias: Option<String>,
        now_ms: i64,
    ) -> anyhow::Result<(fabric::ExternalIdentity, fabric::CapabilityGrant)> {
        let owner = self
            .pending_owners
            .lock()
            .await
            .remove(state)
            .ok_or_else(|| anyhow::anyhow!("google_authorization_state_invalid"))?;
        anyhow::ensure!(&owner == principal, "google_authorization_state_invalid");
        let binding = self
            .oauth
            .lock()
            .await
            .complete_authorization(code, state)
            .await?;
        self.repository
            .lock()
            .unwrap()
            .bind_google(principal, binding, alias, now_ms)
            .map_err(Into::into)
    }

    pub async fn refresh_singleflight(
        &self,
        identity_id: ExternalIdentityId,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        let observed_version = *self
            .refresh_versions
            .lock()
            .await
            .get(&identity_id)
            .unwrap_or(&0);
        let account_lock = self
            .refresh_locks
            .lock()
            .unwrap()
            .entry(identity_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = account_lock.lock().await;
        let current_version = *self
            .refresh_versions
            .lock()
            .await
            .get(&identity_id)
            .unwrap_or(&0);
        if current_version > observed_version {
            return self.oauth.lock().await.access_credential(identity_id);
        }
        let credential = self
            .oauth
            .lock()
            .await
            .refresh_credential(identity_id)
            .await?;
        self.refresh_versions
            .lock()
            .await
            .insert(identity_id, current_version.saturating_add(1));
        Ok(credential)
    }
}

#[async_trait]
impl gateway::handlers::google_read::GoogleChannelAccountDirectory for GoogleIntegration {
    async fn active_account_labels(&self, principal: &str) -> anyhow::Result<Vec<String>> {
        let bindings = self
            .repository
            .lock()
            .unwrap()
            .list(&PrincipalId(principal.to_owned()))?;
        Ok(bindings
            .into_iter()
            .filter(|(identity, grant)| {
                identity.state == ExternalIdentityState::Active
                    && grant.state == GrantState::Active
                    && grant.scopes.iter().all(is_google_read_capability)
                    && grant.scopes.iter().any(|scope| {
                        [
                            ExternalCapabilityId::new("mail.read").unwrap(),
                            ExternalCapabilityId::new("calendar.read").unwrap(),
                            ExternalCapabilityId::new("file.read").unwrap(),
                        ]
                        .contains(scope)
                    })
            })
            .map(|(identity, _)| identity.alias.unwrap_or_else(|| identity.id.to_string()))
            .take(11)
            .collect())
    }
}

impl ExecutiveGoogleAccountResolver {
    pub fn new(repository: Arc<std::sync::Mutex<ExternalIdentityRepository>>) -> Self {
        Self { repository }
    }
}

#[async_trait]
impl GoogleAccountResolver for ExecutiveGoogleAccountResolver {
    async fn resolve_account(
        &self,
        principal: &PrincipalId,
        account_reference: &str,
    ) -> Result<ExternalIdentityId, GoogleApiError> {
        self.repository
            .lock()
            .unwrap()
            .resolve_account(principal, account_reference)
            .map_err(|_| GoogleApiError::UnauthorizedAccount)?
            .ok_or(GoogleApiError::UnauthorizedAccount)
    }
}

impl ExecutiveGoogleCredentialSource {
    pub fn new(
        repository: Arc<std::sync::Mutex<ExternalIdentityRepository>>,
        oauth: Arc<Mutex<GoogleOAuthProvider>>,
    ) -> Self {
        Self { repository, oauth }
    }

    fn authorize(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalCapabilityId,
    ) -> Result<(), GoogleApiError> {
        if is_google_write_capability(&required_scope) {
            return Err(GoogleApiError::ScopeDenied);
        }
        let binding = self
            .repository
            .lock()
            .unwrap()
            .get(principal, account)
            .map_err(|_| GoogleApiError::ProviderUnavailable)?
            .ok_or(GoogleApiError::UnauthorizedAccount)?;
        if binding.0.state != ExternalIdentityState::Active
            || binding.1.state != GrantState::Active
            || !binding.1.scopes.contains(&required_scope)
            || !binding.1.scopes.iter().all(is_google_read_capability)
        {
            return Err(GoogleApiError::ScopeDenied);
        }
        Ok(())
    }
}

#[async_trait]
impl GoogleCredentialSource for ExecutiveGoogleCredentialSource {
    async fn access_token(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalCapabilityId,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        self.authorize(principal, account, required_scope)?;
        let mut oauth = self.oauth.lock().await;
        match oauth.access_credential(account) {
            Ok(token) => Ok(token),
            Err(GoogleApiError::ReauthorizationRequired) => oauth.refresh_credential(account).await,
            Err(error) => Err(error),
        }
    }

    async fn refresh_access_token(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalCapabilityId,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        self.authorize(principal, account, required_scope)?;
        self.oauth.lock().await.refresh_credential(account).await
    }
}

pub use google_use_cases::ProductionGoogleUseCases;
