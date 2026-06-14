use std::path::PathBuf;
use tracing::{info, warn};
use super::manifest::PluginManifest;

/// Plugin loader — discovers and loads plugins from directories.
pub struct PluginLoader {
    search_dirs: Vec<PathBuf>,
}

impl PluginLoader {
    pub fn new(search_dirs: Vec<PathBuf>) -> Self {
        Self { search_dirs }
    }

    /// Get the configured search directories.
    pub fn search_dirs(&self) -> &[PathBuf] {
        &self.search_dirs
    }

    /// Discover all plugins in search directories.
    pub fn discover(&self) -> Result<Vec<PluginManifest>, anyhow::Error> {
        let mut plugins = Vec::new();

        for dir in &self.search_dirs {
            if !dir.exists() {
                continue;
            }

            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    let manifest_path = path.join("plugin.toml");
                    if manifest_path.exists() {
                        match PluginManifest::from_file(&manifest_path) {
                            Ok(manifest) => {
                                if let Err(e) = manifest.validate() {
                                    warn!(path = %path.display(), error = %e, "Invalid plugin manifest");
                                    continue;
                                }
                                info!(id = %manifest.id, version = %manifest.version, "Discovered plugin");
                                plugins.push(manifest);
                            }
                            Err(e) => {
                                warn!(path = %path.display(), error = %e, "Failed to load plugin manifest");
                            }
                        }
                    }
                }
            }
        }

        Ok(plugins)
    }

    /// Resolve plugin dependencies and return a topologically sorted load order.
    pub fn resolve_dependencies(&self, plugins: &[PluginManifest]) -> Result<Vec<String>, String> {
        let plugin_ids: std::collections::HashSet<&str> = plugins.iter().map(|p| p.id.as_str()).collect();
        let mut load_order = Vec::new();
        let mut loaded = std::collections::HashSet::new();

        for plugin in plugins {
            self.resolve_plugin(plugin, &plugin_ids, &mut loaded, &mut load_order)?;
        }

        Ok(load_order)
    }

    fn resolve_plugin<'a>(
        &self,
        plugin: &'a PluginManifest,
        available: &std::collections::HashSet<&str>,
        loaded: &mut std::collections::HashSet<&'a str>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if loaded.contains(plugin.id.as_str()) {
            return Ok(());
        }

        for dep in &plugin.dependencies {
            if !dep.optional && !available.contains(dep.id.as_str()) {
                return Err(format!(
                    "Plugin '{}' requires '{}' which is not available",
                    plugin.id, dep.id
                ));
            }
        }

        loaded.insert(&plugin.id);
        order.push(plugin.id.clone());
        Ok(())
    }
}
