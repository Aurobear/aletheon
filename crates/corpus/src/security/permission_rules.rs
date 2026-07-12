//! TOML-based permission rule loader.
//!
//! Loads `PermissionContext` from a `settings.toml` file with a `[permissions]` section.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use fabric::{PermissionBehavior, PermissionContext, PermissionMode, PermissionRule};

// -- TOML schema structs (internal) ------------------------------------------

#[derive(Debug, Deserialize)]
struct SettingsFile {
    permissions: Option<PermissionsSection>,
}

#[derive(Debug, Deserialize)]
struct PermissionsSection {
    #[serde(default)]
    mode: PermissionMode,
    #[serde(default)]
    rules: Vec<RuleToml>,
}

#[derive(Debug, Deserialize)]
struct RuleToml {
    tool: String,
    pattern: Option<String>,
    behavior: PermissionBehavior,
}

// -- Public API --------------------------------------------------------------

/// Parse a TOML string and return a [`PermissionContext`].
///
/// Expected format:
/// ```toml
/// [permissions]
/// mode = "default"
///
/// [[permissions.rules]]
/// tool = "bash"
/// pattern = "git *"
/// behavior = "allow"
/// ```
pub fn load_permission_context_from_str(s: &str) -> Result<PermissionContext> {
    let file: SettingsFile = toml::from_str(s).context("failed to parse settings.toml")?;

    let section = match file.permissions {
        Some(s) => s,
        None => return Ok(PermissionContext::default()),
    };

    let rules = section
        .rules
        .into_iter()
        .map(|r| PermissionRule {
            tool: r.tool,
            pattern: r.pattern,
            behavior: r.behavior,
        })
        .collect();

    Ok(PermissionContext {
        mode: section.mode,
        rules,
        session_approvals: Default::default(),
    })
}

/// Load a [`PermissionContext`] from a TOML file on disk.
///
/// Returns a default context if the file does not exist or cannot be parsed.
pub fn load_permission_context(path: &Path) -> PermissionContext {
    match std::fs::read_to_string(path) {
        Ok(content) => load_permission_context_from_str(&content).unwrap_or_default(),
        Err(_) => PermissionContext::default(),
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOML: &str = r#"
[permissions]
mode = "default"

[[permissions.rules]]
tool = "bash"
pattern = "git *"
behavior = "allow"

[[permissions.rules]]
tool = "write_file"
behavior = "deny"
"#;

    #[test]
    fn loads_rules_from_toml_str() {
        let ctx = load_permission_context_from_str(SAMPLE_TOML).unwrap();
        assert_eq!(ctx.mode, PermissionMode::Default);
        assert_eq!(ctx.rules.len(), 2);

        // "git status" should be allowed by the first rule
        assert_eq!(
            ctx.resolve("bash", "git status", false),
            PermissionBehavior::Allow
        );

        // "write_file" with any action should be denied
        assert_eq!(
            ctx.resolve("write_file", "/etc/passwd", false),
            PermissionBehavior::Deny
        );

        // Unmatched dangerous tool → Ask (default mode)
        assert_eq!(
            ctx.resolve("bash", "rm -rf /", true),
            PermissionBehavior::Ask
        );
    }

    #[test]
    fn missing_file_yields_default_context() {
        let ctx = load_permission_context(Path::new("/nonexistent/path/settings.toml"));
        assert_eq!(ctx.mode, PermissionMode::Default);
        assert!(ctx.rules.is_empty());
    }
}
