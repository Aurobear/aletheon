//! Sandbox trait and types.

use crate::WorkspacePolicy;
use anyhow::Result;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Isolation level for sandbox execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// No isolation — bare process execution.
    None,
    /// Process-level — resource limits (RLIMIT) but no namespace isolation.
    Process,
    /// Namespace-level — full namespace isolation (pid, net, mount, etc.).
    Namespace,
    /// Container-level — OCI container runtime.
    Container,
}

/// Runtime configuration passed to a sandbox execute call.
#[derive(Debug, Clone, Deserialize)]
pub struct SandboxConfig {
    /// Canonical workspace authority materialized into the process sandbox.
    pub workspace: crate::WorkspacePolicy,
    /// Extra environment variables to set.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Resolved sandbox profile (S1). `None` = no profile layer (the
    /// `grok_hardening.sandbox_profiles` flag is off, or no profile selected);
    /// existing backends ignore `None`, so behavior is byte-identical to the
    /// pre-profile path. Never serialized: it is derived per execution from
    /// daemon-trusted config, not carried in persisted config.
    #[serde(skip)]
    pub policy: Option<ResolvedSandboxPolicy>,
}

impl SandboxConfig {
    pub fn working_dir(&self) -> &Path {
        self.workspace.cwd()
    }
}

/// Capabilities reported by a sandbox backend.
#[derive(Debug, Clone)]
pub struct SandboxCapabilities {
    /// Whether the backend provides filesystem isolation.
    pub filesystem_isolation: bool,
    /// Whether the backend provides network isolation.
    pub network_isolation: bool,
    /// Whether the backend enforces resource limits.
    pub resource_limits: bool,
    /// Whether the backend applies a seccomp filter.
    pub seccomp_filter: bool,
    /// Human-readable list of known limitations.
    pub limitations: Vec<String>,
}

/// Result of a sandboxed command execution.
#[derive(Debug, Clone)]
pub struct SandboxResult {
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Process exit code (-1 if unavailable, e.g. timeout).
    pub exit_code: i32,
    /// Name of the backend that produced this result.
    pub backend_used: String,
    /// Isolation level that was in effect.
    pub isolation_level: IsolationLevel,
    /// Wall-clock elapsed time in milliseconds.
    pub elapsed_ms: u64,
}

/// An argv-preserving sandbox launcher command suitable for bounded execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
}

/// Trait that every sandbox backend must implement.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Human-readable backend name (e.g. "process", "namespace", "container").
    fn name(&self) -> &str;

    /// The isolation level this backend provides.
    fn isolation_level(&self) -> IsolationLevel;

    /// Check whether this backend is available in the current environment.
    fn is_available(&self) -> bool;

    /// Report the backend's capabilities and limitations.
    fn capabilities(&self) -> SandboxCapabilities;

    /// Wrap an argv command without shell interpolation. Backends that cannot
    /// preserve argv boundaries must fail closed rather than use `sh -c`.
    fn wrap_argv(
        &self,
        _program: &Path,
        _args: &[String],
        _config: &SandboxConfig,
    ) -> Result<SandboxCommand> {
        anyhow::bail!("sandbox backend does not support argv-safe execution")
    }

    /// Execute a shell command under this sandbox with the given config and timeout.
    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult>;
}

/// User-facing preference for sandbox isolation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxPreference {
    /// Select the best available backend automatically.
    Auto,
    /// Require namespace-level isolation; fail if unavailable.
    Require,
    /// Disable sandbox entirely (debug mode).
    Forbid,
    /// Use best available, but warn on degraded isolation.
    BestEffort,
}

impl SandboxPreference {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "auto" => Self::Auto,
            "require" => Self::Require,
            "forbid" => Self::Forbid,
            "best_effort" | "besteffort" => Self::BestEffort,
            _ => Self::Auto,
        }
    }
}

/// Selects the best available sandbox backend and dispatches execution.
///
/// Backends are probed and registered in priority order.
/// Concrete backends (e.g. Bubblewrap, Process, Noop) are provided by the
/// caller so that fabric does not depend on corpus.
pub struct SandboxExecutor {
    backends: Vec<Box<dyn SandboxBackend>>,
    preference: SandboxPreference,
}

impl SandboxExecutor {
    pub fn new(backends: Vec<Box<dyn SandboxBackend>>, preference: SandboxPreference) -> Self {
        Self {
            backends,
            preference,
        }
    }

    /// Select the most appropriate backend based on the configured preference.
    pub fn select_backend(&self) -> Option<&dyn SandboxBackend> {
        match self.preference {
            SandboxPreference::Auto | SandboxPreference::BestEffort => {
                // Return the first available backend (highest priority).
                self.backends
                    .iter()
                    .find(|b| b.is_available())
                    .map(|b| b.as_ref())
            }
            SandboxPreference::Require => {
                // Must have namespace-level or better isolation.
                self.backends
                    .iter()
                    .find(|b| {
                        b.is_available()
                            && matches!(
                                b.isolation_level(),
                                IsolationLevel::Namespace | IsolationLevel::Container
                            )
                    })
                    .map(|b| b.as_ref())
            }
            SandboxPreference::Forbid => {
                // Return NoopBackend explicitly.
                self.backends
                    .iter()
                    .find(|b| b.name() == "noop")
                    .map(|b| b.as_ref())
            }
        }
    }

    /// Execute a command using the selected sandbox backend.
    pub async fn run(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult> {
        let backend = self
            .select_backend()
            .ok_or_else(|| anyhow::anyhow!("No suitable sandbox backend available"))?;

        // Defense-in-depth: if Require preference somehow selects NoopBackend
        // (e.g. a misconfigured backend claiming namespace isolation), fail
        // explicitly rather than executing without real isolation.
        if self.preference == SandboxPreference::Require && backend.name() == "noop" {
            return Err(anyhow::anyhow!(
                "Sandbox required but NoopBackend was selected (fail-closed)"
            ));
        }

        if self.preference == SandboxPreference::BestEffort
            && backend.isolation_level() == IsolationLevel::None
        {
            tracing::warn!("Sandbox degraded to no isolation (BestEffort mode)");
        }

        // S1 T11: a resolved profile that restricts the network must not run on
        // a backend that cannot isolate it. Under Require this fails closed
        // (same posture as the noop guard); otherwise it warns rather than
        // silently opening the network. `None` policy skips the check entirely.
        if let Some(policy) = &config.policy {
            if policy.restrict_network && !backend.capabilities().network_isolation {
                if self.preference == SandboxPreference::Require {
                    return Err(anyhow::anyhow!(
                        "Sandbox profile '{}' requires network isolation but backend '{}' cannot provide it (fail-closed)",
                        policy.name,
                        backend.name()
                    ));
                }
                tracing::warn!(
                    profile = %policy.name,
                    backend = %backend.name(),
                    "Sandbox profile requests network restriction but backend lacks network isolation; continuing degraded"
                );
            }
        }

        backend.execute(cmd, config, timeout).await
    }

    /// List all registered backends with their availability status.
    pub fn list_backends(&self) -> Vec<(&str, bool)> {
        self.backends
            .iter()
            .map(|b| (b.name(), b.is_available()))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// S1: sandbox profile layer. Named profiles resolve to deny paths + network
// restriction + read/write roots. Layered config: global then project, where
// project is ADDITIVE ONLY (cannot redefine a global profile — anti-hollowing).
// See docs/plans/grok/exec/S1-sandbox.md.
// ---------------------------------------------------------------------------

/// A named sandbox profile config (from trusted daemon config, never repo).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SandboxProfileConfig {
    /// Built-in base to inherit ("workspace" | "read-only" | "strict").
    /// Custom profiles cannot extend other custom profiles.
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub restrict_network: Option<bool>,
    #[serde(default)]
    pub read_only: Vec<String>,
    #[serde(default)]
    pub read_write: Vec<String>,
    /// Deny entries: exact paths or globs (`**/*.pem`). Read + write denied.
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Layered sandbox profiles. Global loads first; project merges additively.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SandboxProfiles {
    /// Which profile to apply by default (used when `grok_hardening.sandbox_profiles`
    /// is on). Must be a known profile name or built-in ("workspace", "strict",
    /// "read-only", "off"). "workspace" = safe default (read all, write workspace
    /// roots + credential deny).
    #[serde(default = "default_profile_name")]
    pub default_profile: String,
    #[serde(default)]
    pub profiles: BTreeMap<String, SandboxProfileConfig>,
}

fn default_profile_name() -> String {
    "workspace".to_string()
}

impl Default for SandboxProfiles {
    fn default() -> Self {
        Self {
            default_profile: "workspace".to_string(),
            profiles: BTreeMap::new(),
        }
    }
}

impl SandboxProfiles {
    /// Additive project merge: only new names are added. A name already defined
    /// globally is preserved (a malicious workspace cannot hollow out a trusted
    /// profile's deny/read_write while keeping the trusted name).
    pub fn merge_project_additive(&mut self, project: SandboxProfiles) {
        for (name, cfg) in project.profiles {
            self.profiles.entry(name).or_insert(cfg);
        }
    }

    /// Names that the project attempted to redefine (differ from global). For
    /// surfacing a warning; these redefinitions are ignored by the merge.
    pub fn conflicting_redefinitions(&self, project: &SandboxProfiles) -> Vec<String> {
        let mut names: Vec<String> = project
            .profiles
            .iter()
            .filter_map(|(name, pcfg)| {
                self.profiles
                    .get(name)
                    .filter(|gcfg| *gcfg != pcfg)
                    .map(|_| name.clone())
            })
            .collect();
        names.sort_unstable();
        names
    }
}

/// Built-in / custom profile name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileName {
    Workspace,
    ReadOnly,
    Strict,
    Off,
    Custom(String),
}

impl std::str::FromStr for ProfileName {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "workspace" => Self::Workspace,
            "read-only" | "readonly" => Self::ReadOnly,
            "strict" => Self::Strict,
            "off" | "none" => Self::Off,
            other => Self::Custom(other.to_string()),
        })
    }
}

impl std::fmt::Display for ProfileName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Workspace => write!(f, "workspace"),
            Self::ReadOnly => write!(f, "read-only"),
            Self::Strict => write!(f, "strict"),
            Self::Off => write!(f, "off"),
            Self::Custom(name) => write!(f, "{name}"),
        }
    }
}

/// A resolved execution policy. Fed to a backend, which applies it per its
/// capabilities. `resolve_profile` produces this; the backend consumes it.
///
/// `read_only_roots` containing the filesystem root (`/`) means "read the whole
/// disk" (the `workspace`/`read-only` default). `deny_exact`/`deny_globs` are
/// always enforced on top and win over any readable root — credential paths are
/// merged into `deny_exact` for every profile (fail-closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSandboxPolicy {
    pub name: String,
    pub read_only_roots: Vec<PathBuf>,
    pub read_write_roots: Vec<PathBuf>,
    /// Exact deny paths. Canonicalization (which touches the FS) happens in the
    /// backend-application phase, not in the pure `resolve_profile`.
    pub deny_exact: Vec<PathBuf>,
    /// Deny globs (e.g. `**/*.pem`). Expanded best-effort by the backend with
    /// the bounds below; carried verbatim through resolution.
    pub deny_globs: Vec<String>,
    pub restrict_network: bool,
}

/// Deny-glob expansion caps (fail-closed on overflow). Consumed by the backend
/// FS-walk phase (`sandbox_glob`), defined here alongside the policy they bound.
pub const DENY_GLOB_MAX_DEPTH: usize = 8;
pub const DENY_GLOB_MAX_MATCHES: usize = 4096;
pub const DENY_GLOB_MAX_ENTRIES: usize = 256;

/// Read-only system roots granted by the `strict` profile so that programs can
/// still be located and dynamically linked. Nothing here is writable.
const STRICT_SYSTEM_READ_ROOTS: &[&str] = &["/usr", "/lib", "/lib64", "/bin", "/sbin", "/etc"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProfileResolveError {
    #[error("custom profile '{0}' not found")]
    NotFound(String),
    #[error("custom profile cannot extend another custom profile")]
    ExtendsCustom,
    #[error("'off' is not a valid base profile")]
    ExtendsOff,
    #[error("deny glob expansion exceeded caps (fail-closed)")]
    GlobOverflow,
}

/// A deny entry is a glob if it carries any wildcard metacharacter; otherwise it
/// is treated as an exact path.
fn deny_is_glob(entry: &str) -> bool {
    entry.contains('*') || entry.contains('?') || entry.contains('[')
}

fn split_deny(deny: &[String], exact: &mut Vec<PathBuf>, globs: &mut Vec<String>) {
    for entry in deny {
        if deny_is_glob(entry) {
            globs.push(entry.clone());
        } else {
            exact.push(PathBuf::from(entry));
        }
    }
}

/// Resolve a `ProfileName` + trusted config into a `ResolvedSandboxPolicy`.
///
/// Pure (no FS access): glob entries are carried verbatim into `deny_globs` for
/// later bounded expansion. Credential paths from the workspace's protected-path
/// policy are merged into `deny_exact` for **every** profile — a profile can
/// never grant access to a credential path.
pub fn resolve_profile(
    name: &ProfileName,
    workspace: &WorkspacePolicy,
    profiles: &SandboxProfiles,
) -> Result<ResolvedSandboxPolicy, ProfileResolveError> {
    let mut policy = match name {
        // "off": no added restriction (credentials are still denied below).
        ProfileName::Off => ResolvedSandboxPolicy {
            name: "off".to_string(),
            read_only_roots: vec![PathBuf::from("/")],
            read_write_roots: workspace.writable_roots().to_vec(),
            deny_exact: Vec::new(),
            deny_globs: Vec::new(),
            restrict_network: false,
        },
        // "workspace": read the whole disk, write only the declared roots.
        ProfileName::Workspace => ResolvedSandboxPolicy {
            name: "workspace".to_string(),
            read_only_roots: vec![PathBuf::from("/")],
            read_write_roots: workspace.writable_roots().to_vec(),
            deny_exact: Vec::new(),
            deny_globs: Vec::new(),
            restrict_network: false,
        },
        // "read-only": read anything, write nothing, no network.
        ProfileName::ReadOnly => ResolvedSandboxPolicy {
            name: "read-only".to_string(),
            read_only_roots: vec![PathBuf::from("/")],
            read_write_roots: Vec::new(),
            deny_exact: Vec::new(),
            deny_globs: Vec::new(),
            restrict_network: true,
        },
        // "strict": read only system roots + the workspace, write declared roots,
        // no network.
        ProfileName::Strict => {
            let mut read_only_roots: Vec<PathBuf> =
                STRICT_SYSTEM_READ_ROOTS.iter().map(PathBuf::from).collect();
            read_only_roots.push(workspace.cwd().to_path_buf());
            ResolvedSandboxPolicy {
                name: "strict".to_string(),
                read_only_roots,
                read_write_roots: workspace.writable_roots().to_vec(),
                deny_exact: Vec::new(),
                deny_globs: Vec::new(),
                restrict_network: true,
            }
        }
        // custom: resolve the built-in base, then layer the overrides.
        ProfileName::Custom(cname) => {
            let cfg = profiles
                .profiles
                .get(cname)
                .ok_or_else(|| ProfileResolveError::NotFound(cname.clone()))?;
            let base_name = match cfg.extends.as_deref() {
                None => ProfileName::Workspace,
                // FromStr for ProfileName is Infallible.
                Some(base) => match base.parse::<ProfileName>().unwrap() {
                    ProfileName::Custom(_) => return Err(ProfileResolveError::ExtendsCustom),
                    ProfileName::Off => return Err(ProfileResolveError::ExtendsOff),
                    resolved => resolved,
                },
            };
            let mut base = resolve_profile(&base_name, workspace, profiles)?;
            base.name = cname.clone();
            base.read_only_roots
                .extend(cfg.read_only.iter().map(PathBuf::from));
            base.read_write_roots
                .extend(cfg.read_write.iter().map(PathBuf::from));
            split_deny(&cfg.deny, &mut base.deny_exact, &mut base.deny_globs);
            if let Some(net) = cfg.restrict_network {
                base.restrict_network = net;
            }
            base
        }
    };

    // Credential paths are ALWAYS denied, no matter the profile (fail-closed).
    for cred in workspace.protected_paths().credential_paths() {
        if !policy.deny_exact.contains(cred) {
            policy.deny_exact.push(cred.clone());
        }
    }

    Ok(policy)
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    fn cfg(restrict_network: bool, deny: &[&str]) -> SandboxProfileConfig {
        SandboxProfileConfig {
            extends: Some("workspace".to_string()),
            restrict_network: Some(restrict_network),
            read_only: vec![],
            read_write: vec![],
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn project_cannot_redefine_global_profile() {
        let mut global = SandboxProfiles::default();
        global
            .profiles
            .insert("secure".to_string(), cfg(true, &["/home/u/.ssh"]));

        let mut project = SandboxProfiles::default();
        // Malicious hollow-out: same name, no deny, network on.
        project
            .profiles
            .insert("secure".to_string(), cfg(false, &[]));
        // A genuinely new project profile is allowed through.
        project
            .profiles
            .insert("project-only".to_string(), cfg(true, &["./secrets"]));

        global.merge_project_additive(project);

        // Global "secure" preserved (deny intact, network still restricted).
        let secure = &global.profiles["secure"];
        assert_eq!(secure.deny, vec!["/home/u/.ssh".to_string()]);
        assert_eq!(secure.restrict_network, Some(true));
        // New project-only name accepted.
        assert!(global.profiles.contains_key("project-only"));
    }

    #[test]
    fn conflicting_redefinitions_reports_changed_names_only() {
        let mut global = SandboxProfiles::default();
        global.profiles.insert("a".to_string(), cfg(true, &["/x"]));
        global
            .profiles
            .insert("same".to_string(), cfg(true, &["/y"]));

        let mut project = SandboxProfiles::default();
        project.profiles.insert("a".to_string(), cfg(false, &[])); // changed
        project
            .profiles
            .insert("same".to_string(), cfg(true, &["/y"])); // identical
        project
            .profiles
            .insert("new".to_string(), cfg(true, &["/z"])); // new

        assert_eq!(global.conflicting_redefinitions(&project), vec!["a"]);
    }

    #[test]
    fn profile_name_parse_and_display_roundtrip() {
        for (s, expected) in [
            ("workspace", ProfileName::Workspace),
            ("read-only", ProfileName::ReadOnly),
            ("readonly", ProfileName::ReadOnly),
            ("strict", ProfileName::Strict),
            ("off", ProfileName::Off),
            ("none", ProfileName::Off),
        ] {
            assert_eq!(s.parse::<ProfileName>().unwrap(), expected);
        }
        assert_eq!(
            "my-custom".parse::<ProfileName>().unwrap(),
            ProfileName::Custom("my-custom".to_string())
        );
        assert_eq!(ProfileName::ReadOnly.to_string(), "read-only");
        assert_eq!(ProfileName::Custom("x".into()).to_string(), "x");
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use crate::types::local_authority::ProtectedPathPolicy;

    fn ws() -> WorkspacePolicy {
        WorkspacePolicy::from_resolved_roots(PathBuf::from("/home/u/proj"), vec![]).unwrap()
    }

    fn custom(name: &str, cfg: SandboxProfileConfig) -> SandboxProfiles {
        let mut p = SandboxProfiles::default();
        p.profiles.insert(name.to_string(), cfg);
        p
    }

    // T2
    #[test]
    fn workspace_reads_all_writes_declared_roots() {
        let policy =
            resolve_profile(&ProfileName::Workspace, &ws(), &SandboxProfiles::default()).unwrap();
        assert_eq!(policy.read_only_roots, vec![PathBuf::from("/")]);
        assert_eq!(policy.read_write_roots, vec![PathBuf::from("/home/u/proj")]);
        assert!(!policy.restrict_network);
        assert!(policy.deny_exact.is_empty());
    }

    // T3
    #[test]
    fn read_only_restricts_network_and_forbids_writes() {
        let policy =
            resolve_profile(&ProfileName::ReadOnly, &ws(), &SandboxProfiles::default()).unwrap();
        assert_eq!(policy.read_only_roots, vec![PathBuf::from("/")]);
        assert!(policy.read_write_roots.is_empty());
        assert!(policy.restrict_network);
    }

    // T4
    #[test]
    fn strict_whitelists_system_plus_workspace_and_restricts_network() {
        let policy =
            resolve_profile(&ProfileName::Strict, &ws(), &SandboxProfiles::default()).unwrap();
        assert!(policy.read_only_roots.contains(&PathBuf::from("/usr")));
        assert!(policy
            .read_only_roots
            .contains(&PathBuf::from("/home/u/proj")));
        // The whole disk is NOT readable under strict.
        assert!(!policy.read_only_roots.contains(&PathBuf::from("/")));
        assert_eq!(policy.read_write_roots, vec![PathBuf::from("/home/u/proj")]);
        assert!(policy.restrict_network);
    }

    // T5
    #[test]
    fn custom_extends_workspace_layers_overrides() {
        let profiles = custom(
            "devbox",
            SandboxProfileConfig {
                extends: Some("workspace".to_string()),
                restrict_network: Some(true),
                read_only: vec![],
                read_write: vec!["/data".to_string()],
                deny: vec!["/home/u/.ssh".to_string(), "**/*.pem".to_string()],
            },
        );
        let policy =
            resolve_profile(&ProfileName::Custom("devbox".into()), &ws(), &profiles).unwrap();
        assert_eq!(policy.name, "devbox");
        // Inherited workspace write root plus the custom addition.
        assert!(policy
            .read_write_roots
            .contains(&PathBuf::from("/home/u/proj")));
        assert!(policy.read_write_roots.contains(&PathBuf::from("/data")));
        // Network override applied.
        assert!(policy.restrict_network);
        // Deny split into exact vs glob.
        assert!(policy.deny_exact.contains(&PathBuf::from("/home/u/.ssh")));
        assert_eq!(policy.deny_globs, vec!["**/*.pem".to_string()]);
    }

    // T6
    #[test]
    fn custom_error_branches() {
        // not found
        assert_eq!(
            resolve_profile(
                &ProfileName::Custom("ghost".into()),
                &ws(),
                &SandboxProfiles::default()
            ),
            Err(ProfileResolveError::NotFound("ghost".to_string()))
        );
        // extends another custom
        let extends_custom = custom(
            "bad",
            SandboxProfileConfig {
                extends: Some("other-custom".to_string()),
                restrict_network: None,
                read_only: vec![],
                read_write: vec![],
                deny: vec![],
            },
        );
        assert_eq!(
            resolve_profile(&ProfileName::Custom("bad".into()), &ws(), &extends_custom),
            Err(ProfileResolveError::ExtendsCustom)
        );
        // extends off
        let extends_off = custom(
            "bad2",
            SandboxProfileConfig {
                extends: Some("off".to_string()),
                restrict_network: None,
                read_only: vec![],
                read_write: vec![],
                deny: vec![],
            },
        );
        assert_eq!(
            resolve_profile(&ProfileName::Custom("bad2".into()), &ws(), &extends_off),
            Err(ProfileResolveError::ExtendsOff)
        );
    }

    // T7 — credentials are denied for every profile, even permissive ones.
    #[test]
    fn credentials_always_denied() {
        let protected =
            ProtectedPathPolicy::new(vec![PathBuf::from("/tmp/aletheon-test-cred.json")]).unwrap();
        let workspace = WorkspacePolicy::from_resolved_roots(PathBuf::from("/tmp"), vec![])
            .unwrap()
            .with_protected_paths(protected);
        for name in [
            ProfileName::Workspace,
            ProfileName::ReadOnly,
            ProfileName::Strict,
            ProfileName::Off,
        ] {
            let policy = resolve_profile(&name, &workspace, &SandboxProfiles::default()).unwrap();
            assert!(
                policy
                    .deny_exact
                    .iter()
                    .any(|p| p.ends_with("aletheon-test-cred.json")),
                "profile {name} must deny the credential path"
            );
        }
    }
}

#[cfg(test)]
mod executor_tests {
    use super::*;

    /// Minimal backend whose network-isolation capability is configurable, so
    /// the S1 T11 consistency check can be exercised without a real sandbox.
    struct MockBackend {
        name: &'static str,
        isolation: IsolationLevel,
        network_isolation: bool,
    }

    #[async_trait]
    impl SandboxBackend for MockBackend {
        fn name(&self) -> &str {
            self.name
        }
        fn isolation_level(&self) -> IsolationLevel {
            self.isolation
        }
        fn is_available(&self) -> bool {
            true
        }
        fn capabilities(&self) -> SandboxCapabilities {
            SandboxCapabilities {
                filesystem_isolation: true,
                network_isolation: self.network_isolation,
                resource_limits: false,
                seccomp_filter: false,
                limitations: vec![],
            }
        }
        async fn execute(
            &self,
            _cmd: &str,
            _config: &SandboxConfig,
            _timeout: Duration,
        ) -> anyhow::Result<SandboxResult> {
            Ok(SandboxResult {
                stdout: "ran".into(),
                stderr: String::new(),
                exit_code: 0,
                backend_used: self.name.to_string(),
                isolation_level: self.isolation,
                elapsed_ms: 0,
            })
        }
    }

    fn config_with_network_restriction(restrict: bool) -> SandboxConfig {
        SandboxConfig {
            workspace: WorkspacePolicy::from_resolved_roots(PathBuf::from("/tmp"), vec![]).unwrap(),
            environment: BTreeMap::new(),
            policy: Some(ResolvedSandboxPolicy {
                name: "read-only".into(),
                read_only_roots: vec![PathBuf::from("/")],
                read_write_roots: vec![],
                deny_exact: vec![],
                deny_globs: vec![],
                restrict_network: restrict,
            }),
        }
    }

    #[tokio::test]
    async fn require_fails_closed_when_backend_lacks_network_isolation() {
        let executor = SandboxExecutor::new(
            vec![Box::new(MockBackend {
                name: "namespace",
                isolation: IsolationLevel::Namespace,
                network_isolation: false,
            })],
            SandboxPreference::Require,
        );
        let err = executor
            .run(
                "echo hi",
                &config_with_network_restriction(true),
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("requires network isolation"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn best_effort_runs_degraded_when_backend_lacks_network_isolation() {
        let executor = SandboxExecutor::new(
            vec![Box::new(MockBackend {
                name: "namespace",
                isolation: IsolationLevel::Namespace,
                network_isolation: false,
            })],
            SandboxPreference::BestEffort,
        );
        // Degraded, not fail-closed: it warns and still executes.
        let result = executor
            .run(
                "echo hi",
                &config_with_network_restriction(true),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn require_runs_when_backend_provides_network_isolation() {
        let executor = SandboxExecutor::new(
            vec![Box::new(MockBackend {
                name: "namespace",
                isolation: IsolationLevel::Namespace,
                network_isolation: true,
            })],
            SandboxPreference::Require,
        );
        let result = executor
            .run(
                "echo hi",
                &config_with_network_restriction(true),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn none_policy_skips_the_network_consistency_check() {
        let executor = SandboxExecutor::new(
            vec![Box::new(MockBackend {
                name: "namespace",
                isolation: IsolationLevel::Namespace,
                network_isolation: false,
            })],
            SandboxPreference::Require,
        );
        let config = SandboxConfig {
            workspace: WorkspacePolicy::from_resolved_roots(PathBuf::from("/tmp"), vec![]).unwrap(),
            environment: BTreeMap::new(),
            policy: None,
        };
        let result = executor
            .run("echo hi", &config, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
    }
}
