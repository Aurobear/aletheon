//! Subscribes to ToolObservationEvent, uses LLM to reflect on tool execution results.
//!
//! Emits:
//! - ReflectionEvent (after each tool call)
//! - RuleExtractedEvent (when batch threshold reached)
//! - EvolutionTriggeredEvent (when evolution conditions met)

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use uuid::Uuid;

use fabric::evolution::*;
use fabric::message::{ContentBlock, Message, Role};

use crate::adapters::inference::scheduler::LlmScheduler;

/// Configuration for the tool observation handler.
#[derive(Debug, Clone)]
pub struct ObserverConfig {
    /// Number of reflections before extracting rules.
    pub batch_size: usize,
    /// Consecutive failures to trigger evolution.
    pub consecutive_failure_threshold: usize,
    /// Confidence drop threshold (0.0-1.0).
    pub confidence_drop_threshold: f64,
}

impl Default for ObserverConfig {
    fn default() -> Self {
        Self {
            batch_size: 3,
            consecutive_failure_threshold: 3,
            confidence_drop_threshold: 0.2,
        }
    }
}

/// Handles ToolObservationEvent: reflects via LLM, extracts rules, triggers evolution.
pub struct ToolObservationHandler {
    scheduler: Arc<LlmScheduler>,
    config: ObserverConfig,
    reflection_buffer: Mutex<Vec<ReflectionPayload>>,
    consecutive_failures: AtomicUsize,
    last_confidence: AtomicU64,
}

impl ToolObservationHandler {
    pub fn new(scheduler: Arc<LlmScheduler>, config: ObserverConfig) -> Self {
        Self {
            scheduler,
            config,
            reflection_buffer: Mutex::new(Vec::new()),
            consecutive_failures: AtomicUsize::new(0),
            last_confidence: AtomicU64::new(1.0_f64.to_bits()),
        }
    }

    /// Process a tool observation. Returns events to emit.
    pub async fn handle(&self, obs: &ToolObservationPayload) -> Result<Vec<EvolutionEvent>> {
        let mut events = Vec::new();

        // 1. LLM reflection
        let reflection = self.reflect(obs).await?;
        events.push(EvolutionEvent::Reflection(reflection.clone()));

        // 2. Track consecutive failures
        match reflection.assessment {
            Assessment::Failure => {
                self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                self.consecutive_failures.store(0, Ordering::Relaxed);
            }
        }

        // 3. Buffer reflection
        let mut buffer = self.reflection_buffer.lock().await;
        buffer.push(reflection.clone());

        // 4. Check batch threshold for rule extraction
        if buffer.len() >= self.config.batch_size {
            let rules = self.extract_rules(&buffer).await?;
            if !rules.is_empty() {
                events.push(EvolutionEvent::RuleExtracted(RuleExtractedPayload {
                    source_reflections: buffer.iter().map(|r| r.turn_id).collect(),
                    rules: rules.clone(),
                }));
            }

            // 5. Check evolution trigger conditions
            let failures = self.consecutive_failures.load(Ordering::Relaxed);
            let should_trigger = failures >= self.config.consecutive_failure_threshold;
            let confidence_dropped = {
                let last = f64::from_bits(self.last_confidence.load(Ordering::Relaxed));
                last - reflection.confidence > self.config.confidence_drop_threshold
            };

            if should_trigger || confidence_dropped {
                let reason = if should_trigger {
                    "consecutive_failures".to_string()
                } else {
                    "confidence_drop".to_string()
                };
                events.push(EvolutionEvent::EvolutionTriggered(
                    EvolutionTriggeredPayload {
                        trigger_reason: reason,
                        recent_reflections: buffer.iter().map(|r| r.turn_id).collect(),
                        current_rules_snapshot: rules,
                    },
                ));
            }

            // Update confidence tracking
            self.last_confidence
                .store(reflection.confidence.to_bits(), Ordering::Relaxed);

            // Clear buffer after processing
            buffer.clear();
        }

        Ok(events)
    }

    /// Use LLM to reflect on a tool observation.
    async fn reflect(&self, obs: &ToolObservationPayload) -> Result<ReflectionPayload> {
        let prompt = format!(
            r#"You are analyzing a tool execution result for a self-evolving agent.

Tool: {tool}
Input: {input}
Output: {output}
Duration: {duration}ms
Error: {error}

Respond with JSON:
{{
  "assessment": "Success" | "PartialSuccess" | "Failure",
  "root_cause": "string or null",
  "suggested_rule": {{"condition": "...", "action": "..."}} or null,
  "confidence": 0.0-1.0
}}"#,
            tool = obs.tool_name,
            input = serde_json::to_string_pretty(&obs.input).unwrap_or_default(),
            output = serde_json::to_string_pretty(&obs.output).unwrap_or_default(),
            duration = obs.duration_ms,
            error = obs.error.as_deref().unwrap_or("none"),
        );

        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }];

        let response = self
            .scheduler
            .complete(&LlmPurpose::Reflect, &messages, &[])
            .await?;

        let text = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        self.parse_reflection(obs.turn_id, &text)
    }

    /// Parse LLM response into ReflectionPayload.
    fn parse_reflection(&self, turn_id: Uuid, text: &str) -> Result<ReflectionPayload> {
        // Try JSON parse
        let json: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);

        let assessment = match json["assessment"].as_str().unwrap_or("Failure") {
            "Success" => Assessment::Success,
            "PartialSuccess" => Assessment::PartialSuccess,
            _ => Assessment::Failure,
        };

        let suggested_rule = json["suggested_rule"].as_object().map(|obj| LearnedRule {
            id: Uuid::new_v4(),
            condition: obj["condition"].as_str().unwrap_or_default().to_string(),
            action: obj["action"].as_str().unwrap_or_default().to_string(),
            confidence: json["confidence"].as_f64().unwrap_or(0.5),
            source_reflections: vec![turn_id],
        });

        Ok(ReflectionPayload {
            turn_id,
            assessment,
            root_cause: json["root_cause"].as_str().map(String::from),
            suggested_rule,
            confidence: json["confidence"].as_f64().unwrap_or(0.5),
        })
    }

    /// Extract generalized rules from a batch of reflections.
    async fn extract_rules(&self, reflections: &[ReflectionPayload]) -> Result<Vec<LearnedRule>> {
        let summary = reflections
            .iter()
            .enumerate()
            .map(|(i, r)| {
                format!(
                    "{}. {:?}: {} (confidence: {:.2})",
                    i + 1,
                    r.assessment,
                    r.root_cause.as_deref().unwrap_or("no cause"),
                    r.confidence
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            r#"Analyze these reflections from a self-evolving agent and extract generalized rules.

Reflections:
{summary}

Respond with JSON array of rules:
[
  {{"condition": "when X happens", "action": "do Y", "confidence": 0.0-1.0}}
]

Only extract rules that are genuinely useful. Return empty array [] if no patterns found."#,
            summary = summary,
        );

        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }];

        let response = self
            .scheduler
            .complete(&LlmPurpose::ExtractRules, &messages, &[])
            .await?;

        let text = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        let json: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Array(vec![]));

        let rules = json
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                Some(LearnedRule {
                    id: Uuid::new_v4(),
                    condition: r["condition"].as_str()?.to_string(),
                    action: r["action"].as_str()?.to_string(),
                    confidence: r["confidence"].as_f64().unwrap_or(0.5),
                    source_reflections: reflections.iter().map(|r| r.turn_id).collect(),
                })
            })
            .collect();

        Ok(rules)
    }
}

/// Events emitted by ToolObservationHandler.
pub enum EvolutionEvent {
    Reflection(ReflectionPayload),
    RuleExtracted(RuleExtractedPayload),
    EvolutionTriggered(EvolutionTriggeredPayload),
}
