//! DefaultMetaRuntime — concrete implementation of MetaRuntimeOps.
//!
//! Wires together: SelfReader → CandidateGenerator → SandboxRunner → Evaluator → MigrationManager → RollbackManager

use anyhow::Result;
use async_trait::async_trait;
use fabric::{
    Clock, Evaluation, Genome, MetaRuntimeOps, MigrationResult, MutationIntent, RuntimeCandidate,
    Subsystem, SubsystemContext, SubsystemHealth, TestResult, Version,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::evolution::candidate::CandidateGenerator;
use crate::evolution::candidate_evaluator::Evaluator;
use crate::evolution::lineage::LineageTracker;
use crate::evolution::migration::MigrationManager;
use crate::evolution::rollback::RollbackManager;
use crate::evolution::sandbox_runner::SandboxRunner;
use crate::genome::loader::GenomeLoader;
use crate::governance::self_reader::SelfReader;

/// Concrete MetaRuntime implementation.
///
/// Composes the individual meta-runtime components:
/// - SelfReader: reads genome from the runtime environment
/// - CandidateGenerator: applies mutation intents to produce candidates
/// - SandboxRunner: tests candidates in isolation
/// - Evaluator: evaluates candidates after testing
/// - MigrationManager: applies genome changes and records lineage
/// - RollbackManager: reverts to previous genome versions
///
/// **Lineage vs Rollback**: These are complementary, not duplicated.
/// - `LineageTracker` (inside `MigrationManager`) records version history as
///   metadata — which versions existed, parent-child relationships, and why
///   changes happened. Persisted to JSONL via `with_lineage_path()`.
/// - `RollbackManager` holds full `Genome` snapshots in memory so that
///   `rollback()` can restore the actual genome state. It does not persist.
pub struct DefaultMetaRuntime {
    version: Version,
    /// Self-reader for genome introspection — reserved for future use.
    #[allow(dead_code)]
    self_reader: SelfReader,
    candidate_gen: CandidateGenerator,
    sandbox_runner: SandboxRunner,
    evaluator: Evaluator,
    migration_mgr: MigrationManager,
    rollback_mgr: RollbackManager,
    current_genome: Mutex<Option<Genome>>,
    genome_path: Option<PathBuf>,
}

impl DefaultMetaRuntime {
    pub fn new(version: Version, clock: Arc<dyn Clock>) -> Self {
        Self {
            version,
            self_reader: SelfReader::new(),
            candidate_gen: CandidateGenerator::new(clock.clone()),
            sandbox_runner: SandboxRunner::new(clock.clone()),
            evaluator: Evaluator::new(),
            migration_mgr: MigrationManager::new(clock.clone()),
            rollback_mgr: RollbackManager::new(),
            current_genome: Mutex::new(None),
            genome_path: None,
        }
    }

    /// Create with a custom genome file path.
    pub fn with_genome_path(mut self, path: PathBuf) -> Self {
        self.migration_mgr.set_genome_path(path.clone());
        self.genome_path = Some(path);
        self
    }

    /// Create with a custom working directory for sandbox tests.
    pub fn with_work_dir(mut self, dir: PathBuf, clock: Arc<dyn Clock>) -> Self {
        self.sandbox_runner = SandboxRunner::with_work_dir(dir, clock);
        self
    }

    /// Create with a JSONL file path for lineage persistence.
    ///
    /// If the file exists, its entries are loaded on construction.
    /// New migrations are appended to the file.
    pub fn with_lineage_path(self, path: PathBuf, clock: Arc<dyn Clock>) -> anyhow::Result<Self> {
        let tracker = LineageTracker::with_path(path, clock)?;
        Ok(Self {
            migration_mgr: MigrationManager::with_lineage(tracker),
            ..self
        })
    }

    /// Get the rollback manager for external rollback control.
    pub fn rollback_manager(&self) -> &RollbackManager {
        &self.rollback_mgr
    }
}

// Subsystem trait required by MetaRuntimeOps
#[async_trait]
impl Subsystem for DefaultMetaRuntime {
    fn name(&self) -> &str {
        "meta-runtime"
    }
    fn version(&self) -> Version {
        self.version.clone()
    }
    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        Ok(())
    }
    async fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
    async fn health(&self) -> SubsystemHealth {
        SubsystemHealth::Healthy
    }
}

#[async_trait]
impl MetaRuntimeOps for DefaultMetaRuntime {
    /// Read the current genome.
    ///
    /// If a genome path is configured, loads from disk.
    /// Otherwise, uses the cached genome (if any).
    async fn read_genome(&self) -> Result<Genome> {
        // Try loading from file first
        if let Some(ref path) = self.genome_path {
            let loader = GenomeLoader::new();
            let genome = loader.load(path)?;
            let mut cached = self.current_genome.lock().unwrap();
            *cached = Some(genome.clone());
            return Ok(genome);
        }

        // Fall back to cached genome
        let cached = self.current_genome.lock().unwrap();
        match cached.as_ref() {
            Some(genome) => Ok(genome.clone()),
            None => {
                // No file and no cache — return a default genome
                let loader = GenomeLoader::new();
                Ok(loader.load(std::path::Path::new("/nonexistent/default.genome.yaml"))?)
            }
        }
    }

    /// Generate a candidate runtime from a mutation intent.
    ///
    /// Reads the current genome, then applies the mutation intent to produce
    /// a RuntimeCandidate.
    async fn generate_candidate(&self, intent: &MutationIntent) -> Result<RuntimeCandidate> {
        let genome = self.read_genome().await?;
        let candidate = self.candidate_gen.generate(&genome, intent).await?;

        // Save snapshot for rollback before migration
        {
            let version_str = self.version.to_string();
            self.rollback_mgr.save_snapshot(&version_str, &genome);
        }

        Ok(candidate)
    }

    /// Test a candidate in sandbox.
    async fn sandbox_test(&self, candidate: &RuntimeCandidate) -> Result<TestResult> {
        self.sandbox_runner.run_tests(candidate).await
    }

    /// Evaluate a candidate after testing.
    async fn evaluate(
        &self,
        candidate: &RuntimeCandidate,
        test: &TestResult,
    ) -> Result<Evaluation> {
        self.evaluator.evaluate(candidate, test).await
    }

    /// Migrate to a new runtime.
    ///
    /// Records the migration in the lineage and updates the cached genome.
    async fn migrate(&self, candidate: &RuntimeCandidate) -> Result<MigrationResult> {
        // Use the migration manager to record the version transition
        let result = self.migration_mgr.migrate(candidate).await?;

        // Update cached genome
        {
            let mut cached = self.current_genome.lock().unwrap();
            *cached = Some(candidate.genome.clone());
        }

        Ok(result)
    }

    /// Rollback to the previous runtime version.
    async fn rollback(&self) -> Result<()> {
        let genome = self.rollback_mgr.rollback().await?;

        // Update cached genome to the rolled-back version
        {
            let mut cached = self.current_genome.lock().unwrap();
            *cached = Some(genome);
        }

        Ok(())
    }

    /// Get the current runtime version.
    fn current_version(&self) -> Version {
        self.version.clone()
    }
}
