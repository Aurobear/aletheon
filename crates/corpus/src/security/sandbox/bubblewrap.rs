use anyhow::Result;
use async_trait::async_trait;
use fabric::Clock;
use fabric::Timer;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::BTreeMap, path::Path, path::PathBuf};
use tracing::{info, warn};

use crate::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxCommand, SandboxConfig,
    SandboxResult,
};
use crate::security::sandbox::bwrap_builder::append_mount_plan;
use crate::security::sandbox::policy::FilesystemPolicy;

/// Bubblewrap-based sandbox backend — full namespace isolation.
/// Requires: bwrap binary, user namespace support.
pub struct BubblewrapBackend {
    bwrap_path: String,
    clock: Arc<dyn Clock>,
}

impl BubblewrapBackend {
    /// Probe for bubblewrap availability.
    pub fn probe(clock: Arc<dyn Clock>) -> Option<Self> {
        let bwrap_path = which::which("bwrap").ok()?;
        let path_str = bwrap_path.to_string_lossy().to_string();

        match std::process::Command::new(&bwrap_path)
            .arg("--version")
            .output()
        {
            Ok(output) => {
                let version = String::from_utf8_lossy(&output.stdout);
                info!(version = version.trim(), path = %path_str, "Bubblewrap detected");
                Some(Self {
                    bwrap_path: path_str,
                    clock,
                })
            }
            Err(e) => {
                warn!(error = %e, "Failed to run bwrap --version");
                None
            }
        }
    }

    /// Async probe for runtime bootstrap paths; avoids blocking an executor
    /// thread while validating the configured launcher.
    pub async fn probe_async(clock: Arc<dyn Clock>) -> Option<Self> {
        let bwrap_path = which::which("bwrap").ok()?;
        let path_str = bwrap_path.to_string_lossy().to_string();
        match tokio::process::Command::new(&bwrap_path)
            .arg("--version")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout);
                info!(version = version.trim(), path = %path_str, "Bubblewrap detected");
                Some(Self {
                    bwrap_path: path_str,
                    clock,
                })
            }
            Ok(output) => {
                warn!(status = ?output.status.code(), "bwrap --version failed");
                None
            }
            Err(error) => {
                warn!(%error, "Failed to run bwrap --version");
                None
            }
        }
    }

    fn build_args(&self, cmd: &str, config: &SandboxConfig) -> Vec<String> {
        self.build_argv_args(Path::new("/bin/bash"), &["-c".into(), cmd.into()], config)
    }

    fn build_argv_args(
        &self,
        program: &Path,
        command_args: &[String],
        config: &SandboxConfig,
    ) -> Vec<String> {
        let mut args = vec![
            "--die-with-parent".into(),
            "--unshare-pid".into(),
            "--unshare-ipc".into(),
        ];

        // S1 D1-T5: network isolation is controlled by the resolved policy.
        // Default to unshared; a policy with `restrict_network: false` explicitly
        // opts out so the process can reach the network.
        let restrict_network = config
            .policy
            .as_ref()
            .map(|p| p.restrict_network)
            .unwrap_or(true);
        if restrict_network {
            args.push("--unshare-net".into());
        }
        args.push("--clearenv".into());

        // S1 D1-T5: filesystem mount plan. When the resolved policy specifies
        // explicit `read_only_roots`, use those instead of the default
        // workspace-driven `--ro-bind / /`. Additional `read_write_roots` from
        // the policy are added as `--bind` mounts (skipping duplicates of
        // workspace writable roots).
        if let Some(resolved) = &config.policy {
            if !resolved.read_only_roots.is_empty() {
                push_policy_fs_mounts(&mut args, resolved, &config.workspace);
            } else {
                // Policy exists but has empty read_only_roots → fall back to
                // the workspace-driven mount plan (backward-compatible path).
                let policy = FilesystemPolicy::from_workspace(&config.workspace);
                append_mount_plan(&mut args, &policy);
            }
            // S1 T10: enforce the resolved sandbox profile's exact deny set.
            // Later bwrap mounts override earlier ones, so this must come after
            // the workspace mount plan.
            push_policy_denies(&mut args, resolved);
        } else {
            let policy = FilesystemPolicy::from_workspace(&config.workspace);
            append_mount_plan(&mut args, &policy);
        }

        // Fresh devtmpfs and proc on top of the read-only root
        args.push("--dev".into());
        args.push("/dev".into());
        args.push("--proc".into());
        args.push("/proc".into());

        // bwrap's --dev creates a fresh devtmpfs, but the device nodes
        // (including /dev/null) can be unwritable for non-root users.
        // Explicitly dev-bind the host's /dev/null over the devtmpfs copy
        // so the sandboxed process can redirect output to /dev/null.
        args.push("--dev-bind".into());
        args.push("/dev/null".into());
        args.push("/dev/null".into());

        // Environment variables
        for (key, value) in &config.environment {
            args.push("--setenv".into());
            args.push(key.clone());
            args.push(value.clone());
        }

        // The command to execute
        args.push("--".into());
        args.push(program.to_string_lossy().into_owned());
        args.extend(command_args.iter().cloned());

        args
    }
}

/// Append bwrap args for the filesystem mount plan driven by a resolved policy's
/// `read_only_roots` and `read_write_roots` (S1 D1-T5).
///
/// Each read-only root gets a `--ro-bind <root> <root>`. Each read-write root
/// gets a `--bind <root> <root>` *unless* it is already a workspace writable
/// root (those are handled separately by `append_mount_plan`).
///
/// Protected metadata directories (`.git`, `.aletheon`) inside writable roots
/// are rebound read-only on top, matching the workspace-driven convention.
fn push_policy_fs_mounts(
    args: &mut Vec<String>,
    policy: &fabric::ResolvedSandboxPolicy,
    workspace: &fabric::WorkspacePolicy,
) {
    let workspace_writable: std::collections::HashSet<PathBuf> =
        workspace.writable_roots().iter().cloned().collect();

    // Mount read-only roots.
    for root in &policy.read_only_roots {
        args.push("--ro-bind".into());
        args.push(root.to_string_lossy().into_owned());
        args.push(root.to_string_lossy().into_owned());
    }

    // Mount read-write roots (skip duplicates of workspace writable roots).
    for root in &policy.read_write_roots {
        if workspace_writable.contains(root) {
            continue;
        }
        args.push("--bind".into());
        args.push(root.to_string_lossy().into_owned());
        args.push(root.to_string_lossy().into_owned());
    }

    // Re-protect metadata inside every writable root (both policy-supplied and
    // workspace-supplied).
    let all_writable: std::collections::HashSet<PathBuf> = {
        let mut set = workspace_writable;
        for root in &policy.read_write_roots {
            set.insert(root.clone());
        }
        set
    };
    for root in &all_writable {
        for name in &[".git", ".aletheon"] {
            let sub = root.join(name);
            if sub.exists() {
                args.push("--ro-bind".into());
                args.push(sub.to_string_lossy().into_owned());
                args.push(sub.to_string_lossy().into_owned());
            }
        }
    }
}

/// Append bwrap args that mask a resolved profile's exact deny paths (S1 T10).
///
/// Files and symlinks are bound over with `/dev/null` (reads fail); directories
/// get an empty `--tmpfs` overlay so their real contents are hidden. Paths that
/// do not exist need no masking. `deny_globs` are left to the assembly layer,
/// which expands them fail-closed before execution (T13).
fn push_policy_denies(args: &mut Vec<String>, policy: &fabric::ResolvedSandboxPolicy) {
    for path in &policy.deny_exact {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.is_dir() => {
                args.push("--tmpfs".into());
                args.push(path.to_string_lossy().into_owned());
            }
            Ok(_) => {
                args.push("--ro-bind".into());
                args.push("/dev/null".into());
                args.push(path.to_string_lossy().into_owned());
            }
            Err(_) => {}
        }
    }
}

#[async_trait]
impl SandboxBackend for BubblewrapBackend {
    fn name(&self) -> &str {
        "bubblewrap"
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Namespace
    }

    fn is_available(&self) -> bool {
        which::which("bwrap").is_ok()
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: true,
            network_isolation: true,
            resource_limits: true,
            seccomp_filter: true,
            limitations: vec![
                "Requires user namespace support".into(),
                "Some paths may not be accessible in sandbox".into(),
            ],
        }
    }

    fn wrap_argv(
        &self,
        program: &Path,
        args: &[String],
        config: &SandboxConfig,
    ) -> Result<SandboxCommand> {
        if !program.is_absolute() {
            anyhow::bail!("bubblewrap requires an absolute command path");
        }
        Ok(SandboxCommand {
            program: PathBuf::from(&self.bwrap_path),
            args: self.build_argv_args(program, args, config),
            environment: BTreeMap::new(),
        })
    }

    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult> {
        info!(command = cmd, "Executing command in bubblewrap sandbox");

        let args = self.build_args(cmd, config);
        let start = self.clock.mono_now();

        let result = aletheon_kernel::chronos::SystemTimer
            .timeout(timeout, async {
                tokio::process::Command::new(&self.bwrap_path)
                    .args(&args)
                    .current_dir(config.working_dir())
                    .output()
                    .await
            })
            .await;

        let elapsed = self.clock.mono_now().0.saturating_sub(start.0);

        match result {
            Ok(Ok(output)) => Ok(SandboxResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                backend_used: "bubblewrap".to_string(),
                isolation_level: IsolationLevel::Namespace,
                elapsed_ms: elapsed,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("Bubblewrap execution failed: {}", e)),
            Err(_) => Ok(SandboxResult {
                stdout: String::new(),
                stderr: format!("Command timed out after {} seconds", timeout.as_secs()),
                exit_code: -1,
                backend_used: "bubblewrap".to_string(),
                isolation_level: IsolationLevel::Namespace,
                elapsed_ms: elapsed,
            }),
        }
    }

    async fn execute_streaming(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
        sink: &fabric::ToolEventSink,
    ) -> Result<SandboxResult> {
        let mut command = tokio::process::Command::new(&self.bwrap_path);
        command
            .args(self.build_args(cmd, config))
            .current_dir(config.working_dir());
        super::streaming::execute_command_streaming(
            command,
            timeout,
            "bubblewrap",
            IsolationLevel::Namespace,
            self.clock.clone(),
            sink,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use std::collections::BTreeMap;

    #[test]
    fn argv_wrapper_is_networkless_and_only_worktree_is_writable() {
        let backend = BubblewrapBackend {
            bwrap_path: "/usr/bin/bwrap".into(),
            clock: Arc::new(TestClock::default()),
        };
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                "/managed/job-1".into(),
                vec![],
            )
            .unwrap(),
            environment: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            policy: None,
        };
        let wrapped = backend
            .wrap_argv(
                Path::new("/opt/pi/bin/pi"),
                &["--task".into(), "literal;not-shell".into()],
                &config,
            )
            .unwrap();
        assert_eq!(wrapped.program, PathBuf::from("/usr/bin/bwrap"));
        assert!(wrapped
            .args
            .windows(3)
            .any(|items| items == ["--ro-bind", "/", "/"]));
        assert!(wrapped
            .args
            .windows(3)
            .any(|items| { items == ["--bind", "/managed/job-1", "/managed/job-1"] }));
        assert!(wrapped.args.iter().any(|arg| arg == "--unshare-net"));
        assert!(wrapped.args.iter().any(|arg| arg == "--clearenv"));
        let separator = wrapped.args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(
            &wrapped.args[separator + 1..],
            ["/opt/pi/bin/pi", "--task", "literal;not-shell"]
        );
        assert!(!wrapped.args.iter().any(|arg| arg == "-c"));
    }

    #[test]
    fn protected_metadata_is_rebound_after_writable_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let work = temp.path().join("project");
        std::fs::create_dir_all(work.join(".git")).unwrap();
        let backend = BubblewrapBackend {
            bwrap_path: "/usr/bin/bwrap".into(),
            clock: Arc::new(TestClock::default()),
        };
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(work.clone(), vec![]).unwrap(),
            environment: Default::default(),
            policy: None,
        };
        let args = backend.build_argv_args(Path::new("/bin/true"), &[], &config);
        let working_dir = config.working_dir().to_string_lossy().into_owned();
        let writable = args
            .windows(3)
            .position(|items| items[0] == "--bind" && items[1] == working_dir)
            .unwrap();
        let git = work.join(".git").to_string_lossy().into_owned();
        let protected = args
            .windows(3)
            .position(|items| items[0] == "--ro-bind" && items[1] == git)
            .unwrap();
        assert!(protected > writable);
    }

    fn resolved_policy(deny_exact: Vec<PathBuf>) -> fabric::ResolvedSandboxPolicy {
        fabric::ResolvedSandboxPolicy {
            name: "test".into(),
            read_only_roots: vec![PathBuf::from("/")],
            read_write_roots: vec![],
            deny_exact,
            deny_globs: vec![],
            restrict_network: true,
        }
    }

    #[test]
    fn deny_exact_file_is_masked_with_devnull() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join("id_rsa");
        std::fs::write(&secret, b"KEY").unwrap();
        let backend = BubblewrapBackend {
            bwrap_path: "/usr/bin/bwrap".into(),
            clock: Arc::new(TestClock::default()),
        };
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                temp.path().to_path_buf(),
                vec![],
            )
            .unwrap(),
            environment: Default::default(),
            policy: Some(resolved_policy(vec![secret.clone()])),
        };
        let args = backend.build_argv_args(Path::new("/bin/true"), &[], &config);
        let secret_str = secret.to_string_lossy().into_owned();
        assert!(
            args.windows(3)
                .any(|w| w[0] == "--ro-bind" && w[1] == "/dev/null" && w[2] == secret_str),
            "expected deny file masked with /dev/null: {args:?}"
        );
    }

    #[test]
    fn deny_exact_directory_is_masked_with_tmpfs() {
        let temp = tempfile::tempdir().unwrap();
        let ssh = temp.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        let backend = BubblewrapBackend {
            bwrap_path: "/usr/bin/bwrap".into(),
            clock: Arc::new(TestClock::default()),
        };
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                temp.path().to_path_buf(),
                vec![],
            )
            .unwrap(),
            environment: Default::default(),
            policy: Some(resolved_policy(vec![ssh.clone()])),
        };
        let args = backend.build_argv_args(Path::new("/bin/true"), &[], &config);
        let ssh_str = ssh.to_string_lossy().into_owned();
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--tmpfs" && w[1] == ssh_str),
            "expected deny directory hidden with tmpfs: {args:?}"
        );
    }

    #[test]
    fn none_policy_adds_no_deny_masks() {
        let backend = BubblewrapBackend {
            bwrap_path: "/usr/bin/bwrap".into(),
            clock: Arc::new(TestClock::default()),
        };
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots("/tmp/work".into(), vec![])
                .unwrap(),
            environment: Default::default(),
            policy: None,
        };
        let args = backend.build_argv_args(Path::new("/bin/true"), &[], &config);
        // No profile → no /dev/null read-only masks and no tmpfs overlays.
        assert_eq!(
            args.windows(3)
                .filter(|w| w[0] == "--ro-bind" && w[1] == "/dev/null")
                .count(),
            0
        );
        assert!(!args.iter().any(|a| a == "--tmpfs"));
    }

    /// Process-level coverage for S1 T10. The argv tests above protect mount
    /// ordering; this test proves that bubblewrap actually applies those mounts.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn denied_file_content_is_hidden_while_permitted_file_is_readable() {
        let Some(backend) = BubblewrapBackend::probe(Arc::new(TestClock::default())) else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let denied = temp.path().join("denied.txt");
        let permitted = temp.path().join("permitted.txt");
        std::fs::write(&denied, "DENIED_SECRET").unwrap();
        std::fs::write(&permitted, "PERMITTED_VALUE").unwrap();
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                temp.path().to_path_buf(),
                vec![],
            )
            .unwrap(),
            environment: BTreeMap::from([
                ("DENIED_PATH".into(), denied.to_string_lossy().into_owned()),
                (
                    "PERMITTED_PATH".into(),
                    permitted.to_string_lossy().into_owned(),
                ),
            ]),
            policy: Some(resolved_policy(vec![denied.clone()])),
        };

        // A present bwrap binary can still be unusable when the host disables
        // unprivileged user namespaces. Skip only that backend-reported case.
        let probe = backend
            .execute("true", &config, Duration::from_secs(5))
            .await
            .expect("bubblewrap probe must launch");
        if probe.exit_code != 0
            && [
                "operation not permitted",
                "permission denied",
                "no permissions to create new namespace",
                "creating new namespace failed",
            ]
            .iter()
            .any(|message| probe.stderr.to_ascii_lowercase().contains(message))
        {
            return;
        }
        assert_eq!(
            probe.exit_code, 0,
            "bubblewrap probe failed: {}",
            probe.stderr
        );

        let allowed = backend
            .execute(
                "cat -- \"$PERMITTED_PATH\"",
                &config,
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(
            allowed.exit_code, 0,
            "permitted read failed: {}",
            allowed.stderr
        );
        assert_eq!(allowed.stdout, "PERMITTED_VALUE");

        let blocked = backend
            .execute("cat -- \"$DENIED_PATH\"", &config, Duration::from_secs(5))
            .await
            .unwrap();
        assert_ne!(blocked.stdout, "DENIED_SECRET");
    }
}
