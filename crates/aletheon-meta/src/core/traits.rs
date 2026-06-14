//! DefaultMetaRuntime — concrete implementation of MetaRuntimeOps.
//!
//! Design skeleton. Implementation comes in a future round.

use async_trait::async_trait;
use anyhow::Result;
use aletheon_abi::{
    MetaRuntimeOps, RuntimeCandidate, TestResult, Evaluation, MigrationResult,
    Genome, MutationIntent, Subsystem, SubsystemHealth, SubsystemContext, Version,
};

/// Concrete MetaRuntime implementation (design skeleton).
pub struct DefaultMetaRuntime {
    version: Version,
}

impl DefaultMetaRuntime {
    pub fn new(version: Version) -> Self {
        Self { version }
    }
}

// Subsystem trait required by MetaRuntimeOps
#[async_trait]
impl Subsystem for DefaultMetaRuntime {
    fn name(&self) -> &str { "meta-runtime" }
    fn version(&self) -> Version { self.version.clone() }
    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> { Ok(()) }
    async fn shutdown(&mut self) -> Result<()> { Ok(()) }
    async fn health(&self) -> SubsystemHealth {
        SubsystemHealth::Healthy
    }
}

#[async_trait]
impl MetaRuntimeOps for DefaultMetaRuntime {
    /// Read the current genome.
    async fn read_genome(&self) -> Result<Genome> {
        todo!("MetaRuntime: read_genome not yet implemented")
    }

    /// Generate a candidate runtime from a mutation intent.
    async fn generate_candidate(&self, _intent: &MutationIntent) -> Result<RuntimeCandidate> {
        todo!("MetaRuntime: generate_candidate not yet implemented")
    }

    /// Test a candidate in sandbox.
    async fn sandbox_test(&self, _candidate: &RuntimeCandidate) -> Result<TestResult> {
        todo!("MetaRuntime: sandbox_test not yet implemented")
    }

    /// Evaluate a candidate after testing.
    async fn evaluate(&self, _candidate: &RuntimeCandidate, _test: &TestResult) -> Result<Evaluation> {
        todo!("MetaRuntime: evaluate not yet implemented")
    }

    /// Migrate to a new runtime.
    async fn migrate(&self, _candidate: &RuntimeCandidate) -> Result<MigrationResult> {
        todo!("MetaRuntime: migrate not yet implemented")
    }

    /// Rollback to the previous runtime version.
    async fn rollback(&self) -> Result<()> {
        todo!("MetaRuntime: rollback not yet implemented")
    }

    /// Get the current runtime version.
    fn current_version(&self) -> Version {
        self.version.clone()
    }
}
