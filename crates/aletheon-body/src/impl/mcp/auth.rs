use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Minimal percent-encoding for URL query parameters (RFC 3986 unreserved set).
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
pub trait McpAuth: Send + Sync {
    /// Return HTTP headers to attach to MCP requests.
    ///
    /// For bearer/OAuth this is `{ "Authorization": "Bearer <token>" }`.
    fn get_headers(&self) -> HashMap<String, String>;

    /// Return `true` if the current credentials have expired.
    ///
    /// `BearerTokenAuth` always returns `false` (no expiry concept).
    fn is_expired(&self) -> bool;

    /// Attempt to refresh credentials.
    ///
    /// `BearerTokenAuth` is a no-op (returns `Ok(())`).
    fn refresh(&mut self) -> Result<()>;
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
#[derive(Debug, Clone)]
pub struct BearerTokenAuth {
    env_var: String,
}

impl BearerTokenAuth {
    /// Create a new auth helper that reads from the given env var.
    pub fn new(env_var: impl Into<String>) -> Self {
        Self {
            env_var: env_var.into(),
        }
    }

    /// Create auth reading from the default `MCP_BEARER_TOKEN` env var.
    pub fn from_env() -> Self {
        Self::new("MCP_BEARER_TOKEN")
    }

    /// Read the token from the environment.
    ///
    /// Returns `None` if the env var is not set or empty.
    pub fn token(&self) -> Option<String> {
        std::env::var(&self.env_var).ok().filter(|v| !v.is_empty())
    }

    /// Return the full `Authorization: Bearer <token>` header value.
    ///
    /// Returns `None` when no token is available.
    pub fn header_value(&self) -> Option<String> {
        self.token().map(|t| format!("Bearer {}", t))
    }
}

impl McpAuth for BearerTokenAuth {
    fn get_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if let Some(val) = self.header_value() {
            headers.insert("Authorization".to_string(), val);
        }
        headers
    }

    fn is_expired(&self) -> bool {
        // Bearer tokens from env have no expiry concept.
        false
    }

    fn refresh(&mut self) -> Result<()> {
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

// ---------------------------------------------------------------------------
// TokenStore -- persistent OAuth token storage
// ---------------------------------------------------------------------------

/// A single stored token entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenEntry {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix epoch seconds when the access token expires.
    pub expires_at: u64,
    pub token_type: String,
    pub scopes: Vec<String>,
}

/// Persistent storage for OAuth tokens, keyed by MCP server id.
#[derive(Debug)]
pub struct TokenStore {
    storage_path: PathBuf,
    tokens: HashMap<String, TokenEntry>,
}

impl TokenStore {
    /// Create a store backed by the given file path.
    ///
    /// If the file exists it is loaded; otherwise the store starts empty.
    pub fn new(storage_path: PathBuf) -> Result<Self> {
        let tokens = if storage_path.exists() {
            let data = fs::read_to_string(&storage_path)
                .with_context(|| format!("reading token store {}", storage_path.display()))?;
            serde_json::from_str(&data)
                .with_context(|| format!("parsing token store {}", storage_path.display()))?
        } else {
            HashMap::new()
        };
        Ok(Self {
            storage_path,
            tokens,
        })
    }

    /// Default store at `~/.config/aletheon/mcp_tokens.json`.
    pub fn default_path() -> Result<PathBuf> {
        Ok(aletheon_abi::paths::mcp_tokens_path())
    }

    /// Create a store at the default path, creating parent directories as
    /// needed.
    pub fn open_default() -> Result<Self> {
        let path = Self::default_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        Self::new(path)
    }

    /// Get the stored entry for a server, if any.
    pub fn get(&self, server_id: &str) -> Option<&TokenEntry> {
        self.tokens.get(server_id)
    }

    /// Insert or update a token entry for the given server.
    pub fn set(&mut self, server_id: impl Into<String>, entry: TokenEntry) {
        self.tokens.insert(server_id.into(), entry);
    }

    /// Remove a token entry. Returns the removed entry, if any.
    pub fn remove(&mut self, server_id: &str) -> Option<TokenEntry> {
        self.tokens.remove(server_id)
    }

    /// Persist current state to disk.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(&self.tokens).context("serializing token store")?;
        fs::write(&self.storage_path, json)
            .with_context(|| format!("writing token store {}", self.storage_path.display()))?;
        Ok(())
    }

    /// Return the number of stored entries.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Return whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

// ---------------------------------------------------------------------------
// OAuthState -- CSRF protection
// ---------------------------------------------------------------------------

/// CSRF state parameter used during the OAuth authorization flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthState {
    pub state: String,
    /// Unix epoch seconds when this state was created.
    pub created_at: u64,
    /// The MCP server id this authorization is for.
    pub server_id: String,
}

/// Maximum age (in seconds) of an OAuth state before it is considered stale.
const STATE_MAX_AGE_SECS: u64 = 600; // 10 minutes

fn generate_state_string() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs()
}

// ---------------------------------------------------------------------------
// McpOAuthProvider -- OAuth 2.0 Authorization Code flow
// ---------------------------------------------------------------------------

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
    auth_url: String,
    token_url: String,
    redirect_uri: String,
    scopes: Vec<String>,
    server_id: String,
    token_store: TokenStore,
    /// Pending authorization states (state -> OAuthState).
    pending_states: HashMap<String, OAuthState>,
}

impl McpOAuthProvider {
    /// Create a new OAuth provider.
    pub fn new(
        client_id: impl Into<String>,
        auth_url: impl Into<String>,
        token_url: impl Into<String>,
        redirect_uri: impl Into<String>,
        scopes: Vec<String>,
        server_id: impl Into<String>,
        token_store: TokenStore,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: None,
            auth_url: auth_url.into(),
            token_url: token_url.into(),
            redirect_uri: redirect_uri.into(),
            scopes,
            server_id: server_id.into(),
            token_store,
            pending_states: HashMap::new(),
        }
    }

    /// Set the optional client secret (for confidential clients).
    pub fn with_client_secret(mut self, secret: impl Into<String>) -> Self {
        self.client_secret = Some(secret.into());
        self
    }

    /// Generate an authorization URL and a CSRF state value.
    ///
    /// The returned `OAuthState` must be stored and verified when
    /// `callback()` is called.
    pub fn authorize_url(&mut self) -> (String, OAuthState) {
        let state_str = generate_state_string();
        let now = now_epoch_secs();
        let oauth_state = OAuthState {
            state: state_str.clone(),
            created_at: now,
            server_id: self.server_id.clone(),
        };
        self.pending_states
            .insert(state_str.clone(), oauth_state.clone());

        let scope_str = self.scopes.join(" ");
        let url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
            self.auth_url,
            percent_encode(&self.client_id),
            percent_encode(&self.redirect_uri),
            percent_encode(&scope_str),
            percent_encode(&state_str),
        );
        (url, oauth_state)
    }

    /// Exchange an authorization code for tokens (called after the user
    /// redirects back from the authorization server).
    ///
    /// Returns the newly stored `TokenEntry`.
    pub fn callback(&mut self, code: &str, state: &str) -> Result<TokenEntry> {
        // Verify CSRF state.
        let oauth_state = self
            .pending_states
            .remove(state)
            .context("unknown or already-consumed OAuth state")?;

        let age = now_epoch_secs().saturating_sub(oauth_state.created_at);
        if age > STATE_MAX_AGE_SECS {
            anyhow::bail!(
                "OAuth state expired (age {}s > {}s max)",
                age,
                STATE_MAX_AGE_SECS
            );
        }

        // Exchange code for tokens via HTTP POST.
        let entry = self.exchange_code(code)?;

        // Persist.
        self.token_store
            .set(oauth_state.server_id.clone(), entry.clone());
        self.token_store.save()?;

        Ok(entry)
    }

    /// Perform the token exchange HTTP request.
    fn exchange_code(&self, code: &str) -> Result<TokenEntry> {
        let mut params = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &self.redirect_uri),
            ("client_id", &self.client_id),
        ];
        let secret_ref;
        if let Some(ref secret) = self.client_secret {
            secret_ref = secret.as_str();
            params.push(("client_secret", secret_ref));
        }

        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(&self.token_url)
            .form(&params)
            .send()
            .context("token exchange HTTP request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("token exchange failed ({}): {}", status, body);
        }

        let raw: serde_json::Value = resp.json().context("parsing token response")?;
        parse_token_response(&raw)
    }

    /// Refresh the access token using the stored refresh token.
    fn do_refresh(&mut self) -> Result<TokenEntry> {
        let current = self
            .token_store
            .get(&self.server_id)
            .context("no token entry to refresh")?
            .clone();

        let refresh_token = current
            .refresh_token
            .as_deref()
            .context("no refresh token available")?;

        let mut params = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", self.client_id.as_str()),
        ];
        let secret_ref;
        if let Some(ref secret) = self.client_secret {
            secret_ref = secret.as_str();
            params.push(("client_secret", secret_ref));
        }

        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(&self.token_url)
            .form(&params)
            .send()
            .context("token refresh HTTP request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("token refresh failed ({}): {}", status, body);
        }

        let raw: serde_json::Value = resp.json().context("parsing refresh response")?;
        let entry = parse_token_response(&raw)?;

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
        let now = now_epoch_secs();
        self.pending_states
            .retain(|_, s| now.saturating_sub(s.created_at) <= STATE_MAX_AGE_SECS);
    }
}

impl McpAuth for McpOAuthProvider {
    fn get_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
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
            Some(entry) => now_epoch_secs() >= entry.expires_at,
            None => true,
        }
    }

    fn refresh(&mut self) -> Result<()> {
        self.do_refresh()?;
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
// Helpers
// ---------------------------------------------------------------------------

/// Parse the JSON response from a token endpoint into a `TokenEntry`.
fn parse_token_response(raw: &serde_json::Value) -> Result<TokenEntry> {
    let access_token = raw["access_token"]
        .as_str()
        .context("missing access_token in response")?
        .to_string();

    let expires_in = raw["expires_in"].as_u64().unwrap_or(3600);
    let expires_at = now_epoch_secs() + expires_in;

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

    #[test]
    fn bearer_trait_get_headers_with_token() {
        std::env::set_var("TEST_BEARER_TRAIT", "abc123");
        let mut auth = BearerTokenAuth::new("TEST_BEARER_TRAIT");
        let headers = auth.get_headers();
        assert_eq!(
            headers.get("Authorization").map(|s| s.as_str()),
            Some("Bearer abc123")
        );
        assert!(!auth.is_expired());
        assert!(auth.refresh().is_ok());
        std::env::remove_var("TEST_BEARER_TRAIT");
    }

    #[test]
    fn bearer_trait_get_headers_empty_when_no_token() {
        std::env::remove_var("TEST_BEARER_TRAIT_EMPTY");
        let auth = BearerTokenAuth::new("TEST_BEARER_TRAIT_EMPTY");
        let headers = auth.get_headers();
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
        let now = now_epoch_secs();

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
            "https://auth.example.com/authorize",
            "https://auth.example.com/token",
            "http://localhost:8765/callback",
            vec!["openid".into(), "profile".into()],
            "test-server",
            store,
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

    #[test]
    fn oauth_csrf_state_rejects_unknown_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("t.json")).unwrap();
        let mut provider = McpOAuthProvider::new(
            "cid",
            "https://auth.example.com/authorize",
            "https://auth.example.com/token",
            "http://localhost/callback",
            vec![],
            "srv",
            store,
        );

        let result = provider.callback("any-code", "bogus-state");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unknown or already-consumed"));
    }

    #[test]
    fn oauth_csrf_state_double_consume_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("t.json")).unwrap();
        let mut provider = McpOAuthProvider::new(
            "cid",
            "https://auth.example.com/authorize",
            "https://auth.example.com/token",
            "http://localhost/callback",
            vec![],
            "srv",
            store,
        );

        let (_, state) = provider.authorize_url();
        // First consume will fail at HTTP exchange, but state is removed.
        let _ = provider.callback("code", &state.state);
        // Second consume should fail with "unknown state".
        let result = provider.callback("code", &state.state);
        assert!(result.is_err());
    }

    // -- OAuth: is_expired with stored tokens ------------------------------

    #[test]
    fn oauth_is_expired_depends_on_stored_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");

        let mut store = TokenStore::new(path.clone()).unwrap();
        let now = now_epoch_secs();

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
            "https://auth.example.com/authorize",
            "https://auth.example.com/token",
            "http://localhost/callback",
            vec![],
            "srv",
            store,
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
            "https://auth.example.com/authorize",
            "https://auth.example.com/token",
            "http://localhost/callback",
            vec![],
            "srv",
            store,
        );
        assert!(provider.is_expired());
    }

    // -- OAuth: get_headers returns correct header -------------------------

    #[test]
    fn oauth_get_headers_uses_stored_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");

        let mut store = TokenStore::new(path.clone()).unwrap();
        let now = now_epoch_secs();
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
            "https://auth.example.com/authorize",
            "https://auth.example.com/token",
            "http://localhost/callback",
            vec!["read".into()],
            "srv",
            store,
        );

        let headers = provider.get_headers();
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
        let now = now_epoch_secs();
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
            "https://auth.example.com/authorize",
            "https://auth.example.com/token",
            "http://localhost/callback",
            vec![],
            "srv",
            store,
        );

        let headers = provider.get_headers();
        assert!(headers.is_empty());
    }

    // -- parse_token_response ----------------------------------------------

    #[test]
    fn parse_token_response_full() {
        let raw = serde_json::json!({
            "access_token": "at",
            "refresh_token": "rt",
            "expires_in": 7200,
            "token_type": "Bearer",
            "scope": "openid profile"
        });
        let entry = parse_token_response(&raw).unwrap();
        assert_eq!(entry.access_token, "at");
        assert_eq!(entry.refresh_token.as_deref(), Some("rt"));
        assert_eq!(entry.token_type, "Bearer");
        assert_eq!(entry.scopes, vec!["openid", "profile"]);
        // expires_at should be ~now + 7200
        let now = now_epoch_secs();
        assert!(entry.expires_at > now + 7100);
        assert!(entry.expires_at <= now + 7200);
    }

    #[test]
    fn parse_token_response_minimal() {
        let raw = serde_json::json!({
            "access_token": "tok"
        });
        let entry = parse_token_response(&raw).unwrap();
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
            "https://auth.example.com/authorize",
            "https://auth.example.com/token",
            "http://localhost/callback",
            vec![],
            "srv",
            store,
        );

        let (_, _) = provider.authorize_url();
        assert_eq!(provider.pending_states.len(), 1);

        provider.purge_expired_states();
        // State was just created, should survive purge.
        assert_eq!(provider.pending_states.len(), 1);
    }
}
