//! Bounded breadth-scanning policy for repository exploration.

const MIN_EXPLORATION_INPUT_TOKENS: u64 = 10_000;
const MAX_EXPLORATION_INPUT_TOKENS: u64 = 24_000;

pub(super) fn exploration_input_token_budget(context_window_tokens: usize) -> u64 {
    (context_window_tokens as u64 / 100)
        .clamp(MIN_EXPLORATION_INPUT_TOKENS, MAX_EXPLORATION_INPUT_TOKENS)
}

fn is_inspection_tool(name: &str) -> bool {
    matches!(name, "glob" | "grep" | "file_search")
}

fn is_repository_followup_scan(name: &str) -> bool {
    matches!(name, "glob" | "grep" | "file_search")
}

pub(super) fn should_close_exploration<'a>(
    iteration: usize,
    cumulative_input_tokens: u64,
    context_window_tokens: usize,
    repository_context_seen: bool,
    repository_followup_read_batches: usize,
    tool_names: impl IntoIterator<Item = &'a str>,
) -> bool {
    let names = tool_names.into_iter().collect::<Vec<_>>();
    let broad_scan_over_budget = iteration > 1
        && !names.is_empty()
        && names.iter().all(|name| is_inspection_tool(name))
        && cumulative_input_tokens >= exploration_input_token_budget(context_window_tokens);

    // `repo_inspect` is a typed repository-context capability whose successful
    // result already contains previews of known entry files. After that result,
    // permit exact follow-up reads for missing implementation detail. A later
    // pure *discovery* batch over budget is redundant exploration and should be
    // replaced by synthesis. `file_read` is deliberately excluded: its inputs
    // are exact paths, its result can be bounded/truncated, and closing the turn
    // after one such batch prevents an implementation agent from reading the
    // remaining named files it must edit. This remains task- and
    // language-independent and does not constrain implementation turns that
    // never established repository overview context through `repo_inspect`.
    let redundant_repository_scan = repository_context_seen
        && repository_followup_read_batches >= 1
        && !names.is_empty()
        && names.iter().all(|name| is_repository_followup_scan(name))
        && cumulative_input_tokens >= exploration_input_token_budget(context_window_tokens);

    broad_scan_over_budget || redundant_repository_scan
}
