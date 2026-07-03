//! Genome YAML loader — reads genome files and produces a Genome struct.
//!
//! Supports loading/saving genomes from YAML files and computing diffs
//! between two genomes.

use crate::core::types::{ChangeType, Genome, GenomeChange, GenomeMeta};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

/// Loads and saves genomes from YAML files.
pub struct GenomeLoader;

impl Default for GenomeLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl GenomeLoader {
    pub fn new() -> Self {
        Self
    }

    /// Load genome from a YAML file.
    ///
    /// Returns a default genome if the file does not exist.
    pub fn load(&self, path: &Path) -> Result<Genome> {
        if !path.exists() {
            tracing::warn!("Genome file not found: {}, using default", path.display());
            return Ok(Self::default_genome());
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read genome file: {}", path.display()))?;
        let genome: Genome = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse genome YAML: {}", path.display()))?;
        Ok(genome)
    }

    /// Returns a sensible default genome for first-run or missing file scenarios.
    fn default_genome() -> Genome {
        use crate::core::types::*;
        Genome {
            topology: Topology { subsystems: vec![] },
            identity: IdentitySpec {
                name: "aletheon-default".to_string(),
                description: "Default genome — first-run fallback".to_string(),
                self_model: "self-0.1.0".to_string(),
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
                priorities: vec![
                    CarePriority {
                        topic: "safety".to_string(),
                        weight: 1.0,
                    },
                    CarePriority {
                        topic: "helpfulness".to_string(),
                        weight: 0.9,
                    },
                ],
            },
            memory: MemorySpec {
                backends: vec!["default".to_string()],
                compaction_strategy: "lru".to_string(),
            },
            mutation: MutationSpec {
                allowed_targets: vec!["care.priorities".to_string(), "boundary.rules".to_string()],
                require_sandbox: true,
                require_self_field_approval: true,
            },
            lifecycle: LifecycleSpec {
                auto_compact: true,
                health_check_interval_secs: 60,
                max_idle_time_secs: 3600,
            },
        }
    }

    /// Save genome to a YAML file.
    pub fn save(&self, genome: &Genome, path: &Path) -> Result<()> {
        let content =
            serde_yaml::to_string(genome).context("Failed to serialize genome to YAML")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write genome file: {}", path.display()))?;
        Ok(())
    }

    /// Load a GenomeMeta (extended genome with evolution config) from YAML.
    pub fn load_meta(&self, path: &Path) -> Result<GenomeMeta> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read genome meta file: {}", path.display()))?;
        let meta: GenomeMeta = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse genome meta YAML: {}", path.display()))?;
        Ok(meta)
    }

    /// Save a GenomeMeta to YAML.
    pub fn save_meta(&self, meta: &GenomeMeta, path: &Path) -> Result<()> {
        let content =
            serde_yaml::to_string(meta).context("Failed to serialize genome meta to YAML")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write genome meta file: {}", path.display()))?;
        Ok(())
    }

    /// Compute the diff between two genomes.
    ///
    /// Compares care priorities and boundary rules, returning a list of changes.
    pub fn diff(&self, old: &Genome, new: &Genome) -> Vec<GenomeChange> {
        let mut changes = Vec::new();

        // Compare care priorities
        let old_care: HashMap<&str, f64> = old
            .care
            .priorities
            .iter()
            .map(|p| (p.topic.as_str(), p.weight))
            .collect();
        let new_care: HashMap<&str, f64> = new
            .care
            .priorities
            .iter()
            .map(|p| (p.topic.as_str(), p.weight))
            .collect();

        // Check for added or modified care weights
        for (topic, &new_weight) in &new_care {
            match old_care.get(topic) {
                Some(&old_weight) if (old_weight - new_weight).abs() > f64::EPSILON => {
                    changes.push(GenomeChange {
                        path: format!("care.weights.{}", topic),
                        change_type: ChangeType::Modified,
                        old_value: Some(serde_json::json!(old_weight)),
                        new_value: Some(serde_json::json!(new_weight)),
                    });
                }
                None => {
                    changes.push(GenomeChange {
                        path: format!("care.weights.{}", topic),
                        change_type: ChangeType::Added,
                        old_value: None,
                        new_value: Some(serde_json::json!(new_weight)),
                    });
                }
                _ => {} // unchanged
            }
        }

        // Check for removed care weights
        for (topic, &old_weight) in &old_care {
            if !new_care.contains_key(topic) {
                changes.push(GenomeChange {
                    path: format!("care.weights.{}", topic),
                    change_type: ChangeType::Removed,
                    old_value: Some(serde_json::json!(old_weight)),
                    new_value: None,
                });
            }
        }

        // Compare identity
        if old.identity.name != new.identity.name {
            changes.push(GenomeChange {
                path: "identity.name".to_string(),
                change_type: ChangeType::Modified,
                old_value: Some(serde_json::json!(old.identity.name)),
                new_value: Some(serde_json::json!(new.identity.name)),
            });
        }
        if old.identity.description != new.identity.description {
            changes.push(GenomeChange {
                path: "identity.description".to_string(),
                change_type: ChangeType::Modified,
                old_value: Some(serde_json::json!(old.identity.description)),
                new_value: Some(serde_json::json!(new.identity.description)),
            });
        }

        // Compare boundary rules
        let old_rules: HashMap<&str, &str> = old
            .boundary
            .rules
            .iter()
            .map(|r| (r.id.as_str(), r.action.as_str()))
            .collect();
        let new_rules: HashMap<&str, &str> = new
            .boundary
            .rules
            .iter()
            .map(|r| (r.id.as_str(), r.action.as_str()))
            .collect();

        for (id, &new_action) in &new_rules {
            match old_rules.get(id) {
                Some(&old_action) if old_action != new_action => {
                    changes.push(GenomeChange {
                        path: format!("boundary.rules.{}", id),
                        change_type: ChangeType::Modified,
                        old_value: Some(serde_json::json!(old_action)),
                        new_value: Some(serde_json::json!(new_action)),
                    });
                }
                None => {
                    changes.push(GenomeChange {
                        path: format!("boundary.rules.{}", id),
                        change_type: ChangeType::Added,
                        old_value: None,
                        new_value: Some(serde_json::json!(new_action)),
                    });
                }
                _ => {}
            }
        }

        for (id, &old_action) in &old_rules {
            if !new_rules.contains_key(id) {
                changes.push(GenomeChange {
                    path: format!("boundary.rules.{}", id),
                    change_type: ChangeType::Removed,
                    old_value: Some(serde_json::json!(old_action)),
                    new_value: None,
                });
            }
        }

        // Compare subsystem versions
        let old_subs: HashMap<&str, &str> = old
            .topology
            .subsystems
            .iter()
            .map(|s| (s.name.as_str(), s.version.as_str()))
            .collect();
        let new_subs: HashMap<&str, &str> = new
            .topology
            .subsystems
            .iter()
            .map(|s| (s.name.as_str(), s.version.as_str()))
            .collect();

        for (name, &new_ver) in &new_subs {
            match old_subs.get(name) {
                Some(&old_ver) if old_ver != new_ver => {
                    changes.push(GenomeChange {
                        path: format!("topology.subsystems.{}.version", name),
                        change_type: ChangeType::Modified,
                        old_value: Some(serde_json::json!(old_ver)),
                        new_value: Some(serde_json::json!(new_ver)),
                    });
                }
                None => {
                    changes.push(GenomeChange {
                        path: format!("topology.subsystems.{}", name),
                        change_type: ChangeType::Added,
                        old_value: None,
                        new_value: Some(serde_json::json!(new_ver)),
                    });
                }
                _ => {}
            }
        }

        changes
    }
}
