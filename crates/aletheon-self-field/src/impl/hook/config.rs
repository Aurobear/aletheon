use std::path::Path;
use anyhow::Result;
use tracing::{info, warn};

use super::types::*;

pub struct HookConfig {
    hooks: Vec<Hook>,
}

impl HookConfig {
    /// Load hooks from all config layers.
    pub fn load() -> Result<Self> {
        let mut hooks = Vec::new();

        // Layer 1: System hooks
        hooks.extend(load_hooks_from_dir(Path::new("/etc/argos/hooks"))?);

        // Layer 2: User hooks
        if let Some(home) = dirs::home_dir() {
            hooks.extend(load_hooks_from_dir(&home.join(".argos/hooks"))?);
        }

        // Layer 3: Project hooks
        hooks.extend(load_hooks_from_dir(Path::new(".argos/hooks"))?);

        info!(count = hooks.len(), "Loaded hooks from all layers");
        Ok(Self { hooks })
    }

    /// Get hooks matching an event.
    pub fn get_hooks(&self, event: HookEventName) -> Vec<&Hook> {
        self.hooks
            .iter()
            .filter(|h| h.enabled && h.event == event)
            .collect()
    }

    /// Get hooks matching an event and context.
    pub fn get_matching_hooks(
        &self,
        event: HookEventName,
        tool: Option<&str>,
        args: Option<&str>,
        risk: Option<&str>,
    ) -> Vec<&Hook> {
        self.hooks
            .iter()
            .filter(|h| h.enabled && h.event == event && h.matcher.matches(tool, args, risk))
            .collect()
    }
}

fn load_hooks_from_dir(dir: &Path) -> Result<Vec<Hook>> {
    let mut hooks = Vec::new();
    if !dir.exists() {
        return Ok(hooks);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "toml") {
            match load_hooks_from_file(&path) {
                Ok(mut h) => {
                    info!(path = %path.display(), count = h.len(), "Loaded hook file");
                    hooks.append(&mut h);
                }
                Err(e) => warn!(path = %path.display(), error = %e, "Failed to load hook file"),
            }
        }
    }
    Ok(hooks)
}

fn load_hooks_from_file(path: &Path) -> Result<Vec<Hook>> {
    let content = std::fs::read_to_string(path)?;
    let parsed: toml::Value = toml::from_str(&content)?;

    let mut hooks = Vec::new();
    if let Some(hook_array) = parsed.get("hooks").and_then(|v| v.as_array()) {
        for hook_val in hook_array {
            let hook: Hook = serde_json::from_value(serde_json::to_value(hook_val)?)?;
            hooks.push(hook);
        }
    }
    Ok(hooks)
}
