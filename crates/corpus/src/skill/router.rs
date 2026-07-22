//! Skill Router — scans skill directories for SKILL.md files, parses
//! YAML frontmatter, and suggests skills based on keyword matching.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

// ── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub tags: Vec<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SkillSuggestion {
    pub name: String,
    pub description: String,
    pub confidence: f64,
    pub path: PathBuf,
}

// ── SkillRouter ──────────────────────────────────────────────────────────────

pub struct SkillRouter {
    skills: Vec<SkillEntry>,
}

impl SkillRouter {
    /// Create an empty router.
    pub fn new() -> Self {
        Self { skills: Vec::new() }
    }

    /// Scan a directory recursively for SKILL.md files and parse their
    /// YAML frontmatter. Returns the number of skills loaded.
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<usize> {
        let before = self.skills.len();
        self.scan_dir(dir)?;
        Ok(self.skills.len() - before)
    }

    fn scan_dir(&mut self, dir: &Path) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(dir).context("reading skill dir")? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                self.scan_dir(&path)?;
            } else if path.file_name().is_some_and(|n| n == "SKILL.md") {
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                if let Some(skill) = parse_skill_md(&content, &path) {
                    self.skills.push(skill);
                }
            }
        }
        Ok(())
    }

    /// Add a single skill entry.
    pub fn load_from_skill(&mut self, entry: SkillEntry) {
        self.skills.push(entry);
    }

    /// Suggest skills for a given prompt.
    ///
    /// Scoring:
    ///   - trigger match: +2.0 per match
    ///   - name substring: +1.5
    ///   - tag match: +0.5 per match
    ///   - confidence = clamp(raw_score / 3.0, 0.0, 0.99)
    pub fn suggest(&self, prompt: &str, min_confidence: f64, top_n: usize) -> Vec<SkillSuggestion> {
        let lower_prompt = prompt.to_lowercase();
        let mut results: Vec<SkillSuggestion> = self
            .skills
            .iter()
            .filter_map(|skill| {
                let mut score: f64 = 0.0;

                // Trigger matches
                for trigger in &skill.triggers {
                    if lower_prompt.contains(&trigger.to_lowercase()) {
                        score += 2.0;
                    }
                }

                // Name substring
                if !skill.name.is_empty() && lower_prompt.contains(&skill.name.to_lowercase()) {
                    score += 1.5;
                }

                // Tag matches
                for tag in &skill.tags {
                    if lower_prompt.contains(&tag.to_lowercase()) {
                        score += 0.5;
                    }
                }

                if score <= 0.0 {
                    return None;
                }

                let confidence = (score / 3.0).clamp(0.0, 0.99);
                if confidence < min_confidence {
                    return None;
                }

                Some(SkillSuggestion {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    confidence,
                    path: skill.path.clone(),
                })
            })
            .collect();

        results.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        results.truncate(top_n);
        results
    }
}

impl Default for SkillRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ── SKILL.md Frontmatter Parsing ─────────────────────────────────────────────

/// Parse YAML frontmatter between `---` markers from a SKILL.md file.
fn parse_skill_md(content: &str, path: &Path) -> Option<SkillEntry> {
    let content = content.trim();
    let (frontmatter, _) = split_frontmatter(content)?;
    let mut name = String::new();
    let mut description = String::new();
    let mut triggers: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();

    // We need to handle multi-line list values as well as inline lists.
    let mut current_key: Option<String> = None;
    let mut in_list = false;
    let mut list_values: Vec<String> = Vec::new();

    for line in frontmatter.lines() {
        let trimmed = line.trim();

        // Check for a key: value line
        if let Some((key, value)) = parse_yaml_kv(trimmed) {
            // Flush previous list if any
            if in_list {
                flush_list(&current_key, &list_values, &mut triggers, &mut tags);
                list_values.clear();
                in_list = false;
            }

            let key_lc = key.to_lowercase();
            let value_trimmed = value.trim();

            if value_trimmed.starts_with('[') && value_trimmed.ends_with(']') {
                // Inline list: ["a", "b", "c"]
                let items = parse_inline_list(value_trimmed);
                match key_lc.as_str() {
                    "name" => name = value_trimmed.trim_matches('"').trim().to_string(),
                    "description" => {
                        description = value_trimmed.trim_matches('"').trim().to_string()
                    }
                    "triggers" => triggers = items,
                    "tags" => tags = items,
                    _ => {}
                }
                current_key = None;
            } else if value_trimmed.is_empty() {
                // Possible start of a multiline list (next lines will be "- item")
                current_key = Some(key_lc.clone());
                match key_lc.as_str() {
                    "name" => {}
                    "description" => {}
                    "triggers" => in_list = true,
                    "tags" => in_list = true,
                    _ => {}
                }
                // For name/description with empty value, just leave them empty
            } else {
                // Scalar value
                let val = unquote(value_trimmed);
                match key_lc.as_str() {
                    "name" => name = val,
                    "description" => description = val,
                    "triggers" => triggers.push(val),
                    "tags" => tags.push(val),
                    _ => {}
                }
                current_key = None;
            }
        } else if in_list && trimmed.starts_with('-') {
            // List item: - value
            let val = unquote(trimmed[1..].trim());
            list_values.push(val);
        }
    }

    // Flush any trailing list
    if in_list {
        flush_list(&current_key, &list_values, &mut triggers, &mut tags);
    }

    if name.is_empty() {
        return None;
    }

    Some(SkillEntry {
        name,
        description,
        triggers,
        tags,
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

/// Parse an inline YAML list like `["a", "b", "c"]` or `[a, b, c]`.
fn parse_inline_list(s: &str) -> Vec<String> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|item| unquote(item.trim()))
        .filter(|s| !s.is_empty())
        .collect()
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

/// Flush accumulated list values into the appropriate vector.
fn flush_list(
    key: &Option<String>,
    values: &[String],
    triggers: &mut Vec<String>,
    tags: &mut Vec<String>,
) {
    match key.as_deref() {
        Some("triggers") => triggers.extend_from_slice(values),
        Some("tags") => tags.extend_from_slice(values),
        _ => {}
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill_md(frontmatter: &str) -> String {
        format!("---\n{frontmatter}\n---\n\nSkill body here.")
    }

    #[test]
    fn test_skill_entry_parse_basic() {
        let content = make_skill_md(
            r#"name: Git Workflow
description: Handle git operations
triggers: ["commit", "push", "merge"]
tags: ["git", "vcs"]"#,
        );
        let entry = parse_skill_md(&content, Path::new("/fake/SKILL.md")).unwrap();
        assert_eq!(entry.name, "Git Workflow");
        assert_eq!(entry.description, "Handle git operations");
        assert_eq!(entry.triggers, vec!["commit", "push", "merge"]);
        assert_eq!(entry.tags, vec!["git", "vcs"]);
    }

    #[test]
    fn test_skill_entry_parse_quoted_triggers() {
        let content = make_skill_md(
            r#"name: Commit Helper
description: Help with commits
triggers: ["commit", "提交"]
tags: ["git"]"#,
        );
        let entry = parse_skill_md(&content, Path::new("/fake/SKILL.md")).unwrap();
        assert_eq!(entry.triggers, vec!["commit", "提交"]);
    }

    #[test]
    fn test_skill_entry_parse_multiline_list() {
        let content = make_skill_md(
            r#"name: Debug
description: Debug help
triggers:
  - debug
  - troubleshoot
  - diagnose
tags:
  - debug
  - ops"#,
        );
        let entry = parse_skill_md(&content, Path::new("/fake/SKILL.md")).unwrap();
        assert_eq!(entry.triggers, vec!["debug", "troubleshoot", "diagnose"]);
        assert_eq!(entry.tags, vec!["debug", "ops"]);
    }

    #[test]
    fn test_suggest_no_match() {
        let router = SkillRouter::new();
        let results = router.suggest("hello world", 0.1, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_suggest_trigger_match() {
        let mut router = SkillRouter::new();
        router.load_from_skill(SkillEntry {
            name: "Git".to_string(),
            description: "Git ops".to_string(),
            triggers: vec!["commit".to_string()],
            tags: vec![],
            path: PathBuf::from("/fake"),
        });
        let results = router.suggest("please commit this change", 0.1, 10);
        assert_eq!(results.len(), 1);
        assert!(results[0].confidence > 0.5);
    }

    #[test]
    fn test_suggest_name_match() {
        let mut router = SkillRouter::new();
        router.load_from_skill(SkillEntry {
            name: "deploy".to_string(),
            description: "Deployment tool".to_string(),
            triggers: vec![],
            tags: vec![],
            path: PathBuf::from("/fake"),
        });
        let results = router.suggest("I need to deploy the app", 0.1, 10);
        assert_eq!(results.len(), 1);
        assert!((results[0].confidence - 0.5).abs() < 1e-10); // 1.5 / 3.0 = 0.5
    }

    #[test]
    fn test_suggest_multiple_sorted() {
        let mut router = SkillRouter::new();
        router.load_from_skill(SkillEntry {
            name: "git".to_string(),
            description: "Git ops".to_string(),
            triggers: vec!["commit".to_string()],
            tags: vec!["vcs".to_string()],
            path: PathBuf::from("/fake/git"),
        });
        router.load_from_skill(SkillEntry {
            name: "deploy".to_string(),
            description: "Deploy".to_string(),
            triggers: vec!["deploy".to_string()],
            tags: vec!["vcs".to_string()],
            path: PathBuf::from("/fake/deploy"),
        });
        let results = router.suggest("commit the code to the vcs", 0.1, 10);
        assert_eq!(results.len(), 2);
        assert!(results[0].confidence >= results[1].confidence);
        // git skill should be first (trigger + tag match vs tag-only match)
        assert_eq!(results[0].name, "git");
    }

    #[test]
    fn test_suggest_threshold_filter() {
        let mut router = SkillRouter::new();
        router.load_from_skill(SkillEntry {
            name: "git".to_string(),
            description: "Git ops".to_string(),
            triggers: vec!["commit".to_string()],
            tags: vec![],
            path: PathBuf::from("/fake"),
        });
        // High threshold should filter out the match
        let results = router.suggest("commit this", 0.99, 10);
        assert!(results.is_empty());
    }
}
