//! Basic Agent Example
//!
//! Minimal example demonstrating how to use `aletheon-runtime` to create
//! an agent that processes user input through the ReAct loop.
//!
//! Run with:  cargo run -p basic-agent-example

use anyhow::Result;
use executive::{AletheonRuntime, RuntimeConfig};
use fabric::body::{Action, ActionResult};
use fabric::brain::{CostEstimate, Plan, PlanStep};
use fabric::context::Context;
use fabric::self_field::{Intent, RiskLevel, Verdict};
use std::path::PathBuf;

/// Stub: policy review -- always approves in this example.
fn review(_intent: &Intent, _ctx: &Context) -> Result<Verdict> {
    Ok(Verdict::Allow)
}

/// Stub: reasoning -- returns a trivial single-step plan.
fn think(intent: &Intent, _ctx: &Context) -> Result<Plan> {
    let action = Action {
        name: "echo".to_string(),
        parameters: serde_json::json!({ "message": intent.description }),
        requires_sandbox: false,
        timeout: None,
    };
    let step = PlanStep {
        id: uuid::Uuid::new_v4(),
        action,
        depends_on: vec![],
        expected_outcome: "Agent echoes the user input".to_string(),
        rollback_action: None,
    };
    Ok(Plan {
        id: uuid::Uuid::new_v4(),
        steps: vec![step],
        estimated_cost: CostEstimate::default(),
        risk_level: RiskLevel::None,
        reasoning: "Simple echo response".to_string(),
        alternatives: vec![],
    })
}

/// Stub: tool execution -- echoes the action back as text.
fn execute(action: &Action, _ctx: &Context) -> Result<ActionResult> {
    let msg = action
        .parameters
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("(no message)");
    Ok(ActionResult {
        success: true,
        output: format!("[echo] {}", msg),
        error: None,
        elapsed_ms: 0,
        truncated: false,
        side_effects: vec![],
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let config = RuntimeConfig::default();
    let mut runtime = AletheonRuntime::new(config);
    let ctx = Context::new("basic-agent-demo", PathBuf::from("."));

    let user_input = "Hello, Aletheon! What can you do?";
    tracing::info!("User input: {}", user_input);

    let response = runtime
        .process(user_input, &ctx, review, think, execute)
        .await?;
    println!("Agent response:\n{}", response);

    Ok(())
}
