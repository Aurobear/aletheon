//! Deterministic verification contracts for isolated coding jobs.

pub mod policy;

use anyhow::{bail, Result};
use fabric::{AttemptId, ChangedFile, CodingJobId, GoalId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub use policy::{VerificationPolicy, VerificationPolicyError};

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
}

impl CapabilityAuditSummary {
    pub fn normalized(mut self) -> Self {
        self.observed_capabilities.sort();
        self.observed_capabilities.dedup();
        self.allowed_capabilities.sort();
        self.allowed_capabilities.dedup();
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
        VerificationSelection::new(self.selection.checks.clone())?;
        Ok(())
    }
}
