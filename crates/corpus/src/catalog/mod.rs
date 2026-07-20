//! Corpus-owned immutable extension catalog.

use std::collections::BTreeMap;

use fabric::types::admission::RiskLevel;
use fabric::{
    CapabilityId, ExtensionDescriptor, ExtensionId, ExtensionKind, ExtensionOrigin,
    ExtensionSnapshot,
};

use crate::service::CorpusError;

/// Deterministic metadata index. Constructing a catalog never activates entries.
#[derive(Debug, Clone, Default)]
pub struct ExtensionCatalog {
    entries: BTreeMap<ExtensionId, ExtensionDescriptor>,
    executable_capabilities: BTreeMap<String, ExtensionId>,
}

impl ExtensionCatalog {
    pub fn new(
        descriptors: impl IntoIterator<Item = ExtensionDescriptor>,
    ) -> Result<Self, CorpusError> {
        let mut catalog = Self::default();
        for descriptor in descriptors {
            catalog.register(descriptor)?;
        }
        Ok(catalog)
    }

    pub fn register(&mut self, descriptor: ExtensionDescriptor) -> Result<(), CorpusError> {
        let id = descriptor.id.clone();
        if self.entries.contains_key(&id) {
            return Err(CorpusError::DuplicateExtension(id.as_str().to_string()));
        }
        if descriptor.capabilities.is_empty() {
            return Err(CorpusError::InvalidDescriptor(format!(
                "{} declares no capabilities",
                id.as_str()
            )));
        }
        if descriptor.is_executable() {
            for capability in &descriptor.capabilities {
                if let Some(existing) = self.executable_capabilities.get(&capability.0) {
                    return Err(CorpusError::ConflictingCapability {
                        capability: capability.0.clone(),
                        first: existing.as_str().to_string(),
                        second: id.as_str().to_string(),
                    });
                }
            }
        }
        for capability in &descriptor.capabilities {
            if descriptor.is_executable() {
                self.executable_capabilities
                    .insert(capability.0.clone(), id.clone());
            }
        }
        self.entries.insert(id, descriptor);
        Ok(())
    }

    pub(crate) fn entries(&self) -> &BTreeMap<ExtensionId, ExtensionDescriptor> {
        &self.entries
    }

    pub(crate) fn into_entries(self) -> BTreeMap<ExtensionId, ExtensionDescriptor> {
        self.entries
    }
}

impl fabric::ExtensionCatalog for ExtensionCatalog {
    fn snapshot(&self) -> ExtensionSnapshot {
        ExtensionSnapshot {
            entries: self.entries.values().cloned().collect(),
        }
    }
}

/// Index every loaded, non-tool runtime extension before activation.
pub fn discover_runtime_extensions(
    skills: &crate::SkillLoader,
    hooks: &crate::HookRegistry,
) -> Result<Vec<ExtensionDescriptor>, CorpusError> {
    let mut descriptors = Vec::new();
    for skill in skills.skills() {
        descriptors.push(descriptor(
            ExtensionKind::Skill,
            &skill.name,
            env!("CARGO_PKG_VERSION"),
            &skill.description,
            format!("skill.{}", skill.name),
            ExtensionOrigin::FileSystem {
                path: "skills".into(),
            },
        )?);
    }
    for plugin in skills.plugins() {
        descriptors.push(descriptor(
            ExtensionKind::Plugin,
            &plugin.name,
            &plugin.version,
            &plugin.description,
            format!("plugin.{}", plugin.name),
            ExtensionOrigin::FileSystem {
                path: plugin.skill_dir.display().to_string(),
            },
        )?);
    }
    for hook in hooks.list() {
        let local_name = hook.name.replace(':', "-");
        descriptors.push(descriptor(
            ExtensionKind::Hook,
            &local_name,
            env!("CARGO_PKG_VERSION"),
            &format!("{} lifecycle hook", hook.name),
            format!("hook.{}", hook.name),
            hook.script_path
                .as_ref()
                .map(|path| ExtensionOrigin::FileSystem {
                    path: path.display().to_string(),
                })
                .unwrap_or(ExtensionOrigin::BuiltIn),
        )?);
    }
    descriptors.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(descriptors)
}

fn descriptor(
    kind: ExtensionKind,
    name: &str,
    version: &str,
    description: &str,
    capability: String,
    origin: ExtensionOrigin,
) -> Result<ExtensionDescriptor, CorpusError> {
    ExtensionDescriptor::new(
        kind,
        name,
        version,
        description,
        CapabilityId(capability),
        RiskLevel::ReadOnly,
    )
    .map(|descriptor| descriptor.with_origin(origin))
    .map_err(|error| CorpusError::InvalidDescriptor(error.to_string()))
}
