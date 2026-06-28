// crates/aletheon-runtime/src/impl/skills/plugin.rs

//! Skill plugin types and registration logic.
//!
//! A SkillPlugin is a parsed skill directory with its manifest,
//! references, and script paths. `register_skill()` wires a plugin's
//! tools and hooks into the runtime.

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};

use base::tool::{PermissionLevel, ToolExposure};
use base::Registry;
use corpus::tools::tools::script_tool::ScriptTool;
use corpus::tools::tools::ToolRegistry;

use super::manifest::{
    parse_exposure, parse_permission, HookManifest, HooksManifest, SkillManifest,
};

use crate::r#impl::hooks::registry::{HookRegistry, RegisteredHook};
use base::hook::HookPoint;

/// A fully parsed skill plugin with all metadata and content.
#[derive(Debug, Clone)]
pub struct SkillPlugin {
    /// Unique skill name.
    pub name: String,
    /// Semver version.
    pub version: String,
    /// Human-readable description.
    pub description: String,
    /// How this skill is triggered.
    pub trigger: TriggerType,
    /// Keywords for keyword-triggered activation.
    pub keywords: Vec<String>,
    /// Tools provided by this skill.
    pub tools: Vec<SkillToolDef>,
    /// Hooks registered by this skill.
    pub hooks: Vec<SkillHookDef>,
    /// System prompt injection (body of SKILL.md).
    pub system_prompt: String,
    /// Reference files from references/ directory.
    pub references: Vec<ReferenceFile>,
    /// Path to the scripts/ directory.
    pub scripts_dir: PathBuf,
    /// Path to the skill root directory.
    pub skill_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerType {
    Manual,
    Auto,
    Keyword,
}

#[derive(Debug, Clone)]
pub struct SkillToolDef {
    pub name: String,
    pub description: String,
    pub script: String,
    pub permission: PermissionLevel,
    pub exposure: ToolExposure,
    pub input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct SkillHookDef {
    pub name: String,
    pub point: HookPoint,
    pub script: String,
    pub priority: i32,
}

#[derive(Debug, Clone)]
pub struct ReferenceFile {
    pub name: String,
    pub content: String,
}

/// Build a SkillPlugin from a parsed manifest and directory.
pub fn build_skill_plugin(
    manifest: SkillManifest,
    body: String,
    skill_dir: PathBuf,
) -> SkillPlugin {
    let trigger = match manifest.trigger.as_deref() {
        Some("auto") => TriggerType::Auto,
        Some("keyword") => TriggerType::Keyword,
        _ => TriggerType::Manual,
    };

    let tools = manifest
        .tools
        .unwrap_or_default()
        .into_iter()
        .map(|t| SkillToolDef {
            name: t.name,
            description: t.description,
            script: t.script,
            permission: t
                .permission
                .as_deref()
                .map(parse_permission)
                .unwrap_or(PermissionLevel::L1),
            exposure: t
                .exposure
                .as_deref()
                .map(parse_exposure)
                .unwrap_or(ToolExposure::Direct),
            input_schema: t.input_schema,
        })
        .collect();

    let hooks = extract_hooks(manifest.hooks);

    // Read reference files
    let refs_dir = skill_dir.join("references");
    let references = read_references(&refs_dir);

    SkillPlugin {
        name: manifest.name,
        version: manifest.version.unwrap_or_else(|| "0.1.0".into()),
        description: manifest.description,
        trigger,
        keywords: manifest.keywords.unwrap_or_default(),
        tools,
        hooks,
        system_prompt: body,
        references,
        scripts_dir: skill_dir.join("scripts"),
        skill_dir,
    }
}

/// Extract hook definitions from the manifest's hooks section.
fn extract_hooks(hooks_manifest: Option<HooksManifest>) -> Vec<SkillHookDef> {
    let hm = match hooks_manifest {
        Some(h) => h,
        None => return Vec::new(),
    };

    let mut result = Vec::new();

    let entries: Vec<(Option<Vec<HookManifest>>, HookPoint)> = vec![
        (hm.on_session_start, HookPoint::OnSessionStart),
        (hm.on_session_end, HookPoint::OnSessionEnd),
        (hm.pre_turn, HookPoint::PreTurn),
        (hm.post_turn, HookPoint::PostTurn),
        (hm.pre_tool, HookPoint::PreTool),
        (hm.post_tool, HookPoint::PostTool),
        (hm.on_memory_store, HookPoint::OnMemoryStore),
        (hm.on_memory_recall, HookPoint::OnMemoryRecall),
    ];

    for (hooks_opt, point) in entries {
        if let Some(hooks) = hooks_opt {
            for h in hooks {
                result.push(SkillHookDef {
                    name: h.name,
                    point,
                    script: h.script,
                    priority: h.priority.unwrap_or(100),
                });
            }
        }
    }

    result
}

/// Read all .md files from a references directory.
fn read_references(dir: &PathBuf) -> Vec<ReferenceFile> {
    if !dir.exists() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut refs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(true, |ext| ext != "md") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            let name = path
                .file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            refs.push(ReferenceFile { name, content });
        }
    }

    refs
}

/// Register a skill's tools and hooks into the runtime.
pub fn register_skill(
    skill: &SkillPlugin,
    tool_registry: &mut ToolRegistry,
    hook_registry: &mut HookRegistry,
) {
    // Register tools
    for tool_def in &skill.tools {
        let script_path = skill.scripts_dir.join(&tool_def.script);
        if !script_path.exists() {
            warn!(
                skill = %skill.name,
                tool = %tool_def.name,
                path = %script_path.display(),
                "Tool script not found, skipping"
            );
            continue;
        }

        let mut tool = ScriptTool::new(
            tool_def.name.clone(),
            tool_def.description.clone(),
            script_path,
            tool_def.permission,
        );
        tool = tool.with_exposure(tool_def.exposure);
        if let Some(ref schema) = tool_def.input_schema {
            tool = tool.with_schema(schema.clone());
        }

        let _ = tool_registry.register(Arc::new(tool));
        info!(skill = %skill.name, tool = %tool_def.name, "Registered skill tool");
    }

    // Register hooks
    for hook_def in &skill.hooks {
        let script_path = skill.scripts_dir.join(&hook_def.script);
        if !script_path.exists() {
            warn!(
                skill = %skill.name,
                hook = %hook_def.name,
                path = %script_path.display(),
                "Hook script not found, skipping"
            );
            continue;
        }

        hook_registry.register(RegisteredHook {
            name: format!("{}:{}", skill.name, hook_def.name),
            source: format!("skill:{}", skill.name),
            script_path: Some(script_path),
            point: hook_def.point,
            priority: hook_def.priority,
        });
        info!(skill = %skill.name, hook = %hook_def.name, "Registered skill hook");
    }
}

#[cfg(test)]
mod tests {
    use super::super::manifest::{parse_skill_md, HookManifest, HooksManifest};
    use super::*;

    fn make_manifest(content: &str) -> (SkillManifest, String) {
        parse_skill_md(content).unwrap()
    }

    #[test]
    fn build_plugin_from_full_manifest() {
        let content = r#"---
name: test-skill
version: 2.0.0
description: Test skill
trigger: keyword
keywords: [test, demo]
tools:
  - name: my_tool
    description: Does something
    script: scripts/tool.sh
    permission: L0
hooks:
  pre_tool:
    - name: validate
      script: scripts/validate.sh
      priority: 5
---

System prompt content here.
"#;
        let (manifest, body) = make_manifest(content);
        let plugin = build_skill_plugin(manifest, body, PathBuf::from("/tmp/skill"));

        assert_eq!(plugin.name, "test-skill");
        assert_eq!(plugin.version, "2.0.0");
        assert_eq!(plugin.trigger, TriggerType::Keyword);
        assert_eq!(plugin.keywords, vec!["test", "demo"]);
        assert_eq!(plugin.tools.len(), 1);
        assert_eq!(plugin.tools[0].name, "my_tool");
        assert_eq!(plugin.tools[0].permission, PermissionLevel::L0);
        assert_eq!(plugin.hooks.len(), 1);
        assert_eq!(plugin.hooks[0].name, "validate");
        assert_eq!(plugin.hooks[0].point, HookPoint::PreTool);
        assert_eq!(plugin.hooks[0].priority, 5);
        assert!(plugin.system_prompt.contains("System prompt"));
    }

    #[test]
    fn build_plugin_minimal() {
        let content = r#"---
name: minimal
description: Minimal skill
---
"#;
        let (manifest, body) = make_manifest(content);
        let plugin = build_skill_plugin(manifest, body, PathBuf::from("/tmp/min"));

        assert_eq!(plugin.name, "minimal");
        assert_eq!(plugin.trigger, TriggerType::Manual);
        assert!(plugin.tools.is_empty());
        assert!(plugin.hooks.is_empty());
    }

    #[test]
    fn extract_hooks_all_points() {
        let hm = HooksManifest {
            on_session_start: Some(vec![HookManifest {
                name: "s".into(),
                script: "s.sh".into(),
                priority: None,
            }]),
            on_session_end: Some(vec![HookManifest {
                name: "e".into(),
                script: "e.sh".into(),
                priority: None,
            }]),
            pre_turn: Some(vec![HookManifest {
                name: "pt".into(),
                script: "pt.sh".into(),
                priority: None,
            }]),
            post_turn: Some(vec![HookManifest {
                name: "pot".into(),
                script: "pot.sh".into(),
                priority: None,
            }]),
            pre_tool: Some(vec![HookManifest {
                name: "prt".into(),
                script: "prt.sh".into(),
                priority: None,
            }]),
            post_tool: Some(vec![HookManifest {
                name: "pot2".into(),
                script: "pot2.sh".into(),
                priority: None,
            }]),
            on_memory_store: Some(vec![HookManifest {
                name: "ms".into(),
                script: "ms.sh".into(),
                priority: None,
            }]),
            on_memory_recall: Some(vec![HookManifest {
                name: "mr".into(),
                script: "mr.sh".into(),
                priority: None,
            }]),
        };
        let hooks = extract_hooks(Some(hm));
        assert_eq!(hooks.len(), 8);
    }

    #[test]
    fn read_references_empty_dir() {
        let refs = read_references(&PathBuf::from("/nonexistent"));
        assert!(refs.is_empty());
    }

    #[test]
    fn read_references_with_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("guide.md"), "# Guide").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not md").unwrap();

        let refs = read_references(&dir.path().to_path_buf());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "guide");
    }
}
