//! Self-Evolution EventBus Loop Demo
//!
//! Demonstrates the full closed loop:
//! 1. Engine executes tool -> emits ToolObservationEvent
//! 2. BrainCore reflects via LLM -> emits ReflectionEvent
//! 3. After batch -> extracts rules -> emits RuleExtractedEvent
//! 4. After consecutive failures -> emits EvolutionTriggeredEvent
//!
//! The demo works without real LLM calls: if DEEPSEEK_API_KEY is not set,
//! the LLM reflection step will fail gracefully and the event flow is still
//! demonstrated through the EventBus wiring.

use std::sync::Arc;

use anyhow::Result;
use uuid::Uuid;

use base::evolution::*;
use base::EventBus;
use cognit::r#impl::event_handlers::{EvolutionEvent, ObserverConfig, ToolObservationHandler};
use cognit::r#impl::llm::scheduler::{
    LlmScheduler, RoutingRule, SchedulerConfig, SchedulerProviderConfig,
};
use base::events::event::{ConcreteEvent, EventType, Priority};
use base::KernelEventBus;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an LlmScheduler configured for DeepSeek (reflector).
fn build_scheduler() -> Result<Arc<LlmScheduler>> {
    let api_key = std::env::var("DEEPSEEK_API_KEY").unwrap_or_default();

    let config = SchedulerConfig {
        providers: vec![SchedulerProviderConfig {
            name: "deepseek".to_string(),
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key,
            kind: "openai".to_string(),
            model: "deepseek-chat".to_string(),
        }],
        routing: vec![
            RoutingRule {
                purpose: LlmPurpose::Reflect,
                provider_name: "deepseek".to_string(),
            },
            RoutingRule {
                purpose: LlmPurpose::ExtractRules,
                provider_name: "deepseek".to_string(),
            },
            RoutingRule {
                purpose: LlmPurpose::GenerateMutations,
                provider_name: "deepseek".to_string(),
            },
        ],
    };

    Ok(Arc::new(LlmScheduler::new(&config)?))
}

/// Create a ConcreteEvent carrying a ToolObservationPayload.
///
/// The payload is boxed as `serde_json::Value` so that `to_json()` works
/// correctly for async handler delivery through the EventBus.
fn make_tool_observation_event(obs: &ToolObservationPayload) -> Box<ConcreteEvent> {
    let json_value = serde_json::to_value(obs).unwrap_or(serde_json::Value::Null);
    Box::new(ConcreteEvent::new(
        EventType::ToolObservation,
        Priority::Normal,
        "demo-engine".to_string(),
        Box::new(json_value),
    ))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    println!("=== Self-Evolution EventBus Loop Demo ===\n");

    // Check API key early
    let has_api_key = !std::env::var("DEEPSEEK_API_KEY")
        .unwrap_or_default()
        .is_empty();

    if !has_api_key {
        println!("Note: DEEPSEEK_API_KEY not set.");
        println!("LLM reflection calls will fail, but the event flow is still demonstrated.");
        println!("Set DEEPSEEK_API_KEY to enable full LLM reflection.\n");
    }

    // -----------------------------------------------------------------------
    // 1. Create EventBus
    // -----------------------------------------------------------------------
    let bus = Arc::new(KernelEventBus::new(10_000));
    println!("[Init] KernelEventBus created (log_capacity=10000)");

    // -----------------------------------------------------------------------
    // 2. Create LLM Scheduler
    // -----------------------------------------------------------------------
    let scheduler = build_scheduler()?;
    println!("[Init] LlmScheduler created");

    // -----------------------------------------------------------------------
    // 3. Create ToolObservationHandler (BrainCore reflection)
    // -----------------------------------------------------------------------
    let observer_config = ObserverConfig {
        batch_size: 3,
        consecutive_failure_threshold: 3,
        confidence_drop_threshold: 0.2,
    };
    let observer = Arc::new(ToolObservationHandler::new(
        scheduler.clone(),
        observer_config,
    ));
    println!("[Init] ToolObservationHandler created (batch_size=3, failure_threshold=3)");

    // -----------------------------------------------------------------------
    // 4. Subscribe handler to EventBus via subscribe_async
    // -----------------------------------------------------------------------
    let observer_for_handler = observer.clone();
    bus.subscribe_async(
        EventType::ToolObservation,
        Box::new(move |json: serde_json::Value| {
            let obs = observer_for_handler.clone();
            Box::pin(async move {
                match serde_json::from_value::<ToolObservationPayload>(json) {
                    Ok(payload) => {
                        println!(
                            "\n  [Handler] Processing tool observation: tool={}, error={:?}",
                            payload.tool_name, payload.error
                        );
                        match obs.handle(&payload).await {
                            Ok(events) => {
                                for event in &events {
                                    match event {
                                        EvolutionEvent::Reflection(r) => {
                                            println!(
                                                "  [Reflection] {:?}: confidence={:.2}, cause={}",
                                                r.assessment,
                                                r.confidence,
                                                r.root_cause.as_deref().unwrap_or("(none)")
                                            );
                                            if let Some(ref rule) = r.suggested_rule {
                                                println!(
                                                    "    Suggested rule: IF {} THEN {}",
                                                    rule.condition, rule.action
                                                );
                                            }
                                        }
                                        EvolutionEvent::RuleExtracted(rules) => {
                                            println!(
                                                "  [Rules Extracted] {} rules from {} reflections:",
                                                rules.rules.len(),
                                                rules.source_reflections.len()
                                            );
                                            for rule in &rules.rules {
                                                println!(
                                                    "    - IF {} THEN {} (conf: {:.2})",
                                                    rule.condition, rule.action, rule.confidence
                                                );
                                            }
                                        }
                                        EvolutionEvent::EvolutionTriggered(trigger) => {
                                            println!(
                                                "  [Evolution Triggered] reason={}, reflections={}",
                                                trigger.trigger_reason,
                                                trigger.recent_reflections.len()
                                            );
                                            println!(
                                                "    Current rules snapshot: {} rules",
                                                trigger.current_rules_snapshot.len()
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                println!("  [Handler Error] {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        println!("  [Deserialization Error] {}", e);
                    }
                }
                true
            })
        }),
    )
    .await?;
    println!("[Init] ToolObservationHandler subscribed to ToolObservation events\n");

    // -----------------------------------------------------------------------
    // 5. Simulate 4 tool observations (1 success + 3 failures)
    // -----------------------------------------------------------------------
    let observations = vec![
        ToolObservationPayload {
            turn_id: Uuid::new_v4(),
            tool_name: "bash_exec".to_string(),
            input: serde_json::json!({"command": "cat /var/log/syslog | head -100"}),
            output: serde_json::json!("Jan  1 00:00:01 host kernel: ...\nJan  1 00:00:02 host sshd[123]: ..."),
            duration_ms: 150,
            error: None,
            rules_applied: vec![],
        },
        ToolObservationPayload {
            turn_id: Uuid::new_v4(),
            tool_name: "bash_exec".to_string(),
            input: serde_json::json!({"command": "sort /var/log/syslog"}),
            output: serde_json::json!("error: permission denied"),
            duration_ms: 50,
            error: Some("permission denied".to_string()),
            rules_applied: vec![],
        },
        ToolObservationPayload {
            turn_id: Uuid::new_v4(),
            tool_name: "bash_exec".to_string(),
            input: serde_json::json!({"command": "grep ERROR /var/log/syslog"}),
            output: serde_json::json!("error: file not found"),
            duration_ms: 30,
            error: Some("file not found".to_string()),
            rules_applied: vec![],
        },
        ToolObservationPayload {
            turn_id: Uuid::new_v4(),
            tool_name: "bash_exec".to_string(),
            input: serde_json::json!({"command": "tail -50 /var/log/syslog"}),
            output: serde_json::json!("error: timeout"),
            duration_ms: 30000,
            error: Some("timeout".to_string()),
            rules_applied: vec![],
        },
    ];

    println!("--- Emitting {} tool observations ---", observations.len());

    for (i, obs) in observations.iter().enumerate() {
        println!(
            "\n[Turn {}] Tool: {}, Error: {:?}, Duration: {}ms",
            i + 1,
            obs.tool_name,
            obs.error,
            obs.duration_ms
        );

        let event = make_tool_observation_event(obs);
        bus.publish(event).await?;

        // Small delay to let async handlers process
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    // Give async handlers time to finish
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    println!("\n=== Demo Complete ===");
    println!("Event flow: ToolObservation -> BrainCore Reflection -> Rule Extraction -> Evolution Trigger");
    println!("In a full system, this would continue to SelfField validation and MetaRuntime morphogenesis.");

    Ok(())
}
