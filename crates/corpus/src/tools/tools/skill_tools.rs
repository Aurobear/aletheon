//! Dynamic skill-access tools.
//!
//! Skills are advertised to the model as a compact catalog (name + description)
//! in the system prompt; the model loads a skill's full instructions on demand
//! with `skill_get` instead of every skill's content being injected up front.
//! Two L0 read-only tools: `SkillListTool` and `SkillGetTool`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::skill::loader::LoadedSkill;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

/// Immutable snapshot of the skills loaded at daemon start.
pub type SharedSkills = Arc<Vec<LoadedSkill>>;

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
    pub fn new(skills: SharedSkills) -> Self {
        Self { skills }
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
        if self.skills.is_empty() {
            return result(ctx, start, "(no skills available)".to_string(), false);
        }
        let mut out = String::new();
        for s in self.skills.iter() {
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
    pub fn new(skills: SharedSkills) -> Self {
        Self { skills }
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
        match self
            .skills
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
        {
            Some(s) => result(ctx, start, format!("# {}\n{}", s.name, s.content), false),
            None => {
                let available = self
                    .skills
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
        Arc::new(vec![LoadedSkill {
            name: "commit".to_string(),
            description: "Commit, push, PR".to_string(),
            content: "Full commit instructions here.".to_string(),
            source: "system".to_string(),
        }])
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
}
