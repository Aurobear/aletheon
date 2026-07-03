use super::*;
use async_trait::async_trait;
use base::body::{Action, ActionResult};
use base::brain::BrainCoreOps;
use base::brain::{ExecutionResult, Experience, Observation, ReflectionEntry, ReflectionOutcome};
use base::context::Context;
use base::self_field::Intent;
use base::Subsystem;
use base::{IntentSource, SubsystemHealth};
use serde_json::json;
use std::path::PathBuf;

use super::experience_summarizer::ExperienceSummarizer;

fn make_config() -> BrainCoreConfig {
    BrainCoreConfig::default()
}

fn make_intent() -> Intent {
    Intent {
        action: "shell.execute".to_string(),
        parameters: json!({"command": "ls -la"}),
        source: IntentSource::User,
        description: "List files".to_string(),
    }
}

fn make_ctx() -> Context {
    Context::new("test_session", PathBuf::from("/tmp"))
}

#[tokio::test]
async fn think_produces_plan() {
    let bc = BrainCore::new(make_config());
    let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
    assert!(!plan.steps.is_empty());
    assert!(!plan.reasoning.is_empty());
}

#[tokio::test]
async fn think_uses_world_state() {
    let bc = BrainCore::new(make_config());
    bc.world_model().update(Observation {
        what: "disk full".to_string(),
        source: "system".to_string(),
        data: json!({"usage": "95%"}),
    });
    let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
    assert!(plan.reasoning.contains("disk full"));
}

#[tokio::test]
async fn critique_plan() {
    let bc = BrainCore::new(make_config());
    let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
    let critiques = bc.critique(&plan).await.unwrap();
    // A simple plan should have minimal critiques
    assert!(critiques
        .iter()
        .all(|c| c.severity <= base::brain::CriticismSeverity::Info));
}

#[tokio::test]
async fn reflect_on_execution() {
    let bc = BrainCore::new(make_config());
    let execution = ExecutionResult {
        plan_id: uuid::Uuid::new_v4(),
        success: true,
        steps_completed: 1,
        steps_total: 1,
        output: "done".to_string(),
        error: None,
        elapsed_ms: 100,
    };
    let reflection = bc.reflect(&execution).await.unwrap();
    assert!(!reflection.what_worked.is_empty());
    assert!(reflection.confidence > 0.5);
}

#[tokio::test]
async fn learn_from_experience() {
    let bc = BrainCore::new(make_config());
    let experience = Experience {
        action: Action {
            name: "shell.execute".to_string(),
            parameters: json!({}),
            requires_sandbox: false,
            timeout: None,
        },
        result: ActionResult {
            success: false,
            output: String::new(),
            error: Some("command not found".to_string()),
            elapsed_ms: 50,
            truncated: false,
            side_effects: vec![],
        },
        context: make_ctx(),
    };
    let rules = bc.learn(&experience).await.unwrap();
    assert!(!rules.is_empty());
    assert!(rules[0].pattern.contains("shell.execute"));
}

#[tokio::test]
async fn update_world() {
    let bc = BrainCore::new(make_config());
    let obs = Observation {
        what: "test event".to_string(),
        source: "test".to_string(),
        data: json!({"key": "value"}),
    };
    bc.update_world(&obs).await.unwrap();
    assert_eq!(bc.world_model().count(), 1);
}

#[tokio::test]
async fn subsystem_lifecycle() {
    let mut bc = BrainCore::new(make_config());
    assert_eq!(bc.name(), "brain_core");
    assert!(matches!(
        bc.health().await,
        SubsystemHealth::Degraded { .. }
    ));

    let ctx = base::SubsystemContext {
        name: "brain_core".to_string(),
        working_dir: PathBuf::from("/tmp"),
        config: json!({}),
        bus: std::sync::Arc::new(base::CommunicationBus::new()),
    };
    bc.init(&ctx).await.unwrap();
    assert!(matches!(bc.health().await, SubsystemHealth::Healthy));

    bc.shutdown().await.unwrap();
    assert!(matches!(
        bc.health().await,
        SubsystemHealth::Degraded { .. }
    ));
    assert_eq!(bc.world_model().count(), 0);
}

#[tokio::test]
async fn full_pipeline_think_critique_execute_reflect_learn() {
    let bc = BrainCore::new(make_config());

    // 1. Think
    let intent = make_intent();
    let ctx = make_ctx();
    let plan = bc.think(&intent, &ctx).await.unwrap();
    assert!(!plan.steps.is_empty());

    // 2. Critique
    let critiques = bc.critique(&plan).await.unwrap();
    // Should be clean for a simple plan
    assert!(critiques
        .iter()
        .all(|c| c.severity <= base::brain::CriticismSeverity::Warning));

    // 3. Simulate execution
    let execution = ExecutionResult {
        plan_id: plan.id,
        success: true,
        steps_completed: plan.steps.len(),
        steps_total: plan.steps.len(),
        output: "success".to_string(),
        error: None,
        elapsed_ms: 200,
    };

    // 4. Reflect
    let reflection = bc.reflect(&execution).await.unwrap();
    assert!(!reflection.what_worked.is_empty());
    assert!(reflection.confidence > 0.7);

    // 5. Learn
    let experience = Experience {
        action: plan.steps[0].action.clone(),
        result: ActionResult {
            success: true,
            output: "success".to_string(),
            error: None,
            elapsed_ms: 200,
            truncated: false,
            side_effects: vec![],
        },
        context: ctx,
    };
    let rules = bc.learn(&experience).await.unwrap();
    // Fast successful non-destructive action — no rules expected
    // (shell.execute is not destructive by name)
    // But this validates the pipeline runs without error
    let _ = rules;
}

#[tokio::test]
async fn think_with_multiple_observations() {
    let bc = BrainCore::new(make_config());

    // Add several observations
    for i in 0..5 {
        bc.world_model().update(Observation {
            what: format!("observation_{}", i),
            source: "sensor".to_string(),
            data: json!({"index": i}),
        });
    }

    let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
    // Reasoning should reference world state
    assert!(plan.reasoning.contains("observation_4") || plan.reasoning.contains("sensor"));
}

// --- ExperienceSummarizer tests ---

fn make_reflection_entry(
    outcome: ReflectionOutcome,
    task: &str,
    confidence: f64,
) -> ReflectionEntry {
    use base::ReflectionTrigger;
    ReflectionEntry {
        id: format!("ref-{}", uuid::Uuid::new_v4()),
        timestamp: chrono::Utc::now(),
        trigger: ReflectionTrigger::TaskComplete,
        task_summary: task.to_string(),
        outcome,
        what_worked: vec![],
        what_failed: vec![],
        learned: vec![],
        behavior_changes: vec![],
        confidence,
    }
}

#[test]
fn summarizer_empty_input() {
    assert!(ExperienceSummarizer::summarize(&[]).is_none());
}

#[test]
fn summarizer_single_reflection_no_pattern() {
    let entries = vec![make_reflection_entry(
        ReflectionOutcome::Success,
        "deploy feature",
        0.9,
    )];
    // Single entry with no strong pattern -> None
    assert!(ExperienceSummarizer::summarize(&entries).is_none());
}

#[test]
fn summarizer_detects_high_failure_rate() {
    let entries = vec![
        make_reflection_entry(ReflectionOutcome::Failure, "parse input", 0.2),
        make_reflection_entry(ReflectionOutcome::Failure, "parse config", 0.1),
        make_reflection_entry(ReflectionOutcome::Success, "list files", 0.9),
    ];
    let result = ExperienceSummarizer::summarize(&entries).unwrap();
    assert!(result
        .patterns_detected
        .iter()
        .any(|p| p.contains("failure rate")));
    assert!(result
        .adjustments
        .iter()
        .any(|a| a.target == "care.safety.weight"));
}

#[test]
fn summarizer_detects_repeated_topics() {
    let entries = vec![
        make_reflection_entry(ReflectionOutcome::Success, "deploy the service", 0.8),
        make_reflection_entry(ReflectionOutcome::Success, "deploy the service", 0.8),
        make_reflection_entry(ReflectionOutcome::Success, "deploy the service", 0.8),
    ];
    let result = ExperienceSummarizer::summarize(&entries).unwrap();
    assert!(result
        .patterns_detected
        .iter()
        .any(|p| p.contains("Repeated topic")));
}

#[test]
fn summarizer_detects_low_confidence() {
    let entries = vec![
        make_reflection_entry(ReflectionOutcome::Partial, "debug crash A", 0.2),
        make_reflection_entry(ReflectionOutcome::Partial, "debug crash B", 0.3),
        make_reflection_entry(ReflectionOutcome::Partial, "debug crash C", 0.1),
        make_reflection_entry(ReflectionOutcome::Partial, "debug crash D", 0.3),
    ];
    let result = ExperienceSummarizer::summarize(&entries).unwrap();
    assert!(result
        .patterns_detected
        .iter()
        .any(|p| p.contains("Low average confidence")));
    assert!(result
        .adjustments
        .iter()
        .any(|a| a.target == "care.efficiency.weight"));
}

#[test]
fn summarizer_success_strategy_with_common_lessons() {
    let mut e1 = make_reflection_entry(ReflectionOutcome::Success, "task A", 0.9);
    e1.learned = vec!["always validate inputs".to_string()];
    let mut e2 = make_reflection_entry(ReflectionOutcome::Success, "task B", 0.85);
    e2.learned = vec!["always validate inputs".to_string()];
    let entries = vec![e1, e2];

    let result = ExperienceSummarizer::summarize(&entries).unwrap();
    assert!(result
        .patterns_detected
        .iter()
        .any(|p| p.contains("Recurring lesson")));
    assert!(result
        .adjustments
        .iter()
        .any(|a| a.target == "care.learning.weight"));
}

// --- Dual-model tests ---

use crate::bridge::dual_model::{DualModelBridge, DualModelConfig, TaskComplexity};
use crate::r#impl::llm::{LlmProvider, LlmResponse, LlmStream, StopReason, ToolDefinition, Usage};
use base::message::Message;
use std::sync::Arc;

/// Stub provider whose name appears in its response text.
struct StubProvider {
    tag: String,
}

#[async_trait]
impl LlmProvider for StubProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: format!("{} response", self.tag),
            }],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        })
    }
    async fn complete_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        unimplemented!()
    }
    fn name(&self) -> &str {
        &self.tag
    }
    fn max_context_length(&self) -> usize {
        128_000
    }
}

fn make_dual_brain_core() -> BrainCore {
    let planner = LlmBridge::new(Arc::new(StubProvider {
        tag: "planner".into(),
    }));
    let executor = LlmBridge::new(Arc::new(StubProvider {
        tag: "executor".into(),
    }));
    let dm = DualModelBridge::new(planner, executor, DualModelConfig::default());
    BrainCore::new(make_config()).with_dual_model(dm)
}

#[tokio::test]
async fn dual_model_think_simple_uses_executor_only() {
    let bc = make_dual_brain_core();
    let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
    // Simple task -> executor only, so reasoning should contain "executor"
    assert!(plan.reasoning.contains("executor response"));
}

#[tokio::test]
async fn dual_model_think_complex_uses_planner_then_executor() {
    let bc = make_dual_brain_core();
    // Build a complex intent (description > 512 chars)
    let long_desc = "x".repeat(600);
    let intent = Intent {
        action: "complex.task".to_string(),
        parameters: json!({"data": "small"}),
        source: IntentSource::User,
        description: long_desc,
    };
    let plan = bc.think(&intent, &make_ctx()).await.unwrap();
    // Complex task -> two-pass: executor is the final responder
    assert!(plan.reasoning.contains("executor response"));
}

#[test]
fn estimate_complexity_simple() {
    let intent = make_intent(); // short description
    assert_eq!(
        BrainCore::estimate_complexity(&intent),
        TaskComplexity::Simple
    );
}

#[test]
fn estimate_complexity_complex() {
    let intent = Intent {
        action: "test".into(),
        parameters: json!({}),
        source: IntentSource::User,
        description: "y".repeat(600),
    };
    assert_eq!(
        BrainCore::estimate_complexity(&intent),
        TaskComplexity::Complex
    );
}

#[test]
fn estimate_complexity_medium() {
    let intent = Intent {
        action: "test".into(),
        parameters: json!({}),
        source: IntentSource::User,
        description: "z".repeat(200),
    };
    assert_eq!(
        BrainCore::estimate_complexity(&intent),
        TaskComplexity::Medium
    );
}

#[tokio::test]
async fn dual_model_fallback_single_llm() {
    // When dual_model is set but task is Simple, effective_llm returns executor
    let bc = make_dual_brain_core();
    let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
    assert!(!plan.steps.is_empty());
}

// --- P4 think_with_refinement tests ---

#[tokio::test]
async fn think_with_refinement_template_fallback() {
    let mut bc = BrainCore::new(make_config());
    let (plan, reasoning) = bc
        .think_with_refinement(&make_intent(), &make_ctx())
        .await
        .unwrap();
    // Template fallback produces a plan
    assert!(!plan.steps.is_empty());
    assert!(!reasoning.is_empty());
}

#[tokio::test]
async fn think_with_refinement_with_llm() {
    let planner_prov = LlmBridge::new(Arc::new(StubProvider {
        tag: "planner".into(),
    }));
    let executor_prov = LlmBridge::new(Arc::new(StubProvider {
        tag: "executor".into(),
    }));
    let dm = DualModelBridge::new(planner_prov, executor_prov, DualModelConfig::default());
    let mut bc = BrainCore::new(make_config()).with_dual_model(dm);
    let (plan, reasoning) = bc
        .think_with_refinement(&make_intent(), &make_ctx())
        .await
        .unwrap();
    assert!(!plan.steps.is_empty());
    // StubProvider returns "{tag} response" — reasoning should contain it
    assert!(reasoning.contains("executor response") || reasoning.contains("planner response"));
}

#[tokio::test]
async fn think_with_refinement_stops_on_no_critical() {
    // A simple read-only plan should not have critical critiques,
    // so refinement loop should exit after round 0
    let mut bc = BrainCore::new(make_config());
    let intent = Intent {
        action: "file.read".to_string(),
        parameters: json!({"path": "/tmp/test"}),
        source: IntentSource::User,
        description: "Read a file".to_string(),
    };
    let (plan, _) = bc
        .think_with_refinement(&intent, &make_ctx())
        .await
        .unwrap();
    // Should produce a valid plan
    assert!(!plan.steps.is_empty());
}

// --- P4 learner.rules_for_context test ---

use super::learner::Learner;

#[test]
fn learner_rules_for_context_matching() {
    let learner = Learner::new(100);
    // Seed some rules by learning from experiences
    let exp = make_experience_for_learner("shell.execute", false, Some("permission denied"), 100);
    learner.learn(&exp);
    let text = learner.rules_for_context("shell.execute something");
    assert!(!text.is_empty());
    assert!(text.contains("Learned rules"));
    assert!(text.contains("permission denied"));
}

#[test]
fn learner_rules_for_context_no_match() {
    let learner = Learner::new(100);
    let exp = make_experience_for_learner("shell.execute", false, Some("timeout"), 100);
    learner.learn(&exp);
    let text = learner.rules_for_context("file.read something completely different");
    // "file.read" doesn't match "shell.execute" pattern, so should be empty
    assert!(text.is_empty());
}

fn make_experience_for_learner(
    action_name: &str,
    success: bool,
    error: Option<&str>,
    elapsed_ms: u64,
) -> Experience {
    use base::body::{Action, ActionResult};
    Experience {
        action: Action {
            name: action_name.to_string(),
            parameters: json!({}),
            requires_sandbox: false,
            timeout: None,
        },
        result: ActionResult {
            success,
            output: "output".to_string(),
            error: error.map(|s| s.to_string()),
            elapsed_ms,
            truncated: false,
            side_effects: vec![],
        },
        context: make_ctx(),
    }
}
