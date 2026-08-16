//! Per-turn adaptive context budgeting.
//!
//! The planner deliberately keeps non-history costs explicit so callers cannot
//! accidentally compare conversation tokens with the model's whole context
//! window.

use ::contracts::{
    ContextCostTokens, HistoryBudgetTokens, HistoryTokens, ModelContextWindowTokens,
    ProfileInputLimitTokens,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudgetInput {
    pub model_context_window: ModelContextWindowTokens,
    pub profile_input_limit: ProfileInputLimitTokens,
    pub system_and_skill_prefix_tokens: ContextCostTokens,
    pub tool_schema_tokens: ContextCostTokens,
    pub reserved_output_tokens: ContextCostTokens,
    pub pending_user_input_tokens: HistoryTokens,
    pub safety_margin_tokens: ContextCostTokens,
    pub current_history_tokens: HistoryTokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetAction {
    None,
    SoftCompact,
    HardCompact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudgetPlan {
    pub effective_context_window: ProfileInputLimitTokens,
    pub history_budget: HistoryBudgetTokens,
    pub soft_watermark: HistoryBudgetTokens,
    pub projected_history_tokens: HistoryTokens,
    pub action: BudgetAction,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ContextBudgetPlanner;

impl ContextBudgetPlanner {
    pub fn plan(input: ContextBudgetInput) -> ContextBudgetPlan {
        let effective_context_window = input
            .model_context_window
            .get()
            .min(input.profile_input_limit.get());
        let fixed_cost = input
            .system_and_skill_prefix_tokens
            .get()
            .saturating_add(input.tool_schema_tokens.get())
            .saturating_add(input.reserved_output_tokens.get())
            .saturating_add(input.safety_margin_tokens.get());
        let history_budget = effective_context_window.saturating_sub(fixed_cost).max(1);
        let projected_history_tokens = input
            .current_history_tokens
            .get()
            .saturating_add(input.pending_user_input_tokens.get());

        // Preserve enough space for approximately one additional normal turn.
        // The reserve adapts to both the available history budget and the
        // profile's output allowance rather than using a global percentage.
        let next_turn_reserve = (history_budget / 8)
            .max(input.reserved_output_tokens.get() / 2)
            .min(history_budget.saturating_sub(1));
        let soft_watermark = history_budget.saturating_sub(next_turn_reserve);
        let action = if projected_history_tokens > history_budget {
            BudgetAction::HardCompact
        } else if projected_history_tokens >= soft_watermark {
            BudgetAction::SoftCompact
        } else {
            BudgetAction::None
        };
        ContextBudgetPlan {
            effective_context_window: effective_context_window.into(),
            history_budget: history_budget.into(),
            soft_watermark: soft_watermark.into(),
            projected_history_tokens: projected_history_tokens.into(),
            action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(history: usize, pending: usize) -> ContextBudgetInput {
        ContextBudgetInput {
            model_context_window: 100_000.into(),
            profile_input_limit: 80_000.into(),
            system_and_skill_prefix_tokens: 10_000.into(),
            tool_schema_tokens: 5_000.into(),
            reserved_output_tokens: 10_000.into(),
            pending_user_input_tokens: u64::try_from(pending).unwrap().into(),
            safety_margin_tokens: 5_000.into(),
            current_history_tokens: u64::try_from(history).unwrap().into(),
        }
    }

    #[test]
    fn subtracts_all_non_history_costs() {
        let plan = ContextBudgetPlanner::plan(input(10_000, 2_000));
        assert_eq!(plan.effective_context_window.get(), 80_000);
        assert_eq!(plan.history_budget.get(), 50_000);
        assert_eq!(plan.projected_history_tokens.get(), 12_000);
        assert_eq!(plan.action, BudgetAction::None);
    }

    #[test]
    fn pending_input_can_cross_hard_watermark() {
        let plan = ContextBudgetPlanner::plan(input(48_000, 3_000));
        assert_eq!(plan.action, BudgetAction::HardCompact);
    }

    #[test]
    fn soft_watermark_reserves_a_future_turn() {
        let plan = ContextBudgetPlanner::plan(input(45_000, 0));
        assert_eq!(plan.soft_watermark.get(), 43_750);
        assert_eq!(plan.action, BudgetAction::SoftCompact);
    }
}
