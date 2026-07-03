//! Spec editor — modifies genome specifications.
//!
//! Applies GenomePatch operations to a Genome: modify care weights,
//! add/remove boundary rules, and other targeted mutations.

use crate::core::types::{CarePriority, Genome, GenomePatch, PatchOperation};
use anyhow::{bail, Result};

pub struct SpecEditor;

impl Default for SpecEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl SpecEditor {
    pub fn new() -> Self {
        Self
    }

    /// Apply a patch to the genome.
    ///
    /// Supports operations on:
    /// - `care.priorities`: Modify/Add/Remove care priority entries
    /// - `boundary.rules`: Modify/Add/Remove boundary rules
    /// - `identity.name` / `identity.description`: Modify identity fields
    pub async fn apply_patch(&self, genome: &mut Genome, patch: &GenomePatch) -> Result<()> {
        match patch.target.as_str() {
            "care.priorities" => {
                self.apply_care_patch(genome, patch)?;
            }
            "boundary.rules" => {
                self.apply_boundary_patch(genome, patch)?;
            }
            "identity.name" => {
                if let PatchOperation::Replace | PatchOperation::Modify = patch.operation {
                    if let Some(name) = patch.value.as_str() {
                        genome.identity.name = name.to_string();
                    }
                }
            }
            "identity.description" => {
                if let PatchOperation::Replace | PatchOperation::Modify = patch.operation {
                    if let Some(desc) = patch.value.as_str() {
                        genome.identity.description = desc.to_string();
                    }
                }
            }
            _ => {
                bail!("Unsupported patch target: {}", patch.target);
            }
        }
        Ok(())
    }

    fn apply_care_patch(&self, genome: &mut Genome, patch: &GenomePatch) -> Result<()> {
        match &patch.operation {
            PatchOperation::Modify | PatchOperation::Replace => {
                // Expect value to be {"topic": "...", "weight": 0.x}
                let topic = patch
                    .value
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("care.priorities patch requires 'topic' field")
                    })?;
                let weight = patch
                    .value
                    .get("weight")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| {
                        anyhow::anyhow!("care.priorities patch requires 'weight' field")
                    })?;

                if let Some(priority) = genome.care.priorities.iter_mut().find(|p| p.topic == topic)
                {
                    priority.weight = weight.clamp(0.0, 1.0);
                } else {
                    // Add new priority if it doesn't exist
                    genome.care.priorities.push(CarePriority {
                        topic: topic.to_string(),
                        weight: weight.clamp(0.0, 1.0),
                    });
                }
            }
            PatchOperation::Add => {
                let topic = patch
                    .value
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("care.priorities add requires 'topic' field"))?;
                let weight = patch
                    .value
                    .get("weight")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5);

                // Only add if not already present
                if !genome.care.priorities.iter().any(|p| p.topic == topic) {
                    genome.care.priorities.push(CarePriority {
                        topic: topic.to_string(),
                        weight: weight.clamp(0.0, 1.0),
                    });
                }
            }
            PatchOperation::Remove => {
                let topic = patch
                    .value
                    .as_str()
                    .or_else(|| patch.value.get("topic").and_then(|v| v.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("care.priorities remove requires 'topic'"))?;
                genome.care.priorities.retain(|p| p.topic != topic);
            }
        }
        Ok(())
    }

    fn apply_boundary_patch(&self, genome: &mut Genome, patch: &GenomePatch) -> Result<()> {
        use crate::core::types::BoundaryRuleSpec;

        match &patch.operation {
            PatchOperation::Add => {
                let id = patch
                    .value
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("boundary.rules add requires 'id'"))?;
                let condition = patch
                    .value
                    .get("condition")
                    .and_then(|v| v.as_str())
                    .unwrap_or("true")
                    .to_string();
                let action = patch
                    .value
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("allow")
                    .to_string();
                let priority = patch
                    .value
                    .get("priority")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100) as u32;

                // Don't add duplicate IDs
                if !genome.boundary.rules.iter().any(|r| r.id == id) {
                    genome.boundary.rules.push(BoundaryRuleSpec {
                        id: id.to_string(),
                        condition,
                        action,
                        priority,
                    });
                }
            }
            PatchOperation::Remove => {
                let id = patch
                    .value
                    .as_str()
                    .or_else(|| patch.value.get("id").and_then(|v| v.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("boundary.rules remove requires 'id'"))?;
                genome.boundary.rules.retain(|r| r.id != id);
            }
            PatchOperation::Modify | PatchOperation::Replace => {
                let id = patch
                    .value
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("boundary.rules modify requires 'id'"))?;

                if let Some(rule) = genome.boundary.rules.iter_mut().find(|r| r.id == id) {
                    if let Some(condition) = patch.value.get("condition").and_then(|v| v.as_str()) {
                        rule.condition = condition.to_string();
                    }
                    if let Some(action) = patch.value.get("action").and_then(|v| v.as_str()) {
                        rule.action = action.to_string();
                    }
                    if let Some(priority) = patch.value.get("priority").and_then(|v| v.as_u64()) {
                        rule.priority = priority as u32;
                    }
                } else {
                    bail!("Boundary rule '{}' not found for modification", id);
                }
            }
        }
        Ok(())
    }
}
