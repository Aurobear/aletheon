use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use anyhow::{Context, Result};
use fabric::Clock;
use serde::{Deserialize, Serialize};

pub use super::token_store::{TokenEntry, TokenStore};
use crate::tools::google::oauth::{AsyncOAuthClient, OAuthClientConfig};

/// MCP-owned authorization to release a credential only to one exact endpoint.
/// The credential itself remains in its environment variable or token store;
/// this grant contains no secret material.
#[derive(Clone)]
pub struct McpEndpointCredentialGrant {
    pub principal: fabric::PrincipalId,
    pub approved_base_url: String,
    pub server_id: String,
    pub expiry_unix: u64,
    pub rotation_generation: u32,
}

impl std::fmt::Debug for McpEndpointCredentialGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpEndpointCredentialGrant")
            .field("principal", &self.principal)
            .field("approved_base_url", &self.approved_base_url)
            .field("server_id", &self.server_id)
            .field("expiry_unix", &self.expiry_unix)
            .field("rotation_generation", &self.rotation_generation)
            .finish()
    }
}

impl McpEndpointCredentialGrant {
    pub fn new(
        principal: impl Into<String>,
        approved_base_url: &str,
        server_id: impl Into<String>,
        expiry_unix: u64,
        rotation_generation: u32,
    ) -> Self {
        Self {
            principal: fabric::PrincipalId(principal.into()),
            approved_base_url: normalize_endpoint(approved_base_url),
            server_id: server_id.into(),
            expiry_unix,
            rotation_generation,
        }
    }

    pub fn approved_for(&self, request_url: &str, now_unix: u64) -> bool {
        let requested = normalize_endpoint(request_url);
        now_unix < self.expiry_unix
            && self.approved_base_url != "\0invalid"
            && requested != "\0invalid"
            && requested == self.approved_base_url
    }
}

fn normalize_endpoint(url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else {
        return "\0invalid".into();
    };
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return "\0invalid".into();
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    format!(
        "{}://{host}{port}{path}",
        parsed.scheme().to_ascii_lowercase()
    )
}

/// Minimal percent-encoding for URL query parameters (RFC 3986 unreserved set).
#[cfg(test)]
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", byte));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// McpAuth trait
// ---------------------------------------------------------------------------

/// Common interface for MCP authentication providers.
///
/// Both `BearerTokenAuth` and `McpOAuthProvider` implement this trait so that
/// transports can be generic over the authentication mechanism.
#[async_trait::async_trait]
pub trait McpAuth: Send + Sync {
    /// Return HTTP headers to attach to MCP requests.
    ///
    /// For bearer/OAuth this is `{ "Authorization": "Bearer <token>" }`.
    ///
    /// When `target_url` is `Some(url)` and the provider has endpoint-scoping
    /// (e.g. a credential grant), the headers are only returned when the
    /// specific URL is approved.
    fn get_headers(&self, target_url: Option<&str>) -> HashMap<String, String>;

    /// Return `true` if the current credentials have expired.
    ///
    /// `BearerTokenAuth` always returns `false` (no expiry concept).
    fn is_expired(&self) -> bool;

    /// Attempt to refresh credentials.
    ///
    /// `BearerTokenAuth` is a no-op (returns `Ok(())`).
    async fn refresh(&mut self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// BearerTokenAuth (existing, now implements McpAuth)
// ---------------------------------------------------------------------------

/// Bearer token authentication for MCP HTTP transports.
///
/// Reads the token from the environment variable specified at construction
/// time (typically `MCP_BEARER_TOKEN`). The token is resolved lazily on
/// each call to `header_value()` so that env changes at runtime are picked
/// up without restarting.
///
/// When an `McpEndpointCredentialGrant` is set (via `with_endpoint_scoping`),
/// the token is only returned for requests whose target URL is approved by
/// the grant. Without a grant, the token is returned unconditionally
/// (backward compatible).
#[derive(Clone)]
pub struct BearerTokenAuth {
    env_var: String,
    grant: Option<McpEndpointCredentialGrant>,
    additional_grants: Vec<McpEndpointCredentialGrant>,
    /// Clock for checking grant expiry. `None` when no grant is set.
    clock: Option<Arc<dyn Clock>>,
}

impl BearerTokenAuth {
    /// Create a new auth helper that reads from the given env var.
    pub fn new(env_var: impl Into<String>) -> Self {
        Self {
            env_var: env_var.into(),
            grant: None,
            additional_grants: Vec::new(),
            clock: None,
        }
    }

    /// Create auth reading from the default `MCP_BEARER_TOKEN` env var.
    pub fn from_env() -> Self {
        Self::new("MCP_BEARER_TOKEN")
    }

    /// Create an auth helper with endpoint-scoping via a credential grant.
    ///
    /// The token is only returned for requests whose target URL is approved
    /// by the grant. Without endpoint-scoping (i.e. no grant), the token is
    /// returned unconditionally.
    pub fn with_endpoint_scoping(
        env_var: impl Into<String>,
        grant: McpEndpointCredentialGrant,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            env_var: env_var.into(),
            grant: Some(grant),
            additional_grants: Vec::new(),
            clock: Some(clock),
        }
    }

    /// Add another explicitly authorized endpoint for a transport that uses
    /// more than one URL (legacy SSE uses both the POST URL and `/sse`).
    pub fn allow_endpoint(mut self, grant: McpEndpointCredentialGrant) -> Self {
        self.additional_grants.push(grant);
        self
    }

    /// Read the token from the environment.
    ///
    /// Returns `None` if the env var is not set or empty.
    pub fn token(&self) -> Option<String> {
        std::env::var(&self.env_var).ok().filter(|v| !v.is_empty())
    }

    /// Return the full `Authorization: Bearer <token>` header value.
    ///
    /// When a credential grant is present and `target_url` is `Some`,
    /// the grant must approve the URL (fail-closed). When no grant is
    /// present, the token is returned unconditionally. When a grant is
    /// present but `target_url` is `None`, returns `None` (cannot
    /// verify without URL).
    ///
    /// **Deprecated-preferred**: Use `get_headers(target_url)` instead
    /// (the `McpAuth` trait method), which gates the header on the
    /// grant's endpoint-scoping check before exposing the token.
    pub fn header_value_for(&self, target_url: Option<&str>) -> Option<String> {
        let token = self.token()?;
        match (&self.grant, &self.clock, target_url) {
            // No grant → always allow (backward compatible).
            (None, _, _) => Some(format!("Bearer {}", token)),
            // Grant present but no URL → fail-closed.
            (Some(_), _, None) => None,
            // Grant present with URL → gate on approved_for.
            (Some(grant), Some(clock), Some(url)) => {
                let now = now_epoch_secs(&**clock);
                if grant.approved_for(url, now)
                    || self
                        .additional_grants
                        .iter()
                        .any(|additional| additional.approved_for(url, now))
                {
                    Some(format!("Bearer {}", token))
                } else {
                    None
                }
            }
            // Grant present but clock missing (should not happen).
            (Some(_), None, Some(_)) => None,
        }
    }

    /// Return the full `Authorization: Bearer <token>` header value
    /// without scoping. Used where the transport does not know the
    /// target URL (e.g. SSE long-poll setup).
    pub fn header_value(&self) -> Option<String> {
        self.token().map(|t| format!("Bearer {}", t))
    }
}

impl fmt::Debug for BearerTokenAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let has_token = self.token().is_some();
        let has_grant = self.grant.is_some();
        f.debug_struct("BearerTokenAuth")
            .field("env_var", &self.env_var)
            .field("has_token", &has_token)
            .field("has_grant", &has_grant)
            .finish()
    }
}

#[async_trait::async_trait]
impl McpAuth for BearerTokenAuth {
    fn get_headers(&self, target_url: Option<&str>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if let Some(val) = self.header_value_for(target_url) {
            headers.insert("Authorization".to_string(), val);
        }
        headers
    }

    fn is_expired(&self) -> bool {
        // Bearer tokens from env have no expiry concept.
        false
    }

    async fn refresh(&mut self) -> Result<()> {
        // Nothing to refresh -- token is read from env on demand.
        Ok(())
    }
}

impl fmt::Display for BearerTokenAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.token() {
            Some(_) => write!(f, "BearerTokenAuth(<redacted>)"),
            None => write!(f, "BearerTokenAuth(no token)"),
        }
    }
}

/// Cloneable HTTP authentication handle retained by MCP transports. OAuth
/// state is shared so callbacks and refreshes update subsequent requests.
#[derive(Clone)]
pub enum McpHttpAuth {
    Bearer(BearerTokenAuth),
    OAuth(Arc<parking_lot::Mutex<McpOAuthProvider>>),
}

impl From<BearerTokenAuth> for McpHttpAuth {
    fn from(value: BearerTokenAuth) -> Self {
        Self::Bearer(value)
    }
}

impl McpHttpAuth {
    pub fn header_value_for(&self, target_url: Option<&str>) -> Option<String> {
        match self {
            Self::Bearer(auth) => auth.header_value_for(target_url),
            Self::OAuth(auth) => auth.lock().get_headers(target_url).remove("Authorization"),
        }
    }

    pub fn oauth_provider(&self) -> Option<Arc<parking_lot::Mutex<McpOAuthProvider>>> {
        match self {
            Self::OAuth(provider) => Some(provider.clone()),
            Self::Bearer(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// OAuthState -- CSRF protection
// ---------------------------------------------------------------------------

/// CSRF state parameter used during the OAuth authorization flow.
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthState {
    pub state: String,
    /// Unix epoch seconds when this state was created.
    pub created_at: u64,
    /// The MCP server id this authorization is for.
    pub server_id: String,
    #[serde(skip)]
    pkce_verifier: String,
}

impl fmt::Debug for OAuthState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthState")
            .field("state", &self.state)
            .field("created_at", &self.created_at)
            .field("server_id", &self.server_id)
            .field("pkce_verifier", &"[REDACTED]")
            .finish()
    }
}

/// Maximum age (in seconds) of an OAuth state before it is considered stale.
const STATE_MAX_AGE_SECS: u64 = 600; // 10 minutes

fn generate_state_string() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn now_epoch_secs(clock: &dyn Clock) -> u64 {
    (clock.wall_now().0 as u64) / 1000
}

// ---------------------------------------------------------------------------
// McpOAuthProvider -- OAuth 2.0 Authorization Code flow
// ---------------------------------------------------------------------------

/// OAuth 2.0 endpoint URLs.
#[derive(Debug, Clone)]
pub struct OAuthEndpoints {
    pub auth_url: String,
    pub token_url: String,
    pub redirect_uri: String,
}

/// OAuth 2.0 authorization code flow for MCP servers.
///
/// Implements the standard three-legged OAuth flow:
/// 1. `authorize_url()` -- build the URL the user visits in a browser.
/// 2. `callback(code, state)` -- exchange the authorization code for tokens.
/// 3. `get_headers()` -- return an `Authorization` header, auto-refreshing if
///    the access token has expired.
pub struct McpOAuthProvider {
    client_id: String,
    client_secret: Option<String>,
    endpoints: OAuthEndpoints,
    scopes: Vec<String>,
    server_id: String,
    token_store: TokenStore,
    /// Pending authorization states (state -> OAuthState).
    pending_states: HashMap<String, OAuthState>,
    clock: Arc<dyn Clock>,
    oauth_client: AsyncOAuthClient,
    endpoint_grant: Option<McpEndpointCredentialGrant>,
}

impl McpOAuthProvider {
    /// Create a new OAuth provider.
    pub fn new(
        client_id: impl Into<String>,
        endpoints: OAuthEndpoints,
        scopes: Vec<String>,
        server_id: impl Into<String>,
        token_store: TokenStore,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let client_id = client_id.into();
        let oauth_client = AsyncOAuthClient::new(OAuthClientConfig {
            client_id: client_id.clone(),
            client_secret: None,
            redirect_uri: endpoints.redirect_uri.clone(),
            auth_url: endpoints.auth_url.clone(),
            token_url: endpoints.token_url.clone(),
            revocation_url: None,
            userinfo_url: None,
            client_auth_method: crate::tools::google::oauth::OAuthClientAuthMethod::None,
        })
        .expect("static MCP OAuth client configuration must build");
        Self {
            client_id,
            client_secret: None,
            endpoints,
            scopes,
            server_id: server_id.into(),
            token_store,
            pending_states: HashMap::new(),
            clock,
            oauth_client,
            endpoint_grant: None,
        }
    }

    /// Restrict token release to the configured MCP endpoint. OAuth tokens are
    /// fail-closed until this grant is installed.
    pub fn with_endpoint_scoping(mut self, approved_base_url: &str) -> Self {
        self.endpoint_grant = Some(McpEndpointCredentialGrant::new(
            format!("mcp:{}", self.server_id),
            approved_base_url,
            self.server_id.clone(),
            u64::MAX,
            0,
        ));
        self
    }

    /// Set the optional client secret (for confidential clients).
    pub fn with_client_secret(mut self, secret: impl Into<String>) -> Self {
        let secret = secret.into();
        self.client_secret = Some(secret.clone());
        self.oauth_client = AsyncOAuthClient::new(OAuthClientConfig {
            client_id: self.client_id.clone(),
            client_secret: Some(secret),
            redirect_uri: self.endpoints.redirect_uri.clone(),
            auth_url: self.endpoints.auth_url.clone(),
            token_url: self.endpoints.token_url.clone(),
            revocation_url: None,
            userinfo_url: None,
            client_auth_method: crate::tools::google::oauth::OAuthClientAuthMethod::None,
        })
        .expect("static MCP OAuth client configuration must build");
        self
    }

    pub fn with_client_auth_method(mut self, method: OAuthClientAuthMethod) -> Result<Self> {
        if method != OAuthClientAuthMethod::None && self.client_secret.is_none() {
            anyhow::bail!("confidential OAuth client auth requires client_secret_env");
        }
        let client_auth_method = match method {
            OAuthClientAuthMethod::None => crate::tools::google::oauth::OAuthClientAuthMethod::None,
            OAuthClientAuthMethod::ClientSecretBasic => {
                crate::tools::google::oauth::OAuthClientAuthMethod::ClientSecretBasic
            }
            OAuthClientAuthMethod::ClientSecretPost => {
                crate::tools::google::oauth::OAuthClientAuthMethod::ClientSecretPost
            }
        };
        self.oauth_client = AsyncOAuthClient::new(OAuthClientConfig {
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            redirect_uri: self.endpoints.redirect_uri.clone(),
            auth_url: self.endpoints.auth_url.clone(),
            token_url: self.endpoints.token_url.clone(),
            revocation_url: None,
            userinfo_url: None,
            client_auth_method,
        })?;
        Ok(self)
    }

    /// Generate an authorization URL and a CSRF state value.
    ///
    /// The returned `OAuthState` must be stored and verified when
    /// `callback()` is called.
    pub fn authorize_url(&mut self) -> (String, OAuthState) {
        let state_str = generate_state_string();
        let now = now_epoch_secs(&*self.clock);
        let oauth_state = OAuthState {
            state: state_str.clone(),
            created_at: now,
            server_id: self.server_id.clone(),
            pkce_verifier: crate::tools::google::oauth::generate_pkce_verifier(),
        };
        self.pending_states
            .insert(state_str.clone(), oauth_state.clone());

        let challenge = crate::tools::google::oauth::pkce_challenge(&oauth_state.pkce_verifier);
        let url = self
            .oauth_client
            .authorization_url(&state_str, &challenge, &self.scopes, false)
            .expect("validated MCP OAuth authorization URL");
        (url, oauth_state)
    }

    /// Exchange an authorization code for tokens (called after the user
    /// redirects back from the authorization server).
    ///
    /// Returns the newly stored `TokenEntry`.
    pub async fn callback(&mut self, code: &str, state: &str) -> Result<TokenEntry> {
        // Verify CSRF state.
        let oauth_state = self
            .pending_states
            .remove(state)
            .context("unknown or already-consumed OAuth state")?;

        let age = now_epoch_secs(&*self.clock).saturating_sub(oauth_state.created_at);
        if age > STATE_MAX_AGE_SECS {
            anyhow::bail!(
                "OAuth state expired (age {}s > {}s max)",
                age,
                STATE_MAX_AGE_SECS
            );
        }

        // Exchange code for tokens via HTTP POST.
        let entry = self.exchange_code(code, &oauth_state.pkce_verifier).await?;

        // Persist.
        self.token_store
            .set(oauth_state.server_id.clone(), entry.clone());
        self.token_store.save()?;

        Ok(entry)
    }

    /// Perform the token exchange HTTP request.
    async fn exchange_code(&self, code: &str, verifier: &str) -> Result<TokenEntry> {
        self.oauth_client
            .exchange_code(code, verifier, &self.scopes, now_epoch_secs(&*self.clock))
            .await
    }

    /// Refresh the access token using the stored refresh token.
    async fn do_refresh(&mut self) -> Result<TokenEntry> {
        let current = self
            .token_store
            .get(&self.server_id)
            .context("no token entry to refresh")?
            .clone();

        let entry = self
            .oauth_client
            .refresh(&current, &self.scopes, now_epoch_secs(&*self.clock))
            .await?;

        self.token_store.set(&self.server_id, entry.clone());
        self.token_store.save()?;

        Ok(entry)
    }

    /// Return the current token entry, if stored.
    pub fn current_token(&self) -> Option<&TokenEntry> {
        self.token_store.get(&self.server_id)
    }

    /// Return the server id this provider is configured for.
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    /// Clear any expired pending states (housekeeping).
    pub fn purge_expired_states(&mut self) {
        let now = now_epoch_secs(&*self.clock);
        self.pending_states
            .retain(|_, s| now.saturating_sub(s.created_at) <= STATE_MAX_AGE_SECS);
    }
}

#[async_trait::async_trait]
impl McpAuth for McpOAuthProvider {
    fn get_headers(&self, target_url: Option<&str>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        let Some(grant) = &self.endpoint_grant else {
            return headers;
        };
        let Some(target_url) = target_url else {
            return headers;
        };
        if !grant.approved_for(target_url, now_epoch_secs(&*self.clock)) {
            return headers;
        }
        if let Some(entry) = self.token_store.get(&self.server_id) {
            if !self.is_expired() {
                let value = format!("{} {}", entry.token_type, entry.access_token);
                headers.insert("Authorization".to_string(), value);
            }
        }
        headers
    }

    fn is_expired(&self) -> bool {
        match self.token_store.get(&self.server_id) {
            Some(entry) => now_epoch_secs(&*self.clock) >= entry.expires_at,
            None => true,
        }
    }

    async fn refresh(&mut self) -> Result<()> {
        self.do_refresh().await?;
        Ok(())
    }
}

impl fmt::Display for McpOAuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "McpOAuthProvider(server={}, client={})",
            self.server_id, self.client_id
        )
    }
}

// ---------------------------------------------------------------------------
// OAuth Metadata Discovery (RFC 8414)
// ---------------------------------------------------------------------------

/// OAuth 2.0 Authorization Server Metadata per RFC 8414.
///
/// Discovered from `/.well-known/oauth-authorization-server` at the MCP
/// server's base URL. When the server config includes OAuth settings, the
/// discovery endpoint is tried first; if it fails, the configured endpoints
/// are used as a fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

/// Discover OAuth 2.0 authorization server metadata per RFC 8414.
///
/// Fetches `{base_url}/.well-known/oauth-authorization-server` and parses
/// the JSON response. Returns an error when the endpoint is unreachable,
/// returns non-200, or the response fails to parse; callers should fall
/// back to statically configured endpoints.
pub async fn discover_oauth_metadata(base_url: &str) -> Result<OAuthMetadata> {
    discover_oauth_metadata_guarded(base_url, crate::tools::outbound::EndpointPolicy::public())
        .await
}

pub(crate) async fn discover_oauth_metadata_guarded(
    base_url: &str,
    policy: crate::tools::outbound::EndpointPolicy,
) -> Result<OAuthMetadata> {
    let url = format!(
        "{}/.well-known/oauth-authorization-server",
        base_url.trim_end_matches('/')
    );

    policy
        .approve(&url)
        .await
        .context("OAuth discovery endpoint denied")?;
    let client = policy
        .client(std::time::Duration::from_secs(10))
        .context("failed to build OAuth discovery HTTP client")?;

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("OAuth discovery request failed for {url}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("OAuth discovery returned HTTP {status} from {url}: {body}");
    }

    let metadata = resp.json::<OAuthMetadata>().await.with_context(|| {
        format!("OAuth discovery response from {url} is not valid RFC 8414 metadata")
    })?;
    validate_oauth_metadata(base_url, &metadata)?;
    Ok(metadata)
}

fn validate_oauth_metadata(base_url: &str, metadata: &OAuthMetadata) -> Result<()> {
    let configured_issuer =
        reqwest::Url::parse(base_url).context("invalid configured OAuth issuer")?;
    let returned_issuer =
        reqwest::Url::parse(&metadata.issuer).context("invalid issuer in OAuth metadata")?;
    anyhow::ensure!(
        configured_issuer.as_str().trim_end_matches('/')
            == returned_issuer.as_str().trim_end_matches('/'),
        "OAuth discovery issuer does not match configured issuer"
    );
    for endpoint in [&metadata.authorization_endpoint, &metadata.token_endpoint] {
        let endpoint = reqwest::Url::parse(endpoint).context("invalid OAuth metadata endpoint")?;
        anyhow::ensure!(
            endpoint.scheme() == "https"
                || endpoint.host_str() == Some("127.0.0.1")
                || endpoint.host_str() == Some("localhost"),
            "OAuth metadata endpoint must use HTTPS"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// OAuth Client Authentication Methods
// ---------------------------------------------------------------------------

/// Client authentication method for the OAuth token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OAuthClientAuthMethod {
    /// No client authentication (public client).
    #[default]
    None,
    /// HTTP Basic Authentication with client_id as username and
    /// client_secret as password (`Authorization: Basic ...` header).
    ClientSecretBasic,
    /// Send client_id and client_secret as POST body parameters.
    ClientSecretPost,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse the JSON response from a token endpoint into a `TokenEntry`.
#[cfg(test)]
fn parse_token_response(raw: &serde_json::Value, clock: &dyn Clock) -> Result<TokenEntry> {
    let access_token = raw["access_token"]
        .as_str()
        .context("missing access_token in response")?
        .to_string();

    let expires_in = raw["expires_in"].as_u64().unwrap_or(3600);
    let expires_at = now_epoch_secs(clock) + expires_in;

    let token_type = raw["token_type"].as_str().unwrap_or("Bearer").to_string();

    let refresh_token = raw["refresh_token"].as_str().map(String::from);

    let scopes = raw["scope"]
        .as_str()
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default();

    Ok(TokenEntry {
        access_token,
        refresh_token,
        expires_at,
        token_type,
        scopes,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;
    use std::sync::Arc;

    fn test_clock() -> Arc<TestClock> {
        Arc::new(TestClock::default())
    }

    #[test]
    fn endpoint_grant_is_exact_expiring_and_secret_free() {
        let grant = McpEndpointCredentialGrant::new(
            "mcp:test",
            "https://MCP.example.test/rpc/",
            "test",
            100,
            1,
        );
        assert!(grant.approved_for("https://mcp.example.test/rpc", 99));
        assert!(!grant.approved_for("https://mcp.example.test/rpc/child", 99));
        assert!(!grant.approved_for("https://mcp.example.test.evil/rpc", 99));
        assert!(!grant.approved_for("https://mcp.example.test/rpc", 100));
        assert!(!format!("{grant:?}").contains("token"));
    }

    // -- BearerTokenAuth tests (existing + trait) --------------------------

    #[test]
    fn reads_token_from_env() {
        let auth = BearerTokenAuth::new("TEST_MCP_TOKEN_READ");
        assert!(auth.token().is_none());

        std::env::set_var("TEST_MCP_TOKEN_READ", "secret123");
        assert_eq!(auth.token().as_deref(), Some("secret123"));
        assert_eq!(auth.header_value().as_deref(), Some("Bearer secret123"));
        std::env::remove_var("TEST_MCP_TOKEN_READ");
    }

    #[test]
    fn empty_env_returns_none() {
        std::env::set_var("TEST_MCP_TOKEN_EMPTY", "");
        let auth = BearerTokenAuth::new("TEST_MCP_TOKEN_EMPTY");
        assert!(auth.token().is_none());
        assert!(auth.header_value().is_none());
        std::env::remove_var("TEST_MCP_TOKEN_EMPTY");
    }

    #[test]
    fn missing_env_returns_none() {
        std::env::remove_var("TEST_MCP_TOKEN_MISSING");
        let auth = BearerTokenAuth::new("TEST_MCP_TOKEN_MISSING");
        assert!(auth.token().is_none());
    }

    #[test]
    fn display_redacts_token() {
        std::env::set_var("TEST_MCP_TOKEN_DISPLAY", "supersecret");
        let auth = BearerTokenAuth::new("TEST_MCP_TOKEN_DISPLAY");
        let display = format!("{}", auth);
        assert!(!display.contains("supersecret"));
        assert!(display.contains("redacted"));
        std::env::remove_var("TEST_MCP_TOKEN_DISPLAY");
    }

    #[tokio::test]
    async fn bearer_trait_get_headers_with_token() {
        std::env::set_var("TEST_BEARER_TRAIT", "abc123");
        let mut auth = BearerTokenAuth::new("TEST_BEARER_TRAIT");
        let headers = auth.get_headers(None);
        assert_eq!(
            headers.get("Authorization").map(|s| s.as_str()),
            Some("Bearer abc123")
        );
        assert!(!auth.is_expired());
        assert!(auth.refresh().await.is_ok());
        std::env::remove_var("TEST_BEARER_TRAIT");
    }

    #[test]
    fn bearer_trait_get_headers_empty_when_no_token() {
        std::env::remove_var("TEST_BEARER_TRAIT_EMPTY");
        let auth = BearerTokenAuth::new("TEST_BEARER_TRAIT_EMPTY");
        let headers = auth.get_headers(None);
        assert!(headers.is_empty());
    }

    // -- TokenStore tests --------------------------------------------------

    #[test]
    fn token_store_load_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");

        // Save
        let mut store = TokenStore::new(path.clone()).unwrap();
        assert!(store.is_empty());
        store.set(
            "server-a",
            TokenEntry {
                access_token: "at_123".into(),
                refresh_token: Some("rt_456".into()),
                expires_at: 9999999999,
                token_type: "Bearer".into(),
                scopes: vec!["read".into(), "write".into()],
            },
        );
        store.save().unwrap();
        assert!(path.exists());

        // Load
        let store2 = TokenStore::new(path.clone()).unwrap();
        assert_eq!(store2.len(), 1);
        let entry = store2.get("server-a").unwrap();
        assert_eq!(entry.access_token, "at_123");
        assert_eq!(entry.refresh_token.as_deref(), Some("rt_456"));
        assert_eq!(entry.scopes, vec!["read", "write"]);
    }

    #[test]
    fn token_store_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");
        let mut store = TokenStore::new(path).unwrap();
        store.set(
            "s1",
            TokenEntry {
                access_token: "t".into(),
                refresh_token: None,
                expires_at: 0,
                token_type: "Bearer".into(),
                scopes: vec![],
            },
        );
        assert_eq!(store.len(), 1);
        let removed = store.remove("s1");
        assert!(removed.is_some());
        assert!(store.is_empty());
        assert!(store.get("s1").is_none());
    }

    // -- Token expiry ------------------------------------------------------

    #[test]
    fn token_entry_expired_detection() {
        let clock = test_clock();
        let now = now_epoch_secs(&*clock);

        let expired = TokenEntry {
            access_token: "old".into(),
            refresh_token: None,
            expires_at: now.saturating_sub(100),
            token_type: "Bearer".into(),
            scopes: vec![],
        };
        assert!(now >= expired.expires_at);

        let valid = TokenEntry {
            access_token: "new".into(),
            refresh_token: None,
            expires_at: now + 3600,
            token_type: "Bearer".into(),
            scopes: vec![],
        };
        assert!(now < valid.expires_at);
    }

    // -- OAuth provider: authorize_url -------------------------------------

    #[test]
    fn oauth_authorize_url_contains_required_params() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("t.json")).unwrap();
        let mut provider = McpOAuthProvider::new(
            "my-client-id",
            OAuthEndpoints {
                auth_url: "https://auth.example.com/authorize".into(),
                token_url: "https://auth.example.com/token".into(),
                redirect_uri: "http://localhost:8765/callback".into(),
            },
            vec!["openid".into(), "profile".into()],
            "test-server",
            store,
            test_clock(),
        );

        let (url, state) = provider.authorize_url();

        assert!(url.starts_with("https://auth.example.com/authorize"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=my-client-id"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope=openid+profile"));
        assert!(url.contains(&format!("state={}", percent_encode(&state.state))));

        // State should be UUID v4 format.
        assert!(uuid::Uuid::parse_str(&state.state).is_ok());
        assert_eq!(state.server_id, "test-server");
    }

    // -- CSRF state verification -------------------------------------------

    #[tokio::test]
    async fn oauth_csrf_state_rejects_unknown_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("t.json")).unwrap();
        let mut provider = McpOAuthProvider::new(
            "cid",
            OAuthEndpoints {
                auth_url: "https://auth.example.com/authorize".into(),
                token_url: "https://auth.example.com/token".into(),
                redirect_uri: "http://localhost/callback".into(),
            },
            vec![],
            "srv",
            store,
            test_clock(),
        );

        let result = provider.callback("any-code", "bogus-state").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unknown or already-consumed"));
    }

    #[tokio::test]
    async fn oauth_csrf_state_double_consume_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("t.json")).unwrap();
        let mut provider = McpOAuthProvider::new(
            "cid",
            OAuthEndpoints {
                auth_url: "https://auth.example.com/authorize".into(),
                token_url: "https://auth.example.com/token".into(),
                redirect_uri: "http://localhost/callback".into(),
            },
            vec![],
            "srv",
            store,
            test_clock(),
        );

        let (_, state) = provider.authorize_url();
        // First consume will fail at HTTP exchange, but state is removed.
        let _ = provider.callback("code", &state.state).await;
        // Second consume should fail with "unknown state".
        let result = provider.callback("code", &state.state).await;
        assert!(result.is_err());
    }

    // -- OAuth: is_expired with stored tokens ------------------------------

    #[test]
    fn oauth_is_expired_depends_on_stored_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");

        let mut store = TokenStore::new(path.clone()).unwrap();
        let clock = test_clock();
        let now = now_epoch_secs(&*clock);

        // Not expired
        store.set(
            "srv",
            TokenEntry {
                access_token: "valid".into(),
                refresh_token: None,
                expires_at: now + 3600,
                token_type: "Bearer".into(),
                scopes: vec![],
            },
        );
        store.save().unwrap();

        let store = TokenStore::new(path.clone()).unwrap();
        let provider = McpOAuthProvider::new(
            "cid",
            OAuthEndpoints {
                auth_url: "https://auth.example.com/authorize".into(),
                token_url: "https://auth.example.com/token".into(),
                redirect_uri: "http://localhost/callback".into(),
            },
            vec![],
            "srv",
            store,
            test_clock(),
        );
        assert!(!provider.is_expired());

        // Expired
        let mut store = TokenStore::new(path.clone()).unwrap();
        store.set(
            "srv",
            TokenEntry {
                access_token: "old".into(),
                refresh_token: Some("rt".into()),
                expires_at: now.saturating_sub(10),
                token_type: "Bearer".into(),
                scopes: vec![],
            },
        );
        store.save().unwrap();

        let store = TokenStore::new(path).unwrap();
        let provider = McpOAuthProvider::new(
            "cid",
            OAuthEndpoints {
                auth_url: "https://auth.example.com/authorize".into(),
                token_url: "https://auth.example.com/token".into(),
                redirect_uri: "http://localhost/callback".into(),
            },
            vec![],
            "srv",
            store,
            test_clock(),
        );
        assert!(provider.is_expired());
    }

    // -- OAuth: get_headers returns correct header -------------------------

    #[test]
    fn oauth_get_headers_uses_stored_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");

        let mut store = TokenStore::new(path.clone()).unwrap();
        let clock = test_clock();
        let now = now_epoch_secs(&*clock);
        store.set(
            "srv",
            TokenEntry {
                access_token: "my_access_token".into(),
                refresh_token: None,
                expires_at: now + 3600,
                token_type: "Bearer".into(),
                scopes: vec!["read".into()],
            },
        );
        store.save().unwrap();

        let store = TokenStore::new(path).unwrap();
        let provider = McpOAuthProvider::new(
            "cid",
            OAuthEndpoints {
                auth_url: "https://auth.example.com/authorize".into(),
                token_url: "https://auth.example.com/token".into(),
                redirect_uri: "http://localhost/callback".into(),
            },
            vec!["read".into()],
            "srv",
            store,
            test_clock(),
        )
        .with_endpoint_scoping("https://mcp.example.com/rpc");

        assert!(provider.get_headers(None).is_empty());
        assert!(provider
            .get_headers(Some("https://mcp.example.com.evil/rpc"))
            .is_empty());
        assert!(provider
            .get_headers(Some("https://redirected.example.com/rpc"))
            .is_empty());
        let headers = provider.get_headers(Some("https://mcp.example.com/rpc"));
        assert_eq!(
            headers.get("Authorization").map(|s| s.as_str()),
            Some("Bearer my_access_token")
        );
    }

    #[test]
    fn oauth_get_headers_empty_when_expired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");

        let mut store = TokenStore::new(path.clone()).unwrap();
        let clock = test_clock();
        let now = now_epoch_secs(&*clock);
        store.set(
            "srv",
            TokenEntry {
                access_token: "expired_token".into(),
                refresh_token: Some("rt".into()),
                expires_at: now.saturating_sub(60),
                token_type: "Bearer".into(),
                scopes: vec![],
            },
        );
        store.save().unwrap();

        let store = TokenStore::new(path).unwrap();
        let provider = McpOAuthProvider::new(
            "cid",
            OAuthEndpoints {
                auth_url: "https://auth.example.com/authorize".into(),
                token_url: "https://auth.example.com/token".into(),
                redirect_uri: "http://localhost/callback".into(),
            },
            vec![],
            "srv",
            store,
            test_clock(),
        )
        .with_endpoint_scoping("https://mcp.example.com/rpc");

        let headers = provider.get_headers(Some("https://mcp.example.com/rpc"));
        assert!(headers.is_empty());
    }

    #[test]
    fn confidential_client_auth_rejects_missing_secret() {
        let dir = tempfile::tempdir().unwrap();
        let provider = McpOAuthProvider::new(
            "cid",
            OAuthEndpoints {
                auth_url: "https://auth.example.test/authorize".into(),
                token_url: "https://auth.example.test/token".into(),
                redirect_uri: "http://127.0.0.1/callback".into(),
            },
            vec![],
            "srv",
            TokenStore::new(dir.path().join("tokens.json")).unwrap(),
            test_clock(),
        );
        assert!(provider
            .with_client_auth_method(OAuthClientAuthMethod::ClientSecretBasic)
            .is_err());
    }

    #[test]
    fn oauth_metadata_rejects_issuer_mismatch_and_insecure_remote_endpoints() {
        let valid = OAuthMetadata {
            issuer: "https://issuer.example.test".into(),
            authorization_endpoint: "https://issuer.example.test/authorize".into(),
            token_endpoint: "https://issuer.example.test/token".into(),
            token_endpoint_auth_methods_supported: vec![],
            scopes_supported: vec![],
        };
        assert!(validate_oauth_metadata("https://other.example.test", &valid).is_err());

        let mut insecure = valid.clone();
        insecure.authorization_endpoint = "http://issuer.example.test/authorize".into();
        assert!(validate_oauth_metadata("https://issuer.example.test", &insecure).is_err());
    }

    // -- parse_token_response ----------------------------------------------

    #[test]
    fn parse_token_response_full() {
        let clock = test_clock();
        let raw = serde_json::json!({
            "access_token": "at",
            "refresh_token": "rt",
            "expires_in": 7200,
            "token_type": "Bearer",
            "scope": "openid profile"
        });
        let entry = parse_token_response(&raw, &*clock).unwrap();
        assert_eq!(entry.access_token, "at");
        assert_eq!(entry.refresh_token.as_deref(), Some("rt"));
        assert_eq!(entry.token_type, "Bearer");
        assert_eq!(entry.scopes, vec!["openid", "profile"]);
        // expires_at should be ~now + 7200
        let now = now_epoch_secs(&*clock);
        assert!(entry.expires_at > now + 7100);
        assert!(entry.expires_at <= now + 7200);
    }

    #[test]
    fn parse_token_response_minimal() {
        let clock = test_clock();
        let raw = serde_json::json!({
            "access_token": "tok"
        });
        let entry = parse_token_response(&raw, &*clock).unwrap();
        assert_eq!(entry.access_token, "tok");
        assert!(entry.refresh_token.is_none());
        assert_eq!(entry.token_type, "Bearer");
        assert!(entry.scopes.is_empty());
    }

    // -- OAuth purge_expired_states ----------------------------------------

    #[test]
    fn oauth_purge_expired_states() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("t.json")).unwrap();
        let mut provider = McpOAuthProvider::new(
            "cid",
            OAuthEndpoints {
                auth_url: "https://auth.example.com/authorize".into(),
                token_url: "https://auth.example.com/token".into(),
                redirect_uri: "http://localhost/callback".into(),
            },
            vec![],
            "srv",
            store,
            test_clock(),
        );

        let (_, _) = provider.authorize_url();
        assert_eq!(provider.pending_states.len(), 1);

        provider.purge_expired_states();
        // State was just created, should survive purge.
        assert_eq!(provider.pending_states.len(), 1);
    }

    // -- OAuth discovery (RFC 8414) ---------------------------------------

    #[tokio::test]
    async fn discover_oauth_metadata_parses_valid_rfc8414_response() {
        use http_body_util::Full;
        use hyper::body::Bytes;
        use hyper::server::conn::http1;
        use hyper::service::service_fn;
        use hyper_util::rt::TokioIo;

        let addr: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let bound_addr = listener.local_addr().unwrap();
        let base_url = format!("http://{bound_addr}");
        let advertised_issuer = base_url.clone();

        let server_task = tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => return,
                };
                let io = TokioIo::new(stream);
                let advertised_issuer = advertised_issuer.clone();
                let svc = service_fn(move |_req: hyper::Request<hyper::body::Incoming>| {
                    let advertised_issuer = advertised_issuer.clone();
                    async move {
                        let body = serde_json::json!({
                            "issuer": advertised_issuer,
                            "authorization_endpoint": format!("{advertised_issuer}/authorize"),
                            "token_endpoint": format!("{advertised_issuer}/token"),
                            "token_endpoint_auth_methods_supported": [
                                "client_secret_basic",
                                "client_secret_post"
                            ],
                            "scopes_supported": ["openid", "profile", "email"]
                        })
                        .to_string();
                        Ok::<_, std::convert::Infallible>(
                            hyper::Response::builder()
                                .status(200)
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from(body)))
                                .unwrap(),
                        )
                    }
                });
                if http1::Builder::new()
                    .serve_connection(io, svc)
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });

        let metadata = discover_oauth_metadata_guarded(
            &base_url,
            crate::tools::outbound::EndpointPolicy::local_loopback(),
        )
        .await
        .unwrap();
        assert_eq!(metadata.issuer, base_url);
        assert_eq!(
            metadata.authorization_endpoint,
            format!("{base_url}/authorize")
        );
        assert_eq!(metadata.token_endpoint, format!("{base_url}/token"));
        assert_eq!(
            metadata.token_endpoint_auth_methods_supported,
            vec!["client_secret_basic", "client_secret_post"]
        );
        assert_eq!(
            metadata.scopes_supported,
            vec!["openid", "profile", "email"]
        );

        server_task.abort();
    }

    #[tokio::test]
    async fn discover_oauth_metadata_404_returns_error_not_panic() {
        use http_body_util::Full;
        use hyper::body::Bytes;
        use hyper::server::conn::http1;
        use hyper::service::service_fn;
        use hyper_util::rt::TokioIo;

        let addr: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let bound_addr = listener.local_addr().unwrap();
        let base_url = format!("http://{bound_addr}");

        let server_task = tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => return,
                };
                let io = TokioIo::new(stream);
                let svc = service_fn(|_req: hyper::Request<hyper::body::Incoming>| async move {
                    Ok::<_, std::convert::Infallible>(
                        hyper::Response::builder()
                            .status(404)
                            .body(Full::new(Bytes::from("not found")))
                            .unwrap(),
                    )
                });
                if http1::Builder::new()
                    .serve_connection(io, svc)
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });

        let result = discover_oauth_metadata_guarded(
            &base_url,
            crate::tools::outbound::EndpointPolicy::local_loopback(),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("404"), "expected 404 in error: {err}");
        server_task.abort();
    }

    #[tokio::test]
    async fn discover_oauth_metadata_rejects_redirects() {
        use http_body_util::Full;
        use hyper::body::Bytes;
        use hyper::server::conn::http1;
        use hyper::service::service_fn;
        use hyper_util::rt::TokioIo;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let response_target =
                "https://other.example.test/.well-known/oauth-authorization-server";
            let service = service_fn(move |_request| async move {
                Ok::<_, std::convert::Infallible>(
                    hyper::Response::builder()
                        .status(302)
                        .header("location", response_target)
                        .body(Full::new(Bytes::new()))
                        .unwrap(),
                )
            });
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });

        let error = discover_oauth_metadata_guarded(
            &base_url,
            crate::tools::outbound::EndpointPolicy::local_loopback(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("302"), "unexpected redirect error: {error}");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn discover_oauth_metadata_connection_refused_returns_error_not_panic() {
        let unreachable_url = "http://127.0.0.1:1";
        let result = discover_oauth_metadata_guarded(
            unreachable_url,
            crate::tools::outbound::EndpointPolicy::local_loopback(),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains(unreachable_url),
            "expected error to mention url: {err}"
        );
    }
}
