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

use base::tool::{
    PermissionLevel, Tool, ToolContext, ToolExposure, ToolResult, ToolResultMeta,
};

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
            "t".into(),
            "d".into(),
            PathBuf::from("/tmp/t.sh"),
            PermissionLevel::L1,
        )
        .with_schema(schema.clone());
        assert_eq!(tool.input_schema(), schema);
    }

    #[test]
    fn script_tool_with_exposure() {
        let tool = ScriptTool::new(
            "t".into(),
            "d".into(),
            PathBuf::from("/tmp/t.sh"),
            PermissionLevel::L1,
        )
        .with_exposure(ToolExposure::Deferred);
        assert_eq!(tool.exposure(), ToolExposure::Deferred);
    }

    #[tokio::test]
    async fn script_tool_execute_missing_script() {
        let tool = ScriptTool::new(
            "t".into(),
            "d".into(),
            PathBuf::from("/nonexistent/script.sh"),
            PermissionLevel::L1,
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
            "hello".into(),
            "says hello".into(),
            script_path,
            PermissionLevel::L0,
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
            "fail".into(),
            "fails".into(),
            script_path,
            PermissionLevel::L1,
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
        std::fs::write(
            &script_path,
            "#!/bin/bash\necho '{\"content\": \"structured\"}'",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let tool = ScriptTool::new(
            "json_out".into(),
            "outputs json".into(),
            script_path,
            PermissionLevel::L0,
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
            "t".into(),
            "d".into(),
            PathBuf::from("/tmp/t.sh"),
            PermissionLevel::L1,
        );
        let cloned = tool.boxed_clone();
        assert_eq!(cloned.name(), "t");
    }
}
