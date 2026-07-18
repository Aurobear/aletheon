//! Tool for compiling Linux kernel modules (.ko files).

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use super::{
    PermissionLevel, Tool, ToolContext, ToolExecutionDescriptor, ToolResult, ToolResultMeta,
};

pub struct ModuleBuildTool;

#[async_trait]
impl Tool for ModuleBuildTool {
    fn name(&self) -> &str {
        "module_build"
    }

    fn description(&self) -> &str {
        "Compile a Linux kernel module from source. \
         Input: source_dir (directory with Makefile and .c files), \
         kernel_version (optional, auto-detected). \
         Requires kernel headers installed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source_dir": {
                    "type": "string",
                    "description": "Directory containing the kernel module source and Makefile"
                },
                "kernel_version": {
                    "type": "string",
                    "description": "Target kernel version (optional, auto-detected from uname)"
                }
            },
            "required": ["source_dir"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L2 // system-level: compiling kernel code
    }

    fn execution_descriptor(&self) -> Option<ToolExecutionDescriptor> {
        Some(ToolExecutionDescriptor::ModuleBuild)
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ModuleBuildTool)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let source_dir = match input["source_dir"].as_str() {
            Some(p) => p,
            None => {
                return ToolResult {
                    content: "Missing required parameter: source_dir".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        // Auto-detect kernel version if not provided
        let kernel_version = match input["kernel_version"].as_str() {
            Some(v) => v.to_string(),
            None => {
                match tokio::process::Command::new("uname")
                    .arg("-r")
                    .output()
                    .await
                {
                    Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
                    Err(e) => {
                        return ToolResult {
                            content: format!("Failed to detect kernel version: {}", e),
                            is_error: true,
                            metadata: ToolResultMeta {
                                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                                truncated: false,
                            },
                        };
                    }
                }
            }
        };

        // Check kernel headers exist
        let headers_path = format!("/lib/modules/{}/build", kernel_version);
        if !std::path::Path::new(&headers_path).exists() {
            return ToolResult {
                content: format!(
                    "Kernel headers not found at {}. Install linux-headers-{}.",
                    headers_path, kernel_version
                ),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            };
        }

        // Check source directory has Makefile
        let makefile = format!("{}/Makefile", source_dir);
        if !std::path::Path::new(&makefile).exists() {
            return ToolResult {
                content: format!("No Makefile found in {}", source_dir),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            };
        }

        info!(
            "Building kernel module in {} for kernel {}",
            source_dir, kernel_version
        );

        // Run: make -C /lib/modules/{kver}/build M={source_dir} modules
        let output = tokio::process::Command::new("make")
            .args(["-C", &headers_path, &format!("M={}", source_dir), "modules"])
            .output()
            .await;

        match output {
            Ok(result) => {
                if result.status.success() {
                    // Find compiled .ko files
                    let ko_files = find_ko_files(source_dir).await;

                    ToolResult {
                        content: format!(
                            "Kernel module compiled successfully.\n\
                             Kernel: {}\n\
                             Source: {}\n\
                             Output files: {}\n\
                             Build time: {}ms",
                            kernel_version,
                            source_dir,
                            ko_files.join(", "),
                            ctx.clock.mono_now().0.saturating_sub(start.0)
                        ),
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    let stdout = String::from_utf8_lossy(&result.stdout);
                    ToolResult {
                        content: format!(
                            "Kernel module build failed:\n\
                             stdout:\n{}\n\
                             stderr:\n{}",
                            stdout, stderr
                        ),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    }
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to run make: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}

/// Find all .ko files in a directory tree.
async fn find_ko_files(dir: &str) -> Vec<String> {
    let mut ko_files = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().map(|e| e == "ko").unwrap_or(false) {
                ko_files.push(path.display().to_string());
            }
        }
    }
    ko_files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_build_schema() {
        let tool = ModuleBuildTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["source_dir"].is_object());
    }

    #[test]
    fn test_module_build_permission() {
        let tool = ModuleBuildTool;
        assert_eq!(tool.permission_level(), PermissionLevel::L2);
    }

    #[tokio::test]
    async fn test_module_build_missing_dir() {
        let tool = ModuleBuildTool;
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let result = tool
            .execute(json!({"source_dir": "/nonexistent"}), &ctx)
            .await;
        assert!(result.is_error);
    }
}
