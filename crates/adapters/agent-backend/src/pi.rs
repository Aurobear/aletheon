//! Fail-closed configuration and registration for the isolated Pi coding runtime.

use ::contracts::sandbox::{IsolationLevel, SandboxBackend};
use ::contracts::{
    resolve_profile, ProfileName, ResolvedSandboxPolicy, SandboxProfiles, WorkspacePolicy,
};
use anyhow::{bail, Context, Result};
use cognit::config::CodingRuntimeConfig;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

pub const PI_CODER_RUNTIME_ID: &str = "pi-coder";

/// Import only reviewed process environment keys. Values are injected into a
/// Pi sandbox and are never copied into Agent results or attempt evidence.
pub fn pi_environment_from_process() -> BTreeMap<String, String> {
    const KEYS: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "GOOGLE_API_KEY",
        "GEMINI_API_KEY",
        "PI_CODING_AGENT_DIR",
    ];
    let mut environment: BTreeMap<String, String> = KEYS
        .iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| ((*key).into(), value)))
        .collect();
    environment.insert("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into());
    environment.insert("HOME".into(), "/tmp".into());
    environment.insert("PI_OFFLINE".into(), "1".into());
    environment.insert("PI_SKIP_VERSION_CHECK".into(), "1".into());
    environment.insert("PI_TELEMETRY".into(), "0".into());
    environment
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPiConfig {
    pub executable: PathBuf,
    pub fixed_args: Vec<String>,
    pub package_version: String,
    pub executable_sha256: String,
    pub json_protocol_version: u32,
    pub worktree_base: PathBuf,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub allowed_paths: Vec<PathBuf>,
    pub forbidden_paths: Vec<PathBuf>,
    pub network_enabled: bool,
}

/// Resolve and validate the immutable Pi executable/protocol policy.
/// The resident Pi delegate uses this function directly so configuration
/// authority is not coupled to a second execution implementation.
pub fn resolve_pi_config(
    config: &CodingRuntimeConfig,
    sandbox: Arc<dyn SandboxBackend>,
) -> Result<Option<ResolvedPiConfig>> {
    if !config.enabled {
        return Ok(None);
    }
    if !config.require_namespace_isolation {
        bail!("Pi runtime requires namespace isolation");
    }
    validate_sandbox(sandbox.as_ref())?;
    let executable = resolve_executable(config)?;
    let executable_sha256 = sha256_file(&executable)?;
    if config.package_version.trim().is_empty() {
        bail!("Pi runtime package_version must pin the reviewed upstream release");
    }
    if config.executable_sha256.len() != 64
        || !config
            .executable_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("Pi runtime executable_sha256 must be a lowercase SHA-256 digest");
    }
    if executable_sha256 != config.executable_sha256 {
        bail!("Pi runtime executable does not match its pinned SHA-256");
    }
    if config.json_protocol_version == 0 {
        bail!("Pi runtime JSON protocol version must be nonzero");
    }
    validate_fixed_args(&config.fixed_args)?;
    let worktree_base = canonical_directory(&config.worktree_base, "worktree base")?;
    if config.timeout_ms == 0 || config.max_output_bytes == 0 {
        bail!("Pi runtime timeout and output limit must be nonzero");
    }
    if config.allowed_paths.is_empty() {
        bail!("Pi runtime allowed path scope must not be empty");
    }
    validate_paths(&config.allowed_paths, "allowed")?;
    validate_paths(&config.forbidden_paths, "forbidden")?;
    Ok(Some(ResolvedPiConfig {
        executable,
        fixed_args: config.fixed_args.clone(),
        package_version: config.package_version.clone(),
        executable_sha256,
        json_protocol_version: config.json_protocol_version,
        worktree_base,
        timeout_ms: config.timeout_ms,
        max_output_bytes: config.max_output_bytes,
        allowed_paths: config.allowed_paths.clone(),
        forbidden_paths: config.forbidden_paths.clone(),
        network_enabled: config.network_enabled,
    }))
}

pub(super) fn validate_sandbox(sandbox: &dyn SandboxBackend) -> Result<()> {
    if !sandbox.is_available() {
        bail!("Pi runtime sandbox '{}' is unavailable", sandbox.name());
    }
    if !matches!(
        sandbox.isolation_level(),
        IsolationLevel::Namespace | IsolationLevel::Container
    ) {
        bail!(
            "Pi runtime rejects sandbox '{}' with {:?} isolation",
            sandbox.name(),
            sandbox.isolation_level()
        );
    }
    let capabilities = sandbox.capabilities();
    if !capabilities.filesystem_isolation || !capabilities.network_isolation {
        bail!("Pi runtime sandbox lacks filesystem or network isolation");
    }
    Ok(())
}

pub fn pi_sandbox_policy(
    workspace: &WorkspacePolicy,
    network_enabled: bool,
) -> Result<ResolvedSandboxPolicy> {
    let mut policy = resolve_profile(
        &ProfileName::Workspace,
        workspace,
        &SandboxProfiles::default(),
    )
    .context("resolving Pi workspace sandbox profile")?;
    // Empty mount roots deliberately select Bubblewrap's established
    // workspace-driven plan: host root read-only, declared worktree writable,
    // and protected metadata re-bound read-only. The resolved deny set remains
    // attached and is applied after that plan.
    policy.name = "pi-workspace".into();
    policy.read_only_roots.clear();
    policy.read_write_roots.clear();
    policy.restrict_network = !network_enabled;
    Ok(policy)
}

fn resolve_executable(config: &CodingRuntimeConfig) -> Result<PathBuf> {
    if config.executable.as_os_str().is_empty() {
        bail!("Pi runtime executable is missing");
    }
    let candidate = if config.executable.is_absolute() {
        config.executable.clone()
    } else {
        let trusted = config
            .trusted_executable_dir
            .as_ref()
            .context("relative Pi executable requires trusted_executable_dir")?;
        if config.executable.components().count() != 1 {
            bail!("relative Pi executable must be a single file name");
        }
        canonical_directory(trusted, "trusted executable directory")?.join(&config.executable)
    };
    let executable = candidate
        .canonicalize()
        .with_context(|| format!("resolving Pi executable: {}", candidate.display()))?;
    if !executable.is_file() {
        bail!("Pi executable is not a file: {}", executable.display());
    }
    if let Some(trusted) = &config.trusted_executable_dir {
        let trusted = canonical_directory(trusted, "trusted executable directory")?;
        if !executable.starts_with(trusted) {
            bail!("Pi executable escapes trusted executable directory");
        }
    }
    Ok(executable)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("Pi runtime {label} is missing");
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving Pi runtime {label}: {}", path.display()))?;
    if !canonical.is_dir() {
        bail!("Pi runtime {label} is not a directory");
    }
    Ok(canonical)
}

fn validate_paths(paths: &[PathBuf], label: &str) -> Result<()> {
    for path in paths {
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            bail!("invalid Pi runtime {label} path: {}", path.display());
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("opening Pi executable for hashing: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_fixed_args(args: &[String]) -> Result<()> {
    let required_flags = [
        "--no-session",
        "--no-context-files",
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
        "--no-approve",
        "--offline",
    ];
    if !args.windows(2).any(|pair| pair == ["--mode", "json"]) {
        bail!("Pi runtime requires --mode json");
    }
    for flag in required_flags {
        if !args.iter().any(|arg| arg == flag) {
            bail!("Pi runtime requires isolation flag {flag}");
        }
    }
    for forbidden in [
        "--extension",
        "-e",
        "--skill",
        "--prompt-template",
        "--theme",
        "--approve",
        "-a",
        "--session",
        "--session-dir",
        "--continue",
        "--resume",
        "--fork",
        "--api-key",
    ] {
        if args.iter().any(|arg| arg == forbidden) {
            bail!("Pi runtime rejects unreviewed resource/session flag {forbidden}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::sandbox::{SandboxCapabilities, SandboxConfig, SandboxResult};
    use async_trait::async_trait;
    use tempfile::TempDir;

    struct FakeSandbox {
        name: &'static str,
        level: IsolationLevel,
        available: bool,
        filesystem: bool,
        network: bool,
    }

    #[async_trait]
    impl SandboxBackend for FakeSandbox {
        fn name(&self) -> &str {
            self.name
        }

        fn isolation_level(&self) -> IsolationLevel {
            self.level
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn capabilities(&self) -> SandboxCapabilities {
            SandboxCapabilities {
                filesystem_isolation: self.filesystem,
                network_isolation: self.network,
                resource_limits: true,
                seccomp_filter: false,
                limitations: vec![],
            }
        }

        async fn execute(
            &self,
            _cmd: &str,
            _config: &SandboxConfig,
            _timeout: std::time::Duration,
        ) -> anyhow::Result<SandboxResult> {
            unreachable!("Task 4 registration must not execute Pi")
        }
    }

    fn sandbox(level: IsolationLevel) -> Arc<dyn SandboxBackend> {
        Arc::new(FakeSandbox {
            name: match level {
                IsolationLevel::Namespace => "bubblewrap",
                IsolationLevel::Process => "process",
                IsolationLevel::None => "noop",
                IsolationLevel::Container => "container",
            },
            level,
            available: true,
            filesystem: level != IsolationLevel::None,
            network: matches!(level, IsolationLevel::Namespace | IsolationLevel::Container),
        })
    }

    fn enabled_config(fixture: &TempDir) -> CodingRuntimeConfig {
        let executable = fixture.path().join("pi");
        std::fs::write(&executable, b"#!/bin/sh\n").unwrap();
        let worktree_base = fixture.path().join("worktrees");
        std::fs::create_dir_all(&worktree_base).unwrap();
        CodingRuntimeConfig {
            enabled: true,
            executable: executable.clone(),
            fixed_args: vec![
                "--mode".into(),
                "json".into(),
                "--no-session".into(),
                "--no-context-files".into(),
                "--no-extensions".into(),
                "--no-skills".into(),
                "--no-prompt-templates".into(),
                "--no-themes".into(),
                "--no-approve".into(),
                "--offline".into(),
            ],
            package_version: "0.0.3-test".into(),
            executable_sha256: sha256_file(&executable).unwrap(),
            worktree_base,
            allowed_paths: vec![PathBuf::from("crates"), PathBuf::from("Cargo.toml")],
            forbidden_paths: vec![PathBuf::from(".git"), PathBuf::from(".env")],
            ..Default::default()
        }
    }

    #[test]
    fn disabled_configuration_does_not_require_a_sandbox() {
        assert!(resolve_pi_config(
            &CodingRuntimeConfig::default(),
            sandbox(IsolationLevel::Process),
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn missing_executable_and_invalid_path_policy_fail_closed() {
        let fixture = TempDir::new().unwrap();
        let mut config = enabled_config(&fixture);
        config.executable = fixture.path().join("missing");
        assert!(resolve_pi_config(&config, sandbox(IsolationLevel::Namespace)).is_err());

        config = enabled_config(&fixture);
        config.allowed_paths = vec![PathBuf::from("../escape")];
        assert!(resolve_pi_config(&config, sandbox(IsolationLevel::Namespace)).is_err());
    }

    #[test]
    fn executable_identity_mismatch_fails_closed() {
        let fixture = TempDir::new().unwrap();
        let mut config = enabled_config(&fixture);
        config.executable_sha256 = "0".repeat(64);
        let error = resolve_pi_config(&config, sandbox(IsolationLevel::Namespace)).unwrap_err();
        assert!(format!("{error:#}").contains("pinned SHA-256"));
    }

    #[test]
    fn noop_and_process_sandboxes_are_rejected() {
        let fixture = TempDir::new().unwrap();
        let config = enabled_config(&fixture);
        assert!(resolve_pi_config(&config, sandbox(IsolationLevel::None)).is_err());
        assert!(resolve_pi_config(&config, sandbox(IsolationLevel::Process)).is_err());
    }

    #[test]
    fn namespace_sandbox_is_accepted_and_debug_is_secret_free() {
        let fixture = TempDir::new().unwrap();
        let mut config = enabled_config(&fixture);
        config.package_version = "super-secret".into();
        let resolved = resolve_pi_config(&config, sandbox(IsolationLevel::Namespace))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.package_version, "super-secret");
    }

    #[test]
    fn network_access_requires_explicit_trusted_configuration() {
        let fixture = TempDir::new().unwrap();
        let workspace =
            WorkspacePolicy::from_resolved_roots(fixture.path().to_path_buf(), vec![]).unwrap();

        let restricted = pi_sandbox_policy(&workspace, false).unwrap();
        assert!(restricted.restrict_network);

        let enabled = pi_sandbox_policy(&workspace, true).unwrap();
        assert!(!enabled.restrict_network);
        assert_eq!(enabled.name, "pi-workspace");
        assert!(enabled.read_only_roots.is_empty());
        assert!(enabled.read_write_roots.is_empty());
        assert_eq!(enabled.deny_exact, restricted.deny_exact);
        assert_eq!(enabled.deny_globs, restricted.deny_globs);
    }

    #[test]
    fn api_keys_are_rejected_from_process_arguments() {
        let fixture = TempDir::new().unwrap();
        let mut config = enabled_config(&fixture);
        config.fixed_args.push("--api-key".into());
        config.fixed_args.push("super-secret".into());
        let error = resolve_pi_config(&config, sandbox(IsolationLevel::Namespace)).unwrap_err();
        assert!(format!("{error:#}").contains("--api-key"));
    }
}
