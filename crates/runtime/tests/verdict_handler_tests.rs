//! Integration tests for the VerdictHandler → AletheonRuntime wiring.
//!
//! Each test verifies that a specific SelfField verdict type is correctly
//! dispatched through `process_react()`.

use base::context::Context;
use base::message::{ContentBlock, Message};
use base::self_field::{Intent, RiskLevel, Verdict};
use base::ToolDefinition;
use cognit::r#impl::llm::provider::{
    LlmProvider, LlmResponse, LlmStream, StopReason, Usage,
};
use runtime::{AletheonRuntime, DefaultVerdictHandler, RuntimeConfig};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn test_ctx() -> Context {
    Context::new("test-session", PathBuf::from("/tmp"))
}

fn test_config() -> RuntimeConfig {
    RuntimeConfig {
        max_iterations: 5,
        session_id: "test".to_string(),
        learning_enabled: false,
        compaction_enabled: false,
        ..RuntimeConfig::default()
    }
}

/// A minimal LLM that just returns a text response immediately.
struct SimpleLlm;

#[async_trait]
impl LlmProvider for SimpleLlm {
    async fn complete(
        &self,
        _m: &[Message],
        _t: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: "LLM executed".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        })
    }

    async fn complete_stream(
        &self,
        _m: &[Message],
        _t: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!("not used in test")
    }

    fn name(&self) -> &str {
        "simple"
    }

    fn max_context_length(&self) -> usize {
        100_000
    }
}

/// Helper: build a review function that always returns the given verdict.
fn fixed_verdict(verdict: Verdict) -> impl Fn(&Intent, &Context) -> anyhow::Result<Verdict> {
    move |_: &Intent, _: &Context| Ok(verdict.clone())
}

#[tokio::test]
async fn allow_verdict_proceeds_to_llm() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();
    let tool_defs: Vec<ToolDefinition> = vec![];

    let (output, metrics) = runtime
        .process_react(
            "hello",
            &ctx,
            fixed_verdict(Verdict::Allow),
            &SimpleLlm,
            &tool_defs,
            |_id, _name, _input| async { ("ok".into(), false) },
        )
        .await
        .unwrap();

    assert_eq!(output, "LLM executed", "Allow should proceed to LLM");
    assert!(metrics.completed_normally);
}

#[tokio::test]
async fn allow_with_modification_proceeds_to_llm() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();
    let tool_defs: Vec<ToolDefinition> = vec![];
    let verdict = Verdict::AllowWithModification {
        modification: json!({"tone": "gentler"}),
    };

    let (output, metrics) = runtime
        .process_react(
            "speak",
            &ctx,
            fixed_verdict(verdict),
            &SimpleLlm,
            &tool_defs,
            |_id, _name, _input| async { ("ok".into(), false) },
        )
        .await
        .unwrap();

    assert_eq!(output, "LLM executed", "AllowWithModification should proceed");
    assert!(metrics.completed_normally);
}

#[tokio::test]
async fn deny_verdict_short_circuits() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();
    let tool_defs: Vec<ToolDefinition> = vec![];
    let verdict = Verdict::Deny {
        reason: "forbidden action".to_string(),
    };

    let (output, metrics) = runtime
        .process_react(
            "delete everything",
            &ctx,
            fixed_verdict(verdict),
            &SimpleLlm,
            &tool_defs,
            |_id, _name, _input| async { ("ok".into(), false) },
        )
        .await
        .unwrap();

    assert!(
        output.contains("Denied by SelfField"),
        "Should be denial: {}",
        output
    );
    assert!(output.contains("forbidden action"));
    assert!(!metrics.completed_normally, "Deny should not complete normally");
}

#[tokio::test]
async fn require_confirmation_with_approving_callback() {
    let handler = Arc::new(DefaultVerdictHandler::with_confirm_callback(Box::new(
        |_, _| true,
    )));
    let mut runtime = AletheonRuntime::new(test_config()).with_verdict_handler(handler);
    let ctx = test_ctx();
    let tool_defs: Vec<ToolDefinition> = vec![];
    let verdict = Verdict::RequireConfirmation {
        reason: "risky operation".to_string(),
        risk_level: RiskLevel::High,
    };

    let (output, metrics) = runtime
        .process_react(
            "deploy",
            &ctx,
            fixed_verdict(verdict),
            &SimpleLlm,
            &tool_defs,
            |_id, _name, _input| async { ("ok".into(), false) },
        )
        .await
        .unwrap();

    assert_eq!(
        output, "LLM executed",
        "Approved confirmation should proceed to LLM"
    );
    assert!(metrics.completed_normally);
}

#[tokio::test]
async fn require_confirmation_with_denying_callback() {
    let handler = Arc::new(DefaultVerdictHandler::with_confirm_callback(Box::new(
        |_, _| false,
    )));
    let mut runtime = AletheonRuntime::new(test_config()).with_verdict_handler(handler);
    let ctx = test_ctx();
    let tool_defs: Vec<ToolDefinition> = vec![];
    let verdict = Verdict::RequireConfirmation {
        reason: "risky operation".to_string(),
        risk_level: RiskLevel::High,
    };

    let (output, metrics) = runtime
        .process_react(
            "deploy",
            &ctx,
            fixed_verdict(verdict),
            &SimpleLlm,
            &tool_defs,
            |_id, _name, _input| async { ("ok".into(), false) },
        )
        .await
        .unwrap();

    assert!(
        output.contains("User declined"),
        "Denied confirmation: {}",
        output
    );
    assert!(!metrics.completed_normally);
}

#[tokio::test]
async fn require_confirmation_without_callback_short_circuits() {
    // Default handler has no confirm_callback -> auto-deny
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();
    let tool_defs: Vec<ToolDefinition> = vec![];
    let verdict = Verdict::RequireConfirmation {
        reason: "needs approval".to_string(),
        risk_level: RiskLevel::Medium,
    };

    let (output, metrics) = runtime
        .process_react(
            "deploy",
            &ctx,
            fixed_verdict(verdict),
            &SimpleLlm,
            &tool_defs,
            |_id, _name, _input| async { ("ok".into(), false) },
        )
        .await
        .unwrap();

    assert!(
        output.contains("no handler"),
        "No-callback confirmation: {}",
        output
    );
    assert!(!metrics.completed_normally);
}

#[tokio::test]
async fn sandbox_first_proceeds_to_llm() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();
    let tool_defs: Vec<ToolDefinition> = vec![];
    let verdict = Verdict::SandboxFirst {
        reason: "untested behavior".to_string(),
    };

    // SandboxThenProceed currently logs a warning and proceeds to LLM.
    let (output, metrics) = runtime
        .process_react(
            "try something new",
            &ctx,
            fixed_verdict(verdict),
            &SimpleLlm,
            &tool_defs,
            |_id, _name, _input| async { ("ok".into(), false) },
        )
        .await
        .unwrap();

    assert_eq!(
        output, "LLM executed",
        "SandboxFirst should proceed to LLM (sandbox not yet wired)"
    );
    assert!(metrics.completed_normally);
}

#[tokio::test]
async fn delay_verdict_short_circuits() {
    let mut runtime = AletheonRuntime::new(test_config());
    let ctx = test_ctx();
    let tool_defs: Vec<ToolDefinition> = vec![];
    let verdict = Verdict::Delay {
        reason: "rate limited".to_string(),
        until: "cooldown period".to_string(),
    };

    let (output, metrics) = runtime
        .process_react(
            "api call",
            &ctx,
            fixed_verdict(verdict),
            &SimpleLlm,
            &tool_defs,
            |_id, _name, _input| async { ("ok".into(), false) },
        )
        .await
        .unwrap();

    assert!(
        output.contains("Delayed"),
        "Delay should short-circuit: {}",
        output
    );
    assert!(output.contains("rate limited"));
    assert!(!metrics.completed_normally);
}
