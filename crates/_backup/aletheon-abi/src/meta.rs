//! MetaRuntime trait — like Linux kernel's module subsystem (modprobe/insmod).
//!
//! MetaRuntime is the self-modification engine. It reads the genome,
//! generates candidate runtimes, tests them in sandbox, and migrates
//! to new versions.

use anyhow::Result;
use async_trait::async_trait;

use crate::genome::Genome;
use crate::subsystem::Subsystem;
use crate::self_field::MutationIntent;

/// A candidate runtime generated from a genome mutation.
#[derive(Debug, Clone)]
pub struct RuntimeCandidate {
    pub id: uuid::Uuid,
    pub genome: Genome,
    pub changes: Vec<String>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

/// Result of sandbox testing a candidate.
#[derive(Debug, Clone)]
pub struct TestResult {
    pub passed: bool,
    pub tests_run: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
    pub failures: Vec<String>,
    pub elapsed_ms: u64,
}

/// Evaluation of a candidate after testing.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub score: f64, // 0.0 to 1.0
    pub strengths: Vec<String>,
    pub weaknesses: Vec<String>,
    pub recommendation: Recommendation,
}

#[derive(Debug, Clone)]
pub enum Recommendation {
    Adopt,
    Reject,
    NeedsMoreTesting,
    PartialAdopt { changes: Vec<String> },
}

/// Result of migrating to a new runtime.
#[derive(Debug, Clone)]
pub struct MigrationResult {
    pub success: bool,
    pub from_version: String,
    pub to_version: String,
    pub memories_migrated: usize,
    pub identity_preserved: bool,
    pub message: String,
}

/// MetaRuntime trait — the self-modification engine.
///
/// Like Linux's module subsystem handles loading/unloading kernel modules,
/// MetaRuntime handles loading/unloading/upgrading the agent's own runtime.
#[async_trait]
pub trait MetaRuntimeOps: Subsystem {
    /// Read the current genome.
    async fn read_genome(&self) -> Result<Genome>;

    /// Generate a candidate runtime from a mutation intent.
    async fn generate_candidate(&self, intent: &MutationIntent) -> Result<RuntimeCandidate>;

    /// Test a candidate in sandbox.
    async fn sandbox_test(&self, candidate: &RuntimeCandidate) -> Result<TestResult>;

    /// Evaluate a candidate after testing.
    async fn evaluate(
        &self,
        candidate: &RuntimeCandidate,
        test: &TestResult,
    ) -> Result<Evaluation>;

    /// Migrate to a new runtime (requires SelfField approval).
    async fn migrate(&self, candidate: &RuntimeCandidate) -> Result<MigrationResult>;

    /// Rollback to the previous runtime version.
    async fn rollback(&self) -> Result<()>;

    /// Get the current runtime version.
    fn current_version(&self) -> crate::subsystem::Version;
}
