//! User Hook Loader — scans `~/.aletheon/hooks/*.toml` for hook definitions.
//!
//! TOML format:
//! ```toml
//! [hook]
//! name = "block-dangerous"
//! point = "PreTool"
//! priority = 1
//! script = "/path/to/script.sh"
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use super::registry::RegisteredHook;
use fabric::hook::HookPoint;

/// A hook definition parsed from a TOML file.
#[derive(Debug, Clone)]
pub struct HookConfig {
    pub name: String,
    pub point: HookPoint,
    pub priority: i32,
    pub script: PathBuf,
}

/// Loads user hook definitions from TOML files.
pub struct HookLoader {
    dir: PathBuf,
}

impl HookLoader {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Load all hook definitions from the directory.
    pub fn load_all(&self) -> Vec<HookConfig> {
        let mut hooks = Vec::new();

        if !self.dir.is_dir() {
            return hooks;
        }

        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return hooks,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
            {
                match load_hook_file(&path) {
                    Ok(hook) => hooks.push(hook),
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "Failed to load hook");
                    }
                }
            }
        }

        hooks
    }

    /// Load hooks and register them in the HookRegistry.
    pub fn register_all(&self, registry: &mut super::registry::HookRegistry) -> usize {
        let configs = self.load_all();
        let count = configs.len();
        for config in configs {
            registry.register(RegisteredHook {
                name: config.name,
                source: "user".into(),
                script_path: Some(config.script),
                point: config.point,
                priority: config.priority,
            });
        }
        count
    }
}

fn load_hook_file(path: &Path) -> anyhow::Result<HookConfig> {
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading {}: {}", path.display(), e))?;

    let mut name = String::new();
    let mut point_str = String::new();
    let mut priority: i32 = 100;
    let mut script = PathBuf::new();

    let mut in_hook_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[hook]" {
            in_hook_section = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_hook_section = false;
            continue;
        }
        if !in_hook_section {
            continue;
        }

        if let Some((key, value)) = parse_toml_kv(trimmed) {
            match key.as_str() {
                "name" => name = unquote(&value),
                "point" => point_str = unquote(&value),
                "priority" => {
                    if let Ok(n) = value.parse::<i32>() {
                        priority = n;
                    }
                }
                "script" => script = PathBuf::from(unquote(&value)),
                _ => {}
            }
        }
    }

    if name.is_empty() || script.as_os_str().is_empty() {
        return Err(anyhow::anyhow!("Missing required fields (name, script)"));
    }

    let point = match point_str.as_str() {
        "SessionStart" | "session_start" => HookPoint::OnSessionStart,
        "PreTool" | "pre_tool" => HookPoint::PreTool,
        "PostTool" | "post_tool" => HookPoint::PostTool,
        "PreResponse" | "pre_response" | "PostTurn" | "post_turn" => HookPoint::PostTurn,
        "SessionEnd" | "session_end" => HookPoint::OnSessionEnd,
        _ => return Err(anyhow::anyhow!("Unknown hook point: {point_str}")),
    };

    Ok(HookConfig {
        name,
        point,
        priority,
        script,
    })
}

fn parse_toml_kv(line: &str) -> Option<(String, String)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim().to_string();
    let value = line[eq + 1..].trim().to_string();
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_hook_file() {
        let dir = TempDir::new().unwrap();
        let hook_toml = r#"
[hook]
name = "block-dangerous"
point = "PreTool"
priority = 1
script = "/usr/local/bin/block-dangerous.sh"
"#;
        std::fs::write(dir.path().join("block.toml"), hook_toml).unwrap();

        let loader = HookLoader::new(dir.path().to_path_buf());
        let hooks = loader.load_all();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].name, "block-dangerous");
        assert_eq!(hooks[0].priority, 1);
    }

    #[test]
    fn test_load_empty_dir() {
        let dir = TempDir::new().unwrap();
        let loader = HookLoader::new(dir.path().to_path_buf());
        let hooks = loader.load_all();
        assert!(hooks.is_empty());
    }
}
