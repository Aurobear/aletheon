//! Tool for compiling eBPF programs from C source to BPF bytecode.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use tracing::info;

use super::{
    PermissionLevel, Tool, ToolContext, ToolExecutionDescriptor, ToolResult, ToolResultMeta,
};

pub struct EbpfCompileTool;

#[async_trait]
impl Tool for EbpfCompileTool {
    fn name(&self) -> &str {
        "ebpf_compile"
    }

    fn description(&self) -> &str {
        "Compile an eBPF program from C source to BPF object file. \
         Input: source_path (path to .c file), output_path (optional, defaults to same name .o). \
         Requires clang and llvm installed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source_path": {
                    "type": "string",
                    "description": "Path to the eBPF C source file"
                },
                "output_path": {
                    "type": "string",
                    "description": "Output path for the .o file (optional)"
                },
                "target_arch": {
                    "type": "string",
                    "description": "Target architecture: bpf (default), x86, arm64",
                    "enum": ["bpf", "x86", "arm64"]
                }
            },
            "required": ["source_path"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L2 // system-level: compiling kernel code
    }

    fn execution_descriptor(&self) -> Option<ToolExecutionDescriptor> {
        Some(ToolExecutionDescriptor::EbpfCompile)
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(EbpfCompileTool)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let source_path = match input["source_path"].as_str() {
            Some(p) => p,
            None => {
                return ToolResult {
                    content: "Missing required parameter: source_path".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };

        let source = Path::new(source_path);
        if !source.exists() {
            return ToolResult {
                content: format!("Source file not found: {}", source_path),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            };
        }

        let output_path = input["output_path"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| source.with_extension("o").to_string_lossy().to_string());

        let target_arch = input["target_arch"].as_str().unwrap_or("bpf");

        // Check clang is available
        let clang_check = tokio::process::Command::new("which")
            .arg("clang")
            .output()
            .await;

        match clang_check {
            Ok(output) if !output.status.success() => {
                return ToolResult {
                    content: "clang not found. Install clang and llvm to compile eBPF programs."
                        .to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
            Err(e) => {
                return ToolResult {
                    content: format!("Failed to check for clang: {}", e),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
            _ => {}
        }

        // Compile: clang -target bpf -O2 -g -c source.c -o output.o
        info!("Compiling eBPF: {} -> {}", source_path, output_path);

        let output = tokio::process::Command::new("clang")
            .args([
                "-target",
                target_arch,
                "-O2",
                "-g",
                "-c",
                source_path,
                "-o",
                &output_path,
            ])
            .output()
            .await;

        match output {
            Ok(result) => {
                if result.status.success() {
                    // Verify the output file exists and has BPF magic
                    let verification = verify_bpf_object(&output_path).await;

                    ToolResult {
                        content: format!(
                            "eBPF program compiled successfully.\n\
                             Output: {}\n\
                             Target: {}\n\
                             Size: {} bytes\n\
                             BPF magic: {}",
                            output_path,
                            target_arch,
                            std::fs::metadata(&output_path)
                                .map(|m| m.len())
                                .unwrap_or(0),
                            verification
                        ),
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                            patch_delta: None,
                        },
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    ToolResult {
                        content: format!("eBPF compilation failed:\n{}", stderr),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                            patch_delta: None,
                        },
                    }
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to run clang: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

/// Verify a compiled BPF object file has the correct ELF magic and BPF target.
async fn verify_bpf_object(path: &str) -> String {
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            if bytes.len() < 16 {
                return "FAIL (file too small)".to_string();
            }
            // Check ELF magic: 7f 45 4c 46
            if bytes[0] != 0x7f || bytes[1] != 0x45 || bytes[2] != 0x4c || bytes[3] != 0x46 {
                return "FAIL (not ELF)".to_string();
            }
            // Check e_machine for BPF (247 = EM_BPF)
            if bytes.len() >= 20 {
                let e_machine = u16::from_le_bytes([bytes[18], bytes[19]]);
                if e_machine == 247 {
                    return "OK (EM_BPF)".to_string();
                } else {
                    return format!("WARN (e_machine={}, expected 247/EM_BPF)", e_machine);
                }
            }
            "OK (ELF valid)".to_string()
        }
        Err(e) => format!("FAIL (cannot read: {})", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_ebpf_compile_schema() {
        let tool = EbpfCompileTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["source_path"].is_object());
    }

    #[test]
    fn test_ebpf_compile_permission_level() {
        let tool = EbpfCompileTool;
        assert_eq!(tool.permission_level(), PermissionLevel::L2);
    }

    #[tokio::test]
    async fn test_ebpf_compile_missing_source() {
        let tool = EbpfCompileTool;
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = tool
            .execute(json!({"source_path": "/nonexistent.c"}), &ctx)
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }
}
