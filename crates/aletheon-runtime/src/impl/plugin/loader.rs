use super::manifest::PluginManifest;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use tracing::{info, warn};

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

    /// Resolve plugin dependencies using Kahn's algorithm and return a
    /// topologically sorted load order. Detects missing non-optional deps
    /// and dependency cycles.
    pub fn resolve_dependencies(&self, plugins: &[PluginManifest]) -> Result<Vec<String>, String> {
        // 1. Build by_id map for quick lookup.
        let by_id: HashMap<&str, &PluginManifest> =
            plugins.iter().map(|p| (p.id.as_str(), p)).collect();

        // 2. Check missing non-optional dependencies.
        for plugin in plugins {
            for dep in &plugin.dependencies {
                if !dep.optional && !by_id.contains_key(dep.id.as_str()) {
                    return Err(format!(
                        "Plugin '{}' requires '{}' which is not available",
                        plugin.id, dep.id
                    ));
                }
            }
        }

        // 3. Build in_degree and dependents (adjacency) maps.
        let mut in_degree: HashMap<&str, usize> =
            plugins.iter().map(|p| (p.id.as_str(), 0)).collect();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

        for plugin in plugins {
            for dep in &plugin.dependencies {
                // edge: dep.id -> plugin.id (plugin depends on dep)
                if by_id.contains_key(dep.id.as_str()) {
                    *in_degree.get_mut(plugin.id.as_str()).unwrap() += 1;
                    dependents
                        .entry(dep.id.as_str())
                        .or_default()
                        .push(plugin.id.as_str());
                }
            }
        }

        // 4. Kahn's algorithm: seed queue with zero-degree nodes.
        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();
        let mut order: Vec<String> = Vec::with_capacity(plugins.len());

        while let Some(id) = queue.pop_front() {
            order.push(id.to_string());
            if let Some(children) = dependents.get(id) {
                for &child in children {
                    let deg = in_degree.get_mut(child).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(child);
                    }
                }
            }
        }

        // 5. Cycle detection.
        if order.len() != plugins.len() {
            let remaining: Vec<&str> = plugins
                .iter()
                .map(|p| p.id.as_str())
                .filter(|id| !order.contains(&id.to_string()))
                .collect();
            return Err(format!(
                "Dependency cycle detected among plugins: {}",
                remaining.join(", ")
            ));
        }

        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::super::manifest::{PluginDependency, PluginManifest};
    use super::*;

    fn make_plugin(id: &str, deps: Vec<(&str, bool)>) -> PluginManifest {
        PluginManifest {
            id: id.into(),
            name: id.into(),
            version: "0.1.0".into(),
            description: String::new(),
            author: String::new(),
            entry: "cmd:./run.sh".into(),
            tools: vec![],
            hooks: vec![],
            dependencies: deps
                .into_iter()
                .map(|(dep_id, optional)| PluginDependency {
                    id: dep_id.into(),
                    version_req: "*".into(),
                    optional,
                })
                .collect(),
            min_agent_version: None,
            permissions: vec![],
            plugin_permissions: None,
        }
    }

    #[test]
    fn test_cycle_detection() {
        // A -> B -> C -> A forms a cycle
        let a = make_plugin("A", vec![("C", false)]);
        let b = make_plugin("B", vec![("A", false)]);
        let c = make_plugin("C", vec![("B", false)]);
        let loader = PluginLoader::new(vec![]);
        let result = loader.resolve_dependencies(&[a, b, c]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Dependency cycle detected"),
            "unexpected error: {}",
            err
        );
    }
}
