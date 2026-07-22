//! Deterministic verification contracts for isolated coding jobs.

pub mod checks;
pub mod command;
pub mod policy;

use anyhow::{bail, Context, Result};
use command::{severity, TrustedVerificationCommand, VerificationCommandRunner};
use fabric::{
    AttemptId, ChangedFile, Clock, CodingJobId, GoalId, VerificationCheck, VerificationReport,
};
use kernel::chronos::SystemClock;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub use policy::{VerificationPolicy, VerificationPolicyError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForbiddenDependencyEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchitecturePolicy {
    pub forbidden_path_prefixes: Vec<PathBuf>,
    pub forbidden_import_prefixes: Vec<String>,
    pub forbidden_dependency_edges: Vec<ForbiddenDependencyEdge>,
}

#[derive(Debug, Clone)]
pub struct VerificationServiceConfig {
    pub cargo_program: PathBuf,
    pub git_program: PathBuf,
    pub compile_args: Vec<String>,
    pub relevant_test_args: Vec<Vec<String>>,
    pub command_timeout: Duration,
    pub output_cap_bytes: usize,
    pub environment: BTreeMap<String, String>,
    pub architecture: ArchitecturePolicy,
}

impl VerificationServiceConfig {
    pub fn production(relevant_test_args: Vec<Vec<String>>) -> Result<Self> {
        // Preserve launcher symlinks (notably rustup's `cargo` shim) because
        // their argv[0] selects the intended tool.
        let cargo_program = which::which("cargo").context("locating cargo for verification")?;
        let git_program = which::which("git").context("locating git for verification")?;
        Ok(Self {
            cargo_program,
            git_program,
            compile_args: vec!["check".into(), "--workspace".into()],
            relevant_test_args,
            command_timeout: Duration::from_secs(10 * 60),
            output_cap_bytes: 1024 * 1024,
            environment: BTreeMap::from([("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into())]),
            architecture: ArchitecturePolicy::default(),
        })
    }

    fn validate(&self) -> Result<()> {
        for (name, program) in [("cargo", &self.cargo_program), ("git", &self.git_program)] {
            if !program.is_absolute() || !program.is_file() {
                bail!("verification {name} program must be an existing absolute file");
            }
        }
        if self.command_timeout.is_zero() || self.output_cap_bytes == 0 {
            bail!("verification timeout and output cap must be positive");
        }
        if self.compile_args.is_empty() || self.relevant_test_args.iter().any(Vec::is_empty) {
            bail!("verification command argv must not be empty");
        }
        Ok(())
    }
}

pub struct VerificationService {
    config: VerificationServiceConfig,
    commands: VerificationCommandRunner,
    policy: VerificationPolicy,
    clock: Arc<dyn Clock>,
}

impl VerificationService {
    pub fn new(config: VerificationServiceConfig) -> Result<Self> {
        Self::with_clock(config, Arc::new(SystemClock::new()))
    }

    pub fn with_clock(config: VerificationServiceConfig, clock: Arc<dyn Clock>) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            commands: VerificationCommandRunner::default(),
            policy: VerificationPolicy,
            clock,
        })
    }

    pub async fn verify(
        &self,
        context: &VerificationContext,
        cancel: CancellationToken,
    ) -> Result<VerificationReport> {
        context.validate()?;
        let started = self.clock.wall_now().0;
        let mut checks = Vec::with_capacity(context.selection.checks().len());
        for kind in context.selection.checks() {
            let check = match kind {
                VerificationCheckKind::DiffScope => {
                    self.run_diff_scope(context, cancel.clone()).await
                }
                VerificationCheckKind::Format => {
                    self.run_cargo(
                        *kind,
                        vec!["fmt".into(), "--all".into(), "--".into(), "--check".into()],
                        context,
                        cancel.clone(),
                    )
                    .await
                }
                VerificationCheckKind::Compile => {
                    self.run_cargo(
                        *kind,
                        self.config.compile_args.clone(),
                        context,
                        cancel.clone(),
                    )
                    .await
                }
                VerificationCheckKind::RelevantTests => {
                    self.run_relevant_tests(context, cancel.clone()).await
                }
                VerificationCheckKind::CapabilityPolicy => checks::capability_policy(context),
                VerificationCheckKind::Clippy => {
                    self.run_cargo(
                        *kind,
                        vec![
                            "clippy".into(),
                            "--workspace".into(),
                            "--all-targets".into(),
                            "--".into(),
                            "-D".into(),
                            "warnings".into(),
                        ],
                        context,
                        cancel.clone(),
                    )
                    .await
                }
                VerificationCheckKind::ArchitectureReview => {
                    checks::architecture_review(context, &self.config.architecture)
                }
            };
            checks.push(check);
        }
        let ended = self.clock.wall_now().0.max(started);
        self.policy
            .evaluate(context, checks, started, ended)
            .map_err(Into::into)
    }

    async fn run_cargo(
        &self,
        kind: VerificationCheckKind,
        args: Vec<String>,
        context: &VerificationContext,
        cancel: CancellationToken,
    ) -> VerificationCheck {
        self.commands
            .run(
                kind,
                &TrustedVerificationCommand {
                    program: self.config.cargo_program.clone(),
                    args,
                    timeout: self.config.command_timeout,
                },
                &context.worktree,
                &self.config.environment,
                self.config.output_cap_bytes,
                cancel,
            )
            .await
    }

    async fn run_relevant_tests(
        &self,
        context: &VerificationContext,
        cancel: CancellationToken,
    ) -> VerificationCheck {
        let kind = VerificationCheckKind::RelevantTests;
        if self.config.relevant_test_args.is_empty() {
            return checks::failed(
                kind,
                "no trusted relevant test argv configured".into(),
                vec![],
            );
        }
        let mut combined = VerificationCheck {
            name: kind.as_str().into(),
            severity: severity(kind),
            passed: true,
            timed_out: false,
            cancelled: false,
            summary: String::new(),
            evidence: vec![],
        };
        for args in &self.config.relevant_test_args {
            let check = self
                .run_cargo(kind, args.clone(), context, cancel.clone())
                .await;
            combined.passed &= check.passed;
            combined.timed_out |= check.timed_out;
            combined.cancelled |= check.cancelled;
            combined.evidence.extend(check.evidence);
            if !check.passed {
                combined.evidence.push(check.summary);
            }
        }
        combined.summary = if combined.passed {
            format!(
                "{} trusted relevant test command(s) passed",
                self.config.relevant_test_args.len()
            )
        } else {
            "one or more trusted relevant test commands failed".into()
        };
        combined
    }

    async fn run_diff_scope(
        &self,
        context: &VerificationContext,
        cancel: CancellationToken,
    ) -> VerificationCheck {
        let kind = VerificationCheckKind::DiffScope;
        let command = TrustedVerificationCommand {
            program: self.config.git_program.clone(),
            args: vec![
                "status".into(),
                "--porcelain=v2".into(),
                "-z".into(),
                "--untracked-files=all".into(),
            ],
            timeout: self.config.command_timeout,
        };
        match self
            .commands
            .execute(
                &command,
                &context.worktree,
                &self.config.environment,
                self.config.output_cap_bytes,
                cancel,
            )
            .await
        {
            Ok(output) if output.exit_code == Some(0) && !output.timed_out && !output.cancelled => {
                checks::diff_scope(context, &output.stdout_bytes)
            }
            Ok(output) => command::from_output(kind, severity(kind), output),
            Err(error) => checks::failed(kind, format!("fresh git status failed: {error}"), vec![]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCheckKind {
    DiffScope,
    Format,
    Compile,
    RelevantTests,
    CapabilityPolicy,
    Clippy,
    ArchitectureReview,
}

impl VerificationCheckKind {
    pub const REQUIRED: [Self; 5] = [
        Self::DiffScope,
        Self::Format,
        Self::Compile,
        Self::RelevantTests,
        Self::CapabilityPolicy,
    ];
    pub const ADVISORY: [Self; 2] = [Self::Clippy, Self::ArchitectureReview];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DiffScope => "diff_scope",
            Self::Format => "format",
            Self::Compile => "compile",
            Self::RelevantTests => "relevant_tests",
            Self::CapabilityPolicy => "capability_policy",
            Self::Clippy => "clippy",
            Self::ArchitectureReview => "architecture_review",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::REQUIRED
            .into_iter()
            .chain(Self::ADVISORY)
            .find(|kind| kind.as_str() == name)
    }

    pub const fn required(self) -> bool {
        matches!(
            self,
            Self::DiffScope
                | Self::Format
                | Self::Compile
                | Self::RelevantTests
                | Self::CapabilityPolicy
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationSelection {
    checks: Vec<VerificationCheckKind>,
}

impl VerificationSelection {
    pub fn new(checks: Vec<VerificationCheckKind>) -> Result<Self> {
        let unique: BTreeSet<_> = checks.iter().copied().collect();
        if unique.len() != checks.len() {
            bail!("verification selection contains duplicate checks");
        }
        for required in VerificationCheckKind::REQUIRED {
            if !unique.contains(&required) {
                bail!(
                    "verification selection omits required check {}",
                    required.as_str()
                );
            }
        }
        let mut checks: Vec<_> = unique.into_iter().collect();
        checks.sort();
        Ok(Self { checks })
    }

    pub fn checks(&self) -> &[VerificationCheckKind] {
        &self.checks
    }

    pub fn contains(&self, kind: VerificationCheckKind) -> bool {
        self.checks.binary_search(&kind).is_ok()
    }
}

impl Default for VerificationSelection {
    fn default() -> Self {
        Self::new(
            VerificationCheckKind::REQUIRED
                .into_iter()
                .chain(VerificationCheckKind::ADVISORY)
                .collect(),
        )
        .expect("built-in verification selection is valid")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityAuditSummary {
    pub audit_present: bool,
    pub observed_capabilities: Vec<String>,
    pub allowed_capabilities: Vec<String>,
    #[serde(default)]
    pub unavailable_capabilities: Vec<String>,
}

impl CapabilityAuditSummary {
    pub fn normalized(mut self) -> Self {
        self.observed_capabilities.sort();
        self.observed_capabilities.dedup();
        self.allowed_capabilities.sort();
        self.allowed_capabilities.dedup();
        self.unavailable_capabilities.sort();
        self.unavailable_capabilities.dedup();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationContext {
    pub job_id: CodingJobId,
    pub goal_id: GoalId,
    pub attempt_id: AttemptId,
    pub worktree: PathBuf,
    pub base_commit: String,
    pub changed_files: Vec<ChangedFile>,
    pub allowed_paths: Vec<PathBuf>,
    pub forbidden_paths: Vec<PathBuf>,
    pub capability_audit: CapabilityAuditSummary,
    pub selection: VerificationSelection,
}

impl VerificationContext {
    pub fn validate(&self) -> Result<()> {
        if !self.worktree.is_absolute() || !self.worktree.is_dir() {
            bail!("verification worktree must be an existing absolute directory");
        }
        if self.base_commit.trim().is_empty()
            || self.base_commit.starts_with('-')
            || self.base_commit.chars().any(char::is_whitespace)
        {
            bail!("verification base commit is invalid");
        }
        if self.allowed_paths.is_empty() {
            bail!("verification allowed path scope must not be empty");
        }
        for path in self.allowed_paths.iter().chain(&self.forbidden_paths) {
            if path.as_os_str().is_empty()
                || path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                bail!("verification path policy contains an invalid relative path");
            }
        }
        VerificationSelection::new(self.selection.checks.clone())?;
        Ok(())
    }
}
