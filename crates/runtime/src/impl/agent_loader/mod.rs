//! Agent Role Loader — scans a directory for `*.md` files with YAML frontmatter
//! and loads them as agent role definitions, compatible with Claude Code's
//! agent markdown format.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

// ── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentRole {
    /// Agent name from frontmatter `name:` field.
    pub name: String,
    /// Description from frontmatter `description:` field.
    pub description: String,
    /// Tool names this agent is allowed to use (e.g. ["Read", "Grep", "Glob"]).
    pub tools: Vec<String>,
    /// Optional model override (e.g. "sonnet", "opus").
    pub model: Option<String>,
    /// The markdown body after the frontmatter.
    pub body: String,
    /// Source file path.
    pub path: PathBuf,
}

// ── AgentLoader ──────────────────────────────────────────────────────────────

/// Loads agent role definitions from markdown files with YAML frontmatter.
///
/// Compatible with Claude Code's agent markdown format:
/// ```markdown
/// ---
/// name: planner
/// description: "Task decomposition and planning"
/// tools: Read, Grep, Glob
/// model: sonnet
/// ---
///
/// You are a planning agent...
/// ```
pub struct AgentLoader {
    agents: Vec<AgentRole>,
}

impl AgentLoader {
    /// Create an empty loader.
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
        }
    }

    /// Scan a directory for `*.md` files and parse their YAML frontmatter.
    /// Returns the number of agents loaded.
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() {
            return Ok(0);
        }
        let before = self.agents.len();
        for entry in fs::read_dir(dir).context("reading agents dir")? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .map_or(false, |ext| ext.eq_ignore_ascii_case("md"))
            {
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                if let Some(agent) = parse_agent_md(&content, &path) {
                    self.agents.push(agent);
                }
            }
        }
        Ok(self.agents.len() - before)
    }

    /// Get an agent by name.
    pub fn get(&self, name: &str) -> Option<&AgentRole> {
        self.agents.iter().find(|a| a.name == name)
    }

    /// All loaded agents.
    pub fn list(&self) -> &[AgentRole] {
        &self.agents
    }

    /// All agent names.
    pub fn names(&self) -> Vec<&str> {
        self.agents.iter().map(|a| a.name.as_str()).collect()
    }
}

impl Default for AgentLoader {
    fn default() -> Self {
        Self::new()
    }
}

// ── Frontmatter Parsing ─────────────────────────────────────────────────────

/// Parse YAML frontmatter between `---` markers from an agent markdown file.
fn parse_agent_md(content: &str, path: &Path) -> Option<AgentRole> {
    let content = content.trim();
    let (frontmatter, body) = split_frontmatter(content)?;

    let mut name = String::new();
    let mut description = String::new();
    let mut tools: Vec<String> = Vec::new();
    let mut model: Option<String> = None;

    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = parse_yaml_kv(trimmed) {
            let key_lc = key.to_lowercase();
            let value_trimmed = value.trim();
            match key_lc.as_str() {
                "name" => name = unquote(value_trimmed),
                "description" => description = unquote(value_trimmed),
                "tools" => {
                    // Comma-separated string: "Read, Grep, Glob"
                    tools = value_trimmed
                        .split(',')
                        .map(|s| unquote(s.trim()))
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "model" => {
                    let v = unquote(value_trimmed);
                    if !v.is_empty() {
                        model = Some(v);
                    }
                }
                _ => {}
            }
        }
    }

    if name.is_empty() {
        return None;
    }

    Some(AgentRole {
        name,
        description,
        tools,
        model,
        body: body.to_string(),
        path: path.to_path_buf(),
    })
}

/// Split content into frontmatter (between first `---` and second `---`) and the rest.
/// Returns `(frontmatter, rest)` or `None` if no frontmatter found.
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

/// Parse a `key: value` YAML line. Returns `Some((key, value))`.
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_agent_frontmatter() {
        let content = r#"---
name: planner
description: "Task decomposition, planning"
tools: Read, Grep, Glob
model: sonnet
---

You are a planning agent. Your job is to break tasks into subtasks.

## Rules
- Keep plans concise
- Identify dependencies"#;

        let path = Path::new("/fake/planner.md");
        let agent = parse_agent_md(content, path).unwrap();
        assert_eq!(agent.name, "planner");
        assert_eq!(agent.description, "Task decomposition, planning");
        assert_eq!(agent.tools, vec!["Read", "Grep", "Glob"]);
        assert_eq!(agent.model.as_deref(), Some("sonnet"));
        assert!(agent.body.contains("You are a planning agent"));
        assert!(agent.body.contains("## Rules"));
        assert_eq!(agent.path, path);
    }

    #[test]
    fn test_parse_agent_frontmatter_no_model() {
        let content = r#"---
name: reviewer
description: Code review
tools: Read
---

Review the code."#;

        let agent = parse_agent_md(content, Path::new("/fake/reviewer.md")).unwrap();
        assert_eq!(agent.name, "reviewer");
        assert_eq!(agent.tools, vec!["Read"]);
        assert!(agent.model.is_none());
    }

    #[test]
    fn test_load_from_dir() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        std::fs::write(
            dir.join("planner.md"),
            r#"---
name: planner
description: "Planning agent"
tools: Read, Grep
model: sonnet
---
Plan things."#,
        )
        .unwrap();

        std::fs::write(
            dir.join("reviewer.md"),
            r#"---
name: reviewer
description: "Review agent"
tools: Read
---
Review things."#,
        )
        .unwrap();

        // Should skip non-md files
        std::fs::write(dir.join("readme.txt"), "not an agent").unwrap();

        let mut loader = AgentLoader::new();
        let count = loader.load_from_dir(dir).unwrap();
        assert_eq!(count, 2);
        assert_eq!(loader.list().len(), 2);
        assert!(loader.names().contains(&"planner"));
        assert!(loader.names().contains(&"reviewer"));
    }

    #[test]
    fn test_get_by_name() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        std::fs::write(
            dir.join("planner.md"),
            r#"---
name: planner
description: "Planning"
tools: Read, Grep
---
Body"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("fixer.md"),
            r#"---
name: fixer
description: "Fixing"
tools: Read
---
Body"#,
        )
        .unwrap();

        let mut loader = AgentLoader::new();
        loader.load_from_dir(dir).unwrap();

        let planner = loader.get("planner").unwrap();
        assert_eq!(planner.description, "Planning");

        let fixer = loader.get("fixer").unwrap();
        assert_eq!(fixer.description, "Fixing");

        assert!(loader.get("nonexistent").is_none());
    }

    #[test]
    fn test_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let mut loader = AgentLoader::new();
        let count = loader.load_from_dir(tmp.path()).unwrap();
        assert_eq!(count, 0);
        assert!(loader.list().is_empty());
        assert!(loader.names().is_empty());
    }

    #[test]
    fn test_nonexistent_dir() {
        let mut loader = AgentLoader::new();
        let count = loader
            .load_from_dir(Path::new("/nonexistent/path"))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_agent_loader_and_skill_router_coexist() {
        use crate::r#impl::skill_router::SkillRouter;

        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();

        // Create agents directory with an agent
        let agents_dir = base_dir.join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(
            agents_dir.join("planner.md"),
            r#"---
name: planner
description: "Planning agent"
tools: Read, Grep
---
Plan things."#,
        )
        .unwrap();

        // Create skills directory with a skill
        let skills_dir = base_dir.join("skills").join("git");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("SKILL.md"),
            r#"---
name: Git Workflow
description: Handle git operations
triggers: ["commit", "push"]
tags: ["git"]
---
Skill body."#,
        )
        .unwrap();

        // Load both from the same base directory
        let mut agent_loader = AgentLoader::new();
        let agent_count = agent_loader.load_from_dir(&agents_dir).unwrap();
        assert_eq!(agent_count, 1);
        assert!(agent_loader.get("planner").is_some());

        let mut skill_router = SkillRouter::new();
        let _ = skill_router.load_from_dir(&skills_dir);
        let suggestions = skill_router.suggest("commit this", 0.1, 10);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].name, "Git Workflow");

        // Both loaded independently without conflict
        assert_eq!(agent_loader.list().len(), 1);
    }
}
