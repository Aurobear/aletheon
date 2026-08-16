use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

const PROBES: &[(&str, &[&str])] = &[
    ("git", &["--version"]),
    ("glab", &["--version"]),
    ("gh", &["--version"]),
    ("cargo", &["--version"]),
    ("rustc", &["--version"]),
    ("docker", &["--version"]),
    ("podman", &["--version"]),
    ("ros2", &["--help"]),
    ("cmake", &["--version"]),
    ("ninja", &["--version"]),
    ("make", &["--version"]),
    ("python3", &["--version"]),
    ("node", &["--version"]),
    ("npm", &["--version"]),
    ("bwrap", &["--version"]),
    ("which", &["--version"]),
];

pub struct ToolchainStatusTool;

#[async_trait]
impl Tool for ToolchainStatusTool {
    fn name(&self) -> &str {
        "toolchain_status"
    }

    fn description(&self) -> &str {
        "Report whether common Linux and third-party engineering CLIs are installed, their resolved executable paths, and bounded version/help probe output. Use this before assuming git, glab, gh, Cargo, Docker, ROS 2, or build tools are available. `cd` is a shell builtin; use exec_command.workdir for durable command placement."
    }

    fn input_schema(&self) -> Value {
        let names = PROBES.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        json!({
            "type":"object",
            "additionalProperties":false,
            "properties":{
                "names":{
                    "type":"array",
                    "maxItems":PROBES.len(),
                    "uniqueItems":true,
                    "items":{"type":"string","enum":names},
                    "description":"Optional subset; omit to inspect every supported CLI"
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let requested = match input.get("names") {
            None => PROBES.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            Some(Value::Array(values)) => {
                let mut names = Vec::with_capacity(values.len());
                for value in values {
                    let Some(name) = value.as_str() else {
                        return error(ctx, start, "toolchain names must be strings");
                    };
                    if !PROBES.iter().any(|(candidate, _)| *candidate == name) {
                        return error(ctx, start, format!("unsupported toolchain probe: {name}"));
                    }
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                names
            }
            Some(_) => return error(ctx, start, "toolchain names must be an array"),
        };

        let mut results = Vec::with_capacity(requested.len());
        for name in requested {
            let args = PROBES
                .iter()
                .find_map(|(candidate, args)| (*candidate == name).then_some(*args))
                .expect("requested probes were validated");
            let path = which::which(name).ok();
            let probe = match &path {
                Some(path) => {
                    let mut command = tokio::process::Command::new(path);
                    command.args(args).kill_on_drop(true);
                    match tokio::time::timeout(Duration::from_secs(3), command.output()).await {
                        Ok(Ok(output)) => {
                            let text = if output.stdout.is_empty() {
                                &output.stderr
                            } else {
                                &output.stdout
                            };
                            json!({
                                "exit_code":output.status.code(),
                                "summary":String::from_utf8_lossy(text).lines().next().unwrap_or("")
                            })
                        }
                        Ok(Err(error)) => json!({"error":error.to_string()}),
                        Err(_) => json!({"error":"probe timed out after 3 seconds"}),
                    }
                }
                None => Value::Null,
            };
            results.push(json!({
                "name":name,
                "available":path.is_some(),
                "path":path.map(|path| path.display().to_string()),
                "probe":probe
            }));
        }

        ToolResult {
            content: serde_json::to_string_pretty(&json!({
                "tools":results,
                "shell_builtins":{"cd":"Use exec_command.workdir; a standalone cd does not persist across calls."}
            }))
            .expect("toolchain status serializes"),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        }
    }
}

fn error(
    ctx: &ToolContext,
    start: ::contracts::MonoTime,
    message: impl Into<String>,
) -> ToolResult {
    ToolResult {
        content: serde_json::to_string(&json!({"error":message.into()}))
            .expect("toolchain error serializes"),
        is_error: true,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
            truncated: false,
            patch_delta: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn context() -> ToolContext {
        ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: std::env::current_dir().unwrap(),
            session_id: "toolchain-test".into(),
            clock: Arc::new(kernel::chronos::SystemClock::new()),
            turn_event_sender: None,
        }
    }

    #[tokio::test]
    async fn reports_known_and_missing_tools_without_shell_interpolation() {
        let result = ToolchainStatusTool
            .execute(json!({"names":["git","glab"]}), &context())
            .await;
        assert!(!result.is_error, "{}", result.content);
        let value: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["tools"].as_array().unwrap().len(), 2);
        assert_eq!(value["tools"][0]["name"], "git");
        assert!(value["shell_builtins"]["cd"]
            .as_str()
            .unwrap()
            .contains("workdir"));
    }

    #[tokio::test]
    async fn rejects_arbitrary_executable_names() {
        let result = ToolchainStatusTool
            .execute(json!({"names":["evil-script"]}), &context())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("unsupported toolchain probe"));
    }
}
