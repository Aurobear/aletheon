//! CandidateBridge — converts between CandidateGenerator outputs and ABI types.

use anyhow::Result;
use fabric::{Clock, Genome, MutationIntent, RuntimeCandidate};
use std::sync::Arc;

use crate::r#impl::morphogenesis::candidate::CandidateGenerator;

/// Bridge for candidate generation — connects MutationIntent to RuntimeCandidate.
pub struct CandidateBridge {
    clock: Arc<dyn Clock>,
}

impl CandidateBridge {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }

    /// Generate a RuntimeCandidate from a genome and mutation intent.
    pub async fn generate_candidate(
        &self,
        genome: &Genome,
        intent: &MutationIntent,
    ) -> Result<RuntimeCandidate> {
        let generator = CandidateGenerator::new(self.clock.clone());
        generator.generate(genome, intent).await
    }

    /// Extract the genome from a RuntimeCandidate.
    pub fn candidate_genome(candidate: &RuntimeCandidate) -> &Genome {
        &candidate.genome
    }

    /// Extract the changes list from a RuntimeCandidate.
    pub fn candidate_changes(candidate: &RuntimeCandidate) -> &[String] {
        &candidate.changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::loader::GenomeLoader;
    use kernel::chronos::TestClock;
    use std::path::Path;

    #[tokio::test]
    async fn test_generate_candidate() {
        let loader = GenomeLoader::new();
        let genome = loader.load(Path::new("/nonexistent")).unwrap();

        let intent = MutationIntent {
            target: "identity.name".to_string(),
            change: serde_json::json!({"new_name": "aletheon-v2"}),
            reason: "test mutation".to_string(),
            reversible: true,
        };

        let bridge = CandidateBridge::new(Arc::new(TestClock::default()));
        let candidate = bridge.generate_candidate(&genome, &intent).await.unwrap();
        assert!(!candidate.changes.is_empty());
    }
}
