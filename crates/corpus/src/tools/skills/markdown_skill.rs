//! Markdown skill definition with YAML frontmatter parsing.
//!
//! Since `serde_yaml` is not a dependency of this crate, frontmatter is
//! parsed with a simple line-oriented YAML parser that handles the subset
//! of YAML produced by the skill authoring tool (flat scalar keys, string
//! values, and plain-list items).

use serde::Deserialize;

/// A skill loaded from a Markdown file with YAML frontmatter.
#[derive(Debug, Clone, Deserialize)]
pub struct MarkdownSkill {
    /// Skill name (from frontmatter).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Trigger command (e.g., "/review").
    pub trigger: String,
    /// Permissions for this skill.
    #[serde(default)]
    pub permissions: SkillPermissions,
    /// Tools this skill can use (empty = all tools).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Model override for this skill (empty = use default).
    #[serde(default)]
    pub model: Option<String>,
    /// The prompt content (everything after frontmatter).
    #[serde(skip)]
    pub content: String,
}

/// Skill permission configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillPermissions {
    #[serde(default = "default_true")]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub execute: bool,
}

fn default_true() -> bool {
    true
}

impl MarkdownSkill {
    /// Parse a Markdown file with YAML frontmatter.
    ///
    /// Frontmatter is delimited by `---` on its own line at the very start
    /// of the file and a matching `---` closing line.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if !raw.starts_with("---") {
            return Err("Missing YAML frontmatter (must start with ---)".to_string());
        }

        let end = raw[3..].find("---").ok_or("Missing closing ---")? + 3;
        let frontmatter = &raw[3..end].trim();
        let content = raw[end + 3..].trim().to_string();

        let mut skill = Self::parse_frontmatter(frontmatter)?;
        skill.content = content;

        if skill.name.is_empty() {
            return Err("Skill name is required".to_string());
        }
        if skill.trigger.is_empty() {
            return Err("Skill trigger is required".to_string());
        }

        Ok(skill)
    }

    /// Get the system prompt for this skill.
    pub fn system_prompt(&self) -> &str {
        &self.content
    }

    // ── Simple frontmatter parser (no serde_yaml dependency) ──────────────

    /// Parse a limited YAML frontmatter block into a `MarkdownSkill`.
    ///
    /// Supports:
    ///   - scalar keys: `key: value`
    ///   - quoted values: `key: "value"` or `key: 'value'`
    ///   - boolean values: `key: true` / `key: false`
    ///   - list items: `- item` under a key
    ///   - nested mapping block for `permissions:` with indented keys
    fn parse_frontmatter(fm: &str) -> Result<Self, String> {
        let mut name = String::new();
        let mut description = String::new();
        let mut trigger = String::new();
        let mut permissions = SkillPermissions::default();
        let mut tools: Vec<String> = Vec::new();
        let mut model: Option<String> = None;

        let mut current_key: Option<String> = None;
        let mut in_permissions = false;
        let mut in_list_key: Option<String> = None;

        for line in fm.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // List item under current list key
            if let Some(list_key) = &in_list_key {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    let item = item.trim_matches('"').trim_matches('\'').to_string();
                    match list_key.as_str() {
                        "tools" => tools.push(item),
                        _ => {}
                    }
                    continue;
                } else {
                    // End of list
                    in_list_key = None;
                }
            }

            // Indented line inside permissions block
            if in_permissions {
                if let Some(indent) = line.chars().position(|c| !c.is_whitespace()) {
                    if indent >= 2 {
                        if let Some((k, v)) = parse_kv(trimmed) {
                            match k.as_str() {
                                "read" => permissions.read = parse_bool(&v),
                                "write" => permissions.write = parse_bool(&v),
                                "execute" => permissions.execute = parse_bool(&v),
                                _ => {}
                            }
                            continue;
                        }
                    }
                }
                in_permissions = false;
            }

            // Key: value pair
            if let Some((k, v)) = parse_kv(trimmed) {
                let v_is_empty = v.is_empty();
                let k_for_list = k.clone();
                match k.as_str() {
                    "name" => name = v,
                    "description" => description = v,
                    "trigger" => trigger = v,
                    "model" => {
                        if v_is_empty || v == "~" || v == "null" {
                            model = None;
                        } else {
                            model = Some(v);
                        }
                    }
                    "tools" => {
                        if v_is_empty || v == "[]" {
                            // Empty list on same line
                        } else {
                            // Single-item list inline
                            tools.push(v);
                        }
                    }
                    "permissions" => {
                        in_permissions = true;
                    }
                    _ => {}
                }
                current_key = Some(k_for_list.clone());

                // Check if value is empty (list follows on next lines)
                if v_is_empty {
                    match k_for_list.as_str() {
                        "tools" => in_list_key = Some("tools".to_string()),
                        _ => {}
                    }
                }
                continue;
            }

            // Standalone list item (e.g. at top level)
            if let Some(item) = trimmed.strip_prefix("- ") {
                let item = item.trim_matches('"').trim_matches('\'').to_string();
                if let Some(ref key) = current_key {
                    match key.as_str() {
                        "tools" => tools.push(item),
                        _ => {}
                    }
                }
            }
        }

        Ok(MarkdownSkill {
            name,
            description,
            trigger,
            permissions,
            tools,
            model,
            content: String::new(), // filled by caller
        })
    }
}

/// Parse a `key: value` pair from a YAML line. Returns `None` if the line
/// is not a simple key-value mapping.
fn parse_kv(line: &str) -> Option<(String, String)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim().to_string();
    let value = line[colon + 1..].trim();
    let value = value.trim_matches('"').trim_matches('\'').to_string();
    Some((key, value))
}

/// Parse a YAML boolean or truthy/falsy string.
fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "true" | "yes" | "on" | "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_skill() {
        let raw = r#"---
name: Review
description: Code review skill
trigger: /review
permissions:
  read: true
  write: false
---
You are a code reviewer."#;

        let skill = MarkdownSkill::parse(raw).unwrap();
        assert_eq!(skill.name, "Review");
        assert_eq!(skill.trigger, "/review");
        assert!(skill.permissions.read);
        assert!(!skill.permissions.write);
        assert_eq!(skill.content, "You are a code reviewer.");
    }

    #[test]
    fn parse_skill_with_tools_and_model() {
        let raw = r#"---
name: Test
description: Test skill
trigger: /test
tools:
  - bash_exec
  - file_read
model: gpt-4
---
Test prompt."#;

        let skill = MarkdownSkill::parse(raw).unwrap();
        assert_eq!(skill.tools, vec!["bash_exec", "file_read"]);
        assert_eq!(skill.model, Some("gpt-4".to_string()));
    }

    #[test]
    fn parse_missing_frontmatter() {
        assert!(MarkdownSkill::parse("no frontmatter here").is_err());
    }

    #[test]
    fn parse_empty_name_fails() {
        let raw = r#"---
name: ""
trigger: /foo
---
Content"#;
        assert!(MarkdownSkill::parse(raw).is_err());
    }
}
