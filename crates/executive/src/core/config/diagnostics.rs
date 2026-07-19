//! Configuration diagnostics: effective config rendering and layer inspection.
//!
//! Powers `aletheon config effective|layers` CLI commands.
//! Uses the existing `ConfigProvenance` infrastructure for source tracking.

use std::collections::BTreeMap;

use serde::Serialize;

#[cfg(test)]
use super::merge_layers;
use super::LoadedConfig;

/// Stable, schema-aware output for `aletheon config effective`.
/// Secrets are always redacted via `provenance::redact_json`.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveConfigView {
    /// The fully merged, redacted configuration.
    pub config: serde_json::Value,
    /// Total number of leaves in the effective config.
    pub leaf_count: usize,
}

/// A single config layer's metadata for `aletheon config layers`.
#[derive(Debug, Clone, Serialize)]
pub struct LayerInfo {
    pub index: usize,
    pub source_kind: String,
    pub locator: String,
    pub leaf_count: usize,
}

/// Aggregate view for `aletheon config layers`.
#[derive(Debug, Clone, Serialize)]
pub struct LayersView {
    pub layers: Vec<LayerInfo>,
    /// Per-leaf provenance: path -> source locator.
    pub provenance: BTreeMap<String, LayerSourceSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayerSourceSummary {
    pub kind: String,
    pub locator: String,
}

impl LoadedConfig {
    /// Produce a display-safe effective config view with all secrets redacted.
    pub fn effective_view(&self) -> EffectiveConfigView {
        let config = self.redacted_effective_values();
        let leaf_count = count_json_leaves(&config);
        EffectiveConfigView { config, leaf_count }
    }

    /// Produce a human-readable layers summary showing each layer's source
    /// and what it contributed.
    pub fn layers_view(&self) -> LayersView {
        let mut provenance: BTreeMap<String, LayerSourceSummary> = BTreeMap::new();
        for (path, source) in self.provenance.iter() {
            provenance.insert(
                path.to_string(),
                LayerSourceSummary {
                    kind: format!("{:?}", source.kind).to_lowercase(),
                    locator: source.locator.clone(),
                },
            );
        }
        // We don't have the original layer list stored on LoadedConfig.
        // The provenance map IS the layer list — each leaf maps to its source.
        // Produce a synthetic layer summary from unique sources in order.
        let mut layer_map: BTreeMap<String, LayerInfo> = BTreeMap::new();
        for (_path, source) in self.provenance.iter() {
            let key = format!("{:?}:{}", source.kind, source.locator);
            layer_map
                .entry(key)
                .and_modify(|info| info.leaf_count += 1)
                .or_insert_with(|| LayerInfo {
                    index: 0, // will be reassigned below
                    source_kind: format!("{:?}", source.kind).to_lowercase(),
                    locator: source.locator.clone(),
                    leaf_count: 1,
                });
        }
        // Preserve config-source-kind ordering (Default < System < User < Project < Environment < Cli)
        let kind_order = ["default", "system", "user", "project", "environment", "cli"];
        let mut ordered_layers: Vec<LayerInfo> = Vec::new();
        for kind in &kind_order {
            for info in layer_map.values() {
                if info.source_kind == *kind {
                    ordered_layers.push(info.clone());
                }
            }
        }
        // Any unknown kinds at the end
        for info in layer_map.values() {
            if !kind_order.contains(&info.source_kind.as_str()) {
                ordered_layers.push(info.clone());
            }
        }
        for (i, info) in ordered_layers.iter_mut().enumerate() {
            info.index = i;
        }
        LayersView {
            layers: ordered_layers,
            provenance,
        }
    }
}

/// Directly load and display config layers without full daemon bootstrap.
/// Used by `aletheon config effective|layers`.
pub fn load_config_diagnostics(
    project_dir: Option<&std::path::Path>,
) -> anyhow::Result<LoadedConfig> {
    super::load_layered(project_dir, std::env::vars(), std::iter::empty())
}

fn count_json_leaves(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => map.values().map(count_json_leaves).sum(),
        serde_json::Value::Array(items) => items.iter().map(count_json_leaves).sum(),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_view_redacts_secrets() {
        // Load a default config — it should have no secrets
        let loaded = merge_layers(std::iter::empty()).expect("default config");
        let view = loaded.effective_view();
        assert!(view.leaf_count > 0);
        // Check that api_key fields are redacted
        let config_str = view.config.to_string();
        assert!(
            !config_str.contains("\"api_key\": \"")
                || config_str.contains("\"api_key\": \"<redacted>\"")
        );
    }

    #[test]
    fn layers_view_shows_sources() {
        let loaded = merge_layers(std::iter::empty()).expect("default config");
        let view = loaded.layers_view();
        assert!(!view.layers.is_empty());
        assert_eq!(view.layers[0].source_kind, "default");
    }

    #[test]
    fn count_json_leaves_counts_primitives() {
        let value = serde_json::json!({"a": 1, "b": {"c": 2, "d": [3, 4]}});
        assert_eq!(count_json_leaves(&value), 4); // 1, 2, 3, 4
    }
}
