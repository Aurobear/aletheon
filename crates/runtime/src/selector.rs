use crate::manifest::{InteractionMode, RuntimeCapability, TaskEncoding, WorkspaceMode};
use crate::RuntimeManifest;
use serde::{Deserialize, Serialize};

const MAX_REJECTIONS: usize = 64;
const MAX_REASONS_PER_RUNTIME: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeSelector {
    Auto,
    Alias(String),
    RequiredCapabilities(Vec<RuntimeCapability>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSelectionRequest {
    pub selector: RuntimeSelector,
    pub required_capabilities: Vec<RuntimeCapability>,
    pub interaction_mode: InteractionMode,
    pub workspace_mode: WorkspaceMode,
    pub task_encoding: TaskEncoding,
    pub max_input_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCandidateRejection {
    pub runtime_id: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSelectionDecision {
    pub selected_runtime_id: String,
    pub effective_capabilities: Vec<RuntimeCapability>,
    pub override_used: bool,
    pub reason: String,
    pub rejections: Vec<RuntimeCandidateRejection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSelectionError {
    pub effective_capabilities: Vec<RuntimeCapability>,
    pub rejections: Vec<RuntimeCandidateRejection>,
}

impl std::fmt::Display for RuntimeSelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "no runtime matches required capabilities and execution constraints"
        )?;
        for rejection in &self.rejections {
            write!(
                formatter,
                "; {}: {}",
                rejection.runtime_id,
                rejection.reasons.join(", ")
            )?;
        }
        Ok(())
    }
}

impl RuntimeSelectionRequest {
    pub fn select<'a>(
        &self,
        manifests: impl IntoIterator<Item = &'a RuntimeManifest>,
    ) -> Result<RuntimeSelectionDecision, RuntimeSelectionError> {
        let mut effective_capabilities = self.required_capabilities.clone();
        if let RuntimeSelector::RequiredCapabilities(extra) = &self.selector {
            effective_capabilities.extend(extra.iter().cloned());
        }
        effective_capabilities.sort();
        effective_capabilities.dedup();

        let mut candidates = Vec::new();
        let mut rejections = Vec::new();
        for manifest in manifests {
            let mut reasons = Vec::new();
            if let Err(message) = manifest.validate() {
                reasons.push(format!("invalid manifest: {message}"));
            }
            if let RuntimeSelector::Alias(alias) = &self.selector {
                if manifest.id != *alias && !manifest.aliases.iter().any(|item| item == alias) {
                    reasons.push(format!("does not match runtime override: {alias}"));
                }
            }
            for capability in &effective_capabilities {
                if !manifest.has(capability) {
                    reasons.push(format!("missing capability: {capability:?}"));
                }
            }
            if !manifest.interaction_modes.contains(&self.interaction_mode) {
                reasons.push(format!(
                    "unsupported interaction mode: {:?}",
                    self.interaction_mode
                ));
            }
            if !manifest.workspace_modes.contains(&self.workspace_mode) {
                reasons.push(format!(
                    "unsupported workspace mode: {:?}",
                    self.workspace_mode
                ));
            }
            if !manifest.task_encodings.contains(&self.task_encoding) {
                reasons.push(format!(
                    "unsupported task encoding: {:?}",
                    self.task_encoding
                ));
            }
            if manifest
                .max_context_tokens
                .is_some_and(|limit| limit < self.max_input_tokens)
            {
                reasons.push(format!(
                    "input token budget {} exceeds runtime context limit {}",
                    self.max_input_tokens,
                    manifest.max_context_tokens.unwrap_or_default()
                ));
            }
            reasons.truncate(MAX_REASONS_PER_RUNTIME);
            if reasons.is_empty() {
                candidates.push(manifest);
            } else if rejections.len() < MAX_REJECTIONS {
                rejections.push(RuntimeCandidateRejection {
                    runtime_id: manifest.id.clone(),
                    reasons,
                });
            }
        }
        rejections.sort_by(|left, right| left.runtime_id.cmp(&right.runtime_id));
        candidates.sort_by(|left, right| {
            (left.priority, left.id.as_str()).cmp(&(right.priority, right.id.as_str()))
        });

        let Some(selected) = candidates.first() else {
            return Err(RuntimeSelectionError {
                effective_capabilities,
                rejections,
            });
        };
        Ok(RuntimeSelectionDecision {
            selected_runtime_id: selected.id.clone(),
            effective_capabilities,
            override_used: matches!(self.selector, RuntimeSelector::Alias(_)),
            reason: format!(
                "selected compatible runtime by priority {} then stable runtime ID",
                selected.priority
            ),
            rejections,
        })
    }
}

impl RuntimeSelector {
    /// Compatibility projection for callers which select only by capabilities.
    pub fn resolve_id<'a>(
        &self,
        manifests: impl IntoIterator<Item = &'a RuntimeManifest>,
        required: &[RuntimeCapability],
    ) -> Result<String, String> {
        RuntimeSelectionRequest {
            selector: self.clone(),
            required_capabilities: required.to_vec(),
            interaction_mode: InteractionMode::Resident,
            workspace_mode: WorkspaceMode::SharedReadOnly,
            task_encoding: TaskEncoding::NaturalLanguage,
            max_input_tokens: 1,
        }
        .select(manifests)
        .map(|decision| decision.selected_runtime_id)
        .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolGovernance;
    use std::collections::BTreeSet;

    fn manifest(
        id: &str,
        aliases: &[&str],
        capabilities: &[RuntimeCapability],
        priority: i32,
    ) -> RuntimeManifest {
        RuntimeManifest {
            id: id.into(),
            aliases: aliases.iter().map(|value| (*value).into()).collect(),
            display_name: id.into(),
            capabilities: capabilities.iter().cloned().collect(),
            interaction_modes: BTreeSet::from([InteractionMode::Resident]),
            workspace_modes: BTreeSet::from([WorkspaceMode::SharedReadOnly]),
            task_encodings: BTreeSet::from([TaskEncoding::NaturalLanguage]),
            tool_governance: ToolGovernance::Observed,
            priority,
            max_context_tokens: Some(1_000_000),
            resource_requirements: Default::default(),
        }
    }

    fn selection_request() -> RuntimeSelectionRequest {
        RuntimeSelectionRequest {
            selector: RuntimeSelector::Auto,
            required_capabilities: vec![RuntimeCapability::CodeRead],
            interaction_mode: InteractionMode::Resident,
            workspace_mode: WorkspaceMode::SharedReadOnly,
            task_encoding: TaskEncoding::NaturalLanguage,
            max_input_tokens: 100,
        }
    }

    #[test]
    fn alias_still_must_satisfy_required_capabilities() {
        let pi = manifest("pi-rpc", &["pi"], &[RuntimeCapability::CodeEdit], 0);
        assert!(RuntimeSelector::Alias("pi".into())
            .resolve_id([&pi], &[RuntimeCapability::Test])
            .is_err());
    }

    #[test]
    fn capability_selection_is_deterministic_and_fail_closed() {
        let pi = manifest(
            "pi-rpc",
            &["pi"],
            &[RuntimeCapability::CodeEdit, RuntimeCapability::Test],
            0,
        );
        assert_eq!(
            RuntimeSelector::RequiredCapabilities(vec![RuntimeCapability::CodeEdit])
                .resolve_id([&pi], &[RuntimeCapability::Test])
                .unwrap(),
            "pi-rpc"
        );
        assert!(RuntimeSelector::Auto
            .resolve_id([&pi], &[RuntimeCapability::DeviceCommand])
            .is_err());
    }

    #[test]
    fn selection_uses_priority_then_id_independent_of_insertion_order() {
        let low = manifest("z-low", &[], &[RuntimeCapability::CodeRead], 20);
        let high = manifest("a-high", &[], &[RuntimeCapability::CodeRead], 10);
        let request = selection_request();
        let left = request.select([&low, &high]).unwrap();
        let right = request.select([&high, &low]).unwrap();
        assert_eq!(left.selected_runtime_id, "a-high");
        assert_eq!(left, right);
    }

    #[test]
    fn alias_override_cannot_bypass_capabilities() {
        let runtime = manifest("analysis", &[], &[RuntimeCapability::CodeRead], 0);
        let mut request = selection_request();
        request.selector = RuntimeSelector::Alias("analysis".into());
        request.required_capabilities = vec![RuntimeCapability::CodeEdit];
        let error = request.select([&runtime]).unwrap_err();
        assert!(error.rejections[0]
            .reasons
            .contains(&"missing capability: CodeEdit".into()));
    }
}
