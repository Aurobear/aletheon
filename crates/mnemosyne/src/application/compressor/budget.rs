//! Per-turn adaptive context budgeting.
//!
//! The planner deliberately keeps non-history costs explicit so callers cannot
//! accidentally compare conversation tokens with the model's whole context
//! window.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudgetInput {
    pub model_context_window: usize,
    pub profile_input_limit: usize,
    pub system_and_skill_prefix_tokens: usize,
    pub tool_schema_tokens: usize,
    pub reserved_output_tokens: usize,
    pub pending_user_input_tokens: usize,
    pub safety_margin_tokens: usize,
    pub current_history_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetAction {
    None,
    SoftCompact,
    HardCompact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudgetPlan {
    pub effective_context_window: usize,
    pub history_budget: usize,
    pub soft_watermark: usize,
    pub projected_history_tokens: usize,
    pub action: BudgetAction,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ContextBudgetPlanner;

impl ContextBudgetPlanner {
    pub fn plan(input: ContextBudgetInput) -> ContextBudgetPlan {
        let effective_context_window = match input.profile_input_limit {
            0 => input.model_context_window,
            limit => input.model_context_window.min(limit),
        };
        let fixed_cost = input
            .system_and_skill_prefix_tokens
            .saturating_add(input.tool_schema_tokens)
            .saturating_add(input.reserved_output_tokens)
            .saturating_add(input.safety_margin_tokens);
        let history_budget = effective_context_window.saturating_sub(fixed_cost).max(1);
        let projected_history_tokens = input
            .current_history_tokens
            .saturating_add(input.pending_user_input_tokens);

        // Preserve enough space for approximately one additional normal turn.
        // The reserve adapts to both the available history budget and the
        // profile's output allowance rather than using a global percentage.
        let next_turn_reserve = (history_budget / 8)
            .max(input.reserved_output_tokens / 2)
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
            effective_context_window,
            history_budget,
            soft_watermark,
            projected_history_tokens,
            action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(history: usize, pending: usize) -> ContextBudgetInput {
        ContextBudgetInput {
            model_context_window: 100_000,
            profile_input_limit: 80_000,
            system_and_skill_prefix_tokens: 10_000,
            tool_schema_tokens: 5_000,
            reserved_output_tokens: 10_000,
            pending_user_input_tokens: pending,
            safety_margin_tokens: 5_000,
            current_history_tokens: history,
        }
    }

    #[test]
    fn subtracts_all_non_history_costs() {
        let plan = ContextBudgetPlanner::plan(input(10_000, 2_000));
        assert_eq!(plan.effective_context_window, 80_000);
        assert_eq!(plan.history_budget, 50_000);
        assert_eq!(plan.projected_history_tokens, 12_000);
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
        assert_eq!(plan.soft_watermark, 43_750);
        assert_eq!(plan.action, BudgetAction::SoftCompact);
    }
}
