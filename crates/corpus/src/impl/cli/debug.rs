//! `aletheon debug` subcommand — CLI tools for debugging the daemon.
//!
//! Subcommands:
//!   topic list            — list registered tracepoints
//!   topic echo            — stream debug events in real time
//!   node info             — show daemon PID, uptime, perf stats
//!   bag record [-o path]  — record debug events to a bag file
//!   bag play <path>       — replay a bag file
//!   bag info <path>       — show bag file metadata
//!   perf [--interval N]   — show performance stats
//!   trace start/stop/status — control runtime tracing
//!
//! Design: `docs/plans/2026-06-19-aletheon-debug-system-design.md` (Layer 3).

use anyhow::{Context, Result};
use clap::Subcommand;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

// ---------------------------------------------------------------------------
// Subcommand definitions
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum DebugCommand {
    /// Debug topic operations
    Topic {
        #[command(subcommand)]
        action: TopicAction,
    },

    /// Debug node operations
    Node {
        #[command(subcommand)]
        action: NodeAction,
    },

    /// Bag recording and playback
    Bag {
        #[command(subcommand)]
        action: BagAction,
    },

    /// Performance stats
    Perf {
        /// Refresh interval in seconds (continuous monitoring)
        #[arg(short, long)]
        interval: Option<u64>,
    },

    /// Runtime tracing control
    Trace {
        #[command(subcommand)]
        action: TraceAction,
    },
}

#[derive(Subcommand)]
pub enum TopicAction {
    /// List all registered tracepoints
    List,

    /// Stream debug events in real time
    Echo {
        /// Filter by module (e.g., module=runtime)
        #[arg(long)]
        r#module: Option<String>,

        /// Filter by minimum level (error/warn/info/debug/trace)
        #[arg(long, default_value = "info")]
        level: String,

        /// Filter by tracepoint name pattern
        #[arg(long)]
        tracepoint: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum NodeAction {
    /// Show daemon info (PID, uptime, memory, perf)
    Info,
}

#[derive(Subcommand)]
pub enum BagAction {
    /// Record debug events to a bag file
    Record {
        /// Output bag file path
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,

        /// Filter by module
        #[arg(long)]
        r#module: Option<String>,

        /// Filter by minimum level
        #[arg(long, default_value = "info")]
        level: String,
    },

    /// Replay a bag file
    Play {
        /// Path to bag file
        path: PathBuf,

        /// Replay speed multiplier
        #[arg(long, default_value = "1.0")]
        speed: f64,
    },

    /// Show bag file metadata
    Info {
        /// Path to bag file
        path: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum TraceAction {
    /// Start tracing
    Start {
        /// Module to trace
        #[arg(short, long)]
        r#module: Option<String>,

        /// Minimum trace level
        #[arg(short, long, default_value = "debug")]
        level: String,
    },

    /// Stop tracing
    Stop,

    /// Show current trace status
    Status,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run a debug subcommand.
pub async fn run(socket: &std::path::Path, cmd: DebugCommand) -> Result<()> {
    match cmd {
        DebugCommand::Topic { action } => match action {
            TopicAction::List => topic_list(socket).await,
            TopicAction::Echo {
                module,
                level,
                tracepoint,
            } => topic_echo(socket, module, level, tracepoint).await,
        },
        DebugCommand::Node { action } => match action {
            NodeAction::Info => node_info(socket).await,
        },
        DebugCommand::Bag { action } => match action {
            BagAction::Record {
                output,
                module,
                level,
            } => bag_record(socket, output, module, level).await,
            BagAction::Play { path, speed } => bag_play(socket, path, speed).await,
            BagAction::Info { path } => bag_info(path).await,
        },
        DebugCommand::Perf { interval } => perf_stats(socket, interval).await,
        DebugCommand::Trace { action } => match action {
            TraceAction::Start { module, level } => trace_start(socket, module, level).await,
            TraceAction::Stop => trace_stop(socket).await,
            TraceAction::Status => trace_status(socket).await,
        },
    }
}

// ---------------------------------------------------------------------------
// topic list
// ---------------------------------------------------------------------------

async fn topic_list(socket: &std::path::Path) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.topics",
        "params": {}
    });

    let response = send_rpc(socket, &request).await?;
    let topics = response["result"]["topics"]
        .as_array()
        .context("No topics in response")?;

    if topics.is_empty() {
        println!("No tracepoints registered.");
        return Ok(());
    }

    // Table header
    println!(
        "{:<30} {:<12} {:<8} {}",
        "TRACEPOINT", "MODULE", "LEVEL", "DESCRIPTION"
    );
    println!("{}", "-".repeat(80));

    for tp in topics {
        let name = tp["name"].as_str().unwrap_or("?");
        let module = tp["module"].as_str().unwrap_or("?");
        let level = tp["level"].as_str().unwrap_or("?");
        let desc = tp["description"].as_str().unwrap_or("");
        println!("{:<30} {:<12} {:<8} {}", name, module, level, desc);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// topic echo
// ---------------------------------------------------------------------------

async fn topic_echo(
    socket: &std::path::Path,
    module: Option<String>,
    level: String,
    tracepoint: Option<String>,
) -> Result<()> {
    let mut params = serde_json::json!({ "level": level });
    if let Some(m) = &module {
        params["module"] = serde_json::json!(m);
    }
    if let Some(t) = &tracepoint {
        params["tracepoint"] = serde_json::json!(t);
    }

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.subscribe",
        "params": params
    });

    // Connect and subscribe
    let mut stream = UnixStream::connect(socket).await?;
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);

    let req_str = serde_json::to_string(&request)?;
    writer.write_all(req_str.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    // Read subscription response
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let resp: serde_json::Value = serde_json::from_str(&line)?;

    if resp["error"].is_object() {
        let msg = resp["error"]["message"].as_str().unwrap_or("unknown error");
        anyhow::bail!("Subscribe failed: {}", msg);
    }

    println!("Listening for debug events (Ctrl+C to stop)...");
    println!(
        "Filter: level={}, module={:?}, tracepoint={:?}",
        level, module, tracepoint
    );
    println!("{}", "-".repeat(80));

    // Stream events until Ctrl+C
    loop {
        let mut event_line = String::new();
        match reader.read_line(&mut event_line).await {
            Ok(0) => break, // Connection closed
            Ok(_) => {}
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        }

        let event_line = event_line.trim();
        if event_line.is_empty() {
            continue;
        }

        match serde_json::from_str::<serde_json::Value>(event_line) {
            Ok(event) => {
                // Check if this is a notification (no id) with method "event"
                if event.get("method").and_then(|m| m.as_str()) == Some("event") {
                    print_event(&event["params"]);
                } else if event.get("ts").is_some() {
                    // Direct debug event
                    print_event(&event);
                }
            }
            Err(_) => {
                // Not JSON, print as-is
                println!("{}", event_line);
            }
        }
    }

    Ok(())
}

fn print_event(event: &serde_json::Value) {
    let ts = event["ts"].as_u64().unwrap_or(0);
    let secs = ts / 1000;
    let millis = ts % 1000;
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    let level = event["level"]
        .as_str()
        .or_else(|| event["type"].as_str())
        .unwrap_or("unknown");
    let module = event["module"].as_str().unwrap_or("?");
    let tracepoint = event["tracepoint"]
        .as_str()
        .or_else(|| event["name"].as_str())
        .unwrap_or("?");

    let data = if event["data"].is_object() && event["data"] != serde_json::Value::Null {
        serde_json::to_string(&event["data"]).unwrap_or_default()
    } else {
        String::new()
    };

    let level_display = format!("{:>5}", level.to_uppercase());
    println!(
        "[{:02}:{:02}:{:02}.{:03}] {} {}.{} {}",
        hours, minutes, seconds, millis, level_display, module, tracepoint, data
    );
}

// ---------------------------------------------------------------------------
// node info
// ---------------------------------------------------------------------------

async fn node_info(socket: &std::path::Path) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.node_info",
        "params": {}
    });

    let response = send_rpc(socket, &request).await?;
    let info = &response["result"]["node_info"];

    let pid = info["pid"].as_u64().unwrap_or(0);
    let uptime_human = info["uptime_human"].as_str().unwrap_or("?");
    let uptime_secs = info["uptime_secs"].as_u64().unwrap_or(0);
    let rss_kb = info["memory_rss_kb"].as_u64().unwrap_or(0);
    let tokens_in = info["tokens_in"].as_u64().unwrap_or(0);
    let tokens_out = info["tokens_out"].as_u64().unwrap_or(0);
    let turn_count = info["turn_count"].as_u64().unwrap_or(0);
    let error_count = info["error_count"].as_u64().unwrap_or(0);

    println!("=== Aletheon Daemon ===");
    println!("PID:       {}", pid);
    println!("Uptime:    {} ({}s)", uptime_human, uptime_secs);
    println!("Memory:    {} KB RSS ({:.1} MB)", rss_kb, rss_kb as f64 / 1024.0);
    println!();
    println!("=== Performance ===");
    println!("Tokens:    {} in / {} out ({} total)", tokens_in, tokens_out, tokens_in + tokens_out);
    println!("Turns:     {}", turn_count);
    println!("Errors:    {}", error_count);

    Ok(())
}

// ---------------------------------------------------------------------------
// bag record
// ---------------------------------------------------------------------------

async fn bag_record(
    socket: &std::path::Path,
    output: Option<PathBuf>,
    module: Option<String>,
    level: String,
) -> Result<()> {
    let mut params = serde_json::json!({ "level": level });
    if let Some(path) = &output {
        params["path"] = serde_json::json!(path.to_string_lossy());
    }
    if let Some(m) = &module {
        params["module"] = serde_json::json!(m);
    }

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.bag_start",
        "params": params
    });

    let response = send_rpc(socket, &request).await?;
    let recording_id = response["result"]["recording_id"]
        .as_str()
        .context("No recording_id in response")?;
    let bag_path = response["result"]["path"]
        .as_str()
        .unwrap_or("unknown");

    println!("Recording started: {}", bag_path);
    println!("Recording ID: {}", recording_id);
    println!("Press Ctrl+C to stop recording...");

    // Wait for Ctrl+C
    tokio::signal::ctrl_c().await?;

    println!("\nStopping recording...");

    let stop_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "debug.bag_stop",
        "params": { "recording_id": recording_id }
    });

    let stop_response = send_rpc(socket, &stop_request).await?;

    if stop_response["error"].is_object() {
        let msg = stop_response["error"]["message"]
            .as_str()
            .unwrap_or("unknown error");
        eprintln!("Error stopping recording: {}", msg);
    } else {
        let events = stop_response["result"]["events"].as_u64().unwrap_or(0);
        let duration = stop_response["result"]["duration_secs"]
            .as_f64()
            .unwrap_or(0.0);
        let path = stop_response["result"]["path"]
            .as_str()
            .unwrap_or(bag_path);
        println!("Recorded {} events in {:.1}s -> {}", events, duration, path);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// bag play
// ---------------------------------------------------------------------------

async fn bag_play(socket: &std::path::Path, path: PathBuf, speed: f64) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.bag_replay",
        "params": {
            "path": path.to_string_lossy(),
            "speed": speed,
        }
    });

    let response = send_rpc(socket, &request).await?;

    if response["error"].is_object() {
        let msg = response["error"]["message"].as_str().unwrap_or("unknown error");
        anyhow::bail!("Replay failed: {}", msg);
    }

    let events = response["result"]["events"].as_u64().unwrap_or(0);
    let replay_path = response["result"]["path"].as_str().unwrap_or("?");
    println!("Replayed {} events from {} (speed: {}x)", events, replay_path, speed);

    Ok(())
}

// ---------------------------------------------------------------------------
// bag info
// ---------------------------------------------------------------------------

async fn bag_info(path: PathBuf) -> Result<()> {
    let contents = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("Failed to read bag file: {}", path.display()))?;

    let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
    let event_count = lines.len();

    if event_count == 0 {
        println!("Bag file is empty: {}", path.display());
        return Ok(());
    }

    // Parse first and last event for time range
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap_or_default();
    let last: serde_json::Value = serde_json::from_str(lines[lines.len() - 1]).unwrap_or_default();

    let first_ts = first["ts"].as_u64().unwrap_or(0);
    let last_ts = last["ts"].as_u64().unwrap_or(0);
    let duration_ms = last_ts.saturating_sub(first_ts);

    // Count events per module
    let mut module_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for line in &lines {
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
            let module = event["module"].as_str().unwrap_or("unknown").to_string();
            *module_counts.entry(module).or_insert(0) += 1;
        }
    }

    // File size
    let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    println!("=== Bag Info: {} ===", path.display());
    println!("Events:    {}", event_count);
    println!("Duration:  {:.1}s", duration_ms as f64 / 1000.0);
    println!("Size:      {} bytes ({:.1} KB)", file_size, file_size as f64 / 1024.0);
    println!();
    println!("Modules:");
    let mut modules: Vec<_> = module_counts.iter().collect();
    modules.sort_by(|a, b| b.1.cmp(a.1));
    for (module, count) in modules {
        println!("  {}({})", module, count);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// perf
// ---------------------------------------------------------------------------

async fn perf_stats(socket: &std::path::Path, interval: Option<u64>) -> Result<()> {
    loop {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "debug.perf",
            "params": {}
        });

        let response = send_rpc(socket, &request).await?;
        let perf = &response["result"]["perf"];

        let tokens_in = perf["tokens_in"].as_u64().unwrap_or(0);
        let tokens_out = perf["tokens_out"].as_u64().unwrap_or(0);
        let tokens_total = perf["tokens_total"].as_u64().unwrap_or(0);
        let turn_count = perf["turn_count"].as_u64().unwrap_or(0);
        let error_count = perf["error_count"].as_u64().unwrap_or(0);
        let tool_calls = &perf["tool_calls"];

        println!("=== Performance Stats ===");
        println!(
            "Tokens:    {} in / {} out ({} total)",
            tokens_in, tokens_out, tokens_total
        );
        println!("Turns:     {}", turn_count);
        println!("Errors:    {}", error_count);

        if tool_calls.is_object() {
            if let Some(obj) = tool_calls.as_object() {
                if !obj.is_empty() {
                    println!("Tool Calls:");
                    let mut tools: Vec<_> = obj.iter().collect();
                    tools.sort_by(|a, b| {
                        b.1.as_u64().unwrap_or(0).cmp(&a.1.as_u64().unwrap_or(0))
                    });
                    for (name, count) in tools {
                        println!("  {}: {}", name, count);
                    }
                }
            }
        }

        match interval {
            Some(secs) => {
                println!();
                println!("Refreshing in {}s (Ctrl+C to stop)...", secs);
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                // Clear screen
                print!("\x1B[2J\x1B[H");
            }
            None => break,
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// trace
// ---------------------------------------------------------------------------

async fn trace_start(
    socket: &std::path::Path,
    module: Option<String>,
    level: String,
) -> Result<()> {
    let mut params = serde_json::json!({ "level": level });
    if let Some(m) = &module {
        params["module"] = serde_json::json!(m);
    }

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.trace_start",
        "params": params
    });

    let response = send_rpc(socket, &request).await?;

    if response["error"].is_object() {
        let msg = response["error"]["message"].as_str().unwrap_or("unknown error");
        anyhow::bail!("Trace start failed: {}", msg);
    }

    let tracing = response["result"]["tracing"].as_bool().unwrap_or(false);
    let resp_level = response["result"]["level"].as_str().unwrap_or("?");
    let resp_module = response["result"]["module"].as_str();

    if tracing {
        println!("Tracing started: level={}", resp_level);
        if let Some(m) = resp_module {
            println!("Module filter: {}", m);
        }
    } else {
        println!("Failed to start tracing");
    }

    Ok(())
}

async fn trace_stop(socket: &std::path::Path) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.trace_stop",
        "params": {}
    });

    let response = send_rpc(socket, &request).await?;

    if response["error"].is_object() {
        let msg = response["error"]["message"].as_str().unwrap_or("unknown error");
        anyhow::bail!("Trace stop failed: {}", msg);
    }

    println!("Tracing stopped");
    Ok(())
}

async fn trace_status(socket: &std::path::Path) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.trace_status",
        "params": {}
    });

    let response = send_rpc(socket, &request).await?;

    if response["error"].is_object() {
        let msg = response["error"]["message"].as_str().unwrap_or("unknown error");
        anyhow::bail!("Trace status failed: {}", msg);
    }

    let tracing = response["result"]["tracing"].as_bool().unwrap_or(false);
    if tracing {
        let level = response["result"]["level"].as_str().unwrap_or("?");
        let module = response["result"]["module"].as_str().unwrap_or("all");
        println!("Tracing: ON (level={}, module={})", level, module);
    } else {
        println!("Tracing: OFF");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// JSON-RPC helper
// ---------------------------------------------------------------------------

/// Send a single JSON-RPC request over the Unix socket and return the response.
async fn send_rpc(socket: &std::path::Path, request: &serde_json::Value) -> Result<serde_json::Value> {
    let mut stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("Cannot connect to daemon socket: {}", socket.display()))?;

    let req_str = serde_json::to_string(request)?;
    stream.write_all(req_str.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let (reader, _) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    let resp: serde_json::Value = serde_json::from_str(&response)
        .context("Failed to parse daemon response")?;

    Ok(resp)
}
