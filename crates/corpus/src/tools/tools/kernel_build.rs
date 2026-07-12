//! Tool for full Linux kernel compilation.
//!
//! Supports: clone, config, build, install actions.
//! All destructive actions (install) require explicit user confirmation.

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct KernelBuildTool;

#[async_trait]
impl Tool for KernelBuildTool {
    fn name(&self) -> &str {
        "kernel_build"
    }

    fn description(&self) -> &str {
        "Build a Linux kernel from source. Actions: \
         clone (git clone kernel source), \
         config (prepare .config from running kernel), \
         build (compile bzImage and modules), \
         install (install kernel and update bootloader). \
         WARNING: install action modifies boot configuration."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["clone", "config", "build", "install"],
                    "description": "Build action to perform"
                },
                "source_dir": {
                    "type": "string",
                    "description": "Kernel source directory (default: /usr/src/linux)"
                },
                "repo_url": {
                    "type": "string",
                    "description": "Git repo URL for clone action (default: kernel.org)"
                },
                "branch": {
                    "type": "string",
                    "description": "Git branch/tag for clone action (default: master)"
                },
                "jobs": {
                    "type": "integer",
                    "description": "Parallel build jobs (default: nproc)"
                }
            },
            "required": ["action"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L3 // destructive: kernel install modifies bootloader
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(KernelBuildTool)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let action = match input["action"].as_str() {
            Some(a) => a,
            None => {
                return ToolResult {
                    content: "Missing required parameter: action".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        let source_dir = input["source_dir"]
            .as_str()
            .unwrap_or("/usr/src/linux")
            .to_string();

        match action {
            "clone" => self.action_clone(&input, &source_dir, &*ctx.clock, start).await,
            "config" => self.action_config(&source_dir, &*ctx.clock, start).await,
            "build" => self.action_build(&input, &source_dir, &*ctx.clock, start).await,
            "install" => self.action_install(&source_dir, &*ctx.clock, start).await,
            _ => ToolResult {
                content: format!(
                    "Unknown action: {}. Valid actions: clone, config, build, install",
                    action
                ),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}

impl KernelBuildTool {
    async fn action_clone(&self, input: &Value, source_dir: &str, clock: &dyn fabric::Clock, start: fabric::MonoTime) -> ToolResult {
        let repo_url = input["repo_url"]
            .as_str()
            .unwrap_or("https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git");

        let branch = input["branch"].as_str().unwrap_or("master");

        info!(
            "Cloning kernel source from {} (branch: {})",
            repo_url, branch
        );

        // Check if directory already exists
        if std::path::Path::new(source_dir).exists() {
            return ToolResult {
                content: format!(
                    "Directory already exists: {}. Remove it first or use a different source_dir.",
                    source_dir
                ),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            };
        }

        let output = tokio::process::Command::new("git")
            .args([
                "clone", "--depth", "1", "--branch", branch, repo_url, source_dir,
            ])
            .output()
            .await;

        match output {
            Ok(result) => {
                if result.status.success() {
                    ToolResult {
                        content: format!(
                            "Kernel source cloned successfully.\n\
                             Repository: {}\n\
                             Branch: {}\n\
                             Directory: {}",
                            repo_url, branch, source_dir
                        ),
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    ToolResult {
                        content: format!("git clone failed:\n{}", stderr),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    }
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to run git: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }

    async fn action_config(&self, source_dir: &str, clock: &dyn fabric::Clock, start: fabric::MonoTime) -> ToolResult {
        info!("Preparing kernel config from running kernel");

        // Step 1: Copy running kernel config
        let copy_result = tokio::process::Command::new("cp")
            .args([
                &format!("/boot/config-{}", get_running_kernel_version()),
                &format!("{}/.config", source_dir),
            ])
            .output()
            .await;

        if let Err(e) = copy_result {
            return ToolResult {
                content: format!("Failed to copy kernel config: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            };
        }

        // Step 2: Run make olddefconfig
        let output = tokio::process::Command::new("make")
            .args(["-C", source_dir, "olddefconfig"])
            .output()
            .await;

        match output {
            Ok(result) => {
                if result.status.success() {
                    ToolResult {
                        content: format!(
                            "Kernel config prepared successfully.\n\
                             Source: {}\n\
                             Method: olddefconfig (based on running kernel {})",
                            source_dir,
                            get_running_kernel_version()
                        ),
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    ToolResult {
                        content: format!("make olddefconfig failed:\n{}", stderr),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    }
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to run make: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }

    async fn action_build(&self, input: &Value, source_dir: &str, clock: &dyn fabric::Clock, start: fabric::MonoTime) -> ToolResult {
        let jobs = input["jobs"]
            .as_u64()
            .map(|j| j.to_string())
            .unwrap_or_else(num_cpus);

        info!("Building kernel with {} jobs", jobs);

        let output = tokio::process::Command::new("make")
            .args([
                "-C",
                source_dir,
                &format!("-j{}", jobs),
                "bzImage",
                "modules",
            ])
            .output()
            .await;

        match output {
            Ok(result) => {
                if result.status.success() {
                    // Find the built kernel image
                    let image_path = format!("{}/arch/x86/boot/bzImage", source_dir);
                    let image_exists = std::path::Path::new(&image_path).exists();

                    ToolResult {
                        content: format!(
                            "Kernel built successfully.\n\
                             Source: {}\n\
                             Jobs: {}\n\
                             Image: {} ({})\n\
                             Build time: {}s",
                            source_dir,
                            jobs,
                            image_path,
                            if image_exists { "found" } else { "not found" },
                            clock.mono_now().0.saturating_sub(start.0) / 1000
                        ),
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    // Truncate output for readability
                    let stderr_tail: String = stderr
                        .chars()
                        .rev()
                        .take(2000)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    ToolResult {
                        content: format!(
                            "Kernel build failed.\n\
                             Last errors:\n{}",
                            stderr_tail
                        ),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                            truncated: true,
                        },
                    }
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to run make: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }

    async fn action_install(&self, source_dir: &str, clock: &dyn fabric::Clock, start: fabric::MonoTime) -> ToolResult {
        info!("Installing kernel from {}", source_dir);

        // Step 1: make modules_install
        let modules_result = tokio::process::Command::new("make")
            .args(["-C", source_dir, "modules_install"])
            .output()
            .await;

        match modules_result {
            Ok(r) if !r.status.success() => {
                return ToolResult {
                    content: format!(
                        "make modules_install failed:\n{}",
                        String::from_utf8_lossy(&r.stderr)
                    ),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
            Err(e) => {
                return ToolResult {
                    content: format!("Failed to run make modules_install: {}", e),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
            _ => {}
        }

        // Step 2: make install
        let install_result = tokio::process::Command::new("make")
            .args(["-C", source_dir, "install"])
            .output()
            .await;

        match install_result {
            Ok(r) if !r.status.success() => {
                return ToolResult {
                    content: format!(
                        "make install failed:\n{}",
                        String::from_utf8_lossy(&r.stderr)
                    ),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
            Err(e) => {
                return ToolResult {
                    content: format!("Failed to run make install: {}", e),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
            _ => {}
        }

        // Step 3: update-grub (if available)
        let grub_result = tokio::process::Command::new("update-grub").output().await;

        let grub_msg = match grub_result {
            Ok(r) if r.status.success() => "GRUB updated successfully.".to_string(),
            Ok(r) => format!(
                "update-grub warning: {}",
                String::from_utf8_lossy(&r.stderr)
            ),
            Err(_) => "update-grub not found (may need manual bootloader update)".to_string(),
        };

        ToolResult {
            content: format!(
                "Kernel installed successfully.\n\
                 Source: {}\n\
                 IMPORTANT: Reboot to use the new kernel.\n\
                 Old kernel is preserved in /boot for rollback.\n\
                 {}",
                source_dir, grub_msg
            ),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
            },
        }
    }
}

fn get_running_kernel_version() -> String {
    std::fs::read_to_string("/proc/version")
        .ok()
        .and_then(|v| v.split_whitespace().nth(2).map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn num_cpus() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .map(|c| {
            c.lines()
                .filter(|l| l.starts_with("processor"))
                .count()
                .to_string()
        })
        .unwrap_or_else(|| "1".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kernel_build_schema() {
        let tool = KernelBuildTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["source_dir"].is_object());
    }

    #[test]
    fn test_kernel_build_permission() {
        let tool = KernelBuildTool;
        assert_eq!(tool.permission_level(), PermissionLevel::L3);
    }

    #[test]
    fn test_get_running_kernel_version() {
        let version = get_running_kernel_version();
        assert_ne!(version, "unknown");
    }

    #[test]
    fn test_num_cpus() {
        let cpus = num_cpus();
        let n: u32 = cpus.parse().unwrap();
        assert!(n >= 1);
    }

    #[tokio::test]
    async fn test_kernel_build_invalid_action() {
        let tool = KernelBuildTool;
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let result = tool.execute(json!({"action": "invalid"}), &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_kernel_build_clone_existing_dir() {
        let tool = KernelBuildTool;
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        // /tmp exists, so clone should fail
        let result = tool
            .execute(json!({"action": "clone", "source_dir": "/tmp"}), &ctx)
            .await;
        assert!(result.is_error);
    }
}
