//! Hook registry — stores hooks and dispatches by event.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::types::{Hook, HookEvent};

/// Config format for a hook TOML file.
#[derive(Debug, Deserialize)]
struct HookConfig {
    name: String,
    event: HookEvent,
    command: String,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30_000
}

/// Registry of all loaded hooks.
pub struct HookRegistry {
    hooks: Vec<Hook>,
}

impl HookRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Load hook configs from all `*.toml` files in the given directory.
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut count = 0;
        for entry in
            std::fs::read_dir(dir).with_context(|| format!("reading hook dir {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let config: HookConfig = toml::from_str(&content)
                    .with_context(|| format!("parsing {}", path.display()))?;
                self.hooks.push(Hook {
                    name: config.name,
                    event: config.event,
                    command: config.command,
                    timeout_ms: config.timeout_ms,
                });
                count += 1;
            }
        }
        Ok(count)
    }

    /// Register a hook directly.
    pub fn register(&mut self, hook: Hook) {
        self.hooks.push(hook);
    }

    /// Return references to all hooks matching the given event.
    pub fn hooks_for_event(&self, event: &HookEvent) -> Vec<&Hook> {
        self.hooks.iter().filter(|h| &h.event == event).collect()
    }

    /// Total number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn empty_registry() {
        let reg = HookRegistry::default();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.hooks_for_event(&HookEvent::SessionStart).is_empty());
    }

    #[test]
    fn register_and_query() {
        let mut reg = HookRegistry::default();
        reg.register(Hook {
            name: "greet".into(),
            event: HookEvent::SessionStart,
            command: "echo hello".into(),
            timeout_ms: 5000,
        });
        reg.register(Hook {
            name: "guard".into(),
            event: HookEvent::PreToolUse,
            command: "echo guard".into(),
            timeout_ms: 5000,
        });

        assert_eq!(reg.len(), 2);
        assert_eq!(reg.hooks_for_event(&HookEvent::SessionStart).len(), 1);
        assert_eq!(reg.hooks_for_event(&HookEvent::PreToolUse).len(), 1);
        assert!(reg.hooks_for_event(&HookEvent::Stop).is_empty());
    }

    #[test]
    fn load_from_dir() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        std::fs::write(
            dir.join("hook1.toml"),
            r#"
name = "audit"
event = "PreToolUse"
command = "echo audit"
timeout_ms = 10000
"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("hook2.toml"),
            r#"
name = "notify"
event = "Stop"
command = "echo bye"
"#,
        )
        .unwrap();

        // Non-toml file should be ignored.
        std::fs::write(dir.join("readme.txt"), "not a hook").unwrap();

        let mut reg = HookRegistry::default();
        let count = reg.load_from_dir(dir).unwrap();
        assert_eq!(count, 2);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.hooks_for_event(&HookEvent::PreToolUse).len(), 1);
        assert_eq!(reg.hooks_for_event(&HookEvent::Stop).len(), 1);
    }

    #[test]
    fn load_from_missing_dir() {
        let mut reg = HookRegistry::default();
        let count = reg.load_from_dir(Path::new("/nonexistent/path")).unwrap();
        assert_eq!(count, 0);
    }
}
