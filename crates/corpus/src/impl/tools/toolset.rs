use std::collections::{HashMap, HashSet};
use std::fmt;

use base::tool::ToolExposure;

/// Named collection of tool names with include chains.
///
/// A toolset groups tools by logical domain (e.g. `core`, `system`, `memory`)
/// and can include other toolsets to compose larger sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolset {
    pub name: String,
    pub tools: Vec<String>,
    pub includes: Vec<String>,
}

/// Error returned when toolset resolution fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolsetError {
    /// A toolset referenced in `includes` was not found in the registry.
    UnknownToolset(String),
    /// An include cycle was detected during resolution.
    CycleDetected(String),
}

impl fmt::Display for ToolsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolsetError::UnknownToolset(name) => write!(f, "unknown toolset: {name}"),
            ToolsetError::CycleDetected(name) => {
                write!(f, "include cycle detected at toolset: {name}")
            }
        }
    }
}

impl std::error::Error for ToolsetError {}

/// Registry of all toolsets.
///
/// Toolsets are resolved lazily via [`resolve`](ToolsetRegistry::resolve), which
/// expands all `includes` chains (with cycle detection) into a flat set of tool
/// names.
#[derive(Debug, Default)]
pub struct ToolsetRegistry {
    toolsets: HashMap<String, Toolset>,
}

impl ToolsetRegistry {
    pub fn new() -> Self {
        Self {
            toolsets: HashMap::new(),
        }
    }

    /// Register a toolset. Overwrites any existing toolset with the same name.
    pub fn register(&mut self, toolset: Toolset) {
        self.toolsets.insert(toolset.name.clone(), toolset);
    }

    /// Look up a toolset by name (without resolving includes).
    pub fn get(&self, name: &str) -> Option<&Toolset> {
        self.toolsets.get(name)
    }

    /// Resolve a toolset by name into the full set of tool names, expanding
    /// all `includes` chains transitively.
    ///
    /// Returns an error if a referenced toolset is missing or a cycle is
    /// detected.
    pub fn resolve(&self, name: &str) -> Result<HashSet<String>, ToolsetError> {
        let mut tools = HashSet::new();
        let mut visiting = HashSet::new();
        let mut seen = HashSet::new();
        self.resolve_inner(name, &mut tools, &mut visiting, &mut seen)?;
        Ok(tools)
    }

    fn resolve_inner(
        &self,
        name: &str,
        tools: &mut HashSet<String>,
        visiting: &mut HashSet<String>,
        seen: &mut HashSet<String>,
    ) -> Result<(), ToolsetError> {
        if seen.contains(name) {
            return Ok(());
        }

        if !visiting.insert(name.to_string()) {
            return Err(ToolsetError::CycleDetected(name.to_string()));
        }

        let ts = self
            .toolsets
            .get(name)
            .ok_or_else(|| ToolsetError::UnknownToolset(name.to_string()))?
            .clone();

        for inc in &ts.includes {
            self.resolve_inner(inc, tools, visiting, seen)?;
        }

        for t in &ts.tools {
            tools.insert(t.clone());
        }

        visiting.remove(name);
        seen.insert(name.to_string());

        Ok(())
    }

    /// Build a registry pre-loaded with the standard named toolsets.
    pub fn with_defaults() -> Self {
        let mut reg = Self::new();

        // core: basic read-only tools
        reg.register(Toolset {
            name: "core".to_string(),
            tools: vec![
                "file_read".to_string(),
                "system_status".to_string(),
                "process_list".to_string(),
            ],
            includes: vec![],
        });

        // system: system-level mutation tools (includes core)
        reg.register(Toolset {
            name: "system".to_string(),
            tools: vec![
                "bash_exec".to_string(),
                "file_write".to_string(),
                "platform_manage".to_string(),
            ],
            includes: vec!["core".to_string()],
        });

        // perception: kernel/ebpf observability (includes core)
        reg.register(Toolset {
            name: "perception".to_string(),
            tools: vec!["ebpf_compile".to_string(), "module_load".to_string()],
            includes: vec!["core".to_string()],
        });

        // memory: memory subsystem tools
        reg.register(Toolset {
            name: "memory".to_string(),
            tools: vec!["memory_read".to_string(), "memory_write".to_string()],
            includes: vec![],
        });

        // network: network tooling (includes core)
        reg.register(Toolset {
            name: "network".to_string(),
            tools: vec!["network_diag".to_string()],
            includes: vec!["core".to_string()],
        });

        // full: everything (includes all other toolsets)
        reg.register(Toolset {
            name: "full".to_string(),
            tools: vec!["module_build".to_string(), "kernel_build".to_string()],
            includes: vec![
                "system".to_string(),
                "perception".to_string(),
                "memory".to_string(),
                "network".to_string(),
            ],
        });

        reg
    }
}

/// Filter a resolved set of tool names against a session-level exposure tier.
///
/// Only keeps tools whose exposure satisfies the session's visibility
/// requirements:
/// - `Direct` and `DirectModelOnly` tools are kept for model-visible sessions.
/// - `Deferred` tools are excluded from the model tool list (but remain
///   searchable separately).
/// - `Hidden` tools are always excluded.
///
/// `exposure_map` provides the `ToolExposure` for each tool name. Tools not
/// present in the map are excluded (treated as `Hidden`).
pub fn filter_by_exposure(
    tool_names: &HashSet<String>,
    exposure_map: &HashMap<String, ToolExposure>,
) -> HashSet<String> {
    tool_names
        .iter()
        .filter(|name| {
            exposure_map
                .get(*name)
                .map_or(false, |e| e.is_visible_to_model())
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(name: &str, tools: &[&str], includes: &[&str]) -> Toolset {
        Toolset {
            name: name.to_string(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
            includes: includes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn resolve_simple_toolset() {
        let mut reg = ToolsetRegistry::new();
        reg.register(ts("core", &["file_read", "system_status"], &[]));

        let resolved = reg.resolve("core").unwrap();
        assert!(resolved.contains("file_read"));
        assert!(resolved.contains("system_status"));
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn resolve_with_includes() {
        let mut reg = ToolsetRegistry::new();
        reg.register(ts("core", &["file_read", "system_status"], &[]));
        reg.register(ts("system", &["bash_exec", "file_write"], &["core"]));

        let resolved = reg.resolve("system").unwrap();
        assert!(resolved.contains("file_read"));
        assert!(resolved.contains("system_status"));
        assert!(resolved.contains("bash_exec"));
        assert!(resolved.contains("file_write"));
        assert_eq!(resolved.len(), 4);
    }

    #[test]
    fn resolve_transitive_includes() {
        let mut reg = ToolsetRegistry::new();
        reg.register(ts("base", &["a"], &[]));
        reg.register(ts("mid", &["b"], &["base"]));
        reg.register(ts("top", &["c"], &["mid"]));

        let resolved = reg.resolve("top").unwrap();
        assert!(resolved.contains("a"));
        assert!(resolved.contains("b"));
        assert!(resolved.contains("c"));
    }

    #[test]
    fn resolve_cycle_detection() {
        let mut reg = ToolsetRegistry::new();
        reg.register(ts("a", &["tool_a"], &["b"]));
        reg.register(ts("b", &["tool_b"], &["a"]));

        let err = reg.resolve("a").unwrap_err();
        assert!(matches!(err, ToolsetError::CycleDetected(_)));
    }

    #[test]
    fn resolve_unknown_toolset() {
        let reg = ToolsetRegistry::new();
        let err = reg.resolve("nonexistent").unwrap_err();
        assert!(matches!(err, ToolsetError::UnknownToolset(_)));
    }

    #[test]
    fn resolve_deduplicates_across_includes() {
        let mut reg = ToolsetRegistry::new();
        reg.register(ts("shared", &["file_read"], &[]));
        reg.register(ts("left", &["a"], &["shared"]));
        reg.register(ts("right", &["b"], &["shared"]));
        reg.register(ts("merged", &[], &["left", "right"]));

        let resolved = reg.resolve("merged").unwrap();
        // file_read should appear exactly once
        assert!(resolved.contains("file_read"));
        assert!(resolved.contains("a"));
        assert!(resolved.contains("b"));
        assert_eq!(resolved.len(), 3);
    }

    #[test]
    fn filter_by_exposure_removes_hidden_and_deferred() {
        let mut names = HashSet::new();
        names.insert("visible_tool".to_string());
        names.insert("deferred_tool".to_string());
        names.insert("hidden_tool".to_string());

        let mut exp = HashMap::new();
        exp.insert("visible_tool".to_string(), ToolExposure::Direct);
        exp.insert("deferred_tool".to_string(), ToolExposure::Deferred);
        exp.insert("hidden_tool".to_string(), ToolExposure::Hidden);

        let filtered = filter_by_exposure(&names, &exp);
        assert!(filtered.contains("visible_tool"));
        assert!(!filtered.contains("deferred_tool"));
        assert!(!filtered.contains("hidden_tool"));
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn filter_by_exposure_keeps_direct_model_only() {
        let mut names = HashSet::new();
        names.insert("dmo".to_string());
        names.insert("direct".to_string());

        let mut exp = HashMap::new();
        exp.insert("dmo".to_string(), ToolExposure::DirectModelOnly);
        exp.insert("direct".to_string(), ToolExposure::Direct);

        let filtered = filter_by_exposure(&names, &exp);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn with_defaults_resolves_full() {
        let reg = ToolsetRegistry::with_defaults();
        let full = reg.resolve("full").unwrap();
        // full includes system+perception+memory+network, each of which
        // includes core.  Spot-check a few tools from each branch.
        assert!(full.contains("file_read")); // core
        assert!(full.contains("bash_exec")); // system
        assert!(full.contains("ebpf_compile")); // perception
        assert!(full.contains("memory_read")); // memory
        assert!(full.contains("network_diag")); // network
        assert!(full.contains("kernel_build")); // full's own tools
    }

    #[test]
    fn self_include_is_cycle() {
        let mut reg = ToolsetRegistry::new();
        reg.register(ts("self_ref", &["t"], &["self_ref"]));

        let err = reg.resolve("self_ref").unwrap_err();
        assert!(matches!(err, ToolsetError::CycleDetected(_)));
    }
}
