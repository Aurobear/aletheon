use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::loader::PluginLoader;
use super::manifest::PluginManifest;
use super::runtime::PluginRuntime;
use fabric::plugin::{Plugin, PluginContext};
use fabric::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

/// Plugin lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    Discovered,
    Loaded,
    Active,
    Error(String),
    Unloaded,
}

/// Managed plugin instance.
pub struct ManagedPlugin {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub tools: Vec<Arc<dyn Tool>>,
    pub plugin: Option<Box<dyn Plugin>>,
}

/// Plugin manager — manages plugin lifecycle.
pub struct PluginManager {
    plugins: RwLock<HashMap<String, ManagedPlugin>>,
    loader: PluginLoader,
}

impl PluginManager {
    pub fn new(search_dirs: Vec<PathBuf>) -> Self {
        Self {
            plugins: RwLock::new(HashMap::new()),
            loader: PluginLoader::new(search_dirs),
        }
    }

    /// Discover and load all plugins.
    pub async fn load_all(&self) -> Result<usize, anyhow::Error> {
        let manifests = self.loader.discover()?;
        let load_order = self
            .loader
            .resolve_dependencies(&manifests)
            .map_err(|e| anyhow::anyhow!(e))?;

        let mut loaded = 0;
        let mut plugins = self.plugins.write().await;

        for id in &load_order {
            if let Some(manifest) = manifests.iter().find(|m| m.id == *id) {
                info!(id = id.as_str(), entry = %manifest.entry, "Loading plugin");

                // Resolve plugin directory (where plugin.toml lives)
                // For discovered plugins, we need the directory from the loader.
                // Here we construct it from the manifest search dirs.
                let plugin_dir = self.resolve_plugin_dir(manifest);

                // Create runtime from entry field
                let runtime = match PluginRuntime::from_entry(&manifest.entry, &plugin_dir) {
                    Ok(rt) => rt,
                    Err(e) => {
                        warn!(id = id.as_str(), error = %e, "Failed to create plugin runtime");
                        plugins.insert(
                            id.clone(),
                            ManagedPlugin {
                                manifest: manifest.clone(),
                                state: PluginState::Error(e.to_string()),
                                tools: Vec::new(),
                                plugin: None,
                            },
                        );
                        continue;
                    }
                };

                // Create plugin tools with real runtime
                let tools = self.create_plugin_tools(manifest, runtime);

                plugins.insert(
                    id.clone(),
                    ManagedPlugin {
                        manifest: manifest.clone(),
                        state: PluginState::Loaded,
                        tools,
                        plugin: None,
                    },
                );

                loaded += 1;
            }
        }

        info!(count = loaded, "Plugins loaded");
        Ok(loaded)
    }

    /// Load an in-process plugin that implements the `Plugin` lifecycle trait.
    /// Calls `init` and, on success, tracks it as `PluginState::Active`, merging
    /// any capabilities it registers into its tool set. `Tool`-only plugins keep
    /// using `load_all` and are unaffected.
    pub async fn load_native(
        &self,
        manifest: PluginManifest,
        mut plugin: Box<dyn Plugin>,
    ) -> Result<(), anyhow::Error> {
        let plugin_dir = self.resolve_plugin_dir(&manifest);

        // Manifest-declared execute-only tools still work (best-effort; a missing
        // command only warns inside from_entry). Native/unsupported runtimes just
        // yield no manifest tools -- the plugin's capabilities() still apply.
        let mut tools: Vec<Arc<dyn Tool>> =
            match PluginRuntime::from_entry(&manifest.entry, &plugin_dir) {
                Ok(rt) => self.create_plugin_tools(&manifest, rt),
                Err(_) => Vec::new(),
            };

        let ctx = PluginContext {
            plugin_id: manifest.id.clone(),
            working_dir: plugin_dir,
            config: serde_json::Value::Null,
        };

        let id = manifest.id.clone();
        let mut plugins = self.plugins.write().await;

        match plugin.init(&ctx).await {
            Ok(()) => {
                tools.extend(plugin.capabilities());
                info!(id = id.as_str(), "Plugin init complete; activating");
                plugins.insert(
                    id,
                    ManagedPlugin {
                        manifest,
                        state: PluginState::Active,
                        tools,
                        plugin: Some(plugin),
                    },
                );
                Ok(())
            }
            Err(e) => {
                warn!(id = id.as_str(), error = %e, "Plugin init failed");
                plugins.insert(
                    id,
                    ManagedPlugin {
                        manifest,
                        state: PluginState::Error(e.to_string()),
                        tools: Vec::new(),
                        plugin: None,
                    },
                );
                Err(e)
            }
        }
    }

    /// Get all active plugin tools.
    pub async fn get_tools(&self) -> Vec<Arc<dyn Tool>> {
        let plugins = self.plugins.read().await;
        plugins
            .values()
            .filter(|p| p.state == PluginState::Loaded || p.state == PluginState::Active)
            .flat_map(|p| p.tools.clone())
            .collect()
    }

    /// Get plugin state.
    pub async fn get_state(&self, plugin_id: &str) -> Option<PluginState> {
        let plugins = self.plugins.read().await;
        plugins.get(plugin_id).map(|p| p.state.clone())
    }

    /// Unload a plugin.
    pub async fn unload(&self, plugin_id: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().await;
        if let Some(plugin) = plugins.get_mut(plugin_id) {
            if let Some(mut lifecycle) = plugin.plugin.take() {
                if let Err(e) = lifecycle.shutdown().await {
                    warn!(id = plugin_id, error = %e, "Plugin shutdown failed");
                }
            }
            plugin.state = PluginState::Unloaded;
            plugin.tools.clear();
            info!(id = plugin_id, "Plugin unloaded");
            Ok(())
        } else {
            Err(format!("Plugin '{}' not found", plugin_id))
        }
    }

    /// Resolve the directory where a plugin's manifest lives.
    /// Searches the loader's search dirs for a matching plugin.toml.
    fn resolve_plugin_dir(&self, manifest: &PluginManifest) -> PathBuf {
        // Try to find the plugin directory by searching known paths
        for dir in self.loader.search_dirs() {
            let candidate = dir.join(&manifest.id);
            if candidate.join("plugin.toml").exists() {
                return candidate;
            }
        }
        // Fallback: use the first search dir
        self.loader
            .search_dirs()
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(&manifest.id)
    }

    fn create_plugin_tools(
        &self,
        manifest: &PluginManifest,
        runtime: PluginRuntime,
    ) -> Vec<Arc<dyn Tool>> {
        let runtime = Arc::new(runtime);

        manifest
            .tools
            .iter()
            .map(|tool_def| {
                Arc::new(PluginTool {
                    name: tool_def.name.clone(),
                    description: tool_def.description.clone(),
                    schema: tool_def.input_schema.clone(),
                    permission: parse_permission_level(&tool_def.permission_level),
                    runtime: Arc::clone(&runtime),
                    plugin_id: manifest.id.clone(),
                }) as Arc<dyn Tool>
            })
            .collect()
    }
}

fn parse_permission_level(s: &str) -> PermissionLevel {
    match s.to_uppercase().as_str() {
        "L0" => PermissionLevel::L0,
        "L1" => PermissionLevel::L1,
        "L2" => PermissionLevel::L2,
        "L3" => PermissionLevel::L3,
        _ => PermissionLevel::L1,
    }
}

/// Plugin tool backed by a real runtime.
struct PluginTool {
    name: String,
    description: String,
    schema: serde_json::Value,
    permission: PermissionLevel,
    runtime: Arc<PluginRuntime>,
    plugin_id: String,
}

#[async_trait::async_trait]
impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }
    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(PluginTool {
            name: self.name.clone(),
            description: self.description.clone(),
            schema: self.schema.clone(),
            permission: self.permission,
            runtime: self.runtime.clone(),
            plugin_id: self.plugin_id.clone(),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = Instant::now();

        let result = self.runtime.execute(&self.name, &input).await;
        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => {
                let content =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                ToolResult {
                    content,
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: elapsed,
                        truncated: false,
                    },
                }
            }
            Err(e) => {
                warn!(
                    plugin = self.plugin_id.as_str(),
                    tool = self.name.as_str(),
                    error = %e,
                    "Plugin tool execution failed"
                );
                ToolResult {
                    content: format!("Plugin tool '{}' error: {}", self.name, e),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: elapsed,
                        truncated: false,
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use fabric::include::subsystem::Version;
    use fabric::plugin::{Plugin, PluginContext};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    struct SamplePlugin {
        init_calls: StdArc<AtomicUsize>,
        shutdown_calls: StdArc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Plugin for SamplePlugin {
        fn id(&self) -> &str {
            "sample"
        }
        fn version(&self) -> Version {
            Version::new(0, 1, 0)
        }
        async fn init(&mut self, _ctx: &PluginContext) -> anyhow::Result<()> {
            self.init_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn shutdown(&mut self) -> anyhow::Result<()> {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            id: "sample".into(),
            name: "Sample".into(),
            version: "0.1.0".into(),
            description: String::new(),
            author: String::new(),
            entry: "cmd:./noop.sh".into(),
            tools: vec![],
            hooks: vec![],
            dependencies: vec![],
            min_agent_version: None,
            permissions: vec![],
            plugin_permissions: None,
        }
    }

    #[tokio::test]
    async fn load_native_fires_init_and_activates() {
        let mgr = PluginManager::new(vec![std::env::temp_dir()]);
        let init = StdArc::new(AtomicUsize::new(0));
        let down = StdArc::new(AtomicUsize::new(0));
        let plugin = Box::new(SamplePlugin {
            init_calls: init.clone(),
            shutdown_calls: down.clone(),
        });
        mgr.load_native(sample_manifest(), plugin).await.unwrap();
        assert_eq!(
            init.load(Ordering::SeqCst),
            1,
            "init must fire once on load"
        );
        assert_eq!(mgr.get_state("sample").await, Some(PluginState::Active));
    }

    #[tokio::test]
    async fn unload_fires_shutdown_and_unloads() {
        let mgr = PluginManager::new(vec![std::env::temp_dir()]);
        let init = StdArc::new(AtomicUsize::new(0));
        let down = StdArc::new(AtomicUsize::new(0));
        let plugin = Box::new(SamplePlugin {
            init_calls: init.clone(),
            shutdown_calls: down.clone(),
        });
        mgr.load_native(sample_manifest(), plugin).await.unwrap();
        mgr.unload("sample").await.unwrap();
        assert_eq!(
            down.load(Ordering::SeqCst),
            1,
            "shutdown must fire once on unload"
        );
        assert_eq!(mgr.get_state("sample").await, Some(PluginState::Unloaded));
    }

    #[tokio::test]
    async fn tool_only_plugin_unaffected_by_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let pdir = dir.path().join("tool-only");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(
            pdir.join("plugin.toml"),
            r#"
id = "tool-only"
name = "Tool Only"
version = "0.1.0"
entry = "cmd:./run.sh"

[[tools]]
name = "echo"
description = "echo tool"
input_schema = {}
permission_level = "L0"
"#,
        )
        .unwrap();

        let mgr = PluginManager::new(vec![dir.path().to_path_buf()]);
        mgr.load_all().await.unwrap();
        assert_eq!(mgr.get_state("tool-only").await, Some(PluginState::Loaded));
        assert!(
            mgr.get_tools().await.iter().any(|t| t.name() == "echo"),
            "Tool-only plugin's tool must still surface"
        );
        mgr.unload("tool-only").await.unwrap();
        assert_eq!(
            mgr.get_state("tool-only").await,
            Some(PluginState::Unloaded)
        );
    }
}
