use std::time::Duration;
use tokio::process::Command;
use tracing::{warn, debug};

use super::types::*;

pub struct HookDispatcher {
    config: super::config::HookConfig,
}

impl HookDispatcher {
    pub fn new(config: super::config::HookConfig) -> Self {
        Self { config }
    }

    /// Try to create a HookDispatcher by loading config from all layers.
    /// Returns None if no hooks are configured (graceful degradation).
    pub fn try_load() -> Option<Self> {
        match super::config::HookConfig::load() {
            Ok(config) => Some(Self::new(config)),
            Err(e) => {
                warn!(error = %e, "Failed to load hook config; hook system disabled");
                None
            }
        }
    }

    /// Fire hooks for an event. Returns aggregated result.
    pub async fn fire(&self, event: HookEventName, context: &HookContext) -> HandlerResult {
        let hooks = self.config.get_matching_hooks(
            event,
            context.tool.as_deref(),
            context.args.as_deref(),
            context.risk.as_deref(),
        );

        if hooks.is_empty() {
            return HandlerResult::Continue;
        }

        debug!(event = ?event, count = hooks.len(), "Firing hooks");

        for hook in &hooks {
            match self.execute_hook(hook, context).await {
                HandlerResult::Continue => continue,
                result => return result, // Block/ModifyArgs/InjectContext stop the chain
            }
        }

        HandlerResult::Continue
    }

    async fn execute_hook(&self, hook: &Hook, context: &HookContext) -> HandlerResult {
        match &hook.hook_type {
            HookType::Command => self.execute_command_hook(hook, context).await,
            HookType::Prompt => {
                if let Some(ref text) = hook.prompt_text {
                    HandlerResult::InjectContext(text.clone())
                } else {
                    HandlerResult::Failed("Prompt hook missing prompt_text".into())
                }
            }
            HookType::Agent => {
                // Agent hooks delegate to sub-agents -- not implemented yet
                HandlerResult::Failed("Agent hooks not yet implemented".into())
            }
        }
    }

    async fn execute_command_hook(&self, hook: &Hook, context: &HookContext) -> HandlerResult {
        let cmd = match &hook.command {
            Some(c) => c.clone(),
            None => return HandlerResult::Failed("Command hook missing command".into()),
        };

        let mut command = Command::new(&cmd);
        if let Some(ref args) = hook.command_args {
            command.args(args);
        }

        // Pass context as env vars
        if let Some(ref tool) = context.tool {
            command.env("HOOK_TOOL", tool);
        }
        if let Some(ref args) = context.args {
            command.env("HOOK_ARGS", args);
        }
        command.env("HOOK_EVENT", format!("{:?}", hook.event));

        let timeout = Duration::from_secs(hook.timeout_sec);

        match tokio::time::timeout(timeout, command.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if output.status.success() {
                    if stdout.trim().is_empty() {
                        HandlerResult::Continue
                    } else {
                        // Non-empty stdout means modification
                        HandlerResult::ModifyArgs(serde_json::json!({ "output": stdout.trim() }))
                    }
                } else {
                    // Non-zero exit means block
                    HandlerResult::Block(format!("Hook '{}' blocked: {}", hook.id, stderr.trim()))
                }
            }
            Ok(Err(e)) => HandlerResult::Failed(format!("Hook '{}' failed: {}", hook.id, e)),
            Err(_) => HandlerResult::TimedOut,
        }
    }
}
