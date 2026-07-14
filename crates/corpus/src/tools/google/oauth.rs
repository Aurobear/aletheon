//! Asynchronous OAuth 2.0 Authorization Code + PKCE client for Google.

use super::client::{GoogleAccessToken, GoogleApiError};
use crate::tools::mcp::token_store::{TokenEntry, TokenKey, TokenStore};
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::OsRng;
use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fabric::{Clock, ExternalIdentityId, ExternalScope, IdentityProvider};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_REVOCATION_URL: &str = "https://oauth2.googleapis.com/revoke";
pub const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const STATE_LIFETIME_SECS: u64 = 600;

#[derive(Clone)]
pub struct OAuthClientConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub auth_url: String,
    pub token_url: String,
    pub revocation_url: Option<String>,
    pub userinfo_url: Option<String>,
}

impl fmt::Debug for OAuthClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthClientConfig")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("redirect_uri", &self.redirect_uri)
            .field("auth_url", &self.auth_url)
            .field("token_url", &self.token_url)
            .field("revocation_url", &self.revocation_url)
            .field("userinfo_url", &self.userinfo_url)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct AsyncOAuthClient {
    config: OAuthClientConfig,
    client: reqwest::Client,
}

impl AsyncOAuthClient {
    pub fn new(config: OAuthClientConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .context("building OAuth HTTP client")?;
        Ok(Self { config, client })
    }

    pub fn authorization_url(
        &self,
        state: &str,
        challenge: &str,
        scopes: &[String],
        google_incremental: bool,
    ) -> Result<String> {
        let mut url = reqwest::Url::parse(&self.config.auth_url)
            .context("invalid OAuth authorization endpoint")?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", &self.config.redirect_uri)
                .append_pair("scope", &scopes.join(" "))
                .append_pair("state", state)
                .append_pair("code_challenge", challenge)
                .append_pair("code_challenge_method", "S256");
            if google_incremental {
                query
                    .append_pair("access_type", "offline")
                    .append_pair("include_granted_scopes", "true");
            }
        }
        Ok(url.into())
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        requested_scopes: &[String],
        now_secs: u64,
    ) -> Result<TokenEntry> {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("client_id", self.config.client_id.as_str()),
            ("code_verifier", verifier),
        ];
        if let Some(secret) = self.config.client_secret.as_deref() {
            form.push(("client_secret", secret));
        }
        let response = self
            .client
            .post(&self.config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("OAuth token exchange transport failed"))?;
        parse_response(response, requested_scopes, None, now_secs).await
    }

    pub async fn refresh(
        &self,
        current: &TokenEntry,
        requested_scopes: &[String],
        now_secs: u64,
    ) -> Result<TokenEntry> {
        let refresh = current
            .refresh_token
            .as_deref()
            .context("OAuth reauthorization required")?;
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", self.config.client_id.as_str()),
        ];
        if let Some(secret) = self.config.client_secret.as_deref() {
            form.push(("client_secret", secret));
        }
        let response = self
            .client
            .post(&self.config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("OAuth token refresh transport failed"))?;
        parse_response(
            response,
            requested_scopes,
            current.refresh_token.clone(),
            now_secs,
        )
        .await
    }

    pub async fn revoke(&self, token: &str) -> Result<()> {
        let endpoint = self
            .config
            .revocation_url
            .as_deref()
            .context("OAuth revocation endpoint unavailable")?;
        let response = self
            .client
            .post(endpoint)
            .form(&[("token", token)])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("OAuth revocation transport failed"))?;
        anyhow::ensure!(response.status().is_success(), "OAuth revocation rejected");
        Ok(())
    }

    async fn google_userinfo(&self, access_token: &str) -> Result<RawGoogleUserInfo> {
        let endpoint = self
            .config
            .userinfo_url
            .as_deref()
            .context("Google user-info endpoint unavailable")?;
        let response = self
            .client
            .get(endpoint)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Google user-info transport failed"))?;
        anyhow::ensure!(
            response.status().is_success(),
            "Google user-info request rejected"
        );
        let profile: RawGoogleUserInfo = response
            .json()
            .await
            .context("Google user-info response was malformed")?;
        anyhow::ensure!(
            profile.verified_email.unwrap_or(true),
            "Google account email is unverified"
        );
        anyhow::ensure!(
            !profile.sub.trim().is_empty() && profile.email.contains('@'),
            "Google user-info identity is invalid"
        );
        Ok(profile)
    }
}

async fn parse_response(
    response: reqwest::Response,
    requested_scopes: &[String],
    preserved_refresh: Option<String>,
    now_secs: u64,
) -> Result<TokenEntry> {
    anyhow::ensure!(
        response.status().is_success(),
        "OAuth token endpoint rejected request"
    );
    let raw: RawTokenResponse = response
        .json()
        .await
        .context("OAuth token endpoint returned malformed JSON")?;
    let access_token = raw
        .access_token
        .context("OAuth token response missing access token")?;
    let effective_scopes = raw
        .scope
        .map(|value| {
            value
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| requested_scopes.to_vec());
    ensure_scope_subset(&effective_scopes, requested_scopes)?;
    Ok(TokenEntry {
        access_token,
        refresh_token: raw.refresh_token.or(preserved_refresh),
        expires_at: now_secs.saturating_add(raw.expires_in.unwrap_or(3_600)),
        token_type: raw.token_type.unwrap_or_else(|| "Bearer".into()),
        scopes: effective_scopes,
    })
}

fn ensure_scope_subset(effective: &[String], requested: &[String]) -> Result<()> {
    let requested: HashSet<&str> = requested.iter().map(String::as_str).collect();
    anyhow::ensure!(
        effective
            .iter()
            .all(|scope| requested.contains(scope.as_str())),
        "OAuth provider returned an unauthorized scope"
    );
    Ok(())
}

#[derive(Deserialize)]
struct RawTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct RawGoogleUserInfo {
    sub: String,
    email: String,
    verified_email: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleBinding {
    pub identity_id: ExternalIdentityId,
    pub provider_subject: String,
    pub email: String,
    pub scopes: Vec<ExternalScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationStart {
    pub url: String,
    pub state: String,
    pub expires_at_secs: u64,
}

#[derive(Clone)]
struct PendingAuthorization {
    created_at_secs: u64,
    verifier: String,
    requested_scopes: Vec<String>,
}

impl fmt::Debug for PendingAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingAuthorization")
            .field("created_at_secs", &self.created_at_secs)
            .field("verifier", &"[REDACTED]")
            .field("requested_scopes", &self.requested_scopes)
            .finish()
    }
}

pub struct GoogleOAuthProvider {
    oauth: AsyncOAuthClient,
    scopes: Vec<String>,
    pending: HashMap<String, PendingAuthorization>,
    tokens: TokenStore,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for GoogleOAuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleOAuthProvider")
            .field("oauth", &self.oauth)
            .field("scopes", &self.scopes)
            .field("pending_count", &self.pending.len())
            .field("tokens", &self.tokens)
            .finish()
    }
}

impl GoogleOAuthProvider {
    pub fn new(
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
        scopes: Vec<ExternalScope>,
        tokens: TokenStore,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        Self::with_endpoints(
            OAuthClientConfig {
                client_id,
                client_secret,
                redirect_uri,
                auth_url: GOOGLE_AUTH_URL.into(),
                token_url: GOOGLE_TOKEN_URL.into(),
                revocation_url: Some(GOOGLE_REVOCATION_URL.into()),
                userinfo_url: Some(GOOGLE_USERINFO_URL.into()),
            },
            scopes,
            tokens,
            clock,
        )
    }

    pub fn with_endpoints(
        config: OAuthClientConfig,
        scopes: Vec<ExternalScope>,
        tokens: TokenStore,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        anyhow::ensure!(
            !scopes.is_empty() && scopes.iter().all(|scope| scope.is_m6_allowed()),
            "Google OAuth requested a non-read-only scope"
        );
        let scopes = scopes
            .into_iter()
            .map(|scope| scope.oauth_name().to_owned())
            .collect();
        Ok(Self {
            oauth: AsyncOAuthClient::new(config)?,
            scopes,
            pending: HashMap::new(),
            tokens,
            clock,
        })
    }

    pub fn start_authorization(&mut self) -> Result<AuthorizationStart> {
        let state = random_urlsafe(32);
        let verifier = generate_pkce_verifier();
        let challenge = pkce_challenge(&verifier);
        let now = now_secs(&*self.clock);
        self.pending.insert(
            state.clone(),
            PendingAuthorization {
                created_at_secs: now,
                verifier,
                requested_scopes: self.scopes.clone(),
            },
        );
        Ok(AuthorizationStart {
            url: self
                .oauth
                .authorization_url(&state, &challenge, &self.scopes, true)?,
            state,
            expires_at_secs: now + STATE_LIFETIME_SECS,
        })
    }

    pub async fn complete_authorization(
        &mut self,
        code: &str,
        state: &str,
    ) -> Result<GoogleBinding> {
        let pending = self
            .pending
            .remove(state)
            .context("unknown or already-consumed OAuth state")?;
        let now = now_secs(&*self.clock);
        anyhow::ensure!(
            now.saturating_sub(pending.created_at_secs) <= STATE_LIFETIME_SECS,
            "OAuth state expired"
        );
        let entry = self
            .oauth
            .exchange_code(code, &pending.verifier, &pending.requested_scopes, now)
            .await?;
        let profile = self.oauth.google_userinfo(&entry.access_token).await?;
        let identity_id = ExternalIdentityId::new();
        let scopes = entry
            .scopes
            .iter()
            .map(|scope| external_scope(scope))
            .collect::<Result<Vec<_>>>()?;
        self.tokens.set_key(
            TokenKey::external(IdentityProvider::Google, identity_id),
            entry,
        );
        self.tokens.save()?;
        Ok(GoogleBinding {
            identity_id,
            provider_subject: profile.sub,
            email: profile.email,
            scopes,
        })
    }

    pub async fn refresh(&mut self, identity_id: ExternalIdentityId) -> Result<()> {
        let key = TokenKey::external(IdentityProvider::Google, identity_id);
        let current = self
            .tokens
            .get_key(&key)
            .context("Google account requires authorization")?
            .clone();
        let refreshed = self
            .oauth
            .refresh(&current, &self.scopes, now_secs(&*self.clock))
            .await?;
        self.tokens.set_key(key, refreshed);
        self.tokens.save()
    }

    pub fn access_credential(
        &self,
        identity_id: ExternalIdentityId,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        let key = TokenKey::external(IdentityProvider::Google, identity_id);
        let entry = self
            .tokens
            .get_key(&key)
            .ok_or(GoogleApiError::CredentialUnavailable)?;
        if entry.expires_at <= now_secs(&*self.clock) {
            return Err(GoogleApiError::ReauthorizationRequired);
        }
        GoogleAccessToken::new(entry.access_token.clone())
    }

    pub async fn refresh_credential(
        &mut self,
        identity_id: ExternalIdentityId,
    ) -> Result<GoogleAccessToken, GoogleApiError> {
        self.refresh(identity_id)
            .await
            .map_err(|_| GoogleApiError::ReauthorizationRequired)?;
        self.access_credential(identity_id)
    }

    pub async fn revoke(&mut self, identity_id: ExternalIdentityId) -> Result<()> {
        let key = TokenKey::external(IdentityProvider::Google, identity_id);
        let entry = self.tokens.remove_key(&key);
        self.tokens.save()?;
        if let Some(entry) = entry {
            let token = entry
                .refresh_token
                .as_deref()
                .unwrap_or(&entry.access_token);
            self.oauth.revoke(token).await?;
        }
        Ok(())
    }

    pub fn purge_expired_states(&mut self) {
        let now = now_secs(&*self.clock);
        self.pending.retain(|_, pending| {
            now.saturating_sub(pending.created_at_secs) <= STATE_LIFETIME_SECS
        });
    }
}

fn random_urlsafe(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

pub(crate) fn generate_pkce_verifier() -> String {
    random_urlsafe(64)
}

pub(crate) fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn now_secs(clock: &dyn Clock) -> u64 {
    u64::try_from(clock.wall_now().0.max(0)).unwrap_or(0) / 1_000
}

fn external_scope(scope: &str) -> Result<ExternalScope> {
    [
        ExternalScope::OpenId,
        ExternalScope::UserInfoEmail,
        ExternalScope::GmailReadonly,
        ExternalScope::CalendarReadonly,
    ]
    .into_iter()
    .find(|candidate| candidate.oauth_name() == scope)
    .with_context(|| format!("unsupported Google read scope: {scope}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use http_body_util::{BodyExt, Full};
    use hyper::body::{Bytes, Incoming};
    use hyper::service::service_fn;
    use hyper::{Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    fn provider() -> GoogleOAuthProvider {
        let dir = tempfile::tempdir().unwrap();
        let tokens = TokenStore::new(dir.path().join("tokens.json")).unwrap();
        // Keep the backing directory alive for the duration of the test by
        // leaking this tiny isolated fixture.
        std::mem::forget(dir);
        GoogleOAuthProvider::with_endpoints(
            OAuthClientConfig {
                client_id: "client".into(),
                client_secret: Some("client-secret-value".into()),
                redirect_uri: "http://localhost/callback".into(),
                auth_url: "https://accounts.example/authorize".into(),
                token_url: "https://accounts.example/token".into(),
                revocation_url: Some("https://accounts.example/revoke".into()),
                userinfo_url: Some("https://accounts.example/userinfo".into()),
            },
            vec![ExternalScope::OpenId, ExternalScope::GmailReadonly],
            tokens,
            Arc::new(TestClock::default()),
        )
        .unwrap()
    }

    #[test]
    fn authorization_url_is_pkce_bound_offline_and_incremental() {
        let mut provider = provider();
        let start = provider.start_authorization().unwrap();
        let url = reqwest::Url::parse(&start.url).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert!(query
            .get("code_challenge")
            .is_some_and(|value| value.len() == 43));
        assert_eq!(
            query.get("access_type").map(String::as_str),
            Some("offline")
        );
        assert_eq!(
            query.get("include_granted_scopes").map(String::as_str),
            Some("true")
        );
        assert!(!start.url.contains("client-secret-value"));
        assert!(!format!("{provider:?}").contains("client-secret-value"));
    }

    #[tokio::test]
    async fn unknown_and_replayed_state_fail_before_network() {
        let mut provider = provider();
        assert!(provider
            .complete_authorization("code", "forged")
            .await
            .is_err());
        let start = provider.start_authorization().unwrap();
        let first = provider.complete_authorization("code", &start.state).await;
        assert!(first.is_err());
        let replay = provider.complete_authorization("code", &start.state).await;
        assert!(replay.unwrap_err().to_string().contains("already-consumed"));
    }

    #[test]
    fn write_scopes_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let result = GoogleOAuthProvider::new(
            "client".into(),
            None,
            "http://localhost/callback".into(),
            vec![ExternalScope::GmailSend],
            TokenStore::new(dir.path().join("tokens.json")).unwrap(),
            Arc::new(TestClock::default()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn effective_scope_must_be_subset() {
        assert!(ensure_scope_subset(
            &["openid".into()],
            &["openid".into(), "gmail.readonly".into()]
        )
        .is_ok());
        let error =
            ensure_scope_subset(&["gmail.send".into()], &["gmail.readonly".into()]).unwrap_err();
        assert!(!error.to_string().contains("token"));
    }

    async fn mock_server(
        responses: Vec<(StatusCode, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let response_queue = responses.clone();
        let captured = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let response_queue = response_queue.clone();
                let captured = captured.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<Incoming>| {
                        let response_queue = response_queue.clone();
                        let captured = captured.clone();
                        async move {
                            let bytes = request.into_body().collect().await.unwrap().to_bytes();
                            captured
                                .lock()
                                .unwrap()
                                .push(String::from_utf8_lossy(&bytes).into_owned());
                            let (status, body) = response_queue
                                .lock()
                                .unwrap()
                                .pop_front()
                                .unwrap_or((StatusCode::INTERNAL_SERVER_ERROR, "{}"));
                            Ok::<_, hyper::Error>(
                                Response::builder()
                                    .status(status)
                                    .header("content-type", "application/json")
                                    .body(Full::new(Bytes::from(body)))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        (format!("http://{address}"), requests)
    }

    fn async_client(endpoint: &str) -> AsyncOAuthClient {
        AsyncOAuthClient::new(OAuthClientConfig {
            client_id: "client".into(),
            client_secret: Some("client-secret".into()),
            redirect_uri: "http://localhost/callback".into(),
            auth_url: format!("{endpoint}/authorize"),
            token_url: format!("{endpoint}/token"),
            revocation_url: Some(format!("{endpoint}/revoke")),
            userinfo_url: Some(format!("{endpoint}/userinfo")),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn async_exchange_sends_pkce_and_accepts_reduced_scopes() {
        let (endpoint, requests) = mock_server(vec![(
            StatusCode::OK,
            r#"{"access_token":"access-secret","refresh_token":"refresh-secret","expires_in":60,"scope":"openid"}"#,
        )])
        .await;
        let entry = async_client(&endpoint)
            .exchange_code(
                "authorization-code",
                "pkce-verifier-value",
                &["openid".into(), "gmail.readonly".into()],
                10,
            )
            .await
            .unwrap();
        assert_eq!(entry.scopes, vec!["openid"]);
        let body = &requests.lock().unwrap()[0];
        assert!(body.contains("code_verifier=pkce-verifier-value"));
        assert!(body.contains("grant_type=authorization_code"));
    }

    #[tokio::test]
    async fn binding_is_returned_only_after_authenticated_userinfo() {
        let (endpoint, _) = mock_server(vec![
            (
                StatusCode::OK,
                r#"{"access_token":"access-secret","refresh_token":"refresh-secret","expires_in":60,"scope":"openid"}"#,
            ),
            (
                StatusCode::OK,
                r#"{"sub":"google-subject","email":"owner@example.com","verified_email":true}"#,
            ),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let mut provider = GoogleOAuthProvider::with_endpoints(
            OAuthClientConfig {
                client_id: "client".into(),
                client_secret: None,
                redirect_uri: "http://localhost/callback".into(),
                auth_url: format!("{endpoint}/authorize"),
                token_url: format!("{endpoint}/token"),
                revocation_url: Some(format!("{endpoint}/revoke")),
                userinfo_url: Some(format!("{endpoint}/userinfo")),
            },
            vec![ExternalScope::OpenId],
            TokenStore::new(dir.path().join("tokens.json")).unwrap(),
            Arc::new(TestClock::default()),
        )
        .unwrap();
        let start = provider.start_authorization().unwrap();
        let binding = provider
            .complete_authorization("code", &start.state)
            .await
            .unwrap();
        assert_eq!(binding.provider_subject, "google-subject");
        assert_eq!(binding.email, "owner@example.com");
        assert_eq!(binding.scopes, vec![ExternalScope::OpenId]);
        let rendered = format!("{binding:?}");
        assert!(!rendered.contains("access-secret"));
        assert!(!rendered.contains("refresh-secret"));
    }

    #[tokio::test]
    async fn refresh_preserves_existing_refresh_token_when_omitted() {
        let (endpoint, _) = mock_server(vec![(
            StatusCode::OK,
            r#"{"access_token":"new-access","expires_in":60,"scope":"openid"}"#,
        )])
        .await;
        let current = TokenEntry {
            access_token: "old-access".into(),
            refresh_token: Some("durable-refresh".into()),
            expires_at: 1,
            token_type: "Bearer".into(),
            scopes: vec!["openid".into()],
        };
        let refreshed = async_client(&endpoint)
            .refresh(&current, &["openid".into()], 10)
            .await
            .unwrap();
        assert_eq!(refreshed.refresh_token.as_deref(), Some("durable-refresh"));
        assert_eq!(refreshed.expires_at, 70);
    }

    #[tokio::test]
    async fn escalation_pkce_rejection_and_provider_bodies_are_redacted() {
        let provider_secret = "provider-body-secret";
        let (endpoint, _) = mock_server(vec![
            (
                StatusCode::OK,
                r#"{"access_token":"secret","scope":"gmail.send"}"#,
            ),
            (StatusCode::BAD_REQUEST, provider_secret),
        ])
        .await;
        let client = async_client(&endpoint);
        let escalation = client
            .exchange_code("code", "verifier", &["gmail.readonly".into()], 0)
            .await
            .unwrap_err();
        assert!(!escalation.to_string().contains("secret"));
        let mismatch = client
            .exchange_code("code", "wrong-verifier", &["openid".into()], 0)
            .await
            .unwrap_err();
        assert!(!mismatch.to_string().contains(provider_secret));
        assert!(!mismatch.to_string().contains("wrong-verifier"));
    }

    #[tokio::test]
    async fn revocation_is_async_and_redacts_rejected_tokens() {
        let (endpoint, requests) = mock_server(vec![(StatusCode::OK, "{}")]).await;
        async_client(&endpoint)
            .revoke("revocation-token-secret")
            .await
            .unwrap();
        assert!(requests.lock().unwrap()[0].contains("token=revocation-token-secret"));

        let (endpoint, _) = mock_server(vec![(
            StatusCode::BAD_REQUEST,
            "revocation-token-secret provider details",
        )])
        .await;
        let error = async_client(&endpoint)
            .revoke("revocation-token-secret")
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("revocation-token-secret"));
    }

    #[tokio::test]
    async fn expired_state_is_consumed_before_network() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock::default());
        let mut provider = GoogleOAuthProvider::with_endpoints(
            OAuthClientConfig {
                client_id: "client".into(),
                client_secret: None,
                redirect_uri: "http://localhost/callback".into(),
                auth_url: "http://127.0.0.1:1/authorize".into(),
                token_url: "http://127.0.0.1:1/token".into(),
                revocation_url: None,
                userinfo_url: None,
            },
            vec![ExternalScope::OpenId],
            TokenStore::new(dir.path().join("tokens.json")).unwrap(),
            clock.clone(),
        )
        .unwrap();
        let start = provider.start_authorization().unwrap();
        clock.advance((STATE_LIFETIME_SECS + 1) * 1_000);
        let error = provider
            .complete_authorization("code", &start.state)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("expired"));
        let replay = provider
            .complete_authorization("code", &start.state)
            .await
            .unwrap_err();
        assert!(replay.to_string().contains("already-consumed"));
    }
}
