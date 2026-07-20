use std::path::PathBuf;
use tracing::{debug, info, warn};

use super::manifest::parse_skill_md;
use super::plugin::{build_skill_plugin, SkillPlugin};

/// A parsed skill loaded from a markdown file.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    /// Human-readable skill name (from the `# Name` heading).
    pub name: String,
    /// Short description (lines between heading and first blank line).
    pub description: String,
    /// Full markdown content (everything after the description block).
    pub content: String,
    /// Origin of the skill: "system", "user", or "learned".
    pub source: String,
}

/// Loads skill markdown files from a directory and caches them.
pub struct SkillLoader {
    skills_dir: PathBuf,
    cache: Vec<LoadedSkill>,
    plugins: Vec<SkillPlugin>,
}

impl SkillLoader {
    /// Create a new loader pointing at the given skills directory.
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            cache: Vec::new(),
            plugins: Vec::new(),
        }
    }

    /// Scan the skills directory for `*.md` files, parse each one, and
    /// populate the internal cache. Returns the number of skills loaded.
    pub fn load_all(&mut self) -> usize {
        if !self.skills_dir.exists() {
            debug!(dir = %self.skills_dir.display(), "Skills directory does not exist");
            return 0;
        }

        let entries = match std::fs::read_dir(&self.skills_dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, dir = %self.skills_dir.display(), "Failed to read skills directory");
                return 0;
            }
        };

        let mut skills = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }
            match Self::parse_skill_file(&path) {
                Ok(skill) => {
                    debug!(name = %skill.name, path = %path.display(), "Loaded skill");
                    skills.push(skill);
                }
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "Failed to parse skill file");
                }
            }
        }

        let count = skills.len();
        self.cache = skills;
        if count > 0 {
            info!(count = count, "Skills loaded");
        }
        count
    }

    /// Clear the cache and reload all skills from disk.
    pub fn reload(&mut self) -> usize {
        self.cache.clear();
        self.plugins.clear();
        self.load_all()
    }

    /// Return a reference to the cached skills.
    pub fn skills(&self) -> &[LoadedSkill] {
        &self.cache
    }

    /// Return a reference to the loaded skill plugins.
    pub fn plugins(&self) -> &[SkillPlugin] {
        &self.plugins
    }

    /// Load all skills — both multi-file directories and legacy single .md files.
    /// Returns the total number of skills loaded (legacy + directory-based).
    pub fn load_all_enhanced(&mut self) -> usize {
        if !self.skills_dir.exists() {
            debug!(dir = %self.skills_dir.display(), "Skills directory does not exist");
            return 0;
        }

        let entries = match std::fs::read_dir(&self.skills_dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, dir = %self.skills_dir.display(), "Failed to read skills directory");
                return 0;
            }
        };

        let mut skills = Vec::new();
        let mut plugins = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                // Multi-file skill directory
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    match Self::parse_skill_dir(&path) {
                        Ok(plugin) => {
                            debug!(name = %plugin.name, path = %path.display(), "Loaded skill directory");
                            skills.push(LoadedSkill {
                                name: plugin.name.clone(),
                                description: plugin.description.clone(),
                                content: plugin.system_prompt.clone(),
                                source: "system".into(),
                            });
                            plugins.push(plugin);
                        }
                        Err(e) => {
                            debug!(error = %e, path = %path.display(), "Skipping skill directory (no valid frontmatter)");
                        }
                    }
                }
            } else if path.extension().is_some_and(|ext| ext == "md") {
                // Legacy single .md file
                match Self::parse_skill_file(&path) {
                    Ok(skill) => {
                        debug!(name = %skill.name, path = %path.display(), "Loaded skill");
                        skills.push(skill);
                    }
                    Err(e) => {
                        warn!(error = %e, path = %path.display(), "Failed to parse skill file");
                    }
                }
            }
        }

        let count = skills.len();
        self.cache = skills;
        self.plugins = plugins;
        if count > 0 {
            info!(count = count, "Skills loaded");
        }
        count
    }

    /// Parse a skill directory containing SKILL.md.
    fn parse_skill_dir(dir: &std::path::Path) -> anyhow::Result<SkillPlugin> {
        let skill_md = dir.join("SKILL.md");
        let raw = std::fs::read_to_string(&skill_md)?;
        let (manifest, body) = parse_skill_md(&raw)?;
        Ok(build_skill_plugin(manifest, body, dir.to_path_buf()))
    }

    /// Parse a single skill markdown file into a `LoadedSkill`.
    ///
    /// Format expected:
    /// ```text
    /// # Skill Name
    /// Short description line
    /// (possibly multi-line, ends at first blank line)
    ///
    /// Rest of content...
    /// ```
    fn parse_skill_file(path: &PathBuf) -> anyhow::Result<LoadedSkill> {
        let raw = std::fs::read_to_string(path)?;

        // Determine source from file path or content heuristic
        let source = if path.to_string_lossy().contains("learned") {
            "learned"
        } else if path.to_string_lossy().contains("user") {
            "user"
        } else {
            "system"
        };

        let mut lines = raw.lines();

        // First line: `# Skill Name`
        let name = match lines.next() {
            Some(line) => {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("# ")
                    .or_else(|| trimmed.strip_prefix("#"))
                    .unwrap_or(trimmed)
                    .to_string()
            }
            None => {
                return Err(anyhow::anyhow!("Empty skill file: {}", path.display()));
            }
        };

        // Collect description lines until first blank line
        let mut description_lines = Vec::new();
        let mut content_start = Vec::new();
        let mut found_blank = false;

        for line in lines {
            if !found_blank {
                if line.trim().is_empty() {
                    found_blank = true;
                } else {
                    description_lines.push(line);
                }
            } else {
                content_start.push(line);
            }
        }

        // If no blank line was found, description is everything after heading
        // and content is empty
        if !found_blank {
            description_lines.clear();
            // Re-parse: treat everything after heading as description
            let all_lines: Vec<&str> = raw.lines().skip(1).collect();
            description_lines.extend(all_lines);
        }

        let description = description_lines.join("\n");
        let content = content_start.join("\n");

        Ok(LoadedSkill {
            name,
            description,
            content,
            source: source.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, filename: &str, content: &str) {
        fs::write(dir.join(filename), content).unwrap();
    }

    #[test]
    fn parse_skill_with_heading_and_description() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        write_skill(
            &path,
            "test-skill.md",
            "# Test Skill\nThis is a description.\nAnother line.\n\n## Details\nSome content here.\n",
        );

        let mut loader = SkillLoader::new(path);
        let count = loader.load_all();
        assert_eq!(count, 1);

        let skill = &loader.skills()[0];
        assert_eq!(skill.name, "Test Skill");
        assert!(skill.description.contains("This is a description."));
        assert!(skill.content.contains("## Details"));
        assert!(skill.content.contains("Some content here."));
    }

    #[test]
    fn parse_skill_heading_only() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        write_skill(&path, "minimal.md", "# Minimal Skill\nA one-liner.\n");

        let mut loader = SkillLoader::new(path);
        let count = loader.load_all();
        assert_eq!(count, 1);
        assert_eq!(loader.skills()[0].name, "Minimal Skill");
    }

    #[test]
    fn load_all_skips_non_md_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        write_skill(&path, "good.md", "# Good Skill\nDescription.\n\nContent.\n");
        fs::write(path.join("notes.txt"), "not a skill").unwrap();

        let mut loader = SkillLoader::new(path);
        assert_eq!(loader.load_all(), 1);
    }

    #[test]
    fn load_all_returns_zero_for_missing_dir() {
        let mut loader = SkillLoader::new(PathBuf::from("/nonexistent/skills"));
        assert_eq!(loader.load_all(), 0);
    }

    #[test]
    fn reload_clears_and_reloads() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();

        let mut loader = SkillLoader::new(path.clone());
        assert_eq!(loader.load_all(), 0);

        write_skill(
            &path,
            "new-skill.md",
            "# New Skill\nA new skill.\n\nDetails.\n",
        );
        assert_eq!(loader.reload(), 1);
    }

    #[test]
    fn empty_file_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        write_skill(&path, "empty.md", "");

        let mut loader = SkillLoader::new(path);
        // Should load 0 because empty file fails to parse
        assert_eq!(loader.load_all(), 0);
    }

    #[test]
    fn multiple_skills_loaded() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        write_skill(
            &path,
            "alpha.md",
            "# Alpha\nFirst skill.\n\nAlpha content.\n",
        );
        write_skill(&path, "beta.md", "# Beta\nSecond skill.\n\nBeta content.\n");
        write_skill(
            &path,
            "gamma.md",
            "# Gamma\nThird skill.\n\nGamma content.\n",
        );

        let mut loader = SkillLoader::new(path);
        assert_eq!(loader.load_all(), 3);

        let names: Vec<&str> = loader.skills().iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Alpha"));
        assert!(names.contains(&"Beta"));
        assert!(names.contains(&"Gamma"));
    }

    #[test]
    fn source_determined_from_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        write_skill(&path, "sys.md", "# Sys\nSystem skill.\n");

        let mut loader = SkillLoader::new(path);
        loader.load_all();
        assert_eq!(loader.skills()[0].source, "system");
    }

    #[test]
    fn load_skill_directory_with_manifest() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir(&skill_dir).unwrap();
        std::fs::create_dir(skill_dir.join("scripts")).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: Test\n---\nBody content.\n",
        )
        .unwrap();

        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        let count = loader.load_all_enhanced();
        assert_eq!(count, 1);
        assert_eq!(loader.plugins().len(), 1);
        assert_eq!(loader.plugins()[0].name, "my-skill");
    }

    #[test]
    fn load_mixed_legacy_and_directory() {
        let dir = TempDir::new().unwrap();
        // Legacy file
        std::fs::write(dir.path().join("legacy.md"), "# Legacy\nA legacy skill.\n").unwrap();
        // Directory skill
        let skill_dir = dir.path().join("modern");
        std::fs::create_dir(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: modern\ndescription: Modern skill\n---\nContent.\n",
        )
        .unwrap();

        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        let count = loader.load_all_enhanced();
        assert_eq!(count, 2);
        assert_eq!(loader.plugins().len(), 1); // Only directory skills become plugins
    }

    #[test]
    fn load_enhanced_returns_zero_for_missing_dir() {
        let mut loader = SkillLoader::new(PathBuf::from("/nonexistent/skills"));
        assert_eq!(loader.load_all_enhanced(), 0);
    }

    #[test]
    fn plugins_empty_by_default() {
        let dir = TempDir::new().unwrap();
        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        loader.load_all();
        assert!(loader.plugins().is_empty());
    }

    #[test]
    fn frontmatterless_skill_dir_skipped_without_error() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("no-frontmatter");
        std::fs::create_dir(&skill_dir).unwrap();
        // SKILL.md without --- frontmatter should be silently skipped
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "# Not a valid skill\nJust some text.\n",
        )
        .unwrap();

        let mut loader = SkillLoader::new(dir.path().to_path_buf());
        let count = loader.load_all_enhanced();
        // Should not error, just skip
        assert_eq!(count, 0);
        assert!(loader.plugins().is_empty());
    }
}
