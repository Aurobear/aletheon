use std::path::PathBuf;
use anyhow::Result;
use serde_json::Value;
use tracing::{debug, warn};

/// Plugin runtime types.
#[derive(Debug, Clone)]
pub enum PluginRuntime {
    /// Run as subprocess.
    Command {
        program: String,
        args: Vec<String>,
        work_dir: PathBuf,
    },
    // Future: Native { lib: libloading::Library },
    // Future: Wasm { engine: wasmtime::Engine, module: wasmtime::Module },
    // Future: Agent { agent_id: String },
}

impl PluginRuntime {
    /// Create runtime from entry string (e.g. "cmd:./run.sh", "native:./lib.so").
    pub fn from_entry(entry: &str, plugin_dir: &PathBuf) -> Result<Self> {
        let parts: Vec<&str> = entry.splitn(2, ':').collect();
        match parts.first().copied() {
            Some("cmd") => {
                let cmd_path = parts.get(1).unwrap_or(&"");
                if cmd_path.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Command plugin entry must specify a program path (cmd:<path>)"
                    ));
                }
                let full_path = plugin_dir.join(cmd_path);
                if !full_path.exists() {
                    warn!(
                        entry = entry,
                        path = %full_path.display(),
                        "Plugin command not found (will fail at runtime)"
                    );
                }
                Ok(Self::Command {
                    program: full_path.to_string_lossy().to_string(),
                    args: Vec::new(),
                    work_dir: plugin_dir.clone(),
                })
            }
            Some("native") => {
                Err(anyhow::anyhow!("Native (.so) plugins not yet implemented"))
            }
            Some("wasm") => {
                Err(anyhow::anyhow!("WASM plugins not yet implemented"))
            }
            Some("agent") => {
                Err(anyhow::anyhow!("Agent plugins not yet implemented"))
            }
            _ => Err(anyhow::anyhow!("Unknown plugin entry type: {}", entry)),
        }
    }

    /// Execute a tool call via this runtime.
    pub async fn execute(&self, tool_name: &str, args: &Value) -> Result<Value> {
        match self {
            Self::Command {
                program,
                args: cmd_args,
                work_dir,
            } => {
                debug!(
                    tool = tool_name,
                    program = program.as_str(),
                    "Executing plugin tool via command runtime"
                );

                let mut cmd = tokio::process::Command::new(program);
                cmd.args(cmd_args)
                    .arg("--tool")
                    .arg(tool_name)
                    .arg("--args")
                    .arg(args.to_string())
                    .current_dir(work_dir)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());

                let output = cmd.output().await?;

                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let result: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| {
                        serde_json::json!({ "output": stdout.to_string() })
                    });
                    Ok(result)
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(anyhow::anyhow!(
                        "Plugin tool '{}' failed (exit {}): {}",
                        tool_name,
                        output.status.code().unwrap_or(-1),
                        stderr
                    ))
                }
            }
        }
    }
}
