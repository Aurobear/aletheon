//! `aletheon workflow` subcommand — manage persisted workflow DAGs.
//!
//! Subcommands:
//!   save <name> <path>  — save a WorkflowDef JSON file to the daemon store
//!   load <name>         — load and display a saved workflow
//!   list                — list saved workflow names
//!   delete <name>       — delete a saved workflow
//!   run <name>          — run a saved workflow

use anyhow::{Context, Result};
use clap::Subcommand;
use std::path::PathBuf;

use super::rpc_client::send_rpc;

// ---------------------------------------------------------------------------
// Subcommand definitions
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum WorkflowAction {
    /// Save a workflow definition from a JSON file
    Save {
        /// Workflow name
        name: String,
        /// Path to a JSON file containing a WorkflowDef
        path: PathBuf,
    },
    /// Load and display a saved workflow
    Load {
        /// Workflow name
        name: String,
    },
    /// List saved workflow names
    List,
    /// Delete a saved workflow
    Delete {
        /// Workflow name
        name: String,
    },
    /// Run a saved workflow
    Run {
        /// Workflow name
        name: String,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run a workflow subcommand.
pub async fn run(socket: &std::path::Path, action: WorkflowAction) -> Result<()> {
    match action {
        WorkflowAction::Save { name, path } => cmd_save(socket, &name, &path).await,
        WorkflowAction::Load { name } => cmd_load(socket, &name).await,
        WorkflowAction::List => cmd_list(socket).await,
        WorkflowAction::Delete { name } => cmd_delete(socket, &name).await,
        WorkflowAction::Run { name } => cmd_run(socket, &name).await,
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

async fn cmd_save(socket: &std::path::Path, name: &str, path: &PathBuf) -> Result<()> {
    let json_text = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read workflow file: {}", path.display()))?;
    let def: serde_json::Value =
        serde_json::from_str(&json_text).context("File is not valid JSON")?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "workflow.save",
        "params": { "name": name, "def": def }
    });

    let response = send_rpc(socket, &request).await?;

    if let Some(err) = response.get("error") {
        eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
    } else {
        println!("Workflow '{}' saved.", name);
    }
    Ok(())
}

async fn cmd_load(socket: &std::path::Path, name: &str) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "workflow.load",
        "params": { "name": name }
    });

    let response = send_rpc(socket, &request).await?;

    if let Some(err) = response.get("error") {
        eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&response["result"]["def"]).unwrap_or_default()
        );
    }
    Ok(())
}

async fn cmd_list(socket: &std::path::Path) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "workflow.list",
        "params": {}
    });

    let response = send_rpc(socket, &request).await?;

    if let Some(err) = response.get("error") {
        eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
    } else if let Some(names) = response["result"]["names"].as_array() {
        if names.is_empty() {
            println!("No saved workflows.");
        } else {
            for n in names {
                println!("{}", n.as_str().unwrap_or("?"));
            }
        }
    }
    Ok(())
}

async fn cmd_delete(socket: &std::path::Path, name: &str) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "workflow.delete",
        "params": { "name": name }
    });

    let response = send_rpc(socket, &request).await?;

    if let Some(err) = response.get("error") {
        eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
    } else {
        println!("Workflow '{}' deleted.", name);
    }
    Ok(())
}

async fn cmd_run(socket: &std::path::Path, name: &str) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "workflow.run",
        "params": { "name": name }
    });

    let response = send_rpc(socket, &request).await?;

    if let Some(err) = response.get("error") {
        eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
    } else if let Some(result) = response["result"]["state"].as_object() {
        println!(
            "{}",
            serde_json::to_string_pretty(result).unwrap_or_default()
        );
    } else {
        println!("Workflow '{}' completed.", name);
    }
    Ok(())
}
