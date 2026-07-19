//! GenomeBridge — converts between GenomeMeta (core) and Genome (ABI).
//!
//! Provides helpers for genome loading, conversion, and config extraction.

use crate::core::types::{CareExt, GenomeMeta, ReasoningConfig};
use anyhow::Result;
use fabric::genome::Genome;
use fabric::Clock;
use std::path::Path;
use std::sync::Arc;

use crate::r#impl::genome::loader::GenomeLoader;

/// Bridge between GenomeMeta (extended metadata) and Genome (ABI level).
pub struct GenomeBridge {
    clock: Arc<dyn Clock>,
}

impl GenomeBridge {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }

    /// Load a Genome from a YAML file path.
    pub fn load_genome(path: &Path) -> Result<Genome> {
        let loader = GenomeLoader::new();
        loader.load(path)
    }

    /// Wrap a Genome into a GenomeMeta with default extensions.
    pub fn genome_to_meta(&self, genome: Genome) -> GenomeMeta {
        GenomeMeta {
            genome,
            genome_version: "0.1.0".to_string(),
            lineage_id: format!("lineage-{}", self.clock.wall_now().0),
            parent_version: None,
            identity_ext: crate::core::types::IdentityExt {
                core_values: vec!["truthfulness".into(), "safety".into()],
                fundamental_purpose: "assist the user".into(),
            },
            care_ext: CareExt::default(),
            reasoning: ReasoningConfig::default(),
            evolution: crate::core::types::EvolutionConfig::default(),
        }
    }

    /// Extract the inner Genome from a GenomeMeta.
    pub fn meta_to_genome(meta: &GenomeMeta) -> &Genome {
        &meta.genome
    }

    /// Extract reasoning config from a GenomeMeta.
    pub fn reasoning_config(meta: &GenomeMeta) -> &ReasoningConfig {
        &meta.reasoning
    }

    /// Extract care extension from a GenomeMeta.
    pub fn care_ext(meta: &GenomeMeta) -> &CareExt {
        &meta.care_ext
    }
}

// Default implementation for CareExt (not in types.rs yet)
#[allow(clippy::derivable_impls)]
impl Default for CareExt {
    fn default() -> Self {
        Self {
            weights: std::collections::HashMap::new(),
            boundary_rules: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn Clock> {
        Arc::new(TestClock::default())
    }

    #[test]
    fn test_genome_to_meta_roundtrip() {
        let loader = GenomeLoader::new();
        let genome = loader.load(Path::new("/nonexistent")).unwrap(); // returns default
        let bridge = GenomeBridge::new(test_clock());
        let meta = bridge.genome_to_meta(genome);
        assert_eq!(meta.genome_version, "0.1.0");
        assert!(meta.parent_version.is_none());

        let extracted = GenomeBridge::meta_to_genome(&meta);
        assert_eq!(extracted.identity.name, "aletheon-default");
    }

    #[test]
    fn test_default_configs() {
        let care = CareExt::default();
        assert!(care.weights.is_empty());

        let reasoning = ReasoningConfig::default();
        assert_eq!(reasoning.default_strategy, "plan-then-execute");
        assert_eq!(reasoning.impasse_threshold, 0.3);
    }
}
