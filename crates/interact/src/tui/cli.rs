//! CLI entry point — argument parsing and single-message mode.
//!
//! TUI mode delegates to [`super::run_tui`]. Single-message mode sends one
//! JSON-RPC request over the daemon socket and exits.

use super::debug;
use super::goal;
use super::workflow;

use std::io;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Default socket path for the aletheond daemon.
pub const DEFAULT_SOCKET: &str = "/run/aletheond/aletheond.sock";

#[derive(Parser)]
#[command(
    name = "aletheon",
    about = "Aletheon — self-evolving AI agent",
    version,
    after_help = "Examples:\n  aletheon                    # Start interactive mode\n  aletheon status             # Check daemon status\n  aletheon hello              # Send a chat message\n  aletheon daemon start       # Start the daemon\n  aletheon -m \"what is X?\"   # Single message mode"
)]
pub struct Args {
    /// Socket path
    #[arg(short, long, default_value = DEFAULT_SOCKET, global = true)]
    pub socket: PathBuf,

    /// Single message mode (non-interactive)
    #[arg(short, long)]
    pub message: Option<String>,

    /// Force TUI mode
    #[arg(long)]
    pub tui: bool,

    /// Subcommand or positional message
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Positional message (treated as chat if no subcommand)
    #[arg(trailing_var_arg = true, hide = true)]
    pub message_args: Vec<String>,

    // ── Test flags ──────────────────────────────────────────────
    /// Path to test input file (one line per input)
    #[arg(long)]
    pub test_input: Option<PathBuf>,

    /// Path to write frame snapshots (JSONL, one per render)
    #[arg(long)]
    pub record_frames: Option<PathBuf>,

    /// Path to write daemon->TUI events (JSONL)
    #[arg(long)]
    pub record_events: Option<PathBuf>,

    /// Auto-submit each line from --test-input (no Enter key needed)
    #[arg(long)]
    pub auto_submit: bool,

    /// Exit after N seconds (default: 120)
    #[arg(long, default_value_t = 120)]
    pub test_timeout: u64,
}

#[derive(Subcommand)]
pub enum Command {
    /// Daemon management
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Show reflections
    #[command(alias = "r")]
    Reflect,

    /// Show reflection history (alias: rn)
    #[command(alias = "rn")]
    ReflectNow,

    /// Show evolution history (alias: evo)
    #[command(alias = "evo")]
    Evolution,

    /// Show genome (alias: gene)
    #[command(alias = "gene")]
    Genome,

    /// Show daemon status (alias: st)
    #[command(alias = "st")]
    Status,

    /// Restore terminal to normal state (fix stuck raw mode/mouse capture)
    RestoreTerminal,

    /// Debug tools (topic, node, bag, perf, trace)
    Debug {
        #[command(subcommand)]
        action: debug::DebugCommand,
    },

    /// Persistent goal / objective management
    Goal {
        #[command(subcommand)]
        action: GoalAction,
    },

    /// Governed memory management
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Saved workflow management
    #[command(alias = "wf")]
    Workflow {
        #[command(subcommand)]
        action: workflow::WorkflowAction,
    },
}

#[derive(Subcommand)]
pub enum DaemonAction {
    /// Start the daemon
    Start {
        /// Run in background
        #[arg(short, long)]
        detach: bool,
    },

    /// Stop the daemon
    Stop,

    /// Show daemon status
    Status,
}

#[derive(Subcommand)]
pub enum MemoryAction {
    /// Save a fact: memory add "text" [--scope project] [--subject ...]
    Add {
        text: String,
        #[arg(long, default_value = "session")]
        scope: String,
        #[arg(long, default_value = "")]
        subject: String,
    },
    /// List facts [--scope S] [--all]
    List {
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Search facts: memory search "query" [--scope S]
    Search {
        query: String,
        #[arg(long)]
        scope: Option<String>,
    },
    /// Show one fact by id
    Show { id: i64 },
    /// Forget (archive; --hard to delete)
    Forget {
        id: i64,
        #[arg(long)]
        hard: bool,
    },
    /// Pin a fact
    Pin { id: i64 },
    /// Unpin a fact
    Unpin { id: i64 },
}

#[derive(Subcommand)]
pub enum GoalAction {
    /// Set the active objective
    Set {
        description: String,
        #[arg(long, default_value = "session")]
        scope: String,
    },
    /// Show one objective (with its sub-goals) by id
    Show {
        id: i64,
    },
    /// List objectives, or update one
    Status {
        #[arg(long)]
        id: Option<i64>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        filter: Option<String>,
    },
    /// Resume the active objective
    Resume,
}

/// CLI entry point — parses args and dispatches to the appropriate mode.
pub async fn run() -> Result<()> {
    let args = Args::parse();

    // Handle subcommands
    if let Some(cmd) = args.command {
        return handle_command(&args.socket, cmd).await;
    }

    // Handle positional message args
    if !args.message_args.is_empty() {
        let msg = args.message_args.join(" ");
        return single_message(&args.socket, &msg).await;
    }

    // Handle -m flag
    if let Some(msg) = args.message {
        return single_message(&args.socket, &msg).await;
    }

    // Interactive mode: use the line-based TUI (IME-compatible)
    let test_config = super::TestConfig {
        test_input: args.test_input,
        record_frames: args.record_frames,
        record_events: args.record_events,
        auto_submit: args.auto_submit,
        test_timeout: args.test_timeout,
    };
    super::run_with_config(args.socket.to_str().unwrap_or(DEFAULT_SOCKET), test_config).await
}

/// Handle subcommands.
async fn handle_command(socket: &PathBuf, cmd: Command) -> Result<()> {
    match cmd {
        Command::Daemon { action } => handle_daemon_action(action).await,
        Command::Reflect => single_message(socket, "/reflect").await,
        Command::ReflectNow => single_message(socket, "/reflect_now").await,
        Command::Evolution => single_message(socket, "/evolution").await,
        Command::Genome => single_message(socket, "/genome").await,
        Command::Status => single_message(socket, "/status").await,
        Command::RestoreTerminal => {
            super::restore_terminal();
            println!("Terminal restored to normal state.");
            Ok(())
        }
        Command::Debug { action } => debug::run(socket, action).await,
        Command::Goal { action } => goal::run(socket, action).await,
        Command::Memory { action } => memory_cmd(socket, action).await,
        Command::Workflow { action } => workflow::run(socket, action).await,
    }
}

/// Handle memory subcommands by sending JSON-RPC to the daemon.
async fn memory_cmd(socket: &PathBuf, action: MemoryAction) -> Result<()> {
    let req = match &action {
        MemoryAction::Add { text, scope, subject } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.add",
            "params": { "content": text, "scope": scope, "subject": subject }
        }),
        MemoryAction::List { scope, all } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.list",
            "params": { "scope": scope, "all": all }
        }),
        MemoryAction::Search { query, scope } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.search",
            "params": { "query": query, "scope": scope }
        }),
        MemoryAction::Show { id } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.show",
            "params": { "id": id }
        }),
        MemoryAction::Forget { id, hard } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.forget",
            "params": { "id": id, "hard": hard }
        }),
        MemoryAction::Pin { id } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.pin",
            "params": { "id": id }
        }),
        MemoryAction::Unpin { id } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.unpin",
            "params": { "id": id }
        }),
    };

    let resp = super::rpc_client::send_rpc(socket, &req).await?;

    if let Some(err) = resp.get("error") {
        eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
    } else if let Some(facts) = resp["result"]["facts"].as_array() {
        for f in facts {
            let pinned = if f["pinned"].as_bool().unwrap_or(false) { " [PINNED]" } else { "" };
            println!(
                "[{}] ({}/{}){} {}",
                f["fact_id"].as_i64().unwrap_or(0),
                f["scope"].as_str().unwrap_or("?"),
                f["status"].as_str().unwrap_or("?"),
                pinned,
                f["content"].as_str().unwrap_or(""),
            );
        }
    } else if let Some(fact) = resp["result"]["fact"].as_object() {
        println!("ID:      {}", fact.get("fact_id").map(|v| v.to_string()).unwrap_or_default());
        println!("Content: {}", fact.get("content").and_then(|v| v.as_str()).unwrap_or(""));
        println!("Scope:   {}  Source: {}  Status: {}",
            fact.get("scope").and_then(|v| v.as_str()).unwrap_or("?"),
            fact.get("source").and_then(|v| v.as_str()).unwrap_or("?"),
            fact.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
        );
        println!("Trust:   {}  Tier: {}  TTL: {}d",
            fact.get("trust_score").and_then(|v| v.as_f64()).unwrap_or(0.0),
            fact.get("tier").and_then(|v| v.as_str()).unwrap_or("?"),
            fact.get("ttl_days").and_then(|v| v.as_i64()).unwrap_or(0),
        );
        println!("Pinned:  {}  Retrievals: {}",
            fact.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false),
            fact.get("retrieval_count").and_then(|v| v.as_i64()).unwrap_or(0),
        );
        println!("Created: {}  Updated: {}",
            fact.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
            fact.get("updated_at").and_then(|v| v.as_str()).unwrap_or(""),
        );
    } else {
        println!("{}", serde_json::to_string_pretty(&resp["result"]).unwrap_or_default());
    }
    Ok(())
}

/// Find the aletheond binary.
fn find_aletheond() -> Result<std::path::PathBuf> {
    // Try which first
    if let Ok(path) = which::which("aletheond") {
        return Ok(path);
    }
    // Try same directory as current binary
    let current = std::env::current_exe()?;
    let dir = current.parent().unwrap_or(std::path::Path::new("."));
    let path = dir.join("aletheond");
    if path.exists() {
        return Ok(path);
    }
    Err(anyhow::anyhow!(
        "Cannot find aletheond binary. Install it or add to PATH"
    ))
}

/// Handle daemon subcommands.
async fn handle_daemon_action(action: DaemonAction) -> Result<()> {
    match action {
        DaemonAction::Start { detach } => {
            let exe = find_aletheond()?;

            if detach {
                // Start daemon in background
                let mut cmd = std::process::Command::new(exe);
                cmd.arg("--socket").arg(DEFAULT_SOCKET);
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                cmd.stdin(std::process::Stdio::null());
                let child = cmd.spawn()?;
                println!("Daemon started in background (PID: {})", child.id());
                println!("Socket: {}", DEFAULT_SOCKET);
            } else {
                // Start daemon in foreground
                println!("Starting daemon (Ctrl+C to stop)...");
                let status = std::process::Command::new(exe)
                    .arg("--socket")
                    .arg(DEFAULT_SOCKET)
                    .status()?;
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        DaemonAction::Stop => {
            // Send SIGTERM to daemon
            let pid_file = std::path::Path::new("/tmp/aletheon/aletheond.pid");
            if pid_file.exists() {
                let pid_str = std::fs::read_to_string(pid_file)?;
                let pid: i32 = pid_str.trim().parse()?;
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
                println!("Sent SIGTERM to daemon (PID: {})", pid);
                std::fs::remove_file(pid_file).ok();
            } else {
                println!("No daemon PID file found");
            }
        }
        DaemonAction::Status => {
            println!("Daemon status: checking...");
            // Try to connect to socket
            let socket = std::path::Path::new(DEFAULT_SOCKET);
            if socket.exists() {
                match UnixStream::connect(socket).await {
                    Ok(_) => println!("Daemon is running"),
                    Err(e) => println!("Daemon socket exists but connection failed: {}", e),
                }
            } else {
                println!("Daemon is not running (no socket)");
            }
        }
    }
    Ok(())
}

/// Default timeout for single-message mode (seconds).
const SINGLE_MESSAGE_TIMEOUT_SECS: u64 = 120;

/// Send a single message and print the response.
pub async fn single_message(socket: &PathBuf, msg: &str) -> Result<()> {
    let mut stream = UnixStream::connect(socket).await?;
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);

    // Determine JSON-RPC method from slash commands
    let request = if msg.starts_with('/') {
        let cmd = msg.strip_prefix('/').unwrap_or(msg);
        let (name, _args) = match cmd.find(' ') {
            Some(i) => (&cmd[..i], cmd[i + 1..].trim()),
            None => (cmd, ""),
        };
        match name {
            "reflect" | "r" => serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "reflect"
            }),
            "reflect_now" | "rn" => serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "reflect_now"
            }),
            "evolution" | "evo" => serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "evolution"
            }),
            "genome" | "gene" => serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "genome"
            }),
            "status" | "st" => serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "status"
            }),
            _ => serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "chat",
                "params": { "message": msg }
            }),
        }
    } else {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "chat",
            "params": { "message": msg }
        })
    };
    let req_str = serde_json::to_string(&request)?;
    writer.write_all(req_str.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    // Track whether we received any streaming text to avoid duplicate output.
    let mut had_streaming_text = false;

    // Use tokio::time::timeout to wrap the entire response reading loop.
    // This provides a clean timeout mechanism.
    let timeout_duration = std::time::Duration::from_secs(SINGLE_MESSAGE_TIMEOUT_SECS);

    let result = tokio::time::timeout(timeout_duration, async {
        let mut response_buf = String::new();
        loop {
            response_buf.clear();
            match reader.read_line(&mut response_buf).await {
                Ok(0) => {
                    eprintln!("Connection lost");
                    return Ok::<(), anyhow::Error>(());
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Error reading response: {}", e);
                    return Err(anyhow::anyhow!("Read error: {}", e));
                }
            }

            let resp: serde_json::Value = match serde_json::from_str(response_buf.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Handle out-of-band approval_request notification
            if resp.get("method").and_then(|v| v.as_str()) == Some("approval_request")
                && resp.get("result").is_none()
                && resp.get("id").is_none()
            {
                let params = &resp["params"];
                let tool = params["tool"].as_str().unwrap_or("?");
                let action_summary = params["action_summary"].as_str().unwrap_or("");
                let risk_level = params["risk_level"].as_str().unwrap_or("");
                let approval_id = params["approval_id"].as_str().unwrap_or("");
                eprintln!(
                    "\n\u{26a0}  Approval required [{}] {}\n   {}\n   Approve? [y]es / [a]lways / [N]o: ",
                    risk_level, tool, action_summary,
                );
                let mut line = String::new();
                let stdin = io::stdin();
                let decision = match stdin.read_line(&mut line) {
                    Ok(0) | Err(_) => "deny",
                    Ok(_) => match line.trim().to_lowercase().as_str() {
                        "y" | "yes" => "approve",
                        "a" | "always" => "approve_for_session",
                        _ => "deny",
                    },
                };
                let approval_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "method": "approval_response",
                    "params": {
                        "approval_id": approval_id,
                        "decision": decision,
                    }
                });
                let resp_str = serde_json::to_string(&approval_resp)?;
                writer.write_all(resp_str.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                continue; // wait for the actual response
            }

            // Skip streaming events (notifications have no "id" field).
            // These are text_delta, tool_call_start, tool_call_result, usage,
            // turn_done, awareness_changed, mode_changed, etc.
            if resp.get("id").is_none() && resp.get("method").is_some() {
                // Print tool_call_start for visibility, but skip text_delta
                // since the final response contains the full text (avoids duplication).
                if let Some(params) = resp.get("params") {
                    let event_method = resp["method"].as_str().unwrap_or("");
                    if event_method == "event" {
                        if let Some(event_type) = params.get("type").and_then(|v| v.as_str()) {
                            match event_type {
                                "tool_call_start" => {
                                    if let Some(name) = params.get("tool").and_then(|v| v.as_str()) {
                                        eprintln!("🔧 [{}]", name);
                                    }
                                }
                                "tool_call_result" => {
                                    // Show brief tool result
                                }
                                "text_delta" => {
                                    had_streaming_text = true;
                                    // Skip — final response has full text
                                }
                                _ => {}
                            }
                        }
                    }
                }
                continue;
            }

            // Final response — print the full text (this is the authoritative output)
            if let Some(text) = resp["result"]["response"].as_str() {
                // Deduplicate consecutive identical lines (some models repeat text)
                let deduped = deduplicate_response(text);
                println!("{}", deduped);
            } else if !resp["result"]["reflections"].is_null() {
                println!("{}", format_reflections(&resp["result"]["reflections"]));
            } else if !resp["result"]["genome"].is_null() {
                println!("{}", format_genome(&resp["result"]["genome"]));
            } else if !resp["result"]["evolution"].is_null() {
                println!("{}", format_evolution(&resp["result"]["evolution"]));
            } else if let Some(_status) = resp["result"]["status"].as_object() {
                println!("{}", format_status(&resp["result"]["status"]));
            } else if let Some(err) = resp["error"]["message"].as_str() {
                eprintln!("Error: {}", err);
            }
            return Ok(());
        }
    }).await;

    match result {
        Ok(inner) => inner?,
        Err(_) => {
            eprintln!("\n⏰ Timeout: no response after {}s", SINGLE_MESSAGE_TIMEOUT_SECS);
        }
    }
    Ok(())
}

/// Deduplicate consecutive identical content in response text.
/// Some models (especially with tool calls) repeat the same text twice.
/// This function handles both line-level and sentence-level duplication.
fn deduplicate_response(text: &str) -> String {
    // First, try line-level deduplication
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return text.to_string();
    }

    let mut deduped_lines = Vec::new();
    let mut prev_line = lines[0];
    deduped_lines.push(prev_line);

    for line in &lines[1..] {
        // Skip consecutive identical lines (trimmed comparison)
        if line.trim() == prev_line.trim() {
            continue;
        }
        prev_line = line;
        deduped_lines.push(line);
    }

    let result = deduped_lines.join("\n");

    // Also handle sentence-level duplication (same sentence repeated on same line)
    // This handles cases like "Sentence.Sentence." -> "Sentence."
    deduplicate_sentences(&result)
}

/// Deduplicate consecutive identical text blocks within a string.
/// Handles cases where the same text (one or more sentences) is repeated.
fn deduplicate_sentences(text: &str) -> String {
    // Try to find the longest repeated prefix
    // This handles multi-sentence duplicates like "Sentence1. Sentence2.Sentence1. Sentence2."
    let len = text.len();

    // Try different split points to find the repeated block
    for split_pos in (1..=len / 2).rev() {
        // Ensure we split at a valid UTF-8 boundary
        if !text.is_char_boundary(split_pos) {
            continue;
        }

        let prefix = &text[..split_pos];
        let suffix = &text[split_pos..];

        // Check if the suffix starts with the same prefix
        if suffix.starts_with(prefix) {
            // Found a repeated block - return just the prefix
            return prefix.to_string();
        }
    }

    // No repeated block found, return original text
    text.to_string()
}

/// Format reflection entries for display.
fn format_reflections(entries: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = entries.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "No reflections found.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("=== Reflections ({}) ===\n", arr.len()));
    for (i, entry) in arr.iter().enumerate() {
        let task = entry
            .get("task_summary")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let outcome = entry
            .get("outcome")
            .and_then(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else {
                    serde_json::to_string(v).ok()
                }
            })
            .unwrap_or_else(|| "unknown".to_string());
        let confidence = entry
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let timestamp = entry
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        lines.push(format!(
            "[{}] #{} {} ({}) conf={:.0}%",
            timestamp,
            i + 1,
            task,
            outcome,
            confidence * 100.0
        ));

        if let Some(arr) = entry.get("learned").and_then(|v| v.as_array()) {
            for l in arr {
                if let Some(s) = l.as_str() {
                    lines.push(format!("  learned: {}", s));
                }
            }
        }
        if let Some(arr) = entry.get("what_worked").and_then(|v| v.as_array()) {
            for w in arr {
                if let Some(s) = w.as_str() {
                    lines.push(format!("  worked: {}", s));
                }
            }
        }
        if let Some(arr) = entry.get("what_failed").and_then(|v| v.as_array()) {
            for f in arr {
                if let Some(s) = f.as_str() {
                    lines.push(format!("  failed: {}", s));
                }
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

/// Format genome for display.
fn format_genome(genome: &serde_json::Value) -> String {
    if let Some(s) = genome.as_str() {
        return s.to_string();
    }
    serde_json::to_string_pretty(genome).unwrap_or_else(|_| format!("{:?}", genome))
}

/// Format evolution history for display.
fn format_evolution(evo: &serde_json::Value) -> String {
    if let Some(s) = evo.as_str() {
        return s.to_string();
    }
    if let Some(arr) = evo.as_array() {
        if arr.is_empty() {
            return "No evolution history found.".to_string();
        }
        let mut lines = Vec::new();
        lines.push(format!("=== Evolution History ({}) ===\n", arr.len()));
        for entry in arr {
            lines.push(
                serde_json::to_string_pretty(entry).unwrap_or_else(|_| format!("{:?}", entry)),
            );
            lines.push(String::new());
        }
        return lines.join("\n");
    }
    serde_json::to_string_pretty(evo).unwrap_or_else(|_| format!("{:?}", evo))
}

/// Format status response for display.
fn format_status(status: &serde_json::Value) -> String {
    let session_id = status
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let turn_count = status
        .get("turn_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let reflection_count = status
        .get("reflection_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let evolution_count = status
        .get("evolution_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let boundary_rules = status
        .get("boundary_rules")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let boundary_immutable = status
        .get("boundary_immutable")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let attention_focus = status
        .get("attention_focus")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut lines = Vec::new();
    lines.push("=== Aletheon Status ===".to_string());
    lines.push(format!(
        "Session: {}",
        &session_id[..8.min(session_id.len())]
    ));
    lines.push(format!("Turns: {}", turn_count));
    lines.push(format!("Reflections: {}", reflection_count));
    lines.push(format!("Evolutions: {}", evolution_count));
    lines.push(String::new());
    lines.push("Care Weights:".to_string());

    if let Some(cares) = status.get("care_weights").and_then(|v| v.as_array()) {
        for care in cares {
            let topic = care.get("topic").and_then(|v| v.as_str()).unwrap_or("?");
            let weight = care.get("weight").and_then(|v| v.as_f64()).unwrap_or(0.0);
            lines.push(format!("  {}: {:.2}", topic, weight));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "Boundary Rules: {} (immutable: {})",
        boundary_rules, boundary_immutable
    ));

    let focus_display = if attention_focus.is_empty() {
        "none"
    } else {
        attention_focus
    };
    lines.push(format!("Attention Focus: {}", focus_display));

    lines.join("\n")
}

#[cfg(test)]
mod workflow_cli_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_workflow_list() {
        let args = Args::try_parse_from(["aletheon", "workflow", "list"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Workflow { action: workflow::WorkflowAction::List })
        ));
    }

    #[test]
    fn parses_workflow_run_with_name() {
        let args = Args::try_parse_from(["aletheon", "workflow", "run", "deploy"]).unwrap();
        match args.command {
            Some(Command::Workflow { action: workflow::WorkflowAction::Run { name } }) => {
                assert_eq!(name, "deploy")
            }
            _ => panic!("unexpected parse for workflow run"),
        }
    }

    #[test]
    fn parses_workflow_save_with_name_and_path() {
        let args = Args::try_parse_from([
            "aletheon", "workflow", "save", "mywf", "/tmp/wf.json",
        ])
        .unwrap();
        match args.command {
            Some(Command::Workflow { action: workflow::WorkflowAction::Save { name, path } }) => {
                assert_eq!(name, "mywf");
                assert_eq!(path, std::path::PathBuf::from("/tmp/wf.json"));
            }
            _ => panic!("unexpected parse for workflow save"),
        }
    }

    #[test]
    fn parses_workflow_load_with_name() {
        let args = Args::try_parse_from(["aletheon", "workflow", "load", "mywf"]).unwrap();
        match args.command {
            Some(Command::Workflow { action: workflow::WorkflowAction::Load { name } }) => {
                assert_eq!(name, "mywf")
            }
            _ => panic!("unexpected parse for workflow load"),
        }
    }

    #[test]
    fn parses_workflow_delete_with_name() {
        let args = Args::try_parse_from(["aletheon", "workflow", "delete", "mywf"]).unwrap();
        match args.command {
            Some(Command::Workflow { action: workflow::WorkflowAction::Delete { name } }) => {
                assert_eq!(name, "mywf")
            }
            _ => panic!("unexpected parse for workflow delete"),
        }
    }

    #[test]
    fn parses_workflow_wf_alias() {
        let args = Args::try_parse_from(["aletheon", "wf", "list"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Workflow { action: workflow::WorkflowAction::List })
        ));
    }
}
