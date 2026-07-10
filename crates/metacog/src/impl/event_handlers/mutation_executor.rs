//! Subscribes to MutationIntentEvent, executes Morphogenesis Pipeline.
//!
//! Takes approved MutationIntents from SelfField and runs them through
//! candidate generation -> sandbox testing -> evaluation -> migration.

use crate::r#impl::morphogenesis::pipeline::MorphogenesisPipeline;
use anyhow::Result;
use fabric::evolution::EvolutionResultPayload;
use fabric::self_field::MutationIntent;
use fabric::MetaRuntimeOps;

/// Executes mutation intents through the Morphogenesis Pipeline.
pub struct MutationExecutor<M: MetaRuntimeOps> {
    pipeline: MorphogenesisPipeline<M>,
}

impl<M: MetaRuntimeOps> MutationExecutor<M> {
    pub fn new(pipeline: MorphogenesisPipeline<M>) -> Self {
        Self { pipeline }
    }

    /// Process approved mutation intents. Returns evolution results.
    pub async fn handle(&self, intents: &[MutationIntent]) -> Result<Vec<EvolutionResultPayload>> {
        let mut results = Vec::new();

        for intent in intents {
            let result = self.pipeline.run(intent).await?;

            // Derive version info from the candidate and migration result
            let (version_before, version_after) = match &result.migration {
                Some(migration) => (
                    migration.from_version.clone(),
                    Some(migration.to_version.clone()),
                ),
                None => {
                    // No migration happened — use candidate ID as version reference
                    let candidate_id = result
                        .candidate
                        .as_ref()
                        .map(|c| c.id.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    (candidate_id, None)
                }
            };

            results.push(EvolutionResultPayload {
                adopted: result.success,
                genome_version_before: version_before,
                genome_version_after: version_after,
                summary: result.message,
            });
        }

        Ok(results)
    }
}
