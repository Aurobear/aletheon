//! Durable external account bindings and grant lifecycle.

mod repository;

use async_trait::async_trait;
use corpus::tools::google::oauth::GoogleOAuthProvider;
use corpus::tools::google::{
    GoogleAccessToken, GoogleAccountResolver, GoogleApiError, GoogleCredentialSource,
};
use fabric::{ExternalIdentityId, ExternalIdentityState, ExternalScope, GrantState, PrincipalId};
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
        required_scope: ExternalScope,
    ) -> Result<(), GoogleApiError> {
        if required_scope.is_write() {
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
            || binding.1.scopes.iter().any(|scope| scope.is_write())
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
        required_scope: ExternalScope,
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
        required_scope: ExternalScope,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        self.authorize(principal, account, required_scope)?;
        self.oauth.lock().await.refresh_credential(account).await
    }
}
