//! CLI handlers for the `aletheon goal` subcommand.
//!
//! Each action sends a JSON-RPC request over the daemon Unix socket
//! and prints the result. Mirrors `debug.rs`'s `send_rpc` + `run` pattern.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::cli::GoalAction;

/// Entry point dispatched from `handle_command`.
pub async fn run(socket: &PathBuf, action: GoalAction) -> Result<()> {
    let req = match &action {
        GoalAction::Set {
            description,
            scope,
        } => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "goal.set",
            "params": { "description": description, "scope": scope }
        }),
        GoalAction::Show { id } => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "goal.show",
            "params": { "id": id }
        }),
        GoalAction::Status { id, state, filter } => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "goal.status",
            "params": { "id": id, "status": state, "filter": filter }
        }),
        GoalAction::Resume => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "goal.resume",
            "params": {}
        }),
    };

    let resp = send_rpc(socket, &req).await?;

    // Pretty-print the result
    if let Some(objs) = resp["result"]["objectives"].as_array() {
        for o in objs {
            println!(
                "[{}] ({}) {}",
                o["objective_id"],
                o["status"].as_str().unwrap_or("?"),
                o["description"].as_str().unwrap_or("")
            );
        }
    } else if let Some(obj) = resp["result"]["objective"].as_object() {
        println!(
            "[{}] ({}) {}",
            obj.get("objective_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            obj.get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
            obj.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        );
        if let Some(subs) = resp["result"]["sub_goals"].as_array() {
            if !subs.is_empty() {
                println!("Sub-goals:");
                for s in subs {
                    println!(
                        "  [{}] ({}) {}",
                        s["objective_id"],
                        s["status"].as_str().unwrap_or("?"),
                        s["description"].as_str().unwrap_or("")
                    );
                }
            }
        }
    } else if resp["result"]["objective"].is_null() {
        println!("No active objective.");
    } else if let Some(oid) = resp["result"]["objective_id"].as_i64() {
        println!("Objective created: id={}", oid);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&resp["result"]).unwrap_or_default()
        );
    }

    if let Some(err) = resp["error"].as_object() {
        eprintln!(
            "Error: {}",
            err.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
        );
    }

    Ok(())
}

/// Local copy of `debug.rs:1194`'s `send_rpc` — that function is private.
/// TODO: if `debug::send_rpc` is made `pub(crate)`, collapse this duplicate.
async fn send_rpc(
    socket: &std::path::Path,
    request: &serde_json::Value,
) -> Result<serde_json::Value> {
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
