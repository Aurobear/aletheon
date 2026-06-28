use base::body::{Action, ActionResult};
use base::brain::{CostEstimate, Plan, PlanStep};
use base::context::Context;
use base::self_field::{Intent, RiskLevel, Verdict};
use runtime::{AletheonRuntime, RuntimeConfig};
use std::path::PathBuf;

fn test_ctx() -> Context {
    Context::new("test-session", PathBuf::from("/tmp"))
}

fn test_config() -> RuntimeConfig {
    RuntimeConfig {
        max_iterations: 10,
        session_id: "test".to_string(),
        learning_enabled: false,
        compaction_enabled: false,
        ..RuntimeConfig::default()
    }
}

fn success_action_result(output: &str) -> ActionResult {
    ActionResult {
        success: true,
        output: output.to_string(),
        error: None,
        elapsed_ms: 10,
        truncated: false,
        side_effects: vec![],
    }
}

fn make_plan(steps: usize) -> Plan {
    let plan_steps = (0..steps)
        .map(|i| PlanStep {
            id: uuid::Uuid::new_v4(),
            action: Action {
                name: format!("tool_{}", i),
                parameters: serde_json::json!({}),
                requires_sandbox: false,
                timeout: None,
            },
            depends_on: vec![],
            expected_outcome: "ok".to_string(),
            rollback_action: None,
        })
        .collect();

    Plan {
        id: uuid::Uuid::new_v4(),
        steps: plan_steps,
        estimated_cost: CostEstimate {
            estimated_tokens: 100,
            estimated_time_ms: 1000,
            estimated_tool_calls: steps,
        },
        risk_level: RiskLevel::Low,
        reasoning: "test reasoning".to_string(),
        alternatives: vec![],
    }
}

/// Test the full cognitive path: Allow verdict -> think -> plan -> execute steps.
#[tokio::test]
async fn test_process_cognitive_path() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review =
        |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> { Ok(Verdict::Allow) };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> { Ok(make_plan(1)) };

    let execute = |action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        Ok(success_action_result(&format!("{}: done", action.name)))
    };

    let result = runtime
        .process("test input", &ctx, review, think, execute)
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "tool_0: done");
}

/// Test Deny verdict: routes to Reflex path, which executes the intent action directly.
/// The think_fn should NOT be called.
#[tokio::test]
async fn test_process_denied_reflex() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> {
        Ok(Verdict::Deny {
            reason: "not allowed".to_string(),
        })
    };

    // Should NOT be called: Deny routes to Reflex, skipping BrainCore
    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> {
        panic!("think should not be called when verdict is Deny");
    };

    // Reflex path calls execute_fn with an action derived from the intent
    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        Ok(success_action_result("reflex response"))
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "reflex response");
}

/// Test max_iterations: plan has more steps than allowed, execution should stop early.
#[tokio::test]
async fn test_max_iterations() {
    let config = RuntimeConfig {
        max_iterations: 2,
        ..test_config()
    };
    let mut runtime = AletheonRuntime::new(config);
    let ctx = test_ctx();

    let review =
        |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> { Ok(Verdict::Allow) };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> { Ok(make_plan(5)) };

    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        Ok(success_action_result("ok"))
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_ok());
    // Should have stopped at 2 iterations (max_iterations)
    assert_eq!(runtime.iteration(), 2);
}

/// Test step failure: when a step fails, execution should stop (even if more steps remain).
#[tokio::test]
async fn test_process_step_failure_stops() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review =
        |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> { Ok(Verdict::Allow) };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> { Ok(make_plan(3)) };

    let call_count = std::sync::atomic::AtomicUsize::new(0);
    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 1 {
            // Second step fails
            Ok(ActionResult {
                success: false,
                output: "step failed".to_string(),
                error: Some("tool error".to_string()),
                elapsed_ms: 5,
                truncated: false,
                side_effects: vec![],
            })
        } else {
            Ok(success_action_result("ok"))
        }
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_ok());
    // First step succeeded ("ok"), second step failed ("step failed"), third step not executed
    let output = result.unwrap();
    assert!(output.contains("ok"));
    assert!(output.contains("step failed"));
    // Only 2 steps should have been attempted (iteration advanced once before the failure)
    assert_eq!(runtime.iteration(), 1);
}

/// Test multiple steps produce concatenated output.
#[tokio::test]
async fn test_process_multiple_steps_concatenated() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review =
        |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> { Ok(Verdict::Allow) };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> { Ok(make_plan(3)) };

    let call_count = std::sync::atomic::AtomicUsize::new(0);
    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(success_action_result(&format!("step_{}", n)))
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("step_0"));
    assert!(output.contains("step_1"));
    assert!(output.contains("step_2"));
    assert_eq!(runtime.iteration(), 3);
}

/// Test Volitional path: SandboxFirst verdict routes through think -> execute (same as Cognitive).
#[tokio::test]
async fn test_process_volitional_path() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> {
        Ok(Verdict::SandboxFirst {
            reason: "untested tool".to_string(),
        })
    };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> { Ok(make_plan(1)) };

    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        Ok(success_action_result("sandboxed execution"))
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "sandboxed execution");
}

/// Test review error propagates as Err.
#[tokio::test]
async fn test_process_review_error() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> {
        anyhow::bail!("review system failure")
    };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> {
        panic!("should not be called");
    };
    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        panic!("should not be called");
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("review system failure"));
}

/// Test think error propagates as Err.
#[tokio::test]
async fn test_process_think_error() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review =
        |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> { Ok(Verdict::Allow) };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> {
        anyhow::bail!("planning failed")
    };

    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        panic!("should not be called if think fails");
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("planning failed"));
}

/// Test execute error: the step loop should break on Err from execute_fn.
#[tokio::test]
async fn test_process_execute_error_breaks_loop() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review =
        |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> { Ok(Verdict::Allow) };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> { Ok(make_plan(3)) };

    let call_count = std::sync::atomic::AtomicUsize::new(0);
    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            anyhow::bail!("execution error")
        } else {
            Ok(success_action_result("ok"))
        }
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    // execute Err breaks the loop but doesn't propagate the error to the caller
    // (the loop just breaks and returns whatever was collected so far)
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.is_empty()); // No successful outputs collected
}

/// Test rollback is attempted when a step fails and has a rollback_action.
#[tokio::test]
async fn test_process_rollback_on_failure() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review =
        |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> { Ok(Verdict::Allow) };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> {
        let mut plan = make_plan(1);
        plan.steps[0].rollback_action = Some(Action {
            name: "rollback_tool".to_string(),
            parameters: serde_json::json!({}),
            requires_sandbox: false,
            timeout: None,
        });
        Ok(plan)
    };

    let call_count = std::sync::atomic::AtomicUsize::new(0);
    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            // First call: the main step fails
            Ok(ActionResult {
                success: false,
                output: "failed".to_string(),
                error: Some("step error".to_string()),
                elapsed_ms: 5,
                truncated: false,
                side_effects: vec![],
            })
        } else {
            // Second call: the rollback
            Ok(success_action_result("rolled back"))
        }
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_ok());
    // 2 calls: main step + rollback
    assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// Test process with empty plan (0 steps) returns empty string.
#[tokio::test]
async fn test_process_empty_plan() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();

    let review =
        |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Verdict> { Ok(Verdict::Allow) };

    let think = |_intent: &Intent, _ctx: &Context| -> anyhow::Result<Plan> { Ok(make_plan(0)) };

    let execute = |_action: &Action, _ctx: &Context| -> anyhow::Result<ActionResult> {
        panic!("should not be called for empty plan")
    };

    let result = runtime.process("test", &ctx, review, think, execute).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "");
}
