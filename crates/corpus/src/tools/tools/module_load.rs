//! Tool for loading and unloading Linux kernel modules.

use async_trait::async_trait;
use fabric::Timer;
use serde_json::{json, Value};
use tracing::info;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct ModuleLoadTool;

#[async_trait]
impl Tool for ModuleLoadTool {
    fn name(&self) -> &str {
        "module_load"
    }

    fn description(&self) -> &str {
        "Load a Linux kernel module (.ko file). \
         Input: ko_path (path to .ko file), args (optional module parameters). \
         Requires root or CAP_SYS_MODULE."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ko_path": {
                    "type": "string",
                    "description": "Path to the .ko kernel module file"
                },
                "args": {
                    "type": "string",
                    "description": "Optional module parameters (space-separated key=value pairs)"
                },
                "action": {
                    "type": "string",
                    "enum": ["load", "unload", "reload"],
                    "description": "Action to perform (default: load)"
                }
            },
            "required": ["ko_path"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L3 // destructive: kernel module loading
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ModuleLoadTool)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let ko_path = match input["ko_path"].as_str() {
            Some(p) => p,
            None => {
                return ToolResult {
                    content: "Missing required parameter: ko_path".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        let action = input["action"].as_str().unwrap_or("load");
        let args = input["args"].as_str().unwrap_or("");

        match action {
            "load" => {
                // Check file exists
                if !std::path::Path::new(ko_path).exists() {
                    return ToolResult {
                        content: format!("Module file not found: {}", ko_path),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    };
                }

                info!("Loading kernel module: {}", ko_path);

                // insmod {ko_path} {args}
                let mut cmd = tokio::process::Command::new("insmod");
                cmd.arg(ko_path);
                if !args.is_empty() {
                    for arg in args.split_whitespace() {
                        cmd.arg(arg);
                    }
                }

                let output = cmd.output().await;
                match output {
                    Ok(result) => {
                        if result.status.success() {
                            // Verify with lsmod
                            let module_name = extract_module_name(ko_path);
                            let lsmod_check = tokio::process::Command::new("lsmod").output().await;

                            let loaded = lsmod_check
                                .map(|o| String::from_utf8_lossy(&o.stdout).contains(&module_name))
                                .unwrap_or(false);

                            ToolResult {
                                content: format!(
                                    "Kernel module loaded successfully.\n\
                                     Path: {}\n\
                                     Module: {}\n\
                                     Verified in lsmod: {}",
                                    ko_path, module_name, loaded
                                ),
                                is_error: false,
                                metadata: ToolResultMeta {
                                    execution_time_ms: ctx
                                        .clock
                                        .mono_now()
                                        .0
                                        .saturating_sub(start.0),
                                    truncated: false,
                                },
                            }
                        } else {
                            let stderr = String::from_utf8_lossy(&result.stderr);
                            ToolResult {
                                content: format!("insmod failed:\n{}", stderr),
                                is_error: true,
                                metadata: ToolResultMeta {
                                    execution_time_ms: ctx
                                        .clock
                                        .mono_now()
                                        .0
                                        .saturating_sub(start.0),
                                    truncated: false,
                                },
                            }
                        }
                    }
                    Err(e) => ToolResult {
                        content: format!("Failed to run insmod: {}", e),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    },
                }
            }
            "unload" => {
                let module_name = input["module_name"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| extract_module_name(ko_path));

                info!("Unloading kernel module: {}", module_name);

                let output = tokio::process::Command::new("rmmod")
                    .arg(&module_name)
                    .output()
                    .await;

                match output {
                    Ok(result) => {
                        if result.status.success() {
                            ToolResult {
                                content: format!("Module {} unloaded successfully.", module_name),
                                is_error: false,
                                metadata: ToolResultMeta {
                                    execution_time_ms: ctx
                                        .clock
                                        .mono_now()
                                        .0
                                        .saturating_sub(start.0),
                                    truncated: false,
                                },
                            }
                        } else {
                            let stderr = String::from_utf8_lossy(&result.stderr);
                            ToolResult {
                                content: format!("rmmod failed:\n{}", stderr),
                                is_error: true,
                                metadata: ToolResultMeta {
                                    execution_time_ms: ctx
                                        .clock
                                        .mono_now()
                                        .0
                                        .saturating_sub(start.0),
                                    truncated: false,
                                },
                            }
                        }
                    }
                    Err(e) => ToolResult {
                        content: format!("Failed to run rmmod: {}", e),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    },
                }
            }
            "reload" => {
                // Unload then load
                let module_name = extract_module_name(ko_path);
                let _ = tokio::process::Command::new("rmmod")
                    .arg(&module_name)
                    .output()
                    .await;

                // Small delay
                aletheon_kernel::chronos::SystemTimer
                    .sleep(std::time::Duration::from_millis(100))
                    .await;

                // Re-execute as load
                let load_input = json!({
                    "ko_path": ko_path,
                    "args": args,
                    "action": "load"
                });
                return self.execute(load_input, ctx).await;
            }
            _ => {
                return ToolResult {
                    content: format!("Unknown action: {}. Use load, unload, or reload.", action),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        }
    }
}

/// Extract module name from .ko file path (strip path and .ko extension).
fn extract_module_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_module_name() {
        assert_eq!(extract_module_name("/path/to/my_module.ko"), "my_module");
        assert_eq!(extract_module_name("simple.ko"), "simple");
        assert_eq!(extract_module_name("/deep/path/to/module.ko"), "module");
    }

    #[test]
    fn test_module_load_schema() {
        let tool = ModuleLoadTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["ko_path"].is_object());
        assert!(schema["properties"]["action"].is_object());
    }

    #[test]
    fn test_module_load_permission() {
        let tool = ModuleLoadTool;
        assert_eq!(tool.permission_level(), PermissionLevel::L3);
    }

    #[tokio::test]
    async fn test_module_load_missing_file() {
        let tool = ModuleLoadTool;
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let result = tool
            .execute(json!({"ko_path": "/nonexistent.ko"}), &ctx)
            .await;
        assert!(result.is_error);
    }
}
