//! Non-interactive agent execution for CI/CD and scripting.
//!
//! Usage: aletheon-exec --prompt "fix the bug in main.rs" [--model MODEL] [--max-turns N] [--sandbox auto|require|forbid]
//!
//! Flow:
//! 1. Parse CLI args (prompt, model, max_turns, sandbox, working_dir)
//! 2. Initialize logging
//! 3. Create LLM provider from config
//! 4. Create tool registry
//! 5. Run agent loop:
//!    - Send prompt to LLM with tools
//!    - Process tool calls
//!    - Collect responses
//!    - Repeat until LLM stops calling tools or max_turns reached
//! 6. Print final response to stdout
//! 7. Exit with code 0 (success) or 1 (failure)

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use aletheon_abi::{ContentBlock, Message, Role};
use aletheon_body::r#impl::security::approval::{ApprovalGate, TerminalApprovalGate};
use aletheon_body::r#impl::security::audit::AuditLogger;
use aletheon_body::r#impl::security::runner::ToolRunnerWithGuard;
use aletheon_body::r#impl::tools::{ToolContext, ToolRegistry};
use aletheon_brain::r#impl::llm::LlmProvider;
use aletheon_brain::r#impl::llm::StopReason;
use aletheon_brain::r#impl::provider_registry::ProviderRegistry;

/// Minimal KEY=VALUE .env loader (no shell expansion). Mirrors the daemon's loader so
/// exec resolves provider API keys the same way the daemon does. Does not override
/// already-set process env vars.
fn load_dotenv(path: &std::path::Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if std::env::var(k).is_err() {
                std::env::set_var(k, v);
            }
        }
    }
}

#[derive(Parser)]
#[command(name = "aletheon-exec", about = "Non-interactive agent execution")]
struct Args {
    /// The prompt/task to execute
    #[arg(short, long)]
    prompt: String,

    /// Model spec (e.g., "anthropic/claude-sonnet-4-20250514", "sonnet", "mimo-v2.5-pro")
    #[arg(short, long, default_value = "")]
    model: String,

    /// Maximum number of agentic turns (LLM call + tool execution = 1 turn)
    #[arg(short = 'n', long, default_value_t = 20)]
    max_turns: usize,

    /// Sandbox preference: auto, require, or forbid
    #[arg(long, default_value = "auto")]
    sandbox: String,

    /// Working directory for tool execution
    #[arg(short = 'd', long, default_value = ".")]
    working_dir: PathBuf,

    /// Path to config file (default: ~/.aletheon/config.toml)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Output format: text (default) or json
    #[arg(long, default_value = "text")]
    output: String,
}

/// Result of the exec run.
#[derive(serde::Serialize)]
struct ExecResult {
    success: bool,
    response: String,
    turns_used: usize,
    total_input_tokens: u32,
    total_output_tokens: u32,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let output_format = args.output.clone();

    // Initialize logging to stderr (stdout is reserved for output)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("aletheon_exec=info")),
        )
        .init();

    match run(args).await {
        Ok(result) => {
            if output_format == "json" {
                println!("{}", serde_json::to_string(&result).unwrap());
            } else {
                println!("{}", result.response);
            }
            if !result.success {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: {:#}", e);
            std::process::exit(1);
        }
    }
}

async fn run(args: Args) -> Result<ExecResult> {
    // Load ~/.aletheon/.env so provider API keys resolve (the daemon does this too).
    if let Some(home) = std::env::var_os("HOME") {
        load_dotenv(&std::path::Path::new(&home).join(".aletheon").join(".env"));
    }

    let working_dir = args
        .working_dir
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp")));

    // Load config
    let app_config = if let Some(ref path) = args.config {
        aletheon_brain::config::AppConfig::load_or_default(path)
    } else {
        aletheon_brain::config::AppConfig::load_layered(None)
    };

    // Build provider registry
    let registry = ProviderRegistry::from_config(&app_config)?;

    // Create LLM provider
    let llm: Arc<dyn LlmProvider> = Arc::from(registry.resolve_and_create(&args.model)?);
    info!(provider = llm.name(), model = %args.model, "LLM provider initialized");

    // Create tool registry with default tools
    let tool_registry = ToolRegistry::default();

    // Guarded runner with terminal approval for risky (L2+) tools.
    let audit_path = working_dir.join(".aletheon-audit.jsonl");
    let approval: Arc<dyn ApprovalGate> = Arc::new(TerminalApprovalGate);
    let mut runner = ToolRunnerWithGuard::with_default_sandbox(AuditLogger::new(audit_path)?)
        .with_approval_gate(approval);
    let turn_id = uuid::Uuid::new_v4().to_string();
    runner.on_new_turn(&turn_id);

    // Build tool definitions for LLM
    let tool_defs = tool_registry.definitions();
    info!(tool_count = tool_defs.len(), "Tools registered");

    // Build system prompt
    let system_prompt = format!(
        "You are Aletheon, an AI agent executing a task non-interactively. \
         You have access to tools. Complete the user's request and provide a final response. \
         Working directory: {}",
        working_dir.display()
    );

    // Initialize conversation
    let mut messages: Vec<Message> =
        vec![Message::system(&system_prompt), Message::user(&args.prompt)];

    // Tool execution context
    let tool_ctx = ToolContext {
        working_dir: working_dir.clone(),
        session_id: uuid::Uuid::new_v4().to_string(),
    };

    // Agent loop
    let mut turns_used = 0;
    let mut total_input_tokens = 0u32;
    let mut total_output_tokens = 0u32;
    let final_response;

    loop {
        if turns_used >= args.max_turns {
            warn!(max_turns = args.max_turns, "Max turns reached");
            final_response = format!(
                "Max turns ({}) reached without completing the task.",
                args.max_turns
            );
            break;
        }

        info!(turn = turns_used + 1, "Starting turn");

        // Call LLM
        let response = llm.complete(&messages, &tool_defs).await?;
        total_input_tokens += response.usage.input_tokens;
        total_output_tokens += response.usage.output_tokens;

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens => {
                // LLM is done, extract final text response
                final_response = response
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");

                // Add assistant message to history
                messages.push(Message {
                    role: Role::Assistant,
                    content: response.content.clone(),
                });
                break;
            }
            StopReason::ToolUse => {
                // LLM wants to call tools
                let assistant_content = response.content.clone();
                let mut tool_results = Vec::new();

                for block in &response.content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        info!(tool = %name, "Executing tool");
                        let tool_result = if let Some(tool) = tool_registry.get(name) {
                            let result = runner
                                .run(tool.as_ref(), input.clone(), &tool_ctx, &turn_id)
                                .await;
                            if result.is_error {
                                warn!(tool = %name, error = %result.content, "Tool failed/denied");
                            } else {
                                info!(tool = %name, "Tool succeeded");
                            }
                            ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: result.content,
                                is_error: result.is_error,
                            }
                        } else {
                            warn!(tool = %name, "Unknown tool");
                            ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: format!("Error: Unknown tool '{}'", name),
                                is_error: true,
                            }
                        };
                        tool_results.push(tool_result);
                    }
                }

                // Add assistant message (with tool_use) and tool results to history
                messages.push(Message {
                    role: Role::Assistant,
                    content: assistant_content,
                });
                messages.push(Message {
                    role: Role::User,
                    content: tool_results,
                });

                turns_used += 1;
            }
        }
    }

    let success = turns_used < args.max_turns;
    info!(
        turns = turns_used,
        input_tokens = total_input_tokens,
        output_tokens = total_output_tokens,
        success = success,
        "Execution complete"
    );

    Ok(ExecResult {
        success,
        response: final_response,
        turns_used,
        total_input_tokens,
        total_output_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_parsing() {
        let args = Args::try_parse_from([
            "aletheon-exec",
            "--prompt",
            "hello world",
            "--max-turns",
            "5",
            "--sandbox",
            "require",
        ])
        .unwrap();

        assert_eq!(args.prompt, "hello world");
        assert_eq!(args.max_turns, 5);
        assert_eq!(args.sandbox, "require");
        assert_eq!(args.model, "");
        assert_eq!(args.output, "text");
    }

    #[test]
    fn test_args_defaults() {
        let args = Args::try_parse_from(["aletheon-exec", "--prompt", "test"]).unwrap();

        assert_eq!(args.max_turns, 20);
        assert_eq!(args.sandbox, "auto");
        assert_eq!(args.output, "text");
    }

    #[test]
    fn test_args_prompt_required() {
        let result = Args::try_parse_from(["aletheon-exec"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_args_json_output() {
        let args = Args::try_parse_from(["aletheon-exec", "--prompt", "test", "--output", "json"])
            .unwrap();

        assert_eq!(args.output, "json");
    }
}
