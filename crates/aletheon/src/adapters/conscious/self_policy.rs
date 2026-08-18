use std::sync::Arc;

use async_trait::async_trait;

#[async_trait]
pub trait SelfPolicyPort: Send + Sync {
    async fn review(
        &self,
        intent: &dasein::Intent,
        context: &contracts::Context,
    ) -> anyhow::Result<dasein::Verdict>;
    async fn narrate(&self, event: &str, reason: &str);
    async fn coordinate(&self, turn: usize, output: &str, status: contracts::dasein::OutcomeStatus);
    fn dasein_context_provider(&self) -> Arc<dyn Fn() -> Option<String> + Send + Sync>;
}

pub async fn review_turn_intent(
    policy: &dyn SelfPolicyPort,
    message: &str,
    session_id: &str,
    workspace: &std::path::Path,
    unrestricted_filesystem: bool,
) -> Result<contracts::SandboxRequirement, application::turn::outcome::TurnPipelineRejection> {
    let preview_end = message
        .char_indices()
        .nth(80)
        .map(|(index, _)| index)
        .unwrap_or(message.len());
    let intent = dasein::Intent {
        action: "chat".to_string(),
        parameters: serde_json::json!({"message": message}),
        source: dasein::IntentSource::User,
        description: format!("User chat message: {}", &message[..preview_end]),
    };
    let context = contracts::Context::new(session_id, workspace.to_path_buf());

    let sandbox = match policy.review(&intent, &context).await {
        Ok(dasein::Verdict::Deny { reason }) => {
            tracing::warn!(reason = %reason, "SelfField denied chat intent");
            policy.narrate("chat_denied", &reason).await;
            return Err(
                application::turn::outcome::TurnPipelineRejection::IntentDeniedBySelfField {
                    reason,
                },
            );
        }
        Ok(dasein::Verdict::SandboxFirst { reason }) => {
            tracing::warn!(reason = %reason, "SelfField requires sandbox; tools will be gated through admission");
            policy.narrate("chat_sandbox_required", &reason).await;
            contracts::SandboxRequirement::Required
        }
        Err(error) => {
            tracing::warn!(error = %error, "SelfField review error — denying turn (fail-closed)");
            return Err(
                application::turn::outcome::TurnPipelineRejection::SelfFieldReviewFailed {
                    reason: error.to_string(),
                },
            );
        }
        _ => contracts::SandboxRequirement::NotRequired,
    };

    Ok(if unrestricted_filesystem {
        contracts::SandboxRequirement::NotRequired
    } else {
        sandbox
    })
}

pub async fn complete_turn(
    policy: &dyn SelfPolicyPort,
    turn_count: usize,
    input: &str,
    output: &str,
    stop: &contracts::TurnStop,
    execution_returned: bool,
) {
    let status = match stop {
        contracts::TurnStop::Completed if execution_returned => {
            contracts::dasein::OutcomeStatus::Succeeded
        }
        contracts::TurnStop::Cancelled => contracts::dasein::OutcomeStatus::Cancelled,
        _ => contracts::dasein::OutcomeStatus::Failed,
    };
    policy.coordinate(turn_count, output, status).await;

    let preview_end = input
        .char_indices()
        .nth(60)
        .map_or(input.len(), |(index, _)| index);
    policy
        .narrate(
            "chat_completed",
            &format!(
                "User asked: '{}...' | Response: {} chars",
                &input[..preview_end],
                output.len(),
            ),
        )
        .await;
}
