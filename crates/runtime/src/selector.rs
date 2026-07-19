use crate::manifest::RuntimeCapability;
use crate::RuntimeManifest;

pub enum RuntimeSelector {
    Auto,
    Alias(String),
    RequiredCapabilities(Vec<RuntimeCapability>),
}

impl RuntimeSelector {
    /// Resolve a manifest ID without owning runtime instances or execution
    /// lifecycle. Executive remains the registry and admission authority.
    pub fn resolve_id<'a>(
        &self,
        manifests: impl IntoIterator<Item = &'a RuntimeManifest>,
        required: &[RuntimeCapability],
    ) -> Result<String, String> {
        let mut required_all = required.to_vec();
        if let RuntimeSelector::RequiredCapabilities(extra) = self {
            required_all.extend(extra.iter().cloned());
        }
        required_all.sort();
        required_all.dedup();

        let mut candidates = manifests
            .into_iter()
            .filter(|manifest| {
                required_all
                    .iter()
                    .all(|capability| manifest.has(capability))
            })
            .filter(|manifest| match self {
                RuntimeSelector::Alias(alias) => {
                    manifest.id == *alias || manifest.aliases.iter().any(|item| item == alias)
                }
                RuntimeSelector::Auto | RuntimeSelector::RequiredCapabilities(_) => true,
            })
            .map(|manifest| manifest.id.clone())
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.into_iter().next().ok_or_else(|| {
            format!(
                "no runtime matches selector and required capabilities: {:?}",
                required_all
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InteractionMode, ToolGovernance, WorkspaceMode};
    use std::collections::BTreeSet;

    fn manifest(id: &str, aliases: &[&str], capabilities: &[RuntimeCapability]) -> RuntimeManifest {
        RuntimeManifest {
            id: id.into(),
            aliases: aliases.iter().map(|value| (*value).into()).collect(),
            display_name: id.into(),
            capabilities: capabilities.iter().cloned().collect(),
            interaction_modes: BTreeSet::from([InteractionMode::Resident]),
            workspace_mode: WorkspaceMode::Shared,
            tool_governance: ToolGovernance::Observed,
        }
    }

    #[test]
    fn alias_still_must_satisfy_required_capabilities() {
        let pi = manifest("pi-rpc", &["pi"], &[RuntimeCapability::CodeEdit]);
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
}
