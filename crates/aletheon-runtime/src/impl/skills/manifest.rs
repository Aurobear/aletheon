// crates/aletheon-runtime/src/impl/skills/manifest.rs

//! SKILL.md YAML frontmatter parsing.
//!
//! Parses the `---` delimited YAML header from SKILL.md files into
//! typed manifest structures.

use aletheon_abi::tool::PermissionLevel;
use serde::{Deserialize, Serialize};

/// Raw YAML frontmatter from SKILL.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: String,
    pub trigger: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub tools: Option<Vec<ToolManifest>>,
    pub hooks: Option<HooksManifest>,
}

/// Tool declaration in SKILL.md frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    pub description: String,
    pub script: String,
    pub permission: Option<String>,
    pub exposure: Option<String>,
    pub input_schema: Option<serde_json::Value>,
}

/// Hooks declaration in SKILL.md frontmatter.
/// Each key is a hook point name, value is a list of hook definitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HooksManifest {
    pub on_session_start: Option<Vec<HookManifest>>,
    pub on_session_end: Option<Vec<HookManifest>>,
    pub pre_turn: Option<Vec<HookManifest>>,
    pub post_turn: Option<Vec<HookManifest>>,
    pub pre_tool: Option<Vec<HookManifest>>,
    pub post_tool: Option<Vec<HookManifest>>,
    pub on_memory_store: Option<Vec<HookManifest>>,
    pub on_memory_recall: Option<Vec<HookManifest>>,
}

/// Single hook declaration in SKILL.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookManifest {
    pub name: String,
    pub script: String,
    pub priority: Option<i32>,
}

/// Parse a SKILL.md file content into (manifest, body).
///
/// The frontmatter is between the first pair of `---` markers.
/// Everything after the second `---` is the body (system prompt injection).
pub fn parse_skill_md(content: &str) -> anyhow::Result<(SkillManifest, String)> {
    let trimmed = content.trim();

    // Must start with ---
    if !trimmed.starts_with("---") {
        return Err(anyhow::anyhow!(
            "SKILL.md must start with '---' frontmatter"
        ));
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let end_pos = after_first
        .find("\n---")
        .or_else(|| after_first.find("\r\n---"))
        .ok_or_else(|| anyhow::anyhow!("Missing closing '---' in SKILL.md frontmatter"))?;

    let frontmatter = &after_first[..end_pos];
    let body_start = end_pos + 4; // skip "\n---"
    let body = after_first[body_start..].trim().to_string();

    let manifest: SkillManifest = serde_yaml::from_str(frontmatter)
        .map_err(|e| anyhow::anyhow!("Failed to parse SKILL.md frontmatter: {}", e))?;

    Ok((manifest, body))
}

/// Parse a permission string into PermissionLevel.
pub fn parse_permission(s: &str) -> PermissionLevel {
    match s.to_uppercase().as_str() {
        "L0" => PermissionLevel::L0,
        "L1" => PermissionLevel::L1,
        "L2" => PermissionLevel::L2,
        "L3" => PermissionLevel::L3,
        _ => PermissionLevel::L1,
    }
}

/// Parse an exposure string into ToolExposure.
pub fn parse_exposure(s: &str) -> aletheon_abi::tool::ToolExposure {
    match s.to_lowercase().as_str() {
        "direct" => aletheon_abi::tool::ToolExposure::Direct,
        "deferred" => aletheon_abi::tool::ToolExposure::Deferred,
        "directmodelonly" => aletheon_abi::tool::ToolExposure::DirectModelOnly,
        "hidden" => aletheon_abi::tool::ToolExposure::Hidden,
        _ => aletheon_abi::tool::ToolExposure::Direct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_skill_md() {
        let content = r#"---
name: git-workflow
version: 1.0.0
description: Git workflow automation
trigger: manual
keywords: [git, branch]
tools:
  - name: check_branch
    description: Check branch status
    script: scripts/check.sh
    permission: L0
hooks:
  pre_tool:
    - name: validate
      script: scripts/validate.sh
      priority: 10
---

When working with git, always use feature branches.
"#;
        let (manifest, body) = parse_skill_md(content).unwrap();
        assert_eq!(manifest.name, "git-workflow");
        assert_eq!(manifest.version, Some("1.0.0".into()));
        assert_eq!(manifest.description, "Git workflow automation");
        assert_eq!(manifest.trigger, Some("manual".into()));
        assert_eq!(manifest.keywords, Some(vec!["git".into(), "branch".into()]));
        assert!(manifest.tools.is_some());
        let tools = manifest.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "check_branch");
        assert_eq!(tools[0].permission, Some("L0".into()));
        assert!(manifest.hooks.is_some());
        let hooks = manifest.hooks.unwrap();
        assert!(hooks.pre_tool.is_some());
        assert_eq!(hooks.pre_tool.unwrap()[0].name, "validate");
        assert!(body.contains("feature branches"));
    }

    #[test]
    fn parse_minimal_skill_md() {
        let content = r#"---
name: minimal
description: A minimal skill
---

No tools or hooks.
"#;
        let (manifest, body) = parse_skill_md(content).unwrap();
        assert_eq!(manifest.name, "minimal");
        assert!(manifest.tools.is_none());
        assert!(manifest.hooks.is_none());
        assert!(body.contains("No tools"));
    }

    #[test]
    fn parse_missing_frontmatter() {
        let result = parse_skill_md("No frontmatter here");
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_closing_frontmatter() {
        let content = "---\nname: broken\ndescription: test";
        let result = parse_skill_md(content);
        assert!(result.is_err());
    }

    #[test]
    fn parse_permission_levels() {
        assert_eq!(parse_permission("L0"), PermissionLevel::L0);
        assert_eq!(parse_permission("L1"), PermissionLevel::L1);
        assert_eq!(parse_permission("L2"), PermissionLevel::L2);
        assert_eq!(parse_permission("L3"), PermissionLevel::L3);
        assert_eq!(parse_permission("unknown"), PermissionLevel::L1);
    }

    #[test]
    fn parse_exposure_levels() {
        assert_eq!(
            parse_exposure("direct"),
            aletheon_abi::tool::ToolExposure::Direct
        );
        assert_eq!(
            parse_exposure("deferred"),
            aletheon_abi::tool::ToolExposure::Deferred
        );
        assert_eq!(
            parse_exposure("hidden"),
            aletheon_abi::tool::ToolExposure::Hidden
        );
        assert_eq!(
            parse_exposure("unknown"),
            aletheon_abi::tool::ToolExposure::Direct
        );
    }

    #[test]
    fn parse_body_preserves_content() {
        let content = r#"---
name: test
description: test skill
---

## Instructions

Line 1
Line 2

### Details

More content here.
"#;
        let (_, body) = parse_skill_md(content).unwrap();
        assert!(body.contains("## Instructions"));
        assert!(body.contains("Line 1"));
        assert!(body.contains("### Details"));
    }
}
