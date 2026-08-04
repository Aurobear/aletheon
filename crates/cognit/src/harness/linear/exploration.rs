//! Bounded breadth-scanning policy for repository exploration.

const MIN_EXPLORATION_INPUT_TOKENS: u64 = 10_000;
const MAX_EXPLORATION_INPUT_TOKENS: u64 = 24_000;

pub(super) fn exploration_input_token_budget(context_window_tokens: usize) -> u64 {
    (context_window_tokens as u64 / 100)
        .clamp(MIN_EXPLORATION_INPUT_TOKENS, MAX_EXPLORATION_INPUT_TOKENS)
}

/// Breadth-scanning tools that count toward the exploration budget.
///
/// `file_read` is deliberately excluded: reading a specific file is the
/// productive step the model must always be allowed to take before answering.
fn is_inspection_tool(name: &str) -> bool {
    matches!(name, "glob" | "grep" | "file_search")
}

pub(super) fn should_close_exploration<'a>(
    iteration: usize,
    cumulative_input_tokens: u64,
    context_window_tokens: usize,
    tool_names: impl IntoIterator<Item = &'a str>,
) -> bool {
    let names = tool_names.into_iter().collect::<Vec<_>>();
    iteration > 1
        && !names.is_empty()
        && names.iter().all(|name| is_inspection_tool(name))
        && cumulative_input_tokens >= exploration_input_token_budget(context_window_tokens)
}
