use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct SystemStatusTool;

#[async_trait]
impl Tool for SystemStatusTool {
    fn name(&self) -> &str {
        "system_status"
    }

    fn description(&self) -> &str {
        "Get typed host and Aletheon installed-runtime status: resources, binary provenance, and daemon state"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(SystemStatusTool)
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let mut parts = Vec::new();

        // Memory info from /proc/meminfo
        if let Ok(meminfo) = tokio::fs::read_to_string("/proc/meminfo").await {
            for line in meminfo.lines().take(5) {
                parts.push(line.to_string());
            }
        }

        // Load average
        if let Ok(loadavg) = tokio::fs::read_to_string("/proc/loadavg").await {
            parts.push(format!("Load: {}", loadavg.trim()));
        }

        // Disk usage
        if let Ok(output) = tokio::process::Command::new("df")
            .args(["-h", "/"])
            .output()
            .await
        {
            let df = String::from_utf8_lossy(&output.stdout);
            for line in df.lines().take(2) {
                parts.push(line.to_string());
            }
        }

        let current_exe = std::env::current_exe().ok();
        let release_binary = ctx.working_dir.join("target/release/aletheon");
        let installed_binary = std::path::PathBuf::from("/usr/bin/aletheon");
        let runtime = json!({
            "running_executable": current_exe.as_ref().map(|path| path.display().to_string()),
            "running_sha256": digest_file(current_exe.as_deref()).await,
            "release_binary": release_binary.display().to_string(),
            "release_sha256": digest_file(Some(&release_binary)).await,
            "installed_binary": installed_binary.display().to_string(),
            "installed_sha256": digest_file(Some(&installed_binary)).await,
            "user_daemon": user_daemon_state().await,
        });
        parts.push(format!("AletheonRuntime: {runtime}"));

        ToolResult {
            content: parts.join("\n"),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        }
    }
}

async fn digest_file(path: Option<&std::path::Path>) -> Option<String> {
    let bytes = tokio::fs::read(path?).await.ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
}

async fn user_daemon_state() -> serde_json::Value {
    let output = tokio::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            "aletheon.service",
            "--property=ActiveState",
            "--property=SubState",
            "--property=NRestarts",
            "--no-pager",
        ])
        .output()
        .await;
    match output {
        Ok(output) if output.status.success() => {
            let mut state = serde_json::Map::new();
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some((key, value)) = line.split_once('=') {
                    state.insert(key.to_string(), json!(value));
                }
            }
            serde_json::Value::Object(state)
        }
        Ok(output) => json!({"error": format!("systemctl exited {}", output.status)}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

#[cfg(test)]
mod tests {
    use super::digest_file;

    #[tokio::test]
    async fn digest_file_is_stable_and_missing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload");
        tokio::fs::write(&path, b"aletheon").await.unwrap();
        assert_eq!(
            digest_file(Some(&path)).await.as_deref(),
            Some("0701396227fe8e46cc65a9ddcf02ffa632f6b42eae619bad308a25e0d95f3428")
        );
        assert_eq!(digest_file(Some(&dir.path().join("missing"))).await, None);
    }
}
