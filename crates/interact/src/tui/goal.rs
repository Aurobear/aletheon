//! CLI handlers for the `aletheon goal` subcommand.
//!
//! Each action sends a JSON-RPC request over the daemon Unix socket
//! and prints the result. Mirrors `debug.rs`'s `send_rpc` + `run` pattern.

use std::path::PathBuf;

use anyhow::Result;
use fabric::protocol::client::ClientRpcRequest;

use super::cli::GoalAction;
use super::rpc_client::send_rpc;

/// Entry point dispatched from `handle_command`.
pub async fn run(socket: &PathBuf, action: GoalAction) -> Result<()> {
    let req = match &action {
        GoalAction::Set { description, scope } => ClientRpcRequest::goal_set(description, scope),
        GoalAction::Show { id } => ClientRpcRequest::goal_show(*id),
        GoalAction::Status { id, state, filter } => {
            ClientRpcRequest::goal_status(*id, state.clone(), filter.clone())
        }
        GoalAction::Resume => ClientRpcRequest::GoalResume,
    }
    .to_json_rpc(Some(1))?;

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
            obj.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
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
        println!("Objective created: id={oid}");
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
