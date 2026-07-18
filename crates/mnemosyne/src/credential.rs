//! Endpoint-scoped embedding credentials (G7).
//!
//! Any remote embedding/search credential is bound to an exact normalized
//! origin. The grant is `approved_for` a request only when the request's base
//! URL is byte-equal to the approved one after normalization (fail-closed:
//! hostname-suffix matches and post-redirect origins are rejected). The secret
//! handle hides itself from `Debug` so it never leaks into logs/events/memory.
//!
//! See `docs/plans/grok/exec/G7-memory-search.md`.

/// Operations a grant may authorize. Embedding-only by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingOperation {
    EmbeddingOnly,
}

/// Opaque secret handle. `Debug` never reveals the value.
#[derive(Clone)]
pub struct SecretHandle(String);

impl SecretHandle {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// Reveal the secret. Callers must only use this at the moment of building
    /// an authorized request to an `approved_for` endpoint.
    pub(crate) fn reveal(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretHandle(***)")
    }
}

/// Credential grant for a remote embedding provider, bound to an exact origin.
#[derive(Clone)]
pub struct EmbeddingCredentialGrant {
    pub principal: fabric::PrincipalId,
    /// Normalized scheme+host+port+base-path. A request must match this exactly.
    pub approved_base_url: String,
    pub provider_id: String,
    pub operation: EmbeddingOperation,
    pub expiry_unix: u64,
    pub rotation_generation: u32,
    secret: SecretHandle,
}

impl std::fmt::Debug for EmbeddingCredentialGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingCredentialGrant")
            .field("principal", &self.principal)
            .field("approved_base_url", &self.approved_base_url)
            .field("provider_id", &self.provider_id)
            .field("operation", &self.operation)
            .field("expiry_unix", &self.expiry_unix)
            .field("rotation_generation", &self.rotation_generation)
            .field("secret", &self.secret)
            .finish()
    }
}

impl EmbeddingCredentialGrant {
    pub fn new(
        principal: impl Into<String>,
        approved_base_url: &str,
        provider_id: impl Into<String>,
        expiry_unix: u64,
        rotation_generation: u32,
        secret: impl Into<String>,
    ) -> Self {
        Self {
            principal: fabric::PrincipalId(principal.into()),
            approved_base_url: normalize_url(approved_base_url),
            provider_id: provider_id.into(),
            operation: EmbeddingOperation::EmbeddingOnly,
            expiry_unix,
            rotation_generation,
            secret: SecretHandle::new(secret),
        }
    }

    /// Fail-closed authorization: the request base URL must be byte-equal to the
    /// approved one after normalization, and the grant must be unexpired.
    /// Hostname-suffix widening and post-redirect origins never match.
    pub fn approved_for(&self, request_base_url: &str, now_unix: u64) -> bool {
        let requested = normalize_url(request_base_url);
        now_unix < self.expiry_unix
            && self.approved_base_url != "\0invalid"
            && requested != "\0invalid"
            && requested == self.approved_base_url
    }

    /// Reveal the secret only after `approved_for` has authorized the request.
    pub fn secret_if_approved(&self, request_base_url: &str, now_unix: u64) -> Option<&str> {
        if self.approved_for(request_base_url, now_unix) {
            Some(self.secret.reveal())
        } else {
            tracing::warn!(
                event = "memory.credential.rejected",
                provider = %self.provider_id,
                reason = "origin_or_expiry",
                "embedding credential rejected"
            );
            None
        }
    }
}

/// Normalize an origin for exact comparison: lowercase scheme + host, keep an
/// explicit port, strip a single trailing slash, drop query/fragment. Inputs
/// that do not look like `scheme://host[...]` normalize to a sentinel that can
/// never match a real approved URL (fail-closed).
pub fn normalize_url(url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else {
        return "\0invalid".to_string();
    };
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return "\0invalid".to_string();
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = parsed
        .port()
        .map(|value| format!(":{value}"))
        .unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    format!(
        "{}://{host}{port}{path}",
        parsed.scheme().to_ascii_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> EmbeddingCredentialGrant {
        EmbeddingCredentialGrant::new(
            "local-uid:1000",
            "https://api.embed.example.com/v1",
            "provider-x",
            10_000,
            1,
            "sk-secret-token",
        )
    }

    #[test]
    fn approved_for_exact_match() {
        let g = grant();
        assert!(g.approved_for("https://api.embed.example.com/v1", 5_000));
        // Trailing slash + case-insensitive host normalize to the same origin.
        assert!(g.approved_for("https://API.embed.example.com/v1/", 5_000));
    }

    #[test]
    fn approved_for_rejects_hostname_suffix_widening() {
        let g = grant();
        // Attacker-controlled subdomain / suffix must not match.
        assert!(!g.approved_for("https://api.embed.example.com.evil.com/v1", 5_000));
        assert!(!g.approved_for("https://evil-api.embed.example.com/v1", 5_000));
    }

    #[test]
    fn approved_for_rejects_different_scheme_port_path() {
        let g = grant();
        assert!(!g.approved_for("http://api.embed.example.com/v1", 5_000)); // scheme
        assert!(!g.approved_for("https://api.embed.example.com:8443/v1", 5_000)); // port
        assert!(!g.approved_for("https://api.embed.example.com/v2", 5_000)); // path
    }

    #[test]
    fn approved_for_rejects_expired() {
        let g = grant();
        assert!(!g.approved_for("https://api.embed.example.com/v1", 20_000));
    }

    #[test]
    fn secret_only_revealed_when_approved() {
        let g = grant();
        assert_eq!(
            g.secret_if_approved("https://api.embed.example.com/v1", 5_000),
            Some("sk-secret-token")
        );
        assert_eq!(
            g.secret_if_approved("https://evil.example.com/v1", 5_000),
            None
        );
    }

    #[test]
    fn debug_never_leaks_secret() {
        let g = grant();
        let dbg = format!("{g:?}");
        assert!(dbg.contains("SecretHandle(***)"));
        assert!(!dbg.contains("sk-secret-token"));
        // The handle alone, too.
        assert_eq!(
            format!("{:?}", SecretHandle::new("hunter2")),
            "SecretHandle(***)"
        );
    }

    #[test]
    fn normalize_url_invalid_inputs_never_match_real() {
        // Garbage normalizes to a sentinel distinct from any real origin.
        assert_eq!(normalize_url("not a url"), "\0invalid");
        assert_eq!(normalize_url("https://"), "\0invalid");
        assert_ne!(normalize_url("not a url"), normalize_url("https://a/b"));
        let invalid = EmbeddingCredentialGrant::new("p", "not a url", "x", 10, 0, "secret");
        assert!(!invalid.approved_for("also invalid", 1));
    }

    #[test]
    fn normalize_url_is_idempotent() {
        let once = normalize_url("HTTPS://Host.Example.COM/v1/");
        assert_eq!(normalize_url(&once), once);
        assert_eq!(once, "https://host.example.com/v1");
    }
}
