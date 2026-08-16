//! Composition adapter from extension read models to the Aletheon admin port.

use crate::extensions::extension_snapshot::ExtensionRuntimeView;

pub(super) struct ExtensionSkillCatalog {
    view: ExtensionRuntimeView,
}

impl ExtensionSkillCatalog {
    pub(super) fn new(view: ExtensionRuntimeView) -> Self {
        Self { view }
    }
}

#[async_trait::async_trait]
impl crate::wiring::application::admin_service::ExtensionSkillCatalogPort
    for ExtensionSkillCatalog
{
    async fn list_extension_skills(
        &self,
    ) -> Vec<crate::wiring::application::admin_service::SkillDescriptor> {
        let snapshot = self.view.load().await;
        snapshot
            .skills
            .iter()
            .filter_map(|skill| {
                let package_asset = skill.source.strip_prefix("package:")?;
                let (package_id, _) = package_asset.rsplit_once(':')?;
                Some(crate::wiring::application::admin_service::SkillDescriptor {
                    id: format!("{package_id}:{}", skill.name),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    enabled: true,
                    extension_id: package_id.to_owned(),
                })
            })
            .collect()
    }
}
