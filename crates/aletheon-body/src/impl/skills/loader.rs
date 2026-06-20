//! Skill file loader -- scans a directory for Markdown skill files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::markdown_skill::MarkdownSkill;

/// Loads and manages Markdown skills from disk.
#[derive(Debug)]
pub struct SkillLoader {
    skills_dir: PathBuf,
    skills: HashMap<String, MarkdownSkill>,
}

impl SkillLoader {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            skills: HashMap::new(),
        }
    }

    /// Load all skills from the skills directory.
    ///
    /// Returns the number of skills successfully loaded.
    pub fn load_all(&mut self) -> Result<usize, String> {
        self.skills.clear();
        let mut count = 0;

        if !self.skills_dir.exists() {
            return Ok(0);
        }

        for entry in std::fs::read_dir(&self.skills_dir)
            .map_err(|e| format!("Failed to read skills dir: {}", e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            if path.extension().map_or(false, |ext| ext == "md") {
                match self.load_skill(&path) {
                    Ok(skill) => {
                        self.skills.insert(skill.trigger.clone(), skill);
                        count += 1;
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to load skill {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(count)
    }

    /// Load a single skill file.
    fn load_skill(&self, path: &Path) -> Result<MarkdownSkill, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {:?}: {}", path, e))?;
        MarkdownSkill::parse(&raw)
    }

    /// Get a skill by trigger command.
    pub fn get(&self, trigger: &str) -> Option<&MarkdownSkill> {
        self.skills.get(trigger)
    }

    /// List all loaded skills.
    pub fn list(&self) -> Vec<&MarkdownSkill> {
        self.skills.values().collect()
    }

    /// Get skill names for tab completion.
    pub fn completion_candidates(&self) -> Vec<String> {
        self.skills
            .keys()
            .map(|k| {
                if k.starts_with('/') {
                    k.clone()
                } else {
                    format!("/{}", k)
                }
            })
            .collect()
    }

    /// Reload all skills from disk.
    pub fn reload(&mut self) -> usize {
        self.load_all().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_from_dir() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("review.md");
        fs::write(
            &skill_path,
            r#"---
name: Review
description: Code review
trigger: /review
---
Review this code."#,
        )
        .unwrap();

        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        let count = loader.load_all().unwrap();
        assert_eq!(count, 1);

        let skill = loader.get("/review").unwrap();
        assert_eq!(skill.name, "Review");
        assert_eq!(skill.system_prompt(), "Review this code.");
    }

    #[test]
    fn load_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        assert_eq!(loader.load_all().unwrap(), 0);
    }

    #[test]
    fn load_nonexistent_dir() {
        let mut loader = SkillLoader::new(PathBuf::from("/nonexistent/path"));
        assert_eq!(loader.load_all().unwrap(), 0);
    }

    #[test]
    fn completion_candidates() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.md"),
            "---\nname: A\ntrigger: /alpha\n---\nContent",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.md"),
            "---\nname: B\ntrigger: /beta\n---\nContent",
        )
        .unwrap();

        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();

        let mut candidates = loader.completion_candidates();
        candidates.sort();
        assert_eq!(candidates, vec!["/alpha", "/beta"]);
    }
}
