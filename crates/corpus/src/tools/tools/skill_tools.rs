//! Dynamic skill-access tools.
//!
//! Skills are advertised to the model as a compact catalog (name + description)
//! in the system prompt; the model loads a skill's full instructions on demand
//! with `skill_get` instead of every skill's content being injected up front.
//! Two L0 read-only tools: `SkillListTool` and `SkillGetTool`.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::json;

use crate::skill::loader::LoadedSkill;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

/// Atomically replaceable Skill catalog shared by model tools and extension
/// runtime publication. Legacy entries remain immutable; package entries are
/// replaced as one snapshot.
#[derive(Clone)]
pub struct SharedSkills {
    legacy: Arc<Vec<LoadedSkill>>,
    current: Arc<RwLock<Arc<Vec<LoadedSkill>>>>,
}

impl SharedSkills {
    pub fn new(legacy: Arc<Vec<LoadedSkill>>) -> Self {
        Self {
            current: Arc::new(RwLock::new(legacy.clone())),
            legacy,
        }
    }

    pub fn snapshot(&self) -> Arc<Vec<LoadedSkill>> {
        self.current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn validate_extensions(&self, extensions: &[LoadedSkill]) -> anyhow::Result<()> {
        let mut names = std::collections::BTreeSet::new();
        for skill in self.legacy.iter().chain(extensions) {
            anyhow::ensure!(
                names.insert(skill.name.to_ascii_lowercase()),
                "duplicate public skill name '{}'",
                skill.name
            );
        }
        Ok(())
    }

    pub fn replace_extensions(&self, mut extensions: Vec<LoadedSkill>) -> anyhow::Result<()> {
        self.validate_extensions(&extensions)?;
        let mut combined = self.legacy.as_ref().clone();
        combined.append(&mut extensions);
        combined.sort_by(|left, right| left.name.cmp(&right.name));
        *self
            .current
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(combined);
        Ok(())
    }
}

impl From<Arc<Vec<LoadedSkill>>> for SharedSkills {
    fn from(skills: Arc<Vec<LoadedSkill>>) -> Self {
        Self::new(skills)
    }
}

fn result(
    ctx: &ToolContext,
    start: fabric::MonoTime,
    content: String,
    is_error: bool,
) -> ToolResult {
    ToolResult {
        content,
        is_error,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
            truncated: false,
            patch_delta: None,
        },
    }
}

/// List available skills (name, source, description) so the model can decide
/// which to load with `skill_get`.
pub struct SkillListTool {
    skills: SharedSkills,
}

impl SkillListTool {
    pub fn new(skills: impl Into<SharedSkills>) -> Self {
        Self {
            skills: skills.into(),
        }
    }
}

#[async_trait]
impl Tool for SkillListTool {
    fn name(&self) -> &str {
        "skill_list"
    }
    fn description(&self) -> &str {
        "List available skills (name, description). Load a skill's full instructions with skill_get when a task matches it."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{},"required":[]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(SkillListTool {
            skills: self.skills.clone(),
        })
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let skills = self.skills.snapshot();
        if skills.is_empty() {
            return result(ctx, start, "(no skills available)".to_string(), false);
        }
        let mut out = String::new();
        for s in skills.iter() {
            out.push_str(&format!("- {} [{}]: {}\n", s.name, s.source, s.description));
        }
        result(ctx, start, out, false)
    }
}

/// Load the full markdown instructions of a named skill.
pub struct SkillGetTool {
    skills: SharedSkills,
}

impl SkillGetTool {
    pub fn new(skills: impl Into<SharedSkills>) -> Self {
        Self {
            skills: skills.into(),
        }
    }
}

#[async_trait]
impl Tool for SkillGetTool {
    fn name(&self) -> &str {
        "skill_get"
    }
    fn description(&self) -> &str {
        "Load a named skill's full instructions. Call skill_list first to see available skill names."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"name":{"type":"string","description":"Skill name as shown by skill_list"}},"required":["name"]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(SkillGetTool {
            skills: self.skills.clone(),
        })
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            return result(
                ctx,
                start,
                "skill_get error: 'name' is required".to_string(),
                true,
            );
        }
        let skills = self.skills.snapshot();
        match skills.iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            Some(s) => result(ctx, start, format!("# {}\n{}", s.name, s.content), false),
            None => {
                let available = skills
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                result(
                    ctx,
                    start,
                    format!("skill_get error: no skill named '{name}'. Available: {available}"),
                    true,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;

    fn skills() -> SharedSkills {
        SharedSkills::new(Arc::new(vec![LoadedSkill {
            name: "commit".to_string(),
            description: "Commit, push, PR".to_string(),
            content: "Full commit instructions here.".to_string(),
            source: "system".to_string(),
        }]))
    }

    fn ctx() -> ToolContext {
        ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: Arc::new(TestClock::default()),
            turn_event_sender: None,
        }
    }

    #[tokio::test]
    async fn skill_list_shows_catalog_not_content() {
        let out = SkillListTool::new(skills())
            .execute(json!({}), &ctx())
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("commit"));
        assert!(out.content.contains("Commit, push, PR"));
        assert!(!out.content.contains("Full commit instructions"));
    }

    #[tokio::test]
    async fn skill_get_returns_full_content() {
        let out = SkillGetTool::new(skills())
            .execute(json!({"name":"commit"}), &ctx())
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("Full commit instructions here."));
    }

    #[tokio::test]
    async fn skill_get_unknown_is_error_and_lists_available() {
        let out = SkillGetTool::new(skills())
            .execute(json!({"name":"nope"}), &ctx())
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("commit"));
    }

    #[tokio::test]
    async fn shared_catalog_replaces_extension_skills_without_touching_legacy() {
        let catalog = skills();
        let tool = SkillListTool::new(catalog.clone());
        catalog
            .replace_extensions(vec![LoadedSkill {
                name: "package-review".into(),
                description: "Packaged review".into(),
                content: "Review from package".into(),
                source: "package:test.pkg:skill.review".into(),
            }])
            .unwrap();
        let first = tool.execute(json!({}), &ctx()).await;
        assert!(first.content.contains("commit"));
        assert!(first.content.contains("package-review"));

        catalog.replace_extensions(Vec::new()).unwrap();
        let second = tool.execute(json!({}), &ctx()).await;
        assert!(second.content.contains("commit"));
        assert!(!second.content.contains("package-review"));
    }
}
