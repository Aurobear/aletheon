//! CLI entry point — argument parsing and single-message mode.
//!
//! TUI mode delegates to [`super::ui::run`]. Single-message mode sends one
//! JSON-RPC request over the daemon socket and exits.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Default socket path for the aletheond daemon.
pub const DEFAULT_SOCKET: &str = "/tmp/aletheon/aletheon.sock";

#[derive(Parser)]
#[command(name = "aletheon-cli", about = "Aletheon CLI client")]
pub struct Args {
    /// Socket path
    #[arg(short, long, default_value = DEFAULT_SOCKET)]
    pub socket: PathBuf,

    /// Single message mode (non-interactive)
    #[arg(short, long)]
    pub message: Option<String>,

    /// Force TUI mode (default when no args)
    #[arg(long)]
    pub tui: bool,
}

/// CLI entry point — parses args and dispatches to the appropriate mode.
pub async fn run() -> Result<()> {
    let args = Args::parse();

    // Single message mode: send one message and exit
    if let Some(msg) = args.message {
        return single_message(&args.socket, &msg).await;
    }

    // Interactive mode: use the line-based TUI (IME-compatible)
    super::ui::run(args.socket.to_str().unwrap_or(DEFAULT_SOCKET)).await
}

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

    let mut response = String::new();
    reader.read_line(&mut response).await?;
    let resp: serde_json::Value = serde_json::from_str(&response)?;
    if let Some(text) = resp["result"]["response"].as_str() {
        println!("{}", text);
    } else if !resp["result"]["reflections"].is_null() {
        println!("{}", format_reflections(&resp["result"]["reflections"]));
    } else if !resp["result"]["genome"].is_null() {
        println!("{}", format_genome(&resp["result"]["genome"]));
    } else if !resp["result"]["evolution"].is_null() {
        println!("{}", format_evolution(&resp["result"]["evolution"]));
    } else if let Some(status) = resp["result"]["status"].as_object() {
        println!("{}", format_status(&resp["result"]["status"]));
    } else if let Some(err) = resp["error"]["message"].as_str() {
        eprintln!("Error: {}", err);
    }
    Ok(())
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
        let task = entry.get("task_summary").and_then(|v| v.as_str()).unwrap_or("");
        let outcome = entry.get("outcome").and_then(|v| {
            if let Some(s) = v.as_str() { Some(s.to_string()) }
            else { serde_json::to_string(v).ok() }
        }).unwrap_or_else(|| "unknown".to_string());
        let confidence = entry.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let timestamp = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");

        lines.push(format!(
            "[{}] #{} {} ({}) conf={:.0}%",
            timestamp, i + 1, task, outcome, confidence * 100.0
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
            lines.push(serde_json::to_string_pretty(entry).unwrap_or_else(|_| format!("{:?}", entry)));
            lines.push(String::new());
        }
        return lines.join("\n");
    }
    serde_json::to_string_pretty(evo).unwrap_or_else(|_| format!("{:?}", evo))
}

/// Format status response for display.
fn format_status(status: &serde_json::Value) -> String {
    let session_id = status.get("session_id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let turn_count = status.get("turn_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let reflection_count = status.get("reflection_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let evolution_count = status.get("evolution_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let boundary_rules = status.get("boundary_rules").and_then(|v| v.as_u64()).unwrap_or(0);
    let boundary_immutable = status.get("boundary_immutable").and_then(|v| v.as_u64()).unwrap_or(0);
    let attention_focus = status.get("attention_focus").and_then(|v| v.as_str()).unwrap_or("");

    let mut lines = Vec::new();
    lines.push("=== Aletheon Status ===".to_string());
    lines.push(format!("Session: {}", &session_id[..8.min(session_id.len())]));
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
    lines.push(format!("Boundary Rules: {} (immutable: {})", boundary_rules, boundary_immutable));

    let focus_display = if attention_focus.is_empty() { "none" } else { attention_focus };
    lines.push(format!("Attention Focus: {}", focus_display));

    lines.join("\n")
}
