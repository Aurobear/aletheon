//! Bounded read-only tool result cache (see
//! docs/testing/deepseek-cache.md#read-only-tool-result-cache).
//!
//! The host only consults this cache for tools that (a) explicitly declare a
//! non-`Never` `ToolCachePolicy` **and** (b) are read-only
//! (`permission_level() == L0`). The L0 gate is enforced by the executor, so a
//! mutating tool cannot be misconfigured into caching. A hit still yields an
//! auditable `CapabilityResult` marked `served_from_cache` and never bypasses
//! the permission/approval gate (which runs before the cache is consulted).
//! Cache failures fail open to the real read-only tool.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ::contracts::tool::{PermissionLevel, ToolCacheDependencies, ToolCachePolicy};
use serde_json::Value;
use sha2::{Digest, Sha256};

const KEY_DOMAIN: &[u8] = b"aletheon.read-only-tool-cache.v2\0";

/// Host-owned cache key for a read-only tool result.
///
/// Includes the tool name, implementation version, canonical arguments,
/// workspace identity and the exact scope implied by the policy. Per-turn and
/// session policies can never cross their typed boundary; shared policies omit
/// principal identity only when the tool owner explicitly declares that safe.
#[derive(Debug, Clone, Copy)]
pub struct ToolCacheScope<'a> {
    pub workspace: &'a str,
    pub principal: &'a str,
    pub session: &'a str,
    pub turn: &'a str,
}

/// Immutable identities that define the implementation, schema, dependencies,
/// and effective authority under which a cache entry was produced.
#[derive(Debug, Clone, Copy)]
pub struct ToolCacheIdentity<'a> {
    pub tool_name: &'a str,
    pub impl_version: &'a str,
    pub schema_digest: &'a str,
    pub dependency_fingerprint: &'a str,
    pub authority_context: &'a str,
}

pub fn read_only_cache_key(
    identity: ToolCacheIdentity<'_>,
    args: &Value,
    scope: ToolCacheScope<'_>,
    policy: ToolCachePolicy,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(KEY_DOMAIN);
    hasher.update(identity.tool_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(identity.impl_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(identity.schema_digest.as_bytes());
    hasher.update(b"\0");
    hash_canonical_json(&mut hasher, args);
    hasher.update(b"\0");
    hasher.update(identity.dependency_fingerprint.as_bytes());
    hasher.update(b"\0");
    hasher.update(scope.workspace.as_bytes());
    hasher.update(b"\0");
    // Bind the effective permission level into the key so a permission-context
    // change (e.g. a higher-granted scope) forces a cache miss instead of
    // serving a result computed under a weaker authority (C1-AUDIT-004).
    hasher.update(b"perm\0");
    hasher.update(identity.authority_context.as_bytes());
    hasher.update(b"\0");
    match policy {
        ToolCachePolicy::Never => hasher.update(b"never"),
        ToolCachePolicy::PerTurn => {
            hasher.update(b"turn\0");
            hasher.update(scope.principal.as_bytes());
            hasher.update(b"\0");
            hasher.update(scope.session.as_bytes());
            hasher.update(b"\0");
            hasher.update(scope.turn.as_bytes());
        }
        ToolCachePolicy::Session { .. } => {
            hasher.update(b"session\0");
            hasher.update(scope.principal.as_bytes());
            hasher.update(b"\0");
            hasher.update(scope.session.as_bytes());
        }
        ToolCachePolicy::SharedReadOnly {
            vary_by_principal, ..
        } => {
            hasher.update(b"shared\0");
            if vary_by_principal {
                hasher.update(scope.principal.as_bytes());
            }
        }
    }
    hasher.update(b"\0");
    hasher.update(format!("{policy:?}").as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Recompute the declared dependency identity on every lookup. Any ambiguity
/// returns an error so the executor can record a validation rejection and run
/// the authoritative tool instead of trusting TTL.
pub fn dependency_fingerprint(
    dependencies: ToolCacheDependencies,
    args: &Value,
    workspace: &::contracts::WorkspacePolicy,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(b"aletheon.tool-cache-dependencies.v1\0");
    match dependencies {
        ToolCacheDependencies::ContentAddressed { argument_names } => {
            if argument_names.is_empty() {
                return Err("content-addressed cache dependency list is empty".into());
            }
            hasher.update(b"content-addressed\0");
            for name in argument_names {
                let value = args
                    .get(*name)
                    .ok_or_else(|| format!("missing content identity argument '{name}'"))?;
                let value = value
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| format!("content identity argument '{name}' is not a string"))?;
                hasher.update(name.as_bytes());
                hasher.update(b"\0");
                hasher.update(value.as_bytes());
                hasher.update(b"\0");
            }
        }
        ToolCacheDependencies::WorkspaceFiles {
            argument_names,
            include_repo_state,
        } => {
            if argument_names.is_empty() {
                return Err("workspace cache dependency list is empty".into());
            }
            hasher.update(b"workspace-files\0");
            for name in argument_names {
                let raw = args
                    .get(*name)
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| format!("workspace dependency '{name}' is not a path string"))?;
                let candidate = if Path::new(raw).is_absolute() {
                    PathBuf::from(raw)
                } else {
                    workspace.cwd().join(raw)
                };
                let resolved = std::fs::canonicalize(&candidate).map_err(|error| {
                    format!(
                        "cannot resolve workspace dependency '{}': {error}",
                        candidate.display()
                    )
                })?;
                if !resolved.starts_with(workspace.cwd()) {
                    return Err(format!(
                        "workspace dependency escapes cwd: {}",
                        resolved.display()
                    ));
                }
                let metadata = std::fs::metadata(&resolved).map_err(|error| {
                    format!(
                        "cannot stat workspace dependency '{}': {error}",
                        resolved.display()
                    )
                })?;
                if !metadata.is_file() {
                    return Err(format!(
                        "workspace cache dependency is not a regular file: {}",
                        resolved.display()
                    ));
                }
                let content = std::fs::read(&resolved).map_err(|error| {
                    format!(
                        "cannot read workspace dependency '{}': {error}",
                        resolved.display()
                    )
                })?;
                hasher.update(name.as_bytes());
                hasher.update(b"\0");
                hasher.update(resolved.as_os_str().as_encoded_bytes());
                hasher.update(b"\0");
                hasher.update(metadata.len().to_le_bytes());
                if let Ok(modified) = metadata.modified() {
                    if let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) {
                        hasher.update(since_epoch.as_secs().to_le_bytes());
                        hasher.update(since_epoch.subsec_nanos().to_le_bytes());
                    }
                }
                hasher.update(Sha256::digest(content));
            }
            if include_repo_state {
                hash_repo_state(&mut hasher, workspace.cwd())?;
            }
        }
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn hash_repo_state(hasher: &mut Sha256, cwd: &Path) -> Result<(), String> {
    let Some(repo_root) = cwd
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
    else {
        hasher.update(b"no-repository");
        return Ok(());
    };
    let marker = repo_root.join(".git");
    let git_dir = if marker.is_dir() {
        marker
    } else {
        let pointer = std::fs::read_to_string(&marker)
            .map_err(|error| format!("cannot read gitdir marker: {error}"))?;
        let path = pointer
            .trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .ok_or_else(|| "invalid gitdir marker".to_owned())?;
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            repo_root.join(path)
        }
    };
    hasher.update(b"repository\0");
    for relative in ["HEAD", "index"] {
        let path = git_dir.join(relative);
        hasher.update(relative.as_bytes());
        hasher.update(b"\0");
        match std::fs::read(&path) {
            Ok(bytes) => hasher.update(Sha256::digest(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => hasher.update(b"missing"),
            Err(error) => return Err(format!("cannot read Git {relative}: {error}")),
        }
    }
    if let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) {
        if let Some(reference) = head.trim().strip_prefix("ref: ") {
            let path = git_dir.join(reference);
            hasher.update(reference.as_bytes());
            hasher.update(b"\0");
            match std::fs::read(path) {
                Ok(bytes) => hasher.update(Sha256::digest(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    hasher.update(b"packed-or-missing")
                }
                Err(error) => return Err(format!("cannot read Git HEAD reference: {error}")),
            }
        }
    }
    Ok(())
}

/// Feed JSON into a digest with explicit type tags and lexicographically sorted
/// object keys. This remains deterministic even when serde_json is compiled
/// with insertion-order map preservation.
fn hash_canonical_json(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Null => hasher.update(b"n"),
        Value::Bool(value) => hasher.update(if *value { b"b1" } else { b"b0" }),
        Value::Number(value) => {
            hasher.update(b"d");
            hasher.update(value.to_string().as_bytes());
        }
        Value::String(value) => {
            hasher.update(b"s");
            hasher.update(value.len().to_le_bytes());
            hasher.update(value.as_bytes());
        }
        Value::Array(values) => {
            hasher.update(b"a");
            hasher.update(values.len().to_le_bytes());
            for value in values {
                hash_canonical_json(hasher, value);
            }
        }
        Value::Object(values) => {
            hasher.update(b"o");
            hasher.update(values.len().to_le_bytes());
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                hasher.update(key.len().to_le_bytes());
                hasher.update(key.as_bytes());
                hash_canonical_json(hasher, value);
            }
        }
    }
}

/// TTL for a declared policy. `Never` yields `None` (uncacheable). `PerTurn`
/// uses a short retention TTL only for cleanup; its key contains the exact turn
/// identity, so the value can never be served in another turn.
pub fn cache_ttl(policy: ToolCachePolicy) -> Option<Duration> {
    match policy {
        ToolCachePolicy::Session { ttl_ms } | ToolCachePolicy::SharedReadOnly { ttl_ms, .. } => {
            Some(Duration::from_millis(ttl_ms))
        }
        ToolCachePolicy::PerTurn => Some(Duration::from_secs(300)),
        ToolCachePolicy::Never => None,
    }
}

/// The executor enforces this read-only gate. Only L0 tools that declare a
/// policy are ever served from cache — a mutating tool cannot opt in.
pub fn is_cacheable(policy: ToolCachePolicy, permission: PermissionLevel) -> bool {
    policy != ToolCachePolicy::Never && permission == PermissionLevel::L0
}

/// Bounded FIFO-with-TTL cache of read-only tool results.
#[derive(Debug)]
pub struct ReadOnlyToolResultCache {
    inner: std::sync::Mutex<Inner>,
    capacity: usize,
}

#[derive(Debug, Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
    metrics: BTreeMap<String, ToolCacheCounters>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCacheCounters {
    pub hit_total: u64,
    pub miss_total: u64,
    pub bypass_total: u64,
    pub stale_reject_total: u64,
    pub validation_reject_total: u64,
    pub saved_latency_ms: u64,
    pub saved_output_bytes: u64,
}

pub type ToolCacheMetricsSnapshot = BTreeMap<String, ToolCacheCounters>;

#[derive(Debug, Clone)]
struct Entry {
    stored_at: Instant,
    ttl: Duration,
    value: ::contracts::CapabilityResult,
}

impl ReadOnlyToolResultCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: std::sync::Mutex::new(Inner::default()),
            capacity: capacity.max(1),
        }
    }

    /// Look up a cached result, expiring stale entries and refreshing recency
    /// on a hit. Returns `None` on miss/expiry or lock failure (fail-open).
    pub fn get(&self, tool_name: &str, key: &str) -> Option<::contracts::CapabilityResult> {
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(entry) = inner.entries.get(key) else {
            let metric = inner.metrics.entry(tool_name.to_owned()).or_default();
            metric.miss_total = metric.miss_total.saturating_add(1);
            return None;
        };
        if now.duration_since(entry.stored_at) > entry.ttl {
            inner.entries.remove(key);
            inner.order.retain(|candidate| candidate != key);
            let metric = inner.metrics.entry(tool_name.to_owned()).or_default();
            metric.miss_total = metric.miss_total.saturating_add(1);
            metric.stale_reject_total = metric.stale_reject_total.saturating_add(1);
            return None;
        }
        let value = inner.entries.get(key).map(|entry| entry.value.clone());
        inner.order.retain(|candidate| candidate != key);
        inner.order.push_back(key.to_owned());
        value
    }

    /// Record a candidate value that passed the current policy/audit pipeline
    /// and was actually served to the caller.
    pub fn record_hit(&self, tool_name: &str, saved_latency_ms: u64, saved_output_bytes: u64) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let metric = inner.metrics.entry(tool_name.to_owned()).or_default();
        metric.hit_total = metric.hit_total.saturating_add(1);
        metric.saved_latency_ms = metric.saved_latency_ms.saturating_add(saved_latency_ms);
        metric.saved_output_bytes = metric.saved_output_bytes.saturating_add(saved_output_bytes);
    }

    pub fn record_bypass(&self, tool_name: &str) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let metric = inner.metrics.entry(tool_name.to_owned()).or_default();
        metric.bypass_total = metric.bypass_total.saturating_add(1);
    }

    /// Record a cached candidate rejected by current policy/audit. The caller
    /// executes the authoritative tool, so this is an effective cache miss.
    pub fn record_rejected_candidate(&self, tool_name: &str) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let metric = inner.metrics.entry(tool_name.to_owned()).or_default();
        metric.miss_total = metric.miss_total.saturating_add(1);
        metric.validation_reject_total = metric.validation_reject_total.saturating_add(1);
    }

    pub fn metrics_snapshot(&self) -> ToolCacheMetricsSnapshot {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .metrics
            .clone()
    }

    /// Store a result under a key with the policy TTL.
    pub fn insert(&self, key: String, value: ::contracts::CapabilityResult, ttl: Duration) {
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if inner.entries.contains_key(&key) {
            inner.order.retain(|candidate| candidate != &key);
        } else {
            while inner.order.len() >= self.capacity {
                if let Some(oldest) = inner.order.pop_front() {
                    inner.entries.remove(&oldest);
                }
            }
        }
        inner.entries.insert(
            key.clone(),
            Entry {
                stored_at: now,
                ttl,
                value,
            },
        );
        inner.order.push_back(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::tool::PermissionLevel::{L0, L1};

    fn result(payload: &str) -> ::contracts::CapabilityResult {
        ::contracts::CapabilityResult {
            call_id: "call-1".into(),
            output: payload.into(),
            is_error: false,
            usage: Default::default(),
            audit_id: None,
            patch_delta: None,
            served_from_cache: false,
        }
    }

    fn key(
        args: serde_json::Value,
        workspace: &str,
        principal: &str,
        policy: ToolCachePolicy,
    ) -> String {
        read_only_cache_key(
            ToolCacheIdentity {
                tool_name: "read_file",
                impl_version: "v1",
                schema_digest: "schema-v1",
                dependency_fingerprint: "dependency-v1",
                authority_context: "read",
            },
            &args,
            ToolCacheScope {
                workspace,
                principal,
                session: "session-a",
                turn: "turn-a",
            },
            policy,
        )
    }

    #[test]
    fn cacheable_gate_requires_l0_and_declared_policy() {
        assert!(is_cacheable(ToolCachePolicy::Session { ttl_ms: 1000 }, L0));
        assert!(!is_cacheable(ToolCachePolicy::Session { ttl_ms: 1000 }, L1));
        assert!(!is_cacheable(ToolCachePolicy::Never, L0));
        assert!(!is_cacheable(
            ToolCachePolicy::SharedReadOnly {
                ttl_ms: 1000,
                vary_by_principal: true,
            },
            L1
        ));
    }

    #[test]
    fn key_changes_on_workspace_principal_impl_version_and_policy() {
        let args = serde_json::json!({"path": "/tmp/a"});
        let base = key(
            args.clone(),
            "ws-a",
            "principal-a",
            ToolCachePolicy::Session { ttl_ms: 1000 },
        );
        assert_ne!(
            base,
            key(
                args.clone(),
                "ws-a",
                "principal-b",
                ToolCachePolicy::Session { ttl_ms: 1000 }
            )
        );
        assert_ne!(
            base,
            key(
                args.clone(),
                "ws-b",
                "principal-a",
                ToolCachePolicy::Session { ttl_ms: 1000 }
            )
        );
        assert_ne!(
            base,
            key(
                serde_json::json!({"path": "/tmp/b"}),
                "ws-a",
                "principal-a",
                ToolCachePolicy::Session { ttl_ms: 1000 }
            )
        );
        assert_ne!(
            base,
            key(
                args,
                "ws-a",
                "principal-a",
                ToolCachePolicy::Session { ttl_ms: 999 }
            )
        );
    }

    #[test]
    fn key_changes_on_schema_dependency_and_authority_context() {
        let scope = ToolCacheScope {
            workspace: "workspace",
            principal: "principal",
            session: "session",
            turn: "turn",
        };
        let make = |schema: &str, dependency: &str, authority: &str| {
            read_only_cache_key(
                ToolCacheIdentity {
                    tool_name: "read_file",
                    impl_version: "implementation-v1",
                    schema_digest: schema,
                    dependency_fingerprint: dependency,
                    authority_context: authority,
                },
                &serde_json::json!({"path": "a.txt"}),
                scope,
                ToolCachePolicy::PerTurn,
            )
        };
        let base = make("schema-v1", "dependency-v1", "safe");
        assert_ne!(base, make("schema-v2", "dependency-v1", "safe"));
        assert_ne!(base, make("schema-v1", "dependency-v2", "safe"));
        assert_ne!(base, make("schema-v1", "dependency-v1", "full"));
    }

    #[test]
    fn workspace_file_and_git_state_changes_invalidate_dependency_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("state.txt");
        std::fs::write(&file, "one").unwrap();
        std::fs::create_dir(temp.path().join(".git")).unwrap();
        std::fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/dev\n").unwrap();
        std::fs::write(temp.path().join(".git/index"), b"index-v1").unwrap();
        let workspace = ::contracts::WorkspacePolicy::from_resolved_roots(
            std::fs::canonicalize(temp.path()).unwrap(),
            Vec::new(),
        )
        .unwrap();
        let dependencies = ToolCacheDependencies::WorkspaceFiles {
            argument_names: &["path"],
            include_repo_state: true,
        };
        let args = serde_json::json!({"path": "state.txt"});
        let initial = dependency_fingerprint(dependencies, &args, &workspace).unwrap();
        std::fs::write(&file, "two").unwrap();
        let content_changed = dependency_fingerprint(dependencies, &args, &workspace).unwrap();
        assert_ne!(initial, content_changed);
        std::fs::write(temp.path().join(".git/index"), b"index-v2").unwrap();
        let index_changed = dependency_fingerprint(dependencies, &args, &workspace).unwrap();
        assert_ne!(content_changed, index_changed);
    }

    #[test]
    fn undeclared_or_escaping_workspace_dependencies_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = ::contracts::WorkspacePolicy::from_resolved_roots(
            std::fs::canonicalize(temp.path()).unwrap(),
            Vec::new(),
        )
        .unwrap();
        assert!(dependency_fingerprint(
            ToolCacheDependencies::WorkspaceFiles {
                argument_names: &[],
                include_repo_state: false,
            },
            &serde_json::json!({}),
            &workspace,
        )
        .is_err());
        assert!(dependency_fingerprint(
            ToolCacheDependencies::WorkspaceFiles {
                argument_names: &["path"],
                include_repo_state: false,
            },
            &serde_json::json!({"path": "/etc/hosts"}),
            &workspace,
        )
        .is_err());
    }

    #[test]
    fn canonical_arguments_and_policy_scope_are_exact() {
        let left = serde_json::from_str::<Value>(r#"{"b":2,"a":{"y":1,"x":0}}"#).unwrap();
        let right = serde_json::from_str::<Value>(r#"{"a":{"x":0,"y":1},"b":2}"#).unwrap();
        assert_eq!(
            key(left, "ws-a", "principal-a", ToolCachePolicy::PerTurn),
            key(right, "ws-a", "principal-a", ToolCachePolicy::PerTurn)
        );

        let per_turn_a = read_only_cache_key(
            ToolCacheIdentity {
                tool_name: "read_file",
                impl_version: "v1",
                schema_digest: "schema-v1",
                dependency_fingerprint: "dependency-v1",
                authority_context: "read",
            },
            &serde_json::json!({}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-a",
                session: "session-a",
                turn: "turn-a",
            },
            ToolCachePolicy::PerTurn,
        );
        let per_turn_b = read_only_cache_key(
            ToolCacheIdentity {
                tool_name: "read_file",
                impl_version: "v1",
                schema_digest: "schema-v1",
                dependency_fingerprint: "dependency-v1",
                authority_context: "read",
            },
            &serde_json::json!({}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-a",
                session: "session-a",
                turn: "turn-b",
            },
            ToolCachePolicy::PerTurn,
        );
        assert_ne!(per_turn_a, per_turn_b);

        let shared_a = read_only_cache_key(
            ToolCacheIdentity {
                tool_name: "read_file",
                impl_version: "v1",
                schema_digest: "schema-v1",
                dependency_fingerprint: "dependency-v1",
                authority_context: "read",
            },
            &serde_json::json!({}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-a",
                session: "session-a",
                turn: "turn-a",
            },
            ToolCachePolicy::SharedReadOnly {
                ttl_ms: 1000,
                vary_by_principal: false,
            },
        );
        let shared_b = read_only_cache_key(
            ToolCacheIdentity {
                tool_name: "read_file",
                impl_version: "v1",
                schema_digest: "schema-v1",
                dependency_fingerprint: "dependency-v1",
                authority_context: "read",
            },
            &serde_json::json!({}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-b",
                session: "session-b",
                turn: "turn-b",
            },
            ToolCachePolicy::SharedReadOnly {
                ttl_ms: 1000,
                vary_by_principal: false,
            },
        );
        assert_eq!(shared_a, shared_b);
    }

    #[test]
    fn hit_returns_cached_and_expiry_misses() {
        let cache = ReadOnlyToolResultCache::new(4);
        let k = key(
            serde_json::json!({"path": "/tmp/a"}),
            "ws-a",
            "principal-a",
            ToolCachePolicy::Session { ttl_ms: 60000 },
        );
        assert!(cache.get("read_file", &k).is_none());
        cache.insert(k.clone(), result("cached-content"), Duration::from_secs(60));
        let hit = cache.get("read_file", &k).expect("cached hit");
        cache.record_hit("read_file", 7, 14);
        assert_eq!(hit.output, "cached-content");
        // A different key (implementation version bump) misses.
        let v2 = read_only_cache_key(
            ToolCacheIdentity {
                tool_name: "read_file",
                impl_version: "v2",
                schema_digest: "schema-v1",
                dependency_fingerprint: "dependency-v1",
                authority_context: "read",
            },
            &serde_json::json!({"path": "/tmp/a"}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-a",
                session: "session-a",
                turn: "turn-a",
            },
            ToolCachePolicy::Session { ttl_ms: 60000 },
        );
        assert!(cache.get("read_file", &v2).is_none());
        let expired = key(
            serde_json::json!({"path": "/tmp/expired"}),
            "ws-a",
            "principal-a",
            ToolCachePolicy::Session { ttl_ms: 0 },
        );
        cache.insert(expired.clone(), result("expired-content"), Duration::ZERO);
        std::thread::sleep(Duration::from_millis(1));
        assert!(cache.get("read_file", &expired).is_none());
        let metrics = cache.metrics_snapshot();
        assert_eq!(metrics["read_file"].hit_total, 1);
        assert_eq!(metrics["read_file"].miss_total, 3);
        assert_eq!(metrics["read_file"].stale_reject_total, 1);
        assert_eq!(metrics["read_file"].saved_latency_ms, 7);
        assert_eq!(metrics["read_file"].saved_output_bytes, 14);
    }

    #[test]
    fn bounded_capacity_evicts_oldest() {
        let cache = ReadOnlyToolResultCache::new(2);
        let mut keys = Vec::new();
        for i in 0..3 {
            let k = key(
                serde_json::json!({"path": format!("/tmp/{i}")}),
                "ws-a",
                "principal-a",
                ToolCachePolicy::Session { ttl_ms: 60000 },
            );
            cache.insert(k.clone(), result(&format!("v{i}")), Duration::from_secs(60));
            keys.push(k);
        }
        // Oldest (v0) evicted; v1 and v2 present.
        assert!(cache.get("read_file", &keys[0]).is_none());
        assert!(cache.get("read_file", &keys[1]).is_some());
        assert!(cache.get("read_file", &keys[2]).is_some());
    }

    #[test]
    fn permission_level_change_forces_a_distinct_cache_key() {
        let scope = ToolCacheScope {
            workspace: "ws-a",
            principal: "principal-a",
            session: "session-a",
            turn: "turn-a",
        };
        let read_only = read_only_cache_key(
            ToolCacheIdentity {
                tool_name: "read_file",
                impl_version: "v1",
                schema_digest: "schema-v1",
                dependency_fingerprint: "dependency-v1",
                authority_context: "read",
            },
            &serde_json::json!({"path": "/tmp/x"}),
            scope,
            ToolCachePolicy::Session { ttl_ms: 60000 },
        );
        let elevated = read_only_cache_key(
            ToolCacheIdentity {
                tool_name: "read_file",
                impl_version: "v1",
                schema_digest: "schema-v1",
                dependency_fingerprint: "dependency-v1",
                authority_context: "write",
            },
            &serde_json::json!({"path": "/tmp/x"}),
            scope,
            ToolCachePolicy::Session { ttl_ms: 60000 },
        );
        assert_ne!(
            read_only, elevated,
            "a permission-context change must force a cache miss"
        );
    }
}
