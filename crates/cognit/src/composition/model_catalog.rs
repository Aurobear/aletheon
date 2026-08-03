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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelSpec {
    pub wire_id: String,
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

/// Resolve an operator-facing model spec such as `model[1m]` without sending
/// the capacity annotation to the provider as part of its model identity.
pub fn resolve_spec(model: &str, configured_context: Option<usize>) -> Result<ResolvedModelSpec> {
    let requested = model.trim();
    let (wire_id, suffix_context) = parse_context_suffix(requested)?;
    let catalog_entry = resolve(wire_id)?;
    let catalog_context = catalog_entry
        .as_ref()
        .map(|entry| entry.context_window_tokens);
    let context_window_tokens = suffix_context
        .or(configured_context)
        .or(catalog_context)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "model '{}' is absent from the model catalog and has no explicit context length",
                wire_id
            )
        })?;
    if let Some(actual) = catalog_context {
        anyhow::ensure!(
            context_window_tokens == actual,
            "configured context length for model '{}' conflicts with the model catalog",
            wire_id
        );
    }
    Ok(ResolvedModelSpec {
        wire_id: wire_id.to_owned(),
        context_window_tokens,
        max_output_tokens: catalog_entry.and_then(|entry| entry.max_output_tokens),
    })
}

fn parse_context_suffix(model: &str) -> Result<(&str, Option<usize>)> {
    let Some(prefix) = model.strip_suffix(']') else {
        return Ok((model, None));
    };
    let Some((wire_id, raw_capacity)) = prefix.rsplit_once('[') else {
        return Ok((model, None));
    };
    let raw_capacity = raw_capacity.to_ascii_lowercase();
    let (digits, multiplier) = if let Some(value) = raw_capacity.strip_suffix('m') {
        (value, 1_000_000usize)
    } else if let Some(value) = raw_capacity.strip_suffix('k') {
        (value, 1_000usize)
    } else {
        anyhow::bail!("unsupported model context suffix '[{}]'", raw_capacity);
    };
    let capacity = digits
        .parse::<usize>()
        .context("model context suffix must contain a positive integer")?
        .checked_mul(multiplier)
        .context("model context suffix exceeds the supported integer range")?;
    anyhow::ensure!(capacity > 0, "model context suffix must be positive");
    Ok((wire_id, Some(capacity)))
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

    #[test]
    fn context_suffix_is_configuration_not_wire_identity() {
        let resolved = resolve_spec("deepseek/deepseek-v4-flash[1m]", None).unwrap();
        assert_eq!(resolved.wire_id, "deepseek/deepseek-v4-flash");
        assert_eq!(resolved.context_window_tokens, 1_000_000);
    }
}
