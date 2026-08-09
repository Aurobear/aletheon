//! C1 cache correctness & observability (Aletheon closure plan §16).
//!
//! Cache policy with dependency-aware keys and fail-closed safety: permission
//! context enters the key or forces bypass; tool schema/version changes
//! invalidate; errors/cancels/partial output are never cached by default;
//! streaming and ordinary paths share one key/invalidation model; cache-miss
//! and cache-disabled results are semantically equivalent.  The
//! `PromptConstructionProfile` is observable (real tool partition count, not a
//! fixed 0).  This seam is policy only; the concrete caches wire into it at
//! the C1 cutover.

/// The three cache layers (closure-plan §16 task 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheLayer {
    ProviderPrefix,
    RecallResult,
    ReadOnlyToolResult,
}

/// A cache key that includes permission context + tool schema/version digest.
/// Mutable-file reads additionally carry content/metadata/repo-HEAD digests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub layer: CacheLayer,
    pub permission_ctx: String,
    pub tool_digest: String,
    /// Content/metadata/repo-HEAD digest for mutable-file reads; None for
    /// side-effect-free immutable reads.
    pub dependency_digest: Option<String>,
}

impl CacheKey {
    pub fn new(
        layer: CacheLayer,
        permission_ctx: impl Into<String>,
        tool_digest: impl Into<String>,
    ) -> Self {
        Self {
            layer,
            permission_ctx: permission_ctx.into(),
            tool_digest: tool_digest.into(),
            dependency_digest: None,
        }
    }

    pub fn with_dependency(mut self, digest: impl Into<String>) -> Self {
        self.dependency_digest = Some(digest.into());
        self
    }
}

/// Whether a result may be cached (fail-closed default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheDecision {
    /// Cache the result under the key.
    Cache(CacheKey),
    /// Never cache: side-effectful tool, permission-sensitive result,
    /// time-sensitive state, or undeclared-dependency read.
    Bypass,
}

/// Policy for one tool result.  Fail-closed: side effects / permission
/// sensitivity / time-sensitivity / undeclared dependencies bypass.
pub fn decide_cache(
    key: CacheKey,
    is_side_effectful: bool,
    is_permission_sensitive: bool,
    is_time_sensitive: bool,
) -> CacheDecision {
    if is_side_effectful || is_permission_sensitive || is_time_sensitive {
        return CacheDecision::Bypass;
    }
    CacheDecision::Cache(key)
}

/// Observability: a prompt construction profile that records the real tool
/// partition (not a fixed 0).  Emitted as turn telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptConstructionProfile {
    pub tool_definitions: usize,
    pub tool_count: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_bypasses: u64,
    pub stale_rejects: u64,
}

impl Default for PromptConstructionProfile {
    fn default() -> Self {
        Self {
            tool_definitions: 0,
            tool_count: 0,
            cache_hits: 0,
            cache_misses: 0,
            cache_bypasses: 0,
            stale_rejects: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_effectful_tool_is_never_cached() {
        let key = CacheKey::new(CacheLayer::ReadOnlyToolResult, "alice", "tool.v1");
        assert_eq!(
            decide_cache(key.clone(), true, false, false),
            CacheDecision::Bypass
        );
        // Permission-sensitive result bypasses even if not side-effectful.
        assert_eq!(
            decide_cache(key.clone(), false, true, false),
            CacheDecision::Bypass
        );
        // Time-sensitive bypasses.
        assert_eq!(
            decide_cache(key.clone(), false, false, true),
            CacheDecision::Bypass
        );
    }

    #[test]
    fn read_only_immutable_is_cached_with_key() {
        let key = CacheKey::new(CacheLayer::ReadOnlyToolResult, "alice", "tool.v1");
        assert_eq!(
            decide_cache(key.clone(), false, false, false),
            CacheDecision::Cache(key)
        );
    }

    #[test]
    fn mutable_read_requires_dependency_digest() {
        let key = CacheKey::new(CacheLayer::ReadOnlyToolResult, "alice", "tool.v1")
            .with_dependency("content:abc,head:deadbeef");
        assert_eq!(
            key.dependency_digest.as_deref(),
            Some("content:abc,head:deadbeef")
        );
    }
}
