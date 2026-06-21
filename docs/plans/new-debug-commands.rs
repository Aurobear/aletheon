// New debug commands to add to debug_handler.rs and debug.rs
// These will be merged after the bug fix agent completes.

// ============================================================================
// debug_handler.rs additions
// ============================================================================

// Add to handle_method match:
//   "debug.health" => Some(self.handle_health(id).await),
//   "debug.nodes" => Some(self.handle_nodes(id).await),
//   "debug.param_get" => Some(self.handle_param_get(id, params).await),
//   "debug.param_set" => Some(self.handle_param_set(id, params).await),
//   "debug.param_list" => Some(self.handle_param_list(id).await),

// --- health ---
async fn handle_health(&self, id: &Value) -> Value {
    let perf = self.perf.snapshot();
    let tool_calls = self.perf.tool_calls.lock().await.clone();
    let uptime = self.started_at.elapsed();
    let rss_kb = read_rss_kb().unwrap_or(0);
    let sub_count = self.subscribers.lock().await.len();
    let rec_count = self.recordings.lock().await.len();

    // Determine overall status
    let overall = if perf.error_count == 0 { "HEALTHY" } else { "DEGRADED" };

    let mut warnings = Vec::new();
    if perf.error_count > 0 {
        warnings.push(format!("{} errors recorded", perf.error_count));
    }
    if rss_kb > 500_000 {
        warnings.push(format!("High memory usage: {} MB", rss_kb / 1024));
    }

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "health": {
                "overall": overall,
                "pid": std::process::id(),
                "uptime_secs": uptime.as_secs(),
                "uptime_human": format_duration(uptime),
                "memory_rss_mb": rss_kb / 1024,
                "tokens_in": perf.tokens_in,
                "tokens_out": perf.tokens_out,
                "turn_count": perf.turn_count,
                "error_count": perf.error_count,
                "tool_calls": tool_calls,
                "active_subscribers": sub_count,
                "active_recordings": rec_count,
                "warnings": warnings,
            }
        }
    })
}

// --- nodes (placeholder — needs Observable wiring) ---
async fn handle_nodes(&self, id: &Value) -> Value {
    // For now, report the daemon itself as the only "node"
    // TODO: wire up Observable instances from RequestHandler
    let perf = self.perf.snapshot();

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "nodes": [
                {
                    "name": "daemon",
                    "running": true,
                    "status_line": format!("uptime={}, turns={}", format_duration(self.started_at.elapsed()), perf.turn_count),
                    "details": {
                        "pid": std::process::id(),
                        "tokens_in": perf.tokens_in,
                        "tokens_out": perf.tokens_out,
                        "error_count": perf.error_count,
                    }
                }
            ]
        }
    })
}

// ============================================================================
// debug.rs (CLI) additions
// ============================================================================

// Add to DebugCommand enum:
//   /// Unified health dashboard
//   Health,
//   /// List running subsystems
//   Nodes,
//   /// Runtime parameter inspection
//   Param {
//       #[command(subcommand)]
//       action: ParamAction,
//   },

// #[derive(Subcommand)]
// pub enum ParamAction {
//     /// Get a parameter value
//     Get { key: String },
//     /// Set a parameter value
//     Set { key: String, value: String },
//     /// List all parameters
//     List,
// }

// Add to run() match:
//   DebugCommand::Health => health_dashboard(socket).await,
//   DebugCommand::Nodes => nodes_list(socket).await,
//   DebugCommand::Param { action } => match action {
//       ParamAction::Get { key } => param_get(socket, key).await,
//       ParamAction::Set { key, value } => param_set(socket, key, value).await,
//       ParamAction::List => param_list(socket).await,
//   },

// --- health dashboard ---
async fn health_dashboard(socket: &std::path::Path) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.health",
        "params": {}
    });

    let response = send_rpc(socket, &request).await?;
    let health = &response["result"]["health"];

    let overall = health["overall"].as_str().unwrap_or("UNKNOWN");
    let icon = if overall == "HEALTHY" { "✅" } else { "⚠️" };

    println!("{} Overall: {} (all subsystems)", icon, overall);
    println!();
    println!("  PID:       {}", health["pid"].as_u64().unwrap_or(0));
    println!("  Uptime:    {}", health["uptime_human"].as_str().unwrap_or("?"));
    println!("  Memory:    {} MB", health["memory_rss_mb"].as_u64().unwrap_or(0));
    println!("  Tokens:    {} in / {} out", health["tokens_in"].as_u64().unwrap_or(0), health["tokens_out"].as_u64().unwrap_or(0));
    println!("  Turns:     {}", health["turn_count"].as_u64().unwrap_or(0));
    println!("  Errors:    {}", health["error_count"].as_u64().unwrap_or(0));
    println!("  Subscribers: {}", health["active_subscribers"].as_u64().unwrap_or(0));
    println!("  Recordings:  {}", health["active_recordings"].as_u64().unwrap_or(0));

    if let Some(tools) = health["tool_calls"].as_object() {
        if !tools.is_empty() {
            println!();
            println!("  Tool Calls:");
            let mut sorted: Vec<_> = tools.iter().collect();
            sorted.sort_by(|a, b| b.1.as_u64().unwrap_or(0).cmp(&a.1.as_u64().unwrap_or(0)));
            for (name, count) in sorted {
                println!("    {}: {}", name, count);
            }
        }
    }

    if let Some(warnings) = health["warnings"].as_array() {
        if !warnings.is_empty() {
            println!();
            println!("  Warnings:");
            for w in warnings {
                println!("    ⚠️  {}", w.as_str().unwrap_or(""));
            }
        }
    }

    Ok(())
}

// --- nodes list ---
async fn nodes_list(socket: &std::path::Path) -> Result<()> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug.nodes",
        "params": {}
    });

    let response = send_rpc(socket, &request).await?;
    let nodes = response["result"]["nodes"].as_array().context("No nodes in response")?;

    if nodes.is_empty() {
        println!("No subsystems registered.");
        return Ok(());
    }

    println!("{:<20} {:<10} {}", "NAME", "STATUS", "DETAILS");
    println!("{}", "-".repeat(60));
    for node in nodes {
        let name = node["name"].as_str().unwrap_or("?");
        let running = node["running"].as_bool().unwrap_or(false);
        let status = if running { "running" } else { "stopped" };
        let details = node["status_line"].as_str().unwrap_or("");
        println!("{:<20} {:<10} {}", name, status, details);
    }

    Ok(())
}
