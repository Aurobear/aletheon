//! Morphogenesis Pipeline — the self-evolution flow.
//!
//! Pipeline: read_genome → generate_candidate → sandbox_test → evaluate → migrate
//!
//! Orchestrates the MetaRuntimeOps trait methods in sequence.

use aletheon_abi::{Evaluation, MetaRuntimeOps, MigrationResult, MutationIntent, RuntimeCandidate};
use anyhow::Result;

/// Orchestrates the full morphogenesis pipeline.
pub struct MorphogenesisPipeline<M: MetaRuntimeOps> {
    meta_runtime: M,
}

impl<M: MetaRuntimeOps> MorphogenesisPipeline<M> {
    pub fn new(meta_runtime: M) -> Self {
        Self { meta_runtime }
    }

    /// Run the full pipeline: read_genome → generate_candidate → sandbox_test → evaluate → migrate.
    ///
    /// Takes a pre-generated MutationIntent, produces a candidate, tests it,
    /// evaluates the test results, and migrates if the evaluation recommends adoption.
    pub async fn run(&self, intent: &MutationIntent) -> Result<PipelineResult> {
        tracing::info!(
            "Starting morphogenesis pipeline for target: {}",
            intent.target
        );

        // Step 1: Generate candidate from intent
        let candidate = self.meta_runtime.generate_candidate(intent).await?;
        tracing::info!(
            "Generated candidate {} with {} change(s)",
            candidate.id,
            candidate.changes.len()
        );

        // Step 2: Sandbox test
        let test_result = self.meta_runtime.sandbox_test(&candidate).await?;
        tracing::info!(
            "Sandbox test: {} passed, {} failed ({}ms)",
            test_result.tests_passed,
            test_result.tests_failed,
            test_result.elapsed_ms
        );

        // Step 3: Evaluate
        let evaluation = self.meta_runtime.evaluate(&candidate, &test_result).await?;
        tracing::info!(
            "Evaluation score: {:.2}, recommendation: {:?}",
            evaluation.score,
            evaluation.recommendation
        );

        // Step 4: Migrate if recommended
        let migration = match &evaluation.recommendation {
            aletheon_abi::meta::Recommendation::Adopt => {
                let result = self.meta_runtime.migrate(&candidate).await?;
                tracing::info!(
                    "Migration successful: {} -> {}",
                    result.from_version,
                    result.to_version
                );
                Some(result)
            }
            aletheon_abi::meta::Recommendation::PartialAdopt { changes } => {
                tracing::info!("Partial adopt with {} changes — migrating", changes.len());
                let result = self.meta_runtime.migrate(&candidate).await?;
                Some(result)
            }
            _ => {
                tracing::info!(
                    "Skipping migration — recommendation: {:?}",
                    evaluation.recommendation
                );
                None
            }
        };

        let success = migration.is_some();
        let message = if success {
            format!(
                "Pipeline complete. Candidate {} adopted with score {:.2}.",
                candidate.id, evaluation.score
            )
        } else {
            format!(
                "Pipeline complete. Candidate {} not adopted. Recommendation: {:?}",
                candidate.id, evaluation.recommendation
            )
        };

        Ok(PipelineResult {
            success,
            candidate: Some(candidate),
            evaluation: Some(evaluation),
            migration,
            message,
        })
    }
}

#[derive(Debug)]
pub struct PipelineResult {
    pub success: bool,
    pub candidate: Option<RuntimeCandidate>,
    pub evaluation: Option<Evaluation>,
    pub migration: Option<MigrationResult>,
    pub message: String,
}
