//! Provider-neutral accumulation of canonical evidence from a Turn event stream.

use crate::data_governance::ContentTrust;
use contracts::ipc::TurnEventV1;
use contracts::ItemPayload;
use std::sync::Arc;

pub struct ToolTerminalEvidence {
    pub name: String,
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

pub struct TurnEvidenceAccumulator {
    pub tool_calls: Vec<(String, String, serde_json::Value)>,
    pub tool_results: Vec<(String, String, bool)>,
    pub items: Vec<ItemPayload>,
    pub usage: super::usage::TurnUsageAccumulator,
}

impl Default for TurnEvidenceAccumulator {
    fn default() -> Self {
        Self {
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            items: Vec::new(),
            usage: super::usage::TurnUsageAccumulator::new(),
        }
    }
}

impl TurnEvidenceAccumulator {
    #[allow(clippy::too_many_arguments)]
    pub async fn settle_post_turn(
        &mut self,
        effects: &dyn super::post_turn::TurnPostEffectsPort,
        observability: &dyn super::ports::TurnObservabilityPort,
        sessions: &dyn super::ports::TurnSessionStatePort,
        session_id: &str,
        current_turn_count: usize,
        input: &str,
        result: &contracts::TurnResult,
        succeeded: bool,
        model: Arc<dyn contracts::LlmProvider>,
        profile: super::settings::ResolvedTurnProfile,
        context_costs: super::context::TurnContextBudgetCosts,
    ) -> anyhow::Result<usize> {
        effects
            .complete_policy(current_turn_count, input, result, succeeded)
            .await;
        let (tokens_in, tokens_out) = self.usage.token_totals();
        observability.record_turn(tokens_in, tokens_out);
        let turn = self
            .finish_session(
                sessions,
                session_id,
                succeeded,
                &result.output,
                model,
                profile,
                context_costs,
            )
            .await?;
        effects.observe_assistant(&result.output).await;
        effects.after_terminal(succeeded).await?;
        Ok(turn)
    }

    pub async fn finish_session(
        &mut self,
        sessions: &dyn super::ports::TurnSessionStatePort,
        session_id: &str,
        succeeded: bool,
        output: &str,
        model: Arc<dyn contracts::LlmProvider>,
        profile: super::settings::ResolvedTurnProfile,
        context_costs: super::context::TurnContextBudgetCosts,
    ) -> anyhow::Result<usize> {
        let finish = sessions
            .finish(
                session_id,
                succeeded,
                &self.tool_calls,
                &self.tool_results,
                output,
                model,
                profile,
                context_costs,
            )
            .await?;
        self.items
            .extend(finish.compactions.into_iter().map(|projection| {
                ItemPayload::ContextCompactionProjection {
                    projection: Box::new(projection),
                }
            }));
        Ok(finish.turn_count)
    }

    /// Merge evidence produced outside the canonical event stream and make the
    /// returned runtime result account from the same canonical item set.
    pub fn settle_result(
        &mut self,
        result: &mut contracts::TurnResult,
        inference_items: impl IntoIterator<Item = ItemPayload>,
        capability_receipts: &[contracts::CapabilityTerminalReceipt],
    ) {
        self.items.extend(inference_items);

        let mut receipts = capability_receipts.to_vec();
        receipts.sort_by(|left, right| {
            left.finished_at
                .cmp(&right.finished_at)
                .then_with(|| left.invocation_id.cmp(&right.invocation_id))
        });
        for receipt in receipts {
            let already_recorded = self.items.iter().any(|item| {
                matches!(item, ItemPayload::CapabilityReceipt { receipt: existing }
                    if existing.invocation_id == receipt.invocation_id)
            });
            if !already_recorded {
                self.items.push(ItemPayload::CapabilityReceipt { receipt });
            }
        }

        result.usage =
            contracts::InferenceUsage::aggregate(self.items.iter().filter_map(|item| match item {
                ItemPayload::InferenceReceipt { receipt } => Some(&receipt.usage),
                _ => None,
            }));
    }

    pub fn observe_live(&mut self, event: &TurnEventV1) -> Option<ToolTerminalEvidence> {
        match event {
            TurnEventV1::ToolCallStart { name, call_id } => {
                self.tool_calls
                    .push((call_id.clone(), name.clone(), serde_json::Value::Null));
            }
            TurnEventV1::ToolCallComplete { call_id, args, .. } => {
                if let Some(call) = self.tool_calls.iter_mut().find(|(id, _, _)| id == call_id) {
                    let input = crate::data_governance::scrub_json_for_projection(
                        args,
                        ContentTrust::ToolUntrusted,
                    );
                    call.2 = input.clone();
                    self.items.push(ItemPayload::ToolCall {
                        call_id: call_id.clone(),
                        name: call.1.clone(),
                        input,
                    });
                }
            }
            TurnEventV1::ToolResult {
                name,
                call_id,
                content,
                is_error,
                ..
            } => {
                let projected = crate::data_governance::scrub_for_projection(
                    content,
                    ContentTrust::ToolUntrusted,
                )
                .content;
                self.tool_results
                    .push((call_id.clone(), projected.clone(), *is_error));
                self.items.push(ItemPayload::ToolResult {
                    call_id: call_id.clone(),
                    content: projected,
                    is_error: *is_error,
                    permit_id: None,
                    audit_id: None,
                });
                return Some(ToolTerminalEvidence {
                    name: name.clone(),
                    call_id: call_id.clone(),
                    content: content.clone(),
                    is_error: *is_error,
                });
            }
            _ => self.observe_common(event),
        }
        None
    }

    pub fn observe_drain(&mut self, event: &TurnEventV1) {
        self.observe_common(event);
    }

    fn observe_common(&mut self, event: &TurnEventV1) {
        match event {
            TurnEventV1::RobotEpisodeSettled { receipt } => {
                self.items.push(ItemPayload::RobotEpisodeReceipt {
                    receipt: receipt.clone(),
                });
            }
            TurnEventV1::Usage { usage } => self.usage.observe(usage),
            TurnEventV1::ContextUpdate { used_tokens, .. } => {
                self.usage.observe_active_context((*used_tokens).into());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlement_aggregates_inference_usage() {
        let mut evidence = TurnEvidenceAccumulator::default();
        let inference = contracts::types::inference_receipt::InferenceTerminalReceipt {
            schema_version:
                contracts::types::inference_receipt::INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1,
            inference_id: "inference-1".into(),
            operation_id: "operation-1".into(),
            provider_id: "provider".into(),
            model_id: "model".into(),
            system_prefix_digest: "system".into(),
            tool_schema_digest: "tools".into(),
            status: contracts::types::inference_receipt::InferenceTerminalStatus::Succeeded,
            usage: contracts::InferenceUsage {
                total_input_tokens: Some(7),
                output_tokens: Some(3),
                ..Default::default()
            },
            context_capacity_tokens: None,
            active_context_occupancy_tokens: None,
            failure_kind: None,
            prefix_shape_digest: None,
            local_cache_miss_reason: None,
        };
        let mut result = contracts::TurnResult {
            output: String::new(),
            stop: contracts::TurnStop::Completed,
            failure: None,
            usage: Default::default(),
            metrics: Default::default(),
        };

        evidence.settle_result(
            &mut result,
            [ItemPayload::InferenceReceipt { receipt: inference }],
            &[],
        );

        assert_eq!(result.usage.total_input_tokens, Some(7));
        assert_eq!(result.usage.output_tokens, Some(3));
    }
}
