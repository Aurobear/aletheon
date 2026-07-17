//! Sandbox trait and types.

use anyhow::Result;
use async_trait::async_trait;
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SandboxProfiles {
    #[serde(default)]
    pub profiles: BTreeMap<String, SandboxProfileConfig>,
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
