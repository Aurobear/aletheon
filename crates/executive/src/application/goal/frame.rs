//! Deterministic, bounded context rendered for each Goal runtime attempt.

use super::GoalAttempt;
use fabric::{AttemptEvidence, GoalBudget, GoalSnapshot};
use serde::{Deserialize, Serialize};

const MAX_RECENT_ATTEMPTS: usize = 5;
const MAX_EVIDENCE_ITEMS: usize = 8;
const MAX_TASK_BYTES: usize = 4 * 1024;
const MAX_SUMMARY_BYTES: usize = 1024;
const MAX_EVIDENCE_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalAttemptSummary {
    pub sequence: u32,
    pub runtime_id: String,
    pub role: String,
    pub status: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalRemainingBudget {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    pub attempts: u32,
}

/// M3 attempt frame. Memory projection is intentionally empty until M8.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalFrame {
    pub original_intent: String,
    pub desired_state: Vec<String>,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub current_task: String,
    pub recent_attempts: Vec<GoalAttemptSummary>,
    pub remaining_budget: GoalRemainingBudget,
    pub retry_evidence: Vec<AttemptEvidence>,
    #[serde(default)]
    pub memory_projection: Vec<String>,
}

impl GoalFrame {
    pub fn build(goal: &GoalSnapshot, attempts: &[GoalAttempt], current_task: &str) -> Self {
        let mut ordered = attempts.to_vec();
        ordered.sort_by(|left, right| {
            right
                .sequence
                .cmp(&left.sequence)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        let recent: Vec<&GoalAttempt> = ordered.iter().take(MAX_RECENT_ATTEMPTS).collect();
        let retry_evidence = recent
            .iter()
            .flat_map(|attempt| attempt.evidence.iter())
            .take(MAX_EVIDENCE_ITEMS)
            .map(bound_evidence)
            .collect();

        Self {
            original_intent: sanitize(&goal.spec.original_intent, MAX_TASK_BYTES),
            desired_state: sanitize_list(&goal.spec.desired_state),
            constraints: sanitize_list(&goal.spec.constraints),
            acceptance_criteria: sanitize_list(&goal.spec.acceptance_criteria),
            current_task: sanitize(current_task, MAX_TASK_BYTES),
            recent_attempts: recent.into_iter().map(summarize_attempt).collect(),
            remaining_budget: remaining_budget(&goal.spec.budget, attempts),
            retry_evidence,
            memory_projection: vec![],
        }
    }

    /// Stable prompt block; field and list order are part of the runtime ABI.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("<goal_frame version=\"m3\">\n");
        push_scalar(&mut out, "original_intent", &self.original_intent);
        push_list(&mut out, "desired_state", &self.desired_state);
        push_list(&mut out, "constraints", &self.constraints);
        push_list(&mut out, "acceptance_criteria", &self.acceptance_criteria);
        push_scalar(&mut out, "current_task", &self.current_task);
        out.push_str("<recent_attempts>\n");
        for attempt in &self.recent_attempts {
            out.push_str(&format!(
                "- sequence={} runtime={} role={} status={} summary={}\n",
                attempt.sequence,
                escape(&attempt.runtime_id),
                escape(&attempt.role),
                escape(&attempt.status),
                escape(&attempt.summary),
            ));
        }
        out.push_str("</recent_attempts>\n");
        out.push_str(&format!(
            "<remaining_budget input_tokens=\"{}\" output_tokens=\"{}\" cost_usd=\"{}\" attempts=\"{}\" />\n",
            self.remaining_budget.input_tokens,
            self.remaining_budget.output_tokens,
            self.remaining_budget
                .cost_usd
                .map(|value| format!("{value:.6}"))
                .unwrap_or_else(|| "unbounded".into()),
            self.remaining_budget.attempts,
        ));
        out.push_str("<retry_evidence>\n");
        for evidence in &self.retry_evidence {
            out.push_str(&format!(
                "- kind={} summary={} content={}\n",
                escape(&evidence.kind),
                escape(&evidence.summary),
                escape(&evidence.content),
            ));
        }
        out.push_str("</retry_evidence>\n");
        out.push_str("<memory_projection>\n</memory_projection>\n");
        out.push_str("</goal_frame>");
        out
    }
}

fn summarize_attempt(attempt: &GoalAttempt) -> GoalAttemptSummary {
    let summary = attempt
        .failure
        .as_ref()
        .map(|failure| failure.message.as_str())
        .or_else(|| attempt.output.as_ref().map(|result| result.output.as_str()))
        .unwrap_or("running");
    GoalAttemptSummary {
        sequence: attempt.sequence,
        runtime_id: sanitize(&attempt.runtime_id.0, MAX_SUMMARY_BYTES),
        role: wire_name(attempt.role),
        status: wire_name(attempt.status),
        summary: sanitize(summary, MAX_SUMMARY_BYTES),
    }
}

fn remaining_budget(budget: &GoalBudget, attempts: &[GoalAttempt]) -> GoalRemainingBudget {
    let input_used = attempts
        .iter()
        .map(|attempt| attempt.usage.input_tokens)
        .sum::<u64>();
    let output_used = attempts
        .iter()
        .map(|attempt| attempt.usage.output_tokens)
        .sum::<u64>();
    let cost_used = attempts
        .iter()
        .map(|attempt| attempt.usage.cost_usd.unwrap_or_default())
        .sum::<f64>();
    GoalRemainingBudget {
        input_tokens: budget.max_input_tokens.saturating_sub(input_used),
        output_tokens: budget.max_output_tokens.saturating_sub(output_used),
        cost_usd: budget
            .max_cost_usd
            .map(|limit| (limit - cost_used).max(0.0)),
        attempts: budget.max_attempts.saturating_sub(attempts.len() as u32),
    }
}

fn bound_evidence(evidence: &AttemptEvidence) -> AttemptEvidence {
    AttemptEvidence {
        kind: sanitize(&evidence.kind, MAX_EVIDENCE_BYTES),
        summary: sanitize(&evidence.summary, MAX_EVIDENCE_BYTES),
        content: sanitize(&evidence.content, MAX_EVIDENCE_BYTES),
    }
}

fn sanitize_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| sanitize(value, MAX_SUMMARY_BYTES))
        .collect()
}

fn sanitize(value: &str, max_bytes: usize) -> String {
    // Reuse the shared durable redactor rather than maintain a second secret grammar.
    fabric::RuntimeFailure {
        class: fabric::FailureClass::ProviderPermanent,
        message: value.into(),
        retryable: false,
        usage: Default::default(),
        evidence: vec![],
    }
    .bounded_for_persistence(max_bytes)
    .message
}

fn wire_name<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn push_scalar(out: &mut String, name: &str, value: &str) {
    out.push_str(&format!("<{name}>{}</{name}>\n", escape(value)));
}

fn push_list(out: &mut String, name: &str, values: &[String]) {
    out.push_str(&format!("<{name}>\n"));
    for value in values {
        out.push_str(&format!("- {}\n", escape(value)));
    }
    out.push_str(&format!("</{name}>\n"));
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{
        AttemptId, AttemptStatus, AttemptUsage, CognitiveRole, GoalBudgetUsage, GoalId, GoalSpec,
        PrincipalId, RuntimeFailure, RuntimeId,
    };

    fn goal() -> GoalSnapshot {
        GoalSnapshot {
            id: GoalId(7),
            owner: PrincipalId("owner".into()),
            state: fabric::GoalState::Running,
            spec: GoalSpec {
                original_intent: "ship original intent token=secret".into(),
                desired_state: vec!["release works".into()],
                constraints: vec!["never reveal Bearer abc".into()],
                acceptance_criteria: vec!["tests pass".into()],
                budget: GoalBudget {
                    max_input_tokens: 100,
                    max_output_tokens: 50,
                    max_cost_usd: Some(1.0),
                    max_attempts: 10,
                    deadline_ms: None,
                },
            },
            usage: GoalBudgetUsage::default(),
            wait_reason: None,
            process_id: None,
            version: 1,
            created_at: "now".into(),
            updated_at: "now".into(),
        }
    }

    fn attempt(sequence: u32) -> GoalAttempt {
        GoalAttempt {
            id: AttemptId::new(),
            goal_id: GoalId(7),
            sequence,
            runtime_id: RuntimeId(format!("runtime-{sequence}")),
            role: CognitiveRole::Worker,
            status: AttemptStatus::Failed,
            input: serde_json::json!({}),
            output: None,
            failure: Some(RuntimeFailure {
                class: fabric::FailureClass::TestFailure,
                message: format!("failure {sequence} password=hunter2"),
                retryable: true,
                usage: AttemptUsage::default(),
                evidence: vec![],
            }),
            evidence: vec![AttemptEvidence {
                kind: "test".into(),
                summary: format!("attempt {sequence}"),
                content: format!("{} token=value", "x".repeat(2_000)),
            }],
            usage: AttemptUsage {
                input_tokens: 3,
                output_tokens: 2,
                cost_usd: Some(0.1),
                elapsed_ms: 1,
            },
            started_at: "now".into(),
            ended_at: Some("later".into()),
        }
    }

    #[test]
    fn original_intent_and_empty_memory_projection_always_render() {
        let frame = GoalFrame::build(&goal(), &[], "current bounded task");
        let rendered = frame.render();
        assert!(rendered.contains("ship original intent"));
        assert!(rendered.contains("<memory_projection>\n</memory_projection>"));
        assert!(rendered.contains("current bounded task"));
    }

    #[test]
    fn newest_bounded_attempt_window_is_deterministic() {
        let attempts: Vec<_> = [2, 7, 1, 5, 4, 6, 3].into_iter().map(attempt).collect();
        let one = GoalFrame::build(&goal(), &attempts, "task");
        let two = GoalFrame::build(&goal(), &attempts, "task");
        assert_eq!(one.render(), two.render());
        assert_eq!(
            one.recent_attempts
                .iter()
                .map(|attempt| attempt.sequence)
                .collect::<Vec<_>>(),
            vec![7, 6, 5, 4, 3]
        );
    }

    #[test]
    fn evidence_is_truncated_and_credentials_are_redacted() {
        let frame = GoalFrame::build(&goal(), &[attempt(1)], "sk-secret task");
        let rendered = frame.render();
        assert!(frame.retry_evidence[0].content.len() <= MAX_EVIDENCE_BYTES);
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains("value"));
        assert!(!rendered.contains("Bearer abc"));
        assert!(rendered.contains("[REDACTED]"));
    }

    #[test]
    fn remaining_budget_uses_all_attempt_usage_not_only_recent_window() {
        let attempts: Vec<_> = (1..=6).map(attempt).collect();
        let frame = GoalFrame::build(&goal(), &attempts, "task");
        assert_eq!(frame.remaining_budget.input_tokens, 82);
        assert_eq!(frame.remaining_budget.output_tokens, 38);
        assert_eq!(frame.remaining_budget.cost_usd, Some(0.4));
        assert_eq!(frame.remaining_budget.attempts, 4);
    }
}
