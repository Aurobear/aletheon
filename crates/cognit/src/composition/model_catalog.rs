//! Embedded, versioned model capabilities used to compile provider requests.

use anyhow::{Context, Result};
use serde::Deserialize;

const MODEL_CATALOG_JSON: &str = include_str!("../../resources/model_catalog.json");

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub family: String,
    pub context_window_tokens: usize,
    pub max_output_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ModelCatalogResource {
    schema_version: u32,
    models: Vec<ModelCatalogEntry>,
}

pub fn resolve(model: &str) -> Result<Option<ModelCatalogEntry>> {
    let resource: ModelCatalogResource =
        serde_json::from_str(MODEL_CATALOG_JSON).context("embedded model catalog is invalid")?;
    anyhow::ensure!(
        resource.schema_version == 1,
        "unsupported model catalog schema"
    );
    let requested = model.trim();
    Ok(resource.models.into_iter().find(|entry| {
        entry.id == requested || entry.aliases.iter().any(|alias| alias == requested)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_namespaced_deepseek_id_resolves_actual_context() {
        let entry = resolve("deepseek/deepseek-v4-flash").unwrap().unwrap();
        assert_eq!(entry.id, "deepseek-v4-flash");
        assert_eq!(entry.context_window_tokens, 1_000_000);
    }

    #[test]
    fn aliases_share_one_authoritative_capability_record() {
        assert_eq!(resolve("gpt-5.6").unwrap().unwrap().id, "gpt-5.6-sol");
        assert!(resolve("private-model").unwrap().is_none());
    }
}
