/// Skill loader — reads SKILL.md files from ~/.aletheon/skills/
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// A loaded skill.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Directory name (e.g., "code-review").
    pub name: String,
    /// First paragraph of SKILL.md (used in /help listing).
    pub description: String,
    /// Full SKILL.md content (injected as system context).
    pub content: String,
}

/// Loads skills from a directory containing skill subdirectories.
pub struct SkillLoader {
    skills_dir: PathBuf,
    skills: HashMap<String, Skill>,
}

impl SkillLoader {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            skills: HashMap::new(),
        }
    }

    /// Default skills directory: ~/.aletheon/skills/
    pub fn default_dir() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".aletheon")
            .join("skills")
    }

    /// Scan skills_dir for directories containing SKILL.md.
    pub fn load_all(&mut self) -> anyhow::Result<()> {
        self.skills.clear();

        if !self.skills_dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }

            let content = fs::read_to_string(&skill_md)?;
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let description = extract_description(&content);
            let skill = Skill {
                name: name.clone(),
                description,
                content,
            };
            self.skills.insert(name, skill);
        }

        Ok(())
    }

    /// Get a skill by name.
    #[allow(dead_code)] // TODO: Public API used in tests; may be needed for skill hot-reload
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// List all loaded skills.
    pub fn list(&self) -> Vec<&Skill> {
        self.skills.values().collect()
    }

    /// Build system context string listing available skills (for auto-trigger).
    #[allow(dead_code)] // TODO: Public API used in tests; will be wired into system prompt construction
    pub fn build_system_context(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut ctx = String::from("Available skills (invoke with /name):\n");
        for skill in self.skills.values() {
            ctx.push_str(&format!("- /{}: {}\n", skill.name, skill.description));
        }
        ctx.push_str("\nWhen the user's request matches a skill's purpose, suggest invoking it.\n");
        ctx
    }
}

/// Extract description from SKILL.md (first non-empty, non-heading line).
fn extract_description(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return trimmed.to_string();
    }
    "No description".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_extract_description() {
        let md = "# Code Review\n\nReview code for bugs.\n\n## Usage";
        assert_eq!(extract_description(md), "Review code for bugs.");
    }

    #[test]
    fn test_extract_description_no_content() {
        let md = "# Title\n\n## Section";
        assert_eq!(extract_description(md), "No description");
    }

    #[test]
    fn test_skill_loader_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();
        assert_eq!(loader.list().len(), 0);
    }

    #[test]
    fn test_skill_loader_with_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("test-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Test Skill\n\nA test skill for testing.",
        )
        .unwrap();

        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();

        assert_eq!(loader.list().len(), 1);
        let skill = loader.get("test-skill").unwrap();
        assert_eq!(skill.description, "A test skill for testing.");
    }

    #[test]
    fn test_build_system_context() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# My Skill\n\nDoes things.").unwrap();

        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();

        let ctx = loader.build_system_context();
        assert!(ctx.contains("/my-skill"));
        assert!(ctx.contains("Does things."));
    }
}
