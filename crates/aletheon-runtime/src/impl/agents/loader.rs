//! Agent Definition Loader — scans a directory for `*.md` files with YAML frontmatter
//! and loads them as agent definitions for the AgentTool.
//!
//! Agent definitions include tool restrictions, optional model overrides,
//! and max_iterations settings for sub-agent execution.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// An agent definition parsed from a markdown file with YAML frontmatter.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    /// Agent name from frontmatter `name:` field.
    pub name: String,
    /// Description from frontmatter `description:` field.
    pub description: String,
    /// Tool names this agent is allowed to use (e.g. ["bash_exec", "file_read"]).
    pub tools: Vec<String>,
    /// Optional model override (e.g. "deepseek-v4-flash").
    pub model: Option<String>,
    /// Maximum iterations for the agent's ReAct loop (default: 20).
    pub max_iterations: usize,
    /// The system prompt (markdown body after the frontmatter).
    pub system_prompt: String,
    /// Source file path.
    pub path: PathBuf,
}

/// Loads agent definitions from markdown files with YAML frontmatter.
///
/// Compatible with the agent markdown format:
/// ```markdown
/// ---
/// name: code-agent
/// description: "Handles code execution"
/// tools: [bash_exec, file_read, file_write]
/// model: deepseek-v4-flash
/// max_iterations: 20
/// ---
///
/// System prompt body here...
/// ```
pub struct AgentLoader {
    dir: PathBuf,
}

impl AgentLoader {
    /// Create a new loader for the given agents directory.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Load all agent definitions from the directory.
    /// Returns a HashMap keyed by agent name.
    pub fn load_all(&self) -> HashMap<String, AgentDefinition> {
        let mut agents = HashMap::new();

        if !self.dir.is_dir() {
            return agents;
        }

        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(_) => return agents,
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .map_or(false, |ext| ext.eq_ignore_ascii_case("md"))
            {
                match load_agent_file(&path) {
                    Ok(agent) => {
                        agents.insert(agent.name.clone(), agent);
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "Failed to load agent definition"
                        );
                    }
                }
            }
        }

        agents
    }
}

/// Load a single agent definition from a markdown file.
fn load_agent_file(path: &Path) -> Result<AgentDefinition> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_agent_definition(&content, path)
}

/// Parse agent definition from markdown content with YAML frontmatter.
fn parse_agent_definition(content: &str, path: &Path) -> Result<AgentDefinition> {
    let content = content.trim();
    let (frontmatter, body) =
        split_frontmatter(content).ok_or_else(|| anyhow::anyhow!("No frontmatter found"))?;

    let mut name = String::new();
    let mut description = String::new();
    let mut tools: Vec<String> = Vec::new();
    let mut model: Option<String> = None;
    let mut max_iterations: usize = 20;

    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = parse_yaml_kv(trimmed) {
            let key_lc = key.to_lowercase();
            let value_trimmed = value.trim();
            match key_lc.as_str() {
                "name" => name = unquote(value_trimmed),
                "description" => description = unquote(value_trimmed),
                "tools" => {
                    tools = parse_tools_list(value_trimmed);
                }
                "model" => {
                    let v = unquote(value_trimmed);
                    if !v.is_empty() {
                        model = Some(v);
                    }
                }
                "max_iterations" => {
                    if let Ok(n) = value_trimmed.parse::<usize>() {
                        max_iterations = n;
                    }
                }
                _ => {}
            }
        }
    }

    if name.is_empty() {
        return Err(anyhow::anyhow!("Agent definition missing 'name' field"));
    }

    Ok(AgentDefinition {
        name,
        description,
        tools,
        model,
        max_iterations,
        system_prompt: body.to_string(),
        path: path.to_path_buf(),
    })
}

/// Parse a tools list that can be either `[a, b, c]` syntax or comma-separated.
fn parse_tools_list(value: &str) -> Vec<String> {
    let value = value.trim();
    // Handle array syntax: [a, b, c]
    if value.starts_with('[') && value.ends_with(']') {
        let inner = &value[1..value.len() - 1];
        return inner
            .split(',')
            .map(|s| unquote(s.trim()))
            .filter(|s| !s.is_empty())
            .collect();
    }
    // Handle comma-separated: a, b, c
    value
        .split(',')
        .map(|s| unquote(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split content into frontmatter (between first `---` and second `---`) and the rest.
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let after_first = &content[3..];
    let end = after_first.find("---")?;
    let frontmatter = after_first[..end].trim();
    let rest = after_first[end + 3..].trim();
    Some((frontmatter, rest))
}

/// Parse a `key: value` YAML line.
fn parse_yaml_kv(line: &str) -> Option<(String, String)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim().to_string();
    let value = line[colon + 1..].trim().to_string();
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

/// Remove surrounding quotes from a string.
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
    fn test_parse_agent_definition() {
        let content = r#"---
name: code-agent
description: "Handles code execution"
tools: [bash_exec, file_read, file_write]
model: deepseek-v4-flash
max_iterations: 15
---

You are a code execution agent...
"#;
        let path = Path::new("test.md");
        let agent = parse_agent_definition(content, path).unwrap();
        assert_eq!(agent.name, "code-agent");
        assert_eq!(agent.description, "Handles code execution");
        assert_eq!(agent.tools, vec!["bash_exec", "file_read", "file_write"]);
        assert_eq!(agent.model, Some("deepseek-v4-flash".to_string()));
        assert_eq!(agent.max_iterations, 15);
        assert!(agent.system_prompt.contains("You are a code execution agent"));
    }

    #[test]
    fn test_parse_tools_comma_separated() {
        let content = r#"---
name: test-agent
tools: bash_exec, file_read
---

Test prompt
"#;
        let path = Path::new("test.md");
        let agent = parse_agent_definition(content, path).unwrap();
        assert_eq!(agent.tools, vec!["bash_exec", "file_read"]);
    }

    #[test]
    fn test_default_max_iterations() {
        let content = r#"---
name: test-agent
---

Test prompt
"#;
        let path = Path::new("test.md");
        let agent = parse_agent_definition(content, path).unwrap();
        assert_eq!(agent.max_iterations, 20);
    }

    #[test]
    fn test_load_from_dir() {
        let dir = TempDir::new().unwrap();
        let agent_md = r#"---
name: test-agent
description: "Test agent"
tools: [bash_exec]
---

You are a test agent.
"#;
        std::fs::write(dir.path().join("test-agent.md"), agent_md).unwrap();
        std::fs::write(dir.path().join("not-an-agent.txt"), "ignored").unwrap();

        let loader = AgentLoader::new(dir.path().to_path_buf());
        let agents = loader.load_all();
        assert_eq!(agents.len(), 1);
        assert!(agents.contains_key("test-agent"));
    }
}
