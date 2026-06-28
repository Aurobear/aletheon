//! Self-reader — reads the agent's own genome and runtime state.
//!
//! Reads identity, care, and boundary information from the SelfField
//! and produces a Genome struct.

use anyhow::Result;
use aletheon_abi::{
    SelfFieldOps, Version,
    genome::{
        Topology, IdentitySpec, BoundarySpec, BoundaryRuleSpec,
        CareSpec, CarePriority, MemorySpec, MutationSpec, LifecycleSpec,
    },
};

use crate::core::types::Genome as MetaGenome;

/// Reads the current genome from the runtime environment.
///
/// Takes a reference to a SelfField implementation to extract identity,
/// care, and boundary information.
pub struct SelfReader {
    version: Version,
}

impl SelfReader {
    pub fn new() -> Self {
        Self {
            version: Version::new(0, 1, 0),
        }
    }

    /// Read the current genome from the SelfField.
    ///
    /// Extracts identity, care priorities, and boundary rules from SelfField
    /// and produces a Genome. Uses sensible defaults for fields not available
    /// from SelfField (topology, memory, mutation, lifecycle).
    pub async fn read_genome<S: SelfFieldOps>(&self, self_field: &S) -> Result<MetaGenome> {
        let identity = self_field.identity().await?;
        let cares = self_field.cares().await?;

        Ok(MetaGenome {
            topology: Topology {
                subsystems: vec![],
            },
            identity: IdentitySpec {
                name: identity.name,
                description: identity.description,
                self_model: format!("self-{}", identity.version),
            },
            boundary: BoundarySpec {
                rules: vec![
                    BoundaryRuleSpec {
                        id: "default-safety".to_string(),
                        condition: "safety >= 0.8".to_string(),
                        action: "allow".to_string(),
                        priority: 100,
                    },
                    BoundaryRuleSpec {
                        id: "immutable-core".to_string(),
                        condition: "mutates core identity".to_string(),
                        action: "deny".to_string(),
                        priority: 200,
                    },
                ],
            },
            care: CareSpec {
                priorities: cares
                    .into_iter()
                    .map(|c| CarePriority {
                        topic: c.topic,
                        weight: c.weight,
                    })
                    .collect(),
            },
            memory: MemorySpec {
                backends: vec!["default".to_string()],
                compaction_strategy: "lru".to_string(),
            },
            mutation: MutationSpec {
                allowed_targets: vec![
                    "care.priorities".to_string(),
                    "boundary.rules".to_string(),
                ],
                require_sandbox: true,
                require_self_field_approval: true,
            },
            lifecycle: LifecycleSpec {
                auto_compact: true,
                health_check_interval_secs: 60,
                max_idle_time_secs: 3600,
            },
        })
    }
}

impl Default for SelfReader {
    fn default() -> Self {
        Self::new()
    }
}
