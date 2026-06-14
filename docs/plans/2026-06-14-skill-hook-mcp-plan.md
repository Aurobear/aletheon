# Skill Plugin System, Hook Mechanism & Embedded MCP Server — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate a multi-file skill plugin system, lifecycle hooks, and an embedded MCP server into Aletheon's existing runtime/body/brain layers.

**Architecture:** Layer-Integrated (方案 A). Skills are directories with SKILL.md manifests, parsed by an enhanced SkillLoader. Hooks are synchronous callbacks registered in a HookRegistry and executed at lifecycle points in the ReAct loop. The MCP server is embedded in the daemon and auto-exposes body tools via MCP protocol over a Unix socket.

**Tech Stack:** Rust, tokio, serde/serde_yaml/serde_json, async-trait, anyhow, tracing

**Spec:** `docs/plans/2026-06-14-skill-hook-mcp-design.md`

---

## File Map

### New Files (10)

| # | File | Crate | Purpose |
|---|------|-------|---------|
| 1 | `crates/aletheon-abi/src/hook.rs` | abi | HookPoint, HookContext, HookResult types |
| 2 | `crates/aletheon-body/src/impl/tools/script_tool.rs` | body | ScriptTool: external script as Tool |
| 3 | `crates/aletheon-runtime/src/impl/skills/manifest.rs` | runtime | SKILL.md YAML frontmatter parsing |
| 4 | `crates/aletheon-runtime/src/impl/skills/plugin.rs` | runtime | SkillPlugin type + register_skill() |
| 5 | `crates/aletheon-runtime/src/impl/hooks/mod.rs` | runtime | Hooks module declaration |
| 6 | `crates/aletheon-runtime/src/impl/hooks/registry.rs` | runtime | HookRegistry: register + execute |
| 7 | `crates/aletheon-runtime/src/impl/hooks/builtin/mod.rs` | runtime | Builtin hooks module |
| 8 | `crates/aletheon-runtime/src/impl/hooks/builtin/audit_hook.rs` | runtime | Audit logging hook |
| 9 | `crates/aletheon-runtime/src/impl/daemon/mcp_embedded.rs` | runtime | Embedded MCP server |

### Modified Files (7)

| # | File | Changes |
|---|------|---------|
| 1 | `crates/aletheon-abi/src/lib.rs` | Add `pub mod hook;` |
| 2 | `crates/aletheon-body/src/impl/tools/mod.rs` | Add `pub mod script_tool;` |
| 3 | `crates/aletheon-runtime/src/impl/skills/loader.rs` | Enhance for multi-file skill dirs |
| 4 | `crates/aletheon-runtime/src/impl/skills/mod.rs` | Add `pub mod plugin; pub mod manifest;` |
| 5 | `crates/aletheon-runtime/src/impl/mod.rs` | Add `pub mod hooks;` |
| 6 | `crates/aletheon-runtime/src/impl/daemon/mod.rs` | Add `pub mod mcp_embedded;` |
| 7 | `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Integrate hooks + MCP |

---

## Task 1: ABI Hook Types

**Files:**
- Create: `crates/aletheon-abi/src/hook.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

### Step 1: Create hook.rs

```rust
// crates/aletheon-abi/src/hook.rs

//! Hook types — lifecycle callback definitions for the Aletheon runtime.
//!
//! Hooks are synchronous intervention points in the ReAct loop where
//! external scripts or builtin logic can inspect and modify behavior.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Points in the execution lifecycle where hooks can intervene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookPoint {
    /// Fired once when a new session starts.
    OnSessionStart,
    /// Fired when a session ends.
    OnSessionEnd,
    /// Fired before processing a user message.
    PreTurn,
    /// Fired after LLM response is generated.
    PostTurn,
    /// Fired before a tool executes.
    PreTool,
    /// Fired after a tool executes.
    PostTool,
    /// Fired when a memory entry is stored.
    OnMemoryStore,
    /// Fired when a memory entry is recalled.
    OnMemoryRecall,
}

/// Context passed to hook execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    /// Which hook point triggered this execution.
    pub point: HookPoint,
    /// Current session identifier.
    pub session_id: String,
    /// Number of turns completed in this session.
    pub turn_count: usize,
    /// Tool name (for PreTool/PostTool hooks).
    pub tool_name: Option<String>,
    /// Tool input (for PreTool hooks).
    pub tool_input: Option<serde_json::Value>,
    /// Tool result (for PostTool hooks).
    pub tool_result: Option<HookToolResult>,
    /// User message (for PreTurn hooks).
    pub message: Option<String>,
    /// Arbitrary key-value metadata.
    pub metadata: HashMap<String, String>,
}

/// Simplified tool result for hook context serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookToolResult {
    pub content: String,
    pub is_error: bool,
    pub execution_time_ms: u64,
}

/// Result of hook execution.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// Continue normal execution without modification.
    Continue,
    /// Modify the tool input (only valid for PreTool).
    ModifyInput(serde_json::Value),
    /// Block execution with a reason.
    Block { reason: String },
    /// Inject additional content into the user message.
    Inject(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_point_serde_roundtrip() {
        let points = vec![
            HookPoint::OnSessionStart,
            HookPoint::PreTool,
            HookPoint::PostTurn,
        ];
        for point in points {
            let json = serde_json::to_string(&point).unwrap();
            let back: HookPoint = serde_json::from_str(&json).unwrap();
            assert_eq!(point, back);
        }
    }

    #[test]
    fn hook_context_serde_roundtrip() {
        let ctx = HookContext {
            point: HookPoint::PreTool,
            session_id: "test-session".into(),
            turn_count: 5,
            tool_name: Some("bash_exec".into()),
            tool_input: Some(serde_json::json!({"command": "ls"})),
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let back: HookContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back.point, HookPoint::PreTool);
        assert_eq!(back.tool_name, Some("bash_exec".into()));
    }

    #[test]
    fn hook_result_continue_is_default() {
        let result = HookResult::Continue;
        assert!(matches!(result, HookResult::Continue));
    }

    #[test]
    fn hook_tool_result_serde() {
        let result = HookToolResult {
            content: "output".into(),
            is_error: false,
            execution_time_ms: 100,
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: HookToolResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "output");
        assert!(!back.is_error);
    }
}
```

### Step 2: Add to lib.rs

Add this line to `crates/aletheon-abi/src/lib.rs` after `pub mod tool;`:

```rust
pub mod hook;
```

And add to the re-exports section:

```rust
pub use hook::{HookPoint, HookContext, HookToolResult, HookResult};
```

### Step 3: Verify

```bash
cd /home/aurobear/Bear-ws/work/aletheon
cargo test -p aletheon-abi -- hook
```

Expected: All 4 hook tests pass.

### Step 4: Commit

```bash
git add crates/aletheon-abi/src/hook.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add hook types for lifecycle callbacks

HookPoint enum with 8 lifecycle points (session, turn, tool, memory).
HookContext carries execution state. HookResult supports Continue,
ModifyInput, Block, and Inject outcomes."
```

---

## Task 2: ScriptTool

**Files:**
- Create: `crates/aletheon-body/src/impl/tools/script_tool.rs`
- Modify: `crates/aletheon-body/src/impl/tools/mod.rs`

### Step 1: Create script_tool.rs

```rust
// crates/aletheon-body/src/impl/tools/script_tool.rs

//! A tool backed by an external script.
//!
//! ScriptTool wraps an executable script (bash, python, etc.) as a
//! Tool instance. Input is passed as JSON on stdin; stdout is parsed
//! as the tool result.

use std::path::PathBuf;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;
use tracing::warn;

use aletheon_abi::tool::{PermissionLevel, Tool, ToolContext, ToolExposure, ToolResult, ToolResultMeta};

/// A tool backed by an external executable script.
#[derive(Debug, Clone)]
pub struct ScriptTool {
    name: String,
    description: String,
    script_path: PathBuf,
    permission: PermissionLevel,
    exposure: ToolExposure,
    input_schema: Value,
}

impl ScriptTool {
    pub fn new(
        name: String,
        description: String,
        script_path: PathBuf,
        permission: PermissionLevel,
    ) -> Self {
        Self {
            name,
            description,
            script_path,
            permission,
            exposure: ToolExposure::Direct,
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            }),
        }
    }

    /// Set a custom JSON Schema for input validation.
    pub fn with_schema(mut self, schema: Value) -> Self {
        self.input_schema = schema;
        self
    }

    /// Set the exposure level.
    pub fn with_exposure(mut self, exposure: ToolExposure) -> Self {
        self.exposure = exposure;
        self
    }
}

#[async_trait]
impl Tool for ScriptTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let start = Instant::now();

        // Check script exists
        if !self.script_path.exists() {
            return ToolResult {
                content: format!("Script not found: {}", self.script_path.display()),
                is_error: true,
                metadata: ToolResultMeta::default(),
            };
        }

        // Serialize input as JSON for stdin
        let input_json = serde_json::to_string(&input).unwrap_or_default();

        // Execute script
        let result = Command::new(&self.script_path)
            .current_dir(&ctx.working_dir)
            .env("ALETHEON_SESSION_ID", &ctx.session_id)
            .env("ALETHEON_TOOL_INPUT", &input_json)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await;

        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if output.status.success() {
                    // Try to parse stdout as JSON for structured result
                    if let Ok(value) = serde_json::from_str::<Value>(&stdout) {
                        // If JSON has "content" field, use it
                        if let Some(content) = value.get("content").and_then(|v| v.as_str()) {
                            return ToolResult {
                                content: content.to_string(),
                                is_error: false,
                                metadata: ToolResultMeta {
                                    execution_time_ms: elapsed,
                                    truncated: false,
                                },
                            };
                        }
                    }
                    // Plain text output
                    ToolResult {
                        content: stdout.trim().to_string(),
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: elapsed,
                            truncated: false,
                        },
                    }
                } else {
                    let error_msg = if stderr.is_empty() {
                        format!("Script exited with code {:?}", output.status.code())
                    } else {
                        stderr.trim().to_string()
                    };
                    warn!(script = %self.script_path.display(), error = %error_msg, "Script failed");
                    ToolResult {
                        content: error_msg,
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: elapsed,
                            truncated: false,
                        },
                    }
                }
            }
            Err(e) => {
                warn!(script = %self.script_path.display(), error = %e, "Script spawn failed");
                ToolResult {
                    content: format!("Failed to execute script: {}", e),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: elapsed,
                        truncated: false,
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn script_tool_basic_properties() {
        let tool = ScriptTool::new(
            "test_tool".into(),
            "A test tool".into(),
            PathBuf::from("/tmp/test.sh"),
            PermissionLevel::L0,
        );
        assert_eq!(tool.name(), "test_tool");
        assert_eq!(tool.description(), "A test tool");
        assert_eq!(tool.permission_level(), PermissionLevel::L0);
        assert_eq!(tool.exposure(), ToolExposure::Direct);
    }

    #[test]
    fn script_tool_with_schema() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let tool = ScriptTool::new(
            "t".into(), "d".into(), PathBuf::from("/tmp/t.sh"), PermissionLevel::L1,
        ).with_schema(schema.clone());
        assert_eq!(tool.input_schema(), schema);
    }

    #[test]
    fn script_tool_with_exposure() {
        let tool = ScriptTool::new(
            "t".into(), "d".into(), PathBuf::from("/tmp/t.sh"), PermissionLevel::L1,
        ).with_exposure(ToolExposure::Deferred);
        assert_eq!(tool.exposure(), ToolExposure::Deferred);
    }

    #[tokio::test]
    async fn script_tool_execute_missing_script() {
        let tool = ScriptTool::new(
            "t".into(), "d".into(), PathBuf::from("/nonexistent/script.sh"), PermissionLevel::L1,
        );
        let ctx = ToolContext {
            working_dir: PathBuf::from("/tmp"),
            session_id: "test".into(),
        };
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn script_tool_execute_success() {
        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("hello.sh");
        std::fs::write(&script_path, "#!/bin/bash\necho \"hello world\"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let tool = ScriptTool::new(
            "hello".into(), "says hello".into(), script_path, PermissionLevel::L0,
        );
        let ctx = ToolContext {
            working_dir: dir.path().to_path_buf(),
            session_id: "test".into(),
        };
        let result = tool.execute(json!({}), &ctx).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hello world");
    }

    #[tokio::test]
    async fn script_tool_execute_failure() {
        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("fail.sh");
        std::fs::write(&script_path, "#!/bin/bash\nexit 1").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let tool = ScriptTool::new(
            "fail".into(), "fails".into(), script_path, PermissionLevel::L1,
        );
        let ctx = ToolContext {
            working_dir: dir.path().to_path_buf(),
            session_id: "test".into(),
        };
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn script_tool_execute_json_output() {
        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("json.sh");
        std::fs::write(&script_path, "#!/bin/bash\necho '{\"content\": \"structured\"}'").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let tool = ScriptTool::new(
            "json_out".into(), "outputs json".into(), script_path, PermissionLevel::L0,
        );
        let ctx = ToolContext {
            working_dir: dir.path().to_path_buf(),
            session_id: "test".into(),
        };
        let result = tool.execute(json!({}), &ctx).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "structured");
    }

    #[test]
    fn script_tool_boxed_clone() {
        let tool = ScriptTool::new(
            "t".into(), "d".into(), PathBuf::from("/tmp/t.sh"), PermissionLevel::L1,
        );
        let cloned = tool.boxed_clone();
        assert_eq!(cloned.name(), "t");
    }
}
```

### Step 2: Add to tools/mod.rs

Add this line to `crates/aletheon-body/src/impl/tools/mod.rs` after `pub mod apply_patch;`:

```rust
pub mod script_tool;
```

### Step 3: Verify

```bash
cargo test -p aletheon-body -- script_tool
```

Expected: All 8 script_tool tests pass.

### Step 4: Commit

```bash
git add crates/aletheon-body/src/impl/tools/script_tool.rs crates/aletheon-body/src/impl/tools/mod.rs
git commit -m "feat(body): add ScriptTool for external script-backed tools

ScriptTool wraps executable scripts as Tool instances. Input passed
as JSON on stdin, stdout parsed as result. Supports JSON structured
output and plain text fallback."
```

---

## Task 3: Skill Manifest Parsing

**Files:**
- Create: `crates/aletheon-runtime/src/impl/skills/manifest.rs`

### Step 1: Create manifest.rs

```rust
// crates/aletheon-runtime/src/impl/skills/manifest.rs

//! SKILL.md YAML frontmatter parsing.
//!
//! Parses the `---` delimited YAML header from SKILL.md files into
//! typed manifest structures.

use serde::{Deserialize, Serialize};
use aletheon_abi::tool::PermissionLevel;

/// Raw YAML frontmatter from SKILL.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: String,
    pub trigger: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub tools: Option<Vec<ToolManifest>>,
    pub hooks: Option<HooksManifest>,
}

/// Tool declaration in SKILL.md frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    pub description: String,
    pub script: String,
    pub permission: Option<String>,
    pub exposure: Option<String>,
    pub input_schema: Option<serde_json::Value>,
}

/// Hooks declaration in SKILL.md frontmatter.
/// Each key is a hook point name, value is a list of hook definitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HooksManifest {
    pub on_session_start: Option<Vec<HookManifest>>,
    pub on_session_end: Option<Vec<HookManifest>>,
    pub pre_turn: Option<Vec<HookManifest>>,
    pub post_turn: Option<Vec<HookManifest>>,
    pub pre_tool: Option<Vec<HookManifest>>,
    pub post_tool: Option<Vec<HookManifest>>,
    pub on_memory_store: Option<Vec<HookManifest>>,
    pub on_memory_recall: Option<Vec<HookManifest>>,
}

/// Single hook declaration in SKILL.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookManifest {
    pub name: String,
    pub script: String,
    pub priority: Option<i32>,
}

/// Parse a SKILL.md file content into (manifest, body).
///
/// The frontmatter is between the first pair of `---` markers.
/// Everything after the second `---` is the body (system prompt injection).
pub fn parse_skill_md(content: &str) -> anyhow::Result<(SkillManifest, String)> {
    let trimmed = content.trim();

    // Must start with ---
    if !trimmed.starts_with("---") {
        return Err(anyhow::anyhow!("SKILL.md must start with '---' frontmatter"));
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let end_pos = after_first.find("\n---")
        .or_else(|| after_first.find("\r\n---"))
        .ok_or_else(|| anyhow::anyhow!("Missing closing '---' in SKILL.md frontmatter"))?;

    let frontmatter = &after_first[..end_pos];
    let body_start = end_pos + 4; // skip "\n---"
    let body = after_first[body_start..].trim().to_string();

    let manifest: SkillManifest = serde_yaml::from_str(frontmatter)
        .map_err(|e| anyhow::anyhow!("Failed to parse SKILL.md frontmatter: {}", e))?;

    Ok((manifest, body))
}

/// Parse a permission string into PermissionLevel.
pub fn parse_permission(s: &str) -> PermissionLevel {
    match s.to_uppercase().as_str() {
        "L0" => PermissionLevel::L0,
        "L1" => PermissionLevel::L1,
        "L2" => PermissionLevel::L2,
        "L3" => PermissionLevel::L3,
        _ => PermissionLevel::L1,
    }
}

/// Parse an exposure string into ToolExposure.
pub fn parse_exposure(s: &str) -> aletheon_abi::tool::ToolExposure {
    match s.to_lowercase().as_str() {
        "direct" => aletheon_abi::tool::ToolExposure::Direct,
        "deferred" => aletheon_abi::tool::ToolExposure::Deferred,
        "directmodelonly" => aletheon_abi::tool::ToolExposure::DirectModelOnly,
        "hidden" => aletheon_abi::tool::ToolExposure::Hidden,
        _ => aletheon_abi::tool::ToolExposure::Direct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_skill_md() {
        let content = r#"---
name: git-workflow
version: 1.0.0
description: Git workflow automation
trigger: manual
keywords: [git, branch]
tools:
  - name: check_branch
    description: Check branch status
    script: scripts/check.sh
    permission: L0
hooks:
  pre_tool:
    - name: validate
      script: scripts/validate.sh
      priority: 10
---

When working with git, always use feature branches.
"#;
        let (manifest, body) = parse_skill_md(content).unwrap();
        assert_eq!(manifest.name, "git-workflow");
        assert_eq!(manifest.version, Some("1.0.0".into()));
        assert_eq!(manifest.description, "Git workflow automation");
        assert_eq!(manifest.trigger, Some("manual".into()));
        assert_eq!(manifest.keywords, Some(vec!["git".into(), "branch".into()]));
        assert!(manifest.tools.is_some());
        let tools = manifest.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "check_branch");
        assert_eq!(tools[0].permission, Some("L0".into()));
        assert!(manifest.hooks.is_some());
        let hooks = manifest.hooks.unwrap();
        assert!(hooks.pre_tool.is_some());
        assert_eq!(hooks.pre_tool.unwrap()[0].name, "validate");
        assert!(body.contains("feature branches"));
    }

    #[test]
    fn parse_minimal_skill_md() {
        let content = r#"---
name: minimal
description: A minimal skill
---

No tools or hooks.
"#;
        let (manifest, body) = parse_skill_md(content).unwrap();
        assert_eq!(manifest.name, "minimal");
        assert!(manifest.tools.is_none());
        assert!(manifest.hooks.is_none());
        assert!(body.contains("No tools"));
    }

    #[test]
    fn parse_missing_frontmatter() {
        let result = parse_skill_md("No frontmatter here");
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_closing_frontmatter() {
        let content = "---\nname: broken\ndescription: test";
        let result = parse_skill_md(content);
        assert!(result.is_err());
    }

    #[test]
    fn parse_permission_levels() {
        assert_eq!(parse_permission("L0"), PermissionLevel::L0);
        assert_eq!(parse_permission("L1"), PermissionLevel::L1);
        assert_eq!(parse_permission("L2"), PermissionLevel::L2);
        assert_eq!(parse_permission("L3"), PermissionLevel::L3);
        assert_eq!(parse_permission("unknown"), PermissionLevel::L1);
    }

    #[test]
    fn parse_exposure_levels() {
        assert_eq!(parse_exposure("direct"), aletheon_abi::tool::ToolExposure::Direct);
        assert_eq!(parse_exposure("deferred"), aletheon_abi::tool::ToolExposure::Deferred);
        assert_eq!(parse_exposure("hidden"), aletheon_abi::tool::ToolExposure::Hidden);
        assert_eq!(parse_exposure("unknown"), aletheon_abi::tool::ToolExposure::Direct);
    }

    #[test]
    fn parse_body_preserves_content() {
        let content = r#"---
name: test
description: test skill
---

## Instructions

Line 1
Line 2

### Details

More content here.
"#;
        let (_, body) = parse_skill_md(content).unwrap();
        assert!(body.contains("## Instructions"));
        assert!(body.contains("Line 1"));
        assert!(body.contains("### Details"));
    }
}
```

### Step 2: Add to skills/mod.rs

Add to `crates/aletheon-runtime/src/impl/skills/mod.rs`:

```rust
pub mod manifest;
```

### Step 3: Verify

```bash
cargo test -p aletheon-runtime -- manifest
```

Expected: All 7 manifest tests pass.

### Step 4: Commit

```bash
git add crates/aletheon-runtime/src/impl/skills/manifest.rs crates/aletheon-runtime/src/impl/skills/mod.rs
git commit -m "feat(runtime): add SKILL.md YAML frontmatter parser

Parses --- delimited YAML header into SkillManifest with tools,
hooks, trigger, keywords. Body becomes system prompt injection."
```

---

## Task 4: SkillPlugin + Registration

**Files:**
- Create: `crates/aletheon-runtime/src/impl/skills/plugin.rs`
- Modify: `crates/aletheon-runtime/src/impl/skills/mod.rs`

### Step 1: Create plugin.rs

```rust
// crates/aletheon-runtime/src/impl/skills/plugin.rs

//! Skill plugin types and registration logic.
//!
//! A SkillPlugin is a parsed skill directory with its manifest,
//! references, and script paths. `register_skill()` wires a plugin's
//! tools and hooks into the runtime.

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};

use aletheon_abi::tool::{PermissionLevel, ToolExposure};
use aletheon_body::impl::tools::ToolRegistry;
use aletheon_body::impl::tools::script_tool::ScriptTool;

use super::manifest::{
    parse_exposure, parse_permission, HooksManifest, SkillManifest, ToolManifest,
};

use crate::impl::hooks::registry::{HookRegistry, RegisteredHook};
use aletheon_abi::hook::HookPoint;

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

    let tools = manifest.tools.unwrap_or_default().into_iter().map(|t| {
        SkillToolDef {
            name: t.name,
            description: t.description,
            script: t.script,
            permission: t.permission.as_deref().map(parse_permission).unwrap_or(PermissionLevel::L1),
            exposure: t.exposure.as_deref().map(parse_exposure).unwrap_or(ToolExposure::Direct),
            input_schema: t.input_schema,
        }
    }).collect();

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
            let name = path.file_stem()
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

        tool_registry.register(Arc::new(tool));
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
    use super::*;
    use super::super::manifest::{parse_skill_md, HooksManifest, HookManifest};

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
            on_session_start: Some(vec![HookManifest { name: "s".into(), script: "s.sh".into(), priority: None }]),
            on_session_end: Some(vec![HookManifest { name: "e".into(), script: "e.sh".into(), priority: None }]),
            pre_turn: Some(vec![HookManifest { name: "pt".into(), script: "pt.sh".into(), priority: None }]),
            post_turn: Some(vec![HookManifest { name: "pot".into(), script: "pot.sh".into(), priority: None }]),
            pre_tool: Some(vec![HookManifest { name: "prt".into(), script: "prt.sh".into(), priority: None }]),
            post_tool: Some(vec![HookManifest { name: "pot2".into(), script: "pot2.sh".into(), priority: None }]),
            on_memory_store: Some(vec![HookManifest { name: "ms".into(), script: "ms.sh".into(), priority: None }]),
            on_memory_recall: Some(vec![HookManifest { name: "mr".into(), script: "mr.sh".into(), priority: None }]),
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
```

### Step 2: Update skills/mod.rs

Update `crates/aletheon-runtime/src/impl/skills/mod.rs` to:

```rust
pub mod loader;
pub mod inject;
pub mod manifest;
pub mod plugin;
```

### Step 3: Verify

```bash
cargo test -p aletheon-runtime -- plugin
```

Expected: All 5 plugin tests pass.

### Step 4: Commit

```bash
git add crates/aletheon-runtime/src/impl/skills/plugin.rs crates/aletheon-runtime/src/impl/skills/mod.rs
git commit -m "feat(runtime): add SkillPlugin type and register_skill()

SkillPlugin holds parsed skill with tools, hooks, references.
register_skill() wires ScriptTools into ToolRegistry and hooks
into HookRegistry."
```

---

## Task 5: HookRegistry

**Files:**
- Create: `crates/aletheon-runtime/src/impl/hooks/mod.rs`
- Create: `crates/aletheon-runtime/src/impl/hooks/registry.rs`
- Modify: `crates/aletheon-runtime/src/impl/mod.rs`

### Step 1: Create hooks/mod.rs

```rust
// crates/aletheon-runtime/src/impl/hooks/mod.rs

pub mod registry;
pub mod builtin;
```

### Step 2: Create hooks/registry.rs

```rust
// crates/aletheon-runtime/src/impl/hooks/registry.rs

//! Hook registry — registers and executes lifecycle hooks.
//!
//! Hooks are registered for specific HookPoints and executed in
//! priority order (lower number = earlier execution).

use std::collections::HashMap;
use std::path::PathBuf;

use tracing::warn;

use aletheon_abi::hook::{HookContext, HookPoint, HookResult};

/// A registered hook.
#[derive(Debug, Clone)]
pub struct RegisteredHook {
    /// Unique name (e.g. "git-workflow:validate").
    pub name: String,
    /// Origin: "skill:<name>" | "builtin" | "config".
    pub source: String,
    /// Path to executable script (None for builtin hooks).
    pub script_path: Option<PathBuf>,
    /// Which lifecycle point this hook targets.
    pub point: HookPoint,
    /// Execution priority (lower = earlier).
    pub priority: i32,
}

/// Registry of lifecycle hooks.
pub struct HookRegistry {
    hooks: HashMap<HookPoint, Vec<RegisteredHook>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    /// Register a hook. Hooks are kept sorted by priority.
    pub fn register(&mut self, hook: RegisteredHook) {
        let entry = self.hooks.entry(hook.point).or_default();
        entry.push(hook);
        entry.sort_by_key(|h| h.priority);
    }

    /// Execute all hooks for a given point.
    ///
    /// Returns the aggregate result:
    /// - First `Block` wins (short-circuits).
    /// - First `ModifyInput` wins (short-circuits).
    /// - All `Inject` results are merged.
    /// - `Continue` is returned if no hooks modify behavior.
    pub async fn execute(&self, ctx: &HookContext) -> HookResult {
        let hooks = match self.hooks.get(&ctx.point) {
            Some(h) => h,
            None => return HookResult::Continue,
        };

        let mut injections = Vec::new();

        for hook in hooks {
            let result = self.execute_single(hook, ctx).await;
            match result {
                HookResult::Continue => {}
                HookResult::ModifyInput(v) => return HookResult::ModifyInput(v),
                HookResult::Block { reason } => return HookResult::Block { reason },
                HookResult::Inject(s) => injections.push(s),
            }
        }

        if injections.is_empty() {
            HookResult::Continue
        } else {
            HookResult::Inject(injections.join("\n"))
        }
    }

    /// Get the number of registered hooks for a point.
    pub fn count(&self, point: &HookPoint) -> usize {
        self.hooks.get(point).map_or(0, |h| h.len())
    }

    /// Get total registered hooks across all points.
    pub fn total_count(&self) -> usize {
        self.hooks.values().map(|h| h.len()).sum()
    }

    /// Execute a single hook.
    async fn execute_single(&self, hook: &RegisteredHook, ctx: &HookContext) -> HookResult {
        let script = match hook.script_path {
            Some(ref s) => s,
            None => return HookResult::Continue,
        };

        let ctx_json = serde_json::to_string(ctx).unwrap_or_default();

        let child = tokio::process::Command::new(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                warn!(hook = %hook.name, error = %e, "Hook spawn failed");
                return HookResult::Continue;
            }
        };

        // Write context to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(ctx_json.as_bytes()).await;
        }

        match child.wait_with_output().await {
            Ok(output) => parse_hook_output(&output.stdout),
            Err(e) => {
                warn!(hook = %hook.name, error = %e, "Hook execution failed");
                HookResult::Continue
            }
        }
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse hook script stdout into HookResult.
pub fn parse_hook_output(stdout: &[u8]) -> HookResult {
    let text = String::from_utf8_lossy(stdout).trim().to_string();
    if text.is_empty() {
        return HookResult::Continue;
    }

    // Try JSON structured response
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        match value.get("action").and_then(|v| v.as_str()) {
            Some("block") => {
                let reason = value
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Blocked by hook")
                    .to_string();
                return HookResult::Block { reason };
            }
            Some("inject") => {
                let content = value
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return HookResult::Inject(content);
            }
            Some("modify_input") => {
                if let Some(input) = value.get("input") {
                    return HookResult::ModifyInput(input.clone());
                }
            }
            _ => {}
        }
    }

    // Plain text → inject
    HookResult::Inject(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_hook(name: &str, point: HookPoint, priority: i32) -> RegisteredHook {
        RegisteredHook {
            name: name.into(),
            source: "test".into(),
            script_path: None,
            point,
            priority,
        }
    }

    #[test]
    fn register_and_count() {
        let mut reg = HookRegistry::new();
        reg.register(make_hook("a", HookPoint::PreTool, 10));
        reg.register(make_hook("b", HookPoint::PreTool, 5));
        reg.register(make_hook("c", HookPoint::PostTool, 100));

        assert_eq!(reg.count(&HookPoint::PreTool), 2);
        assert_eq!(reg.count(&HookPoint::PostTool), 1);
        assert_eq!(reg.total_count(), 3);
    }

    #[test]
    fn priority_ordering() {
        let mut reg = HookRegistry::new();
        reg.register(make_hook("low", HookPoint::PreTool, 100));
        reg.register(make_hook("high", HookPoint::PreTool, 1));
        reg.register(make_hook("mid", HookPoint::PreTool, 50));

        let hooks = reg.hooks.get(&HookPoint::PreTool).unwrap();
        assert_eq!(hooks[0].name, "high");
        assert_eq!(hooks[1].name, "mid");
        assert_eq!(hooks[2].name, "low");
    }

    #[tokio::test]
    async fn execute_no_hooks_returns_continue() {
        let reg = HookRegistry::new();
        let ctx = HookContext {
            point: HookPoint::PreTool,
            session_id: "test".into(),
            turn_count: 0,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        assert!(matches!(reg.execute(&ctx).await, HookResult::Continue));
    }

    #[tokio::test]
    async fn execute_script_hook_inject() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("hook.sh");
        std::fs::write(&script, "#!/bin/bash\necho 'injected text'").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut reg = HookRegistry::new();
        reg.register(RegisteredHook {
            name: "test:inject".into(),
            source: "test".into(),
            script_path: Some(script),
            point: HookPoint::PostTurn,
            priority: 10,
        });

        let ctx = HookContext {
            point: HookPoint::PostTurn,
            session_id: "test".into(),
            turn_count: 1,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };

        match reg.execute(&ctx).await {
            HookResult::Inject(text) => assert_eq!(text, "injected text"),
            other => panic!("Expected Inject, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_script_hook_block() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("block.sh");
        std::fs::write(&script, r#"#!/bin/bash
echo '{"action":"block","reason":"not allowed'}""#).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut reg = HookRegistry::new();
        reg.register(RegisteredHook {
            name: "test:block".into(),
            source: "test".into(),
            script_path: Some(script),
            point: HookPoint::PreTool,
            priority: 10,
        });

        let ctx = HookContext {
            point: HookPoint::PreTool,
            session_id: "test".into(),
            turn_count: 0,
            tool_name: Some("bash_exec".into()),
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };

        match reg.execute(&ctx).await {
            HookResult::Block { reason } => assert!(reason.contains("not allowed")),
            other => panic!("Expected Block, got {:?}", other),
        }
    }

    #[test]
    fn parse_output_continue_on_empty() {
        assert!(matches!(parse_hook_output(b""), HookResult::Continue));
    }

    #[test]
    fn parse_output_inject_on_text() {
        match parse_hook_output(b"some context") {
            HookResult::Inject(s) => assert_eq!(s, "some context"),
            _ => panic!("Expected Inject"),
        }
    }

    #[test]
    fn parse_output_block_on_json() {
        let json = r#"{"action":"block","reason":"denied"}"#;
        match parse_hook_output(json.as_bytes()) {
            HookResult::Block { reason } => assert_eq!(reason, "denied"),
            _ => panic!("Expected Block"),
        }
    }

    #[test]
    fn parse_output_inject_on_json() {
        let json = r#"{"action":"inject","content":"extra info"}"#;
        match parse_hook_output(json.as_bytes()) {
            HookResult::Inject(s) => assert_eq!(s, "extra info"),
            _ => panic!("Expected Inject"),
        }
    }

    #[test]
    fn parse_output_modify_input_on_json() {
        let json = r#"{"action":"modify_input","input":{"key":"value"}}"#;
        match parse_hook_output(json.as_bytes()) {
            HookResult::ModifyInput(v) => assert_eq!(v["key"], "value"),
            _ => panic!("Expected ModifyInput"),
        }
    }
}
```

### Step 3: Add to mod.rs

Add to `crates/aletheon-runtime/src/impl/mod.rs`:

```rust
pub mod hooks;
```

### Step 4: Verify

```bash
cargo test -p aletheon-runtime -- hooks::registry
```

Expected: All 10 registry tests pass.

### Step 5: Commit

```bash
git add crates/aletheon-runtime/src/impl/hooks/
git commit -m "feat(runtime): add HookRegistry for lifecycle hooks

HookRegistry registers hooks for HookPoints with priority ordering.
execute() runs hooks in priority order, supports Continue/Block/
ModifyInput/Inject results. Script-based hook execution via stdin."
```

---

## Task 6: Builtin Hooks

**Files:**
- Create: `crates/aletheon-runtime/src/impl/hooks/builtin/mod.rs`
- Create: `crates/aletheon-runtime/src/impl/hooks/builtin/audit_hook.rs`

### Step 1: Create builtin/mod.rs

```rust
// crates/aletheon-runtime/src/impl/hooks/builtin/mod.rs

pub mod audit_hook;
```

### Step 2: Create builtin/audit_hook.rs

```rust
// crates/aletheon-runtime/src/impl/hooks/builtin/audit_hook.rs

//! Audit hook — logs all tool calls to the audit log.
//!
//! This hook registers for PostTool and logs tool name, input summary,
//! success/failure, and execution time.

use tracing::info;

use aletheon_abi::hook::HookPoint;
use crate::impl::hooks::registry::{HookRegistry, RegisteredHook};

/// Register the audit hook in the hook registry.
pub fn register_audit_hook(registry: &mut HookRegistry) {
    registry.register(RegisteredHook {
        name: "builtin:audit".into(),
        source: "builtin".into(),
        script_path: None,
        point: HookPoint::PostTool,
        priority: 1000, // Run last
    });
}

/// Log a tool call result. Called from handler.rs after PostTool hooks.
pub fn log_tool_call(
    tool_name: &str,
    is_error: bool,
    execution_time_ms: u64,
    content_len: usize,
) {
    info!(
        tool = %tool_name,
        error = is_error,
        ms = execution_time_ms,
        bytes = content_len,
        "Tool call completed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_audit_hook_adds_entry() {
        let mut reg = HookRegistry::new();
        register_audit_hook(&mut reg);
        assert_eq!(reg.count(&HookPoint::PostTool), 1);
    }
}
```

### Step 3: Verify

```bash
cargo test -p aletheon-runtime -- audit_hook
```

Expected: 1 test passes.

### Step 4: Commit

```bash
git add crates/aletheon-runtime/src/impl/hooks/builtin/
git commit -m "feat(runtime): add builtin audit hook for tool call logging"
```

---

## Task 7: SkillLoader Enhancement

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/skills/loader.rs`

### Step 1: Enhance loader.rs

Add multi-file skill directory support to the existing `SkillLoader`. The key changes:

1. `load_all()` now scans for subdirectories containing `SKILL.md`
2. Legacy single `.md` files still work (backward compatible)
3. New `loaded_plugins()` method returns `Vec<SkillPlugin>`

Add these methods and types to the existing `loader.rs`:

```rust
// Add to the top of loader.rs:
use super::manifest::parse_skill_md;
use super::plugin::{build_skill_plugin, SkillPlugin, SkillToolDef, SkillHookDef, TriggerType, ReferenceFile};

// Add to SkillLoader struct:
/// Parsed skill plugins (multi-file skills).
plugins: Vec<SkillPlugin>,

// Add to SkillLoader::new():
plugins: Vec::new(),

// Add new methods to SkillLoader impl:

/// Return a reference to the loaded skill plugins.
pub fn plugins(&self) -> &[SkillPlugin] {
    &self.plugins
}

/// Load all skills — both multi-file directories and legacy single .md files.
/// Enhanced load_all that also builds SkillPlugin instances.
pub fn load_all_enhanced(&mut self) -> usize {
    if !self.skills_dir.exists() {
        return 0;
    }

    let entries = match std::fs::read_dir(&self.skills_dir) {
        Ok(e) => e,
        Err(_) => return 0,
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
                        skills.push(LoadedSkill {
                            name: plugin.name.clone(),
                            description: plugin.description.clone(),
                            content: plugin.system_prompt.clone(),
                            source: "system".into(),
                        });
                        plugins.push(plugin);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, path = %path.display(), "Failed to parse skill directory");
                    }
                }
            }
        } else if path.extension().map_or(false, |ext| ext == "md") {
            // Legacy single .md file
            match Self::parse_skill_file(&path) {
                Ok(skill) => skills.push(skill),
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "Failed to parse skill file");
                }
            }
        }
    }

    let count = skills.len();
    self.cache = skills;
    self.plugins = plugins;
    count
}

/// Parse a skill directory containing SKILL.md.
fn parse_skill_dir(dir: &std::path::Path) -> anyhow::Result<SkillPlugin> {
    let skill_md = dir.join("SKILL.md");
    let raw = std::fs::read_to_string(&skill_md)?;
    let (manifest, body) = parse_skill_md(&raw)?;
    Ok(build_skill_plugin(manifest, body, dir.to_path_buf()))
}
```

### Step 2: Add tests to loader.rs

```rust
// Add to the existing tests module in loader.rs:

#[test]
fn load_skill_directory_with_manifest() {
    let dir = TempDir::new().unwrap();
    let skill_dir = dir.path().join("my-skill");
    std::fs::create_dir(&skill_dir).unwrap();
    std::fs::create_dir(skill_dir.join("scripts")).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: Test\n---\nBody content.\n",
    ).unwrap();

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
    std::fs::write(
        dir.path().join("legacy.md"),
        "# Legacy\nA legacy skill.\n",
    ).unwrap();
    // Directory skill
    let skill_dir = dir.path().join("modern");
    std::fs::create_dir(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: modern\ndescription: Modern skill\n---\nContent.\n",
    ).unwrap();

    let mut loader = SkillLoader::new(dir.path().to_path_buf());
    let count = loader.load_all_enhanced();
    assert_eq!(count, 2);
    assert_eq!(loader.plugins().len(), 1); // Only directory skills become plugins
}
```

### Step 3: Verify

```bash
cargo test -p aletheon-runtime -- loader
```

Expected: All loader tests pass (existing + new).

### Step 4: Commit

```bash
git add crates/aletheon-runtime/src/impl/skills/loader.rs
git commit -m "feat(runtime): enhance SkillLoader for multi-file skill directories

SkillLoader now scans for subdirectories containing SKILL.md in
addition to legacy single .md files. Plugins are built from parsed
manifests. Backward compatible with existing skills."
```

---

## Task 8: Embedded MCP Server

**Files:**
- Create: `crates/aletheon-runtime/src/impl/daemon/mcp_embedded.rs`
- Modify: `crates/aletheon-runtime/src/impl/daemon/mod.rs`

### Step 1: Create mcp_embedded.rs

```rust
// crates/aletheon-runtime/src/impl/daemon/mcp_embedded.rs

//! Embedded MCP server — exposes body tools via MCP protocol.
//!
//! The MCP server listens on a Unix socket and responds to
//! `initialize`, `tools/list`, `tools/call`, and `ping` methods.
//! Tools are dynamically sourced from the ToolRegistry.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tracing::{error, info, warn};

use aletheon_body::impl::tools::ToolRegistry;

/// Embedded MCP server that exposes body tools via MCP protocol.
pub struct McpEmbedded {
    tool_registry: Arc<ToolRegistry>,
    socket_path: PathBuf,
}

impl McpEmbedded {
    pub fn new(tool_registry: Arc<ToolRegistry>, socket_path: PathBuf) -> Self {
        Self {
            tool_registry,
            socket_path,
        }
    }

    /// Start the MCP server, listening on a Unix socket.
    pub async fn serve(&self) -> anyhow::Result<()> {
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path.display(), "MCP server listening");

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let registry = self.tool_registry.clone();
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, registry).await {
                            warn!(error = %e, "MCP connection error");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "MCP accept error");
                }
            }
        }
    }

    async fn handle_connection(
        stream: tokio::net::UnixStream,
        registry: Arc<ToolRegistry>,
    ) -> anyhow::Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }

            let request: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, "Invalid JSON-RPC request");
                    continue;
                }
            };

            let response = Self::handle_request(&request, &registry).await;
            let response_str = serde_json::to_string(&response)?;
            writer.write_all(response_str.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        Ok(())
    }

    async fn handle_request(request: &Value, registry: &Arc<ToolRegistry>) -> Value {
        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = request.get("id").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => Self::handle_initialize(id),
            "tools/list" => Self::handle_tools_list(id, registry),
            "tools/call" => Self::handle_tools_call(id, request, registry).await,
            "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("Method not found: {}", method)}
            }),
        }
    }

    fn handle_initialize(id: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "aletheon-embedded-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        })
    }

    fn handle_tools_list(id: Value, registry: &Arc<ToolRegistry>) -> Value {
        let tools: Vec<Value> = registry
            .definitions()
            .into_iter()
            .map(|def| {
                json!({
                    "name": def.name,
                    "description": def.description,
                    "inputSchema": def.input_schema,
                })
            })
            .collect();

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"tools": tools}
        })
    }

    async fn handle_tools_call(id: Value, request: &Value, registry: &Arc<ToolRegistry>) -> Value {
        let params = request.get("params").cloned().unwrap_or(json!({}));
        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let tool = match registry.get(tool_name) {
            Some(t) => t,
            None => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32602, "message": format!("Unknown tool: {}", tool_name)}
                });
            }
        };

        let ctx = aletheon_abi::tool::ToolContext {
            working_dir: std::env::current_dir().unwrap_or_default(),
            session_id: "mcp-session".into(),
        };

        let result = tool.execute(arguments, &ctx).await;
        let content_text = if result.is_error {
            format!("Error: {}", result.content)
        } else {
            result.content
        };

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": content_text}],
                "isError": result.is_error
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_initialize_returns_server_info() {
        let registry = Arc::new(ToolRegistry::new());
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt.block_on(McpEmbedded::handle_request(&request, &registry));

        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(response["result"]["serverInfo"]["name"], "aletheon-embedded-mcp");
    }

    #[test]
    fn handle_tools_list_returns_registry_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(aletheon_body::impl::tools::bash_exec::BashExecTool));
        let registry = Arc::new(reg);

        let request = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt.block_on(McpEmbedded::handle_request(&request, &registry));

        let tools = response["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "bash_exec"));
    }

    #[test]
    fn handle_ping() {
        let registry = Arc::new(ToolRegistry::new());
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "ping"});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt.block_on(McpEmbedded::handle_request(&request, &registry));
        assert!(response["result"].is_object());
    }

    #[test]
    fn handle_unknown_method() {
        let registry = Arc::new(ToolRegistry::new());
        let request = json!({"jsonrpc": "2.0", "id": 4, "method": "unknown"});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt.block_on(McpEmbedded::handle_request(&request, &registry));
        assert_eq!(response["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn handle_tools_call_unknown_tool() {
        let registry = Arc::new(ToolRegistry::new());
        let request = json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "nonexistent", "arguments": {}}
        });
        let response = McpEmbedded::handle_request(&request, &registry).await;
        assert_eq!(response["error"]["code"], -32602);
    }
}
```

### Step 2: Add to daemon/mod.rs

Add to `crates/aletheon-runtime/src/impl/daemon/mod.rs` after `pub mod cache_shape;`:

```rust
pub mod mcp_embedded;
```

### Step 3: Verify

```bash
cargo test -p aletheon-runtime -- mcp_embedded
```

Expected: All 5 MCP tests pass.

### Step 4: Commit

```bash
git add crates/aletheon-runtime/src/impl/daemon/mcp_embedded.rs crates/aletheon-runtime/src/impl/daemon/mod.rs
git commit -m "feat(runtime): add embedded MCP server

MCP server listens on Unix socket, dynamically exposes ToolRegistry
tools via tools/list and tools/call. Supports MCP protocol version
2024-11-05."
```

---

## Task 9: Handler Integration

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

### Step 1: Add fields to RequestHandler

Add these fields to the `RequestHandler` struct:

```rust
/// Lifecycle hook registry.
hook_registry: Arc<HookRegistry>,
/// Embedded MCP server (optional, spawned in background if enabled).
mcp_socket_path: Option<PathBuf>,
```

### Step 2: Initialize hooks and MCP in RequestHandler::new()

After the existing skill loader initialization, add:

```rust
// Create hook registry
let mut hook_registry = HookRegistry::new();

// Register builtin hooks
crate::impl::hooks::builtin::audit_hook::register_audit_hook(&mut hook_registry);

// Register skill hooks from loaded plugins
for plugin in skill_loader.plugins() {
    crate::impl::skills::plugin::register_skill(
        plugin,
        &mut tools,  // existing ToolRegistry
        &mut hook_registry,
    );
}

let hook_registry = Arc::new(hook_registry);

// Initialize embedded MCP server (disabled by default)
let mcp_socket_path = std::env::var("ALETHEON_MCP_SOCKET")
    .ok()
    .map(PathBuf::from);

if let Some(ref socket) = mcp_socket_path {
    let mcp = McpEmbedded::new(Arc::new(/* tool_registry clone */), socket.clone());
    tokio::spawn(async move {
        if let Err(e) = mcp.serve().await {
            error!(error = %e, "MCP server failed");
        }
    });
    info!(socket = %socket.display(), "Embedded MCP server started");
}
```

### Step 3: Add hook execution points in handle("chat")

In the `handle("chat")` method, add hook execution at these points:

```rust
// --- PreTurn hooks ---
{
    let ctx = HookContext {
        point: HookPoint::PreTurn,
        session_id: session_id.clone(),
        turn_count: self.session_manager.lock().await.turn_count(),
        tool_name: None,
        tool_input: None,
        tool_result: None,
        message: Some(message.to_string()),
        metadata: HashMap::new(),
    };
    match self.hook_registry.execute(&ctx).await {
        HookResult::Block { reason } => {
            warn!(reason = %reason, "PreTurn hook blocked");
            return json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32015, "message": format!("Blocked by hook: {}", reason)}
            });
        }
        HookResult::Inject(text) => {
            effective_message.push_str(&text);
            effective_message.push('\n');
        }
        _ => {}
    }
}
```

For tool calls (inside the tool call loop):

```rust
// --- PreTool hooks ---
let pre_ctx = HookContext {
    point: HookPoint::PreTool,
    session_id: session_id.clone(),
    turn_count,
    tool_name: Some(tool_name.to_string()),
    tool_input: Some(tool_input.clone()),
    tool_result: None,
    message: None,
    metadata: HashMap::new(),
};
match self.hook_registry.execute(&pre_ctx).await {
    HookResult::Block { reason } => {
        // Skip this tool call
    }
    HookResult::ModifyInput(new_input) => {
        tool_input = new_input;
    }
    _ => {}
}

// ... execute tool ...

// --- PostTool hooks ---
let post_ctx = HookContext {
    point: HookPoint::PostTool,
    session_id: session_id.clone(),
    turn_count,
    tool_name: Some(tool_name.to_string()),
    tool_input: None,
    tool_result: Some(HookToolResult {
        content: result.content.clone(),
        is_error: result.is_error,
        execution_time_ms: result.metadata.execution_time_ms,
    }),
    message: None,
    metadata: HashMap::new(),
};
self.hook_registry.execute(&post_ctx).await;
```

After LLM response:

```rust
// --- PostTurn hooks ---
{
    let ctx = HookContext {
        point: HookPoint::PostTurn,
        session_id: session_id.clone(),
        turn_count: self.session_manager.lock().await.turn_count(),
        tool_name: None,
        tool_input: None,
        tool_result: None,
        message: None,
        metadata: HashMap::new(),
    };
    self.hook_registry.execute(&ctx).await;
}
```

### Step 4: Verify

```bash
cargo build -p aletheon-runtime
cargo test -p aletheon-runtime
```

Expected: All existing tests still pass. New hook integration points are wired.

### Step 5: Commit

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(runtime): integrate hooks and MCP into request handler

PreTurn/PostTurn hooks around chat processing. PreTool/PostTool
hooks around tool execution. HookRegistry initialized at startup
with builtin audit hook and skill hooks. MCP server optionally
spawned via ALETHEON_MCP_SOCKET env var."
```

---

## Task 10: End-to-End Verification

### Step 1: Full test suite

```bash
cd /home/aurobear/Bear-ws/work/aletheon
cargo test --workspace
```

Expected: All tests pass across all crates.

### Step 2: Build check

```bash
cargo build --workspace
```

Expected: Clean build with no warnings related to new code.

### Step 3: Create example skill

Create a test skill directory to verify the full pipeline:

```bash
mkdir -p ~/.aletheon/skills/example-skill/scripts
```

Create `~/.aletheon/skills/example-skill/SKILL.md`:

```markdown
---
name: example-skill
version: 1.0.0
description: Example skill for testing the plugin system
trigger: manual
keywords: [example, test]
tools:
  - name: echo_tool
    description: Echo input back
    script: scripts/echo.sh
    permission: L0
hooks:
  post_tool:
    - name: log_hook
      script: scripts/log.sh
      priority: 50
---

This is an example skill that demonstrates the plugin system.
```

Create `~/.aletheon/skills/example-skill/scripts/echo.sh`:

```bash
#!/bin/bash
cat /dev/stdin
```

Create `~/.aletheon/skills/example-skill/scripts/log.sh`:

```bash
#!/bin/bash
echo '{"action":"inject","content":"[example-skill] Tool was called"}'
```

```bash
chmod +x ~/.aletheon/skills/example-skill/scripts/*.sh
```

### Step 4: Commit plan document

```bash
git add docs/plans/2026-06-14-skill-hook-mcp-plan.md
git commit -m "docs: add implementation plan for skill/hook/MCP integration"
```

---

## Spec Coverage Checklist

| Spec Requirement | Task |
|---|---|
| SKILL.md multi-file format | Task 3 (manifest) + Task 4 (plugin) |
| ToolDef parsing | Task 3 + Task 4 |
| HookDef parsing | Task 3 + Task 4 |
| ScriptTool (Tool trait) | Task 2 |
| SkillPlugin type | Task 4 |
| register_skill() | Task 4 |
| HookPoint enum | Task 1 |
| HookContext/HookResult | Task 1 |
| HookRegistry | Task 5 |
| Builtin hooks | Task 6 |
| SkillLoader enhancement | Task 7 |
| Embedded MCP server | Task 8 |
| Handler integration | Task 9 |
| Backward compatibility | Task 7 (legacy .md files) |
| MCP config (env var) | Task 8 + Task 9 |
