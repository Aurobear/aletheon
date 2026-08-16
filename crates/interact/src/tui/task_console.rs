//! Daemon-projected task console.
//!
//! Conversation remains a compact transcript, while task progress, diagnostics,
//! and file/artifact references are rendered from the canonical Session read
//! projection.  The console deliberately does not reconstruct activity from
//! tool transcript entries: that would create a second local runtime state.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};
use serde::Deserialize;

use super::{markdown, state::AppState, term_compat::TermCaps};
use ::contracts::protocol::client::{ActivitySnapshot, ActivityState, TaskPhase, TaskSnapshot};

/// The primary work surface for an active Session.
pub struct TaskConsole<'a> {
    pub state: &'a AppState,
    pub caps: &'a TermCaps,
    pub workspace_name: &'a str,
    pub selected_activity: Option<usize>,
    /// Explicit typed Agent runtime requirement waiting for the next turn.
    pub next_agent_runtime: Option<&'a str>,
}

impl Widget for TaskConsole<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let header_height = if area.height >= 8 { 3 } else { 1 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(1)])
            .split(area);
        render_task_header(
            chunks[0],
            buf,
            self.state,
            self.caps,
            self.workspace_name,
            self.next_agent_runtime,
        );

        // Conversation is the normal Aletheon work surface. The dense
        // timeline/diagnostics sidecar is useful while a Robot episode needs
        // supervision, but permanently reserving it for chat and coding turns
        // wastes most of the terminal. Non-Robot activity remains available
        // through the canonical header counters and Ctrl+B/Ctrl+D overlays.
        if !activity_console_visible(self.state, self.selected_activity) {
            render_conversation(chunks[1], buf, self.state, self.caps);
        } else if chunks[1].width >= 110 && chunks[1].height >= 8 {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
                .split(chunks[1]);
            render_conversation(columns[0], buf, self.state, self.caps);
            render_activity_panel(
                columns[1],
                buf,
                self.state,
                self.caps,
                self.selected_activity,
            );
        } else {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
                .split(chunks[1]);
            render_conversation(rows[0], buf, self.state, self.caps);
            render_activity_panel(rows[1], buf, self.state, self.caps, self.selected_activity);
        }
    }
}

fn activity_console_visible(state: &AppState, selected_activity: Option<usize>) -> bool {
    selected_activity.is_some()
        || state.activities.iter().any(|activity| {
            activity.kind == ::contracts::ActivityKind::Robot
                && !matches!(
                    activity.state,
                    ActivityState::Completed | ActivityState::Cancelled
                )
        })
}

fn render_task_header(
    area: Rect,
    buf: &mut Buffer,
    state: &AppState,
    caps: &TermCaps,
    workspace_name: &str,
    next_agent_runtime: Option<&str>,
) {
    let task = active_task(state);
    let task_id = task
        .map(|task| short_id(&task.task_id))
        .unwrap_or("no task");
    let phase = task.map(|task| task_phase(task.phase)).unwrap_or("idle");
    let settlement = task
        .and_then(|task| task.settlement)
        .map(task_settlement)
        .unwrap_or("open");
    let goal = task
        .and_then(|task| task.goal.as_deref())
        .unwrap_or("Waiting for daemon task projection");
    let permission = task
        .map(permission_summary)
        .unwrap_or_else(|| "unknown".to_string());
    let (provider, model, context) = task_runtime_identity(task);
    let session = state
        .projected_session
        .as_ref()
        .map(|session| short_id(&session.id.0))
        .unwrap_or("—");
    let project = workspace_name;
    let activity = activity_summary(&state.activities);
    let target = match &state.execution_target.target {
        ::contracts::ExecutionTarget::General => "general".to_string(),
        ::contracts::ExecutionTarget::Robot {
            device_id,
            environment,
        } => format!("robot:{}/{}", device_id.0, environment.as_str()),
    };
    let theme = caps.theme();

    let task_badge = if caps.color {
        Style::default().fg(Color::Black).bg(theme.accent)
    } else {
        Style::default()
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(" TASK ", task_badge),
        Span::styled(
            format!(" project {project} · {task_id} · {phase} · {settlement} "),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(goal, Style::default().fg(theme.text_muted)),
    ])];
    if area.height >= 2 {
        lines.push(Line::from(Span::styled(
            format!(
                " session {session} · target {target} · next runtime {} · provider {provider} · model {model} · permission {permission}",
                next_agent_runtime.unwrap_or("auto")
            ),
            Style::default().fg(theme.text_muted),
        )));
    }
    if area.height >= 3 {
        let mut runtime = Vec::new();
        if let Some(error) = state.last_error.as_deref() {
            runtime.push(Span::styled(
                format!(" ERROR {error} ·"),
                Style::default().fg(theme.error),
            ));
        }
        runtime.push(Span::styled(
            format!(
                " activity {activity} · context {context} · {}",
                runtime_metrics(task)
            ),
            Style::default().fg(theme.text_muted),
        ));
        lines.push(Line::from(runtime));
    }
    Paragraph::new(lines)
        .style(Style::default().bg(theme.bg_panel))
        .wrap(Wrap { trim: true })
        .render(area, buf);
}

fn render_conversation(area: Rect, buf: &mut Buffer, state: &AppState, caps: &TermCaps) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Conversation ");
    let inner = block.inner(area);
    block.render(area, buf);
    let theme = caps.theme();
    let mut items = state
        .items
        .values()
        .filter(|item| matches!(item.kind.as_str(), "user" | "assistant"))
        .collect::<Vec<_>>();
    items.sort_by_key(|item| item.sequence);
    let mut lines = Vec::new();
    let mut last_assistant_content: Option<&str> = None;
    for item in items {
        if item.kind == "user" {
            last_assistant_content = None;
            lines.push(Line::from(vec![
                Span::styled("> ", Style::default().fg(theme.user_icon)),
                Span::styled(item.content.clone(), Style::default().fg(theme.user_icon)),
            ]));
        } else {
            if last_assistant_content == Some(item.content.as_str()) {
                continue;
            }
            last_assistant_content = Some(item.content.as_str());
            lines.extend(markdown::render_markdown(&item.content, inner.width, caps));
        }
        lines.push(Line::from(""));
    }
    append_work_trace(&mut lines, state, caps);
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No projected conversation yet",
            Style::default().fg(theme.text_muted),
        )));
    }
    let wrapped_line_count = lines
        .iter()
        .map(|line| {
            let width = inner.width.max(1) as usize;
            line.width().max(1).div_ceil(width)
        })
        .sum::<usize>();
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let tail_scroll = wrapped_line_count
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize) as u16;
    let scroll = tail_scroll.saturating_sub(state.conversation_scroll.min(tail_scroll));
    paragraph.scroll((scroll, 0)).render(inner, buf);
}

fn append_work_trace(lines: &mut Vec<Line<'static>>, state: &AppState, caps: &TermCaps) {
    if state.activities.is_empty() {
        return;
    }
    let theme = caps.theme();
    let has_agent_activity = state.activities.iter().any(|activity| {
        matches!(
            activity.label.as_str(),
            "agent" | "agent_spawn" | "agent_wait"
        )
    });
    lines.push(Line::from(vec![
        Span::styled(
            "Work trace",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if has_agent_activity {
                "  (public progress · Ctrl+B details · Ctrl+G child Agents)"
            } else {
                "  (public progress · Ctrl+B full details)"
            },
            Style::default().fg(theme.text_muted),
        ),
    ]));
    let mut activities = state.activities.iter().collect::<Vec<_>>();
    activities.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.activity_id.cmp(&right.activity_id))
    });
    for activity in activities.into_iter().rev().take(12).rev() {
        lines.push(activity_line(activity, caps, false));
    }
    lines.push(Line::from(""));
}

fn render_activity_panel(
    area: Rect,
    buf: &mut Buffer,
    state: &AppState,
    caps: &TermCaps,
    selected_activity: Option<usize>,
) {
    let timeline_percent = if area.height >= 8 { 68 } else { 55 };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(timeline_percent),
            Constraint::Percentage(100 - timeline_percent),
        ])
        .split(area);
    let theme = caps.theme();

    let timeline = Block::default()
        .borders(Borders::ALL)
        .title(" Activity timeline ");
    let timeline_inner = timeline.inner(sections[0]);
    timeline.render(sections[0], buf);
    let mut activities = state.activities.iter().enumerate().collect::<Vec<_>>();
    activities.sort_by(|(_, left), (_, right)| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.activity_id.cmp(&right.activity_id))
    });
    let max = timeline_inner.height as usize;
    let mut lines = activities.into_iter().rev().take(max).collect::<Vec<_>>();
    lines.reverse();
    let rendered = if lines.is_empty() {
        vec![Line::from(Span::styled(
            "No authoritative activity yet",
            Style::default().fg(theme.text_muted),
        ))]
    } else {
        lines
            .into_iter()
            .map(|(index, activity)| {
                activity_line(activity, caps, selected_activity == Some(index))
            })
            .collect()
    };
    Paragraph::new(rendered)
        .wrap(Wrap { trim: true })
        .render(timeline_inner, buf);

    let changes = Block::default()
        .borders(Borders::ALL)
        .title(" Changes / diagnostics ");
    let changes_inner = changes.inner(sections[1]);
    changes.render(sections[1], buf);
    let mut seen_refs = std::collections::HashSet::new();
    let refs = state
        .activities
        .iter()
        .flat_map(|activity| activity.artifact_refs.iter())
        .filter(|reference| seen_refs.insert(reference.as_str()))
        .collect::<Vec<_>>();
    let mut change_lines = Vec::new();
    if let Some(error) = state.last_error.as_deref() {
        change_lines.push(Line::from(Span::styled(
            format!("ERROR {error}"),
            Style::default().fg(theme.error),
        )));
    }
    if let Some(review) = active_task(state).and_then(|task| task.checkpoint_review.as_ref()) {
        let coverage = match review.mutation_coverage {
            ::contracts::CheckpointMutationCoverage::Full => "full",
            ::contracts::CheckpointMutationCoverage::BestEffort => "best-effort",
            ::contracts::CheckpointMutationCoverage::NonRollbackable => "non-rollbackable",
        };
        let rollback = match review.rollback_action {
            ::contracts::CheckpointRollbackAction::AutomaticAllowed => {
                "automatic rollback available"
            }
            ::contracts::CheckpointRollbackAction::ExplicitApprovalRequired => {
                "rollback requires explicit approval"
            }
            ::contracts::CheckpointRollbackAction::Unavailable => "rollback unavailable",
        };
        change_lines.push(Line::from(Span::styled(
            format!("CHECKPOINT {coverage} · {rollback}"),
            Style::default().fg(theme.accent),
        )));
        change_lines.extend(
            review
                .changed_paths
                .iter()
                .take(changes_inner.height as usize)
                .map(|path| Line::from(format!(" {} {path}", caps.bullet()))),
        );
        if matches!(
            review.settlement,
            ::contracts::CheckpointReviewSettlement::Partial
                | ::contracts::CheckpointReviewSettlement::Conflicted
        ) {
            change_lines.extend(review.recovery_evidence.iter().map(|evidence| {
                Line::from(Span::styled(
                    format!(" RECOVERY {evidence}"),
                    Style::default().fg(theme.warning),
                ))
            }));
        }
    }
    if let Some(summary) = robot_summary(&state.activities) {
        change_lines.push(Line::from(Span::styled(
            format!(
                "ROBOT {} · {} · {}",
                summary.device, summary.scene, summary.settlement
            ),
            Style::default().fg(if summary.settlement == "blocked" {
                theme.error
            } else {
                theme.accent
            }),
        )));
        change_lines.push(Line::from(format!(
            " bridge {} · attempts {}",
            summary.bridge_digest, summary.attempt_count
        )));
        change_lines.push(Line::from(format!(
            " report {} · evidence {} ref(s)",
            summary.report_sha256, summary.evidence_count
        )));
    }
    if let Some(activity) = selected_activity.and_then(|index| state.activities.get(index)) {
        change_lines.push(Line::from(Span::styled(
            format!("DETAIL {:?}: {}", activity.kind, activity.label),
            Style::default().fg(theme.accent),
        )));
        if let Some(receipt) = activity.receipt_ref.as_deref() {
            change_lines.push(Line::from(format!(" receipt {receipt}")));
        }
        if let Some(progress) = activity.progress.as_ref() {
            change_lines.push(Line::from(format!(
                " progress {}",
                bounded_value(progress, 512)
            )));
        }
    }
    change_lines.extend(
        refs.into_iter()
            .take(changes_inner.height as usize)
            .map(|reference| Line::from(format!(" {} {reference}", caps.bullet()))),
    );
    if change_lines.is_empty() {
        change_lines.push(Line::from(Span::styled(
            "No changed artifacts or diagnostics",
            Style::default().fg(theme.text_muted),
        )));
    }
    Paragraph::new(change_lines)
        .wrap(Wrap { trim: true })
        .render(changes_inner, buf);
}

fn active_task(state: &AppState) -> Option<&TaskSnapshot> {
    state
        .tasks
        .iter()
        .find(|task| matches!(task.phase, TaskPhase::Active | TaskPhase::Interrupted))
        .or_else(|| state.tasks.first())
}

fn task_runtime_identity(task: Option<&TaskSnapshot>) -> (&str, &str, String) {
    let Some(facts) = task.and_then(|task| task.runtime_facts.as_ref()) else {
        return (
            "—",
            "—",
            "window unknown (missing runtime facts projection)".into(),
        );
    };
    let provider = facts.effective_provider.as_deref().unwrap_or("—");
    let model = facts.effective_model.as_deref().unwrap_or("—");
    let context = match facts.context_budget.as_deref() {
        Some(budget) => format!(
            "window {} · profile input {} · history {} / {}",
            compact_tokens(budget.model_context_tokens.get()),
            compact_tokens(budget.profile_input_limit_tokens.get()),
            compact_tokens(budget.current_history_tokens.get()),
            compact_tokens(budget.admissible_history_tokens.get()),
        ),
        None => match (
            facts.active_context_occupancy_tokens,
            facts.context_capacity_tokens,
        ) {
            (Some(used), Some(capacity)) => format!(
                "window {} · active {} · history unknown (missing context budget projection)",
                compact_tokens(capacity),
                compact_tokens(used),
            ),
            (_, Some(capacity)) => format!(
                "window {} · history unknown (missing context budget projection)",
                compact_tokens(capacity),
            ),
            _ => "window unknown (missing context budget projection)".into(),
        },
    };
    (provider, model, context)
}

fn runtime_metrics(task: Option<&TaskSnapshot>) -> String {
    let Some(task) = task else {
        return "budget unavailable (missing daemon task projection) · cache unavailable (missing inference receipt)".into();
    };
    let Some(facts) = task.runtime_facts.as_ref() else {
        return "budget unavailable (missing runtime facts projection) · cache unavailable (missing inference receipt)".into();
    };
    let budget = facts.context_budget.as_deref().map_or_else(
        || "budget unavailable (missing context budget projection)".into(),
        |budget| {
            format!(
                "output reserve {} · system+tools {} · safety {} · rollout root remaining {} · child limit {} · current Agent remaining {}",
                compact_tokens(budget.reserved_output_tokens.get()),
                compact_tokens(
                    budget
                        .system_and_skill_tokens
                        .get()
                        .saturating_add(budget.tool_schema_tokens.get())
                ),
                compact_tokens(budget.safety_margin_tokens.get()),
                rollout_value(&budget.rollout.root_remaining_tokens),
                rollout_value(&budget.rollout.child_limit_tokens),
                rollout_value(&budget.rollout.current_agent_remaining_tokens),
            )
        },
    );
    let cache = match facts.cumulative_usage.cache_telemetry {
        ::contracts::CacheTelemetry::Reported => format!(
            "read {} / write {}",
            facts
                .cumulative_usage
                .cache_read_tokens
                .map_or_else(|| "unknown".into(), |value| value.to_string()),
            facts
                .cumulative_usage
                .cache_write_tokens
                .map_or_else(|| "unknown".into(), |value| value.to_string())
        ),
        ::contracts::CacheTelemetry::Unsupported => "unsupported".into(),
        ::contracts::CacheTelemetry::Unknown => {
            "unknown (provider receipt did not report cache usage)".into()
        }
    };
    let task_usage = format!(
        "task usage {} in / {} out",
        facts
            .cumulative_usage
            .total_input_tokens
            .map_or_else(|| "unknown".into(), compact_tokens),
        facts
            .cumulative_usage
            .output_tokens
            .map_or_else(|| "unknown".into(), compact_tokens),
    );
    format!(
        "{budget} · {task_usage} · cache {cache} · infer {} · retries {} · tools {}/{}",
        facts.inference_rounds,
        facts
            .provider_retries
            .map_or_else(|| "unknown".into(), |value| value.to_string()),
        facts.terminal_tool_results,
        facts.tool_calls
    )
}

fn rollout_value(value: &::contracts::RolloutBudgetValue) -> String {
    match value {
        ::contracts::RolloutBudgetValue::Known { value, .. } => compact_tokens(value.get()),
        ::contracts::RolloutBudgetValue::Unknown { reason, .. } => {
            format!("unknown ({})", reason.as_str())
        }
    }
}

fn compact_tokens(tokens: u64) -> String {
    if tokens >= 1_000 {
        format!("{}k", grouped_decimal(tokens / 1_000))
    } else {
        grouped_decimal(tokens)
    }
}

fn grouped_decimal(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(ch);
    }
    output
}

fn permission_summary(task: &TaskSnapshot) -> String {
    if !task.pending_approvals.is_empty() {
        format!("approval required ({})", task.pending_approvals.len())
    } else {
        "unknown".into()
    }
}

fn activity_summary(activities: &[ActivitySnapshot]) -> String {
    let running = activities
        .iter()
        .filter(|activity| {
            matches!(
                activity.state,
                ActivityState::Running | ActivityState::Waiting
            )
        })
        .count();
    if running > 0 {
        format!("{running} active")
    } else {
        format!("{} recorded", activities.len())
    }
}

fn activity_line(activity: &ActivitySnapshot, caps: &TermCaps, selected: bool) -> Line<'static> {
    let (state, color) = match activity.state {
        ActivityState::Queued => ("queued", caps.theme().text_muted),
        ActivityState::Running => ("running", caps.theme().warning),
        ActivityState::Waiting => ("waiting", caps.theme().warning),
        ActivityState::Completed => ("done", caps.theme().success),
        ActivityState::Blocked => ("blocked", caps.theme().error),
        ActivityState::Failed | ActivityState::Lost => ("failed", caps.theme().error),
        ActivityState::Cancelled => ("cancelled", caps.theme().text_muted),
    };
    let progress = activity_progress(activity);
    let selection = if selected { ">" } else { " " };
    Line::from(vec![
        Span::styled(selection, Style::default().fg(caps.theme().accent)),
        Span::styled(format!(" {} ", caps.bullet()), Style::default().fg(color)),
        Span::styled(format!("{state:9} "), Style::default().fg(color)),
        Span::raw(format!("{}{progress}", activity.label)),
    ])
}

fn activity_progress(activity: &ActivitySnapshot) -> String {
    let Some(progress) = activity.progress.as_ref() else {
        return String::new();
    };
    let Ok(display) = serde_json::from_value::<ActivityProgressDisplay>(progress.clone()) else {
        return " · unknown progress".into();
    };
    if display.schema_version != ACTIVITY_PROGRESS_SCHEMA_VERSION {
        return " · unknown progress schema".into();
    }
    let args = display
        .args
        .as_ref()
        .filter(|value| !value.is_null())
        .map(|value| format!(" {}", bounded_value(value, 72)))
        .unwrap_or_default();
    let elapsed = display
        .elapsed_ms
        .map(|value| format!(" · {value}ms"))
        .unwrap_or_default();
    let error = display
        .error
        .as_deref()
        .map(|value| format!(" · {}", bounded_text(value, 96)))
        .unwrap_or_default();
    if args.is_empty() && elapsed.is_empty() && error.is_empty() {
        display
            .stage
            .map(|stage| format!(" · {stage}"))
            .unwrap_or_default()
    } else {
        format!("{args}{elapsed}{error}")
    }
}

fn bounded_value(value: &serde_json::Value, limit: usize) -> String {
    let rendered = match value {
        serde_json::Value::String(value) => value.clone(),
        value => value.to_string(),
    };
    bounded_text(&rendered, limit)
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_owned()
    } else {
        format!("{}…", value.chars().take(limit).collect::<String>())
    }
}

fn task_phase(phase: TaskPhase) -> &'static str {
    match phase {
        TaskPhase::Active => "active",
        TaskPhase::Interrupted => "interrupted",
        TaskPhase::Completed => "completed",
        TaskPhase::Blocked => "blocked",
        TaskPhase::Failed => "failed",
    }
}

fn task_settlement(settlement: ::contracts::TaskSettlement) -> &'static str {
    match settlement {
        ::contracts::TaskSettlement::Accepted => "accepted",
        ::contracts::TaskSettlement::RepairRequired => "repair-required",
        ::contracts::TaskSettlement::Blocked => "blocked",
        ::contracts::TaskSettlement::Cancelled => "cancelled",
        ::contracts::TaskSettlement::RolledBack => "rolled-back",
        ::contracts::TaskSettlement::Failed => "failed",
    }
}

const ACTIVITY_PROGRESS_SCHEMA_VERSION: u16 = 1;

/// Versioned display DTO for the bounded activity-progress surface. The
/// renderer never walks arbitrary JSON keys: missing/unknown fields render as
/// `unknown` and cannot be mistaken for authoritative runtime facts.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct ActivityProgressDisplay {
    schema_version: u16,
    args: Option<serde_json::Value>,
    elapsed_ms: Option<u64>,
    error: Option<String>,
    stage: Option<String>,
    device: Option<String>,
    scene: Option<String>,
    settlement: Option<String>,
    bridge_digest: Option<String>,
    report_sha256: Option<String>,
    attempt_count: Option<u64>,
    evidence_refs: Option<Vec<String>>,
}

impl Default for ActivityProgressDisplay {
    fn default() -> Self {
        Self {
            schema_version: ACTIVITY_PROGRESS_SCHEMA_VERSION,
            args: None,
            elapsed_ms: None,
            error: None,
            stage: None,
            device: None,
            scene: None,
            settlement: None,
            bridge_digest: None,
            report_sha256: None,
            attempt_count: None,
            evidence_refs: None,
        }
    }
}

struct RobotSummary {
    device: String,
    scene: String,
    settlement: String,
    bridge_digest: String,
    report_sha256: String,
    attempt_count: u64,
    evidence_count: usize,
}

fn robot_summary(activities: &[ActivitySnapshot]) -> Option<RobotSummary> {
    let display = activities
        .iter()
        .rev()
        .filter(|activity| activity.kind == ::contracts::ActivityKind::Robot)
        .filter_map(|activity| {
            let progress = activity.progress.as_ref()?;
            let display =
                serde_json::from_value::<ActivityProgressDisplay>(progress.clone()).ok()?;
            (display.schema_version == ACTIVITY_PROGRESS_SCHEMA_VERSION
                && display.stage.as_deref() == Some("settle"))
            .then_some(display)
        })
        .next()?;
    Some(RobotSummary {
        device: display.device.unwrap_or_else(|| "unknown".into()),
        scene: display.scene.unwrap_or_else(|| "unknown".into()),
        settlement: display.settlement.unwrap_or_else(|| "unknown".into()),
        bridge_digest: display.bridge_digest.unwrap_or_else(|| "unknown".into()),
        report_sha256: display.report_sha256.unwrap_or_else(|| "unknown".into()),
        attempt_count: display.attempt_count.unwrap_or_default(),
        evidence_count: display.evidence_refs.map_or(0, |refs| refs.len()),
    })
}

fn short_id(value: &str) -> &str {
    value.get(..12).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::{UiItem, UiItemStatus};
    use ::contracts::WorkspacePolicy;

    fn test_caps() -> TermCaps {
        TermCaps {
            color: true,
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        }
    }

    fn rendered_text(width: u16, height: u16, state: &AppState) -> String {
        rendered_text_with_selection(width, height, state, None)
    }

    fn rendered_text_with_selection(
        width: u16,
        height: u16,
        state: &AppState,
        selected_activity: Option<usize>,
    ) -> String {
        let caps = test_caps();
        let workspace = WorkspacePolicy::from_resolved_roots(
            std::env::current_dir().expect("test cwd"),
            Vec::new(),
        )
        .expect("workspace policy");
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        TaskConsole {
            state,
            caps: &caps,
            workspace_name: workspace
                .cwd()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workspace"),
            selected_activity,
            next_agent_runtime: None,
        }
        .render(area, &mut buffer);
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn ordinary_console_keeps_conversation_as_the_single_primary_panel() {
        let rendered = rendered_text(120, 40, &AppState::default());
        assert!(rendered.contains("Conversation"));
        assert!(!rendered.contains("Activity timeline"));
        assert!(!rendered.contains("Changes / diagnostics"));
    }

    #[test]
    fn assistant_markdown_table_is_rendered_on_the_canonical_surface() {
        let mut state = AppState::default();
        state.items.insert(
            "assistant".into(),
            UiItem {
                id: "assistant".into(),
                sequence: 1,
                kind: "assistant".into(),
                content: "架构概览\n\n| 层 | Crate | 证据 |\n|---|---|---|\n| 入口 | aletheon | ACP 协议 |\n| 编排 | executive | turn pipeline |"
                    .into(),
                status: UiItemStatus::Completed,
                collapsed: false,
            },
        );

        for width in [80, 120] {
            let rendered = rendered_text(width, 40, &state);
            // Ratatui stores an empty filler cell after every double-width CJK
            // glyph. Remove display whitespace before asserting semantic text.
            let compact = rendered.split_whitespace().collect::<String>();
            assert!(compact.contains("架构概览"), "width={width}: {rendered}");
            assert!(compact.contains("入口"), "width={width}: {rendered}");
            assert!(compact.contains("ACP协议"), "width={width}: {rendered}");
            assert!(!rendered.contains("|---"), "width={width}: {rendered}");
            assert!(!rendered.contains("||"), "width={width}: {rendered}");
        }
    }

    #[test]
    fn consecutive_identical_assistant_transport_rows_render_once_per_turn() {
        let mut state = AppState::default();
        for (id, sequence) in [("durable-1", 1), ("compat-1", 2)] {
            state.items.insert(
                id.into(),
                UiItem {
                    id: id.into(),
                    sequence,
                    kind: "assistant".into(),
                    content: "same answer".into(),
                    status: UiItemStatus::Completed,
                    collapsed: false,
                },
            );
        }
        state.items.insert(
            "user-2".into(),
            UiItem {
                id: "user-2".into(),
                sequence: 3,
                kind: "user".into(),
                content: "repeat".into(),
                status: UiItemStatus::Completed,
                collapsed: false,
            },
        );
        state.items.insert(
            "durable-2".into(),
            UiItem {
                id: "durable-2".into(),
                sequence: 4,
                kind: "assistant".into(),
                content: "same answer".into(),
                status: UiItemStatus::Completed,
                collapsed: false,
            },
        );

        let rendered = rendered_text(120, 40, &state);
        assert_eq!(rendered.matches("same answer").count(), 2, "{rendered}");
    }

    fn projected_state() -> AppState {
        let mut state = AppState::default();
        state.items.insert(
            "user".into(),
            UiItem {
                id: "user".into(),
                sequence: 1,
                kind: "user".into(),
                content: "inspect the workspace".into(),
                status: UiItemStatus::Completed,
                collapsed: false,
            },
        );
        state.items.insert(
            "assistant".into(),
            UiItem {
                id: "assistant".into(),
                sequence: 2,
                kind: "assistant".into(),
                content: "working from canonical evidence".into(),
                status: UiItemStatus::Streaming,
                collapsed: false,
            },
        );
        state.tasks.push(TaskSnapshot {
            task_id: "task-1234567890".into(),
            session_id: ::contracts::SessionId("session-1234567890".into()),
            goal: Some("inspect repository".into()),
            phase: TaskPhase::Active,
            plan_revision: Some(1),
            steps: vec![],
            active_turn_id: None,
            active_runtime_children: vec!["child-1".into()],
            active_commands: vec!["check".into()],
            pending_approvals: vec![],
            budget: Some(serde_json::json!({"remaining": 12})),
            checkpoint_head: None,
            checkpoint_review: None,
            settlement: None,
            review_findings: vec![],
            runtime_facts: Some(::contracts::TaskRuntimeFacts {
                effective_provider: Some("deepseek".into()),
                effective_model: Some("deepseek-v4-flash".into()),
                context_capacity_tokens: Some(1_000_000),
                active_context_occupancy_tokens: Some(8_000),
                context_budget: Some(Box::new(projected_context_budget())),
                cumulative_usage: ::contracts::InferenceUsage::reported(
                    10_000,
                    500,
                    Some(2_000),
                    Some(8_000),
                    Some(0),
                ),
                inference_rounds: 2,
                provider_retries: Some(0),
                tool_calls: 1,
                terminal_tool_results: 0,
            }),
        });
        state.activities.push(ActivitySnapshot {
            activity_id: "activity-1".into(),
            task_id: "task-1234567890".into(),
            turn_id: ::contracts::TurnId::new(),
            parent_activity_id: None,
            kind: ::contracts::ActivityKind::Command,
            label: "cargo check".into(),
            state: ActivityState::Running,
            started_at: 1,
            updated_at: 2,
            progress: Some(serde_json::json!({
                "schema_version": ACTIVITY_PROGRESS_SCHEMA_VERSION,
                "stage": "42/100 lines"
            })),
            artifact_refs: vec!["src/lib.rs".into()],
            receipt_ref: None,
        });
        state
    }

    #[test]
    fn u_tui_001_task_phase_is_visible_in_the_fixed_header() {
        let rendered = rendered_text(120, 40, &projected_state());
        assert!(rendered.contains("active"));
        assert!(rendered.contains("inspect repository"));
        assert!(rendered.contains("deepseek-v4-flash"));
    }

    #[test]
    fn non_robot_activity_is_visible_inline_without_a_permanent_side_panel() {
        let rendered = rendered_text(120, 40, &projected_state());
        assert!(rendered.contains("1 active"));
        assert!(rendered.contains("Work trace"));
        assert!(rendered.contains("cargo check"));
        assert!(rendered.contains("42/100 lines"));
        assert!(!rendered.contains("Changes / diagnostics"));
    }

    #[test]
    fn projected_activity_console_opens_only_when_requested() {
        let state = projected_state();
        let rendered = rendered_text_with_selection(120, 40, &state, Some(0));
        assert!(rendered.contains("Activity timeline"));
        assert!(rendered.contains("Changes / diagnostics"));
        assert!(rendered.contains("cargo check"));
        assert!(rendered.contains("42/100 lines"));
    }

    #[test]
    fn u_tui_003_failed_runtime_is_visible_as_public_progress() {
        let mut state = projected_state();
        state.activities[0].kind = ::contracts::ActivityKind::Runtime;
        state.activities[0].state = ActivityState::Failed;
        state.activities[0].label = "sub-agent reviewer".into();
        let rendered = rendered_text(120, 40, &state);
        assert!(rendered.contains("1 recorded"));
        assert!(rendered.contains("sub-agent reviewer"));
        assert!(rendered.contains("failed"));
    }

    #[test]
    fn cancelled_activity_status_is_separated_from_its_label() {
        let mut state = projected_state();
        state.activities[0].state = ActivityState::Cancelled;
        state.activities[0].label = "Model inference round 2".into();
        let rendered = rendered_text(120, 40, &state);
        assert!(rendered.contains("cancelled Model inference round 2"));
        assert!(!rendered.contains("cancelledModel inference round 2"));
    }

    #[test]
    fn work_trace_uses_call_start_order_and_exposes_bounded_failure_reason() {
        let mut state = projected_state();
        state.activities[0].activity_id = "later".into();
        state.activities[0].label = "agent_spawn".into();
        state.activities[0].state = ActivityState::Failed;
        state.activities[0].started_at = 20;
        state.activities[0].updated_at = 30;
        state.activities[0].progress = Some(serde_json::json!({
            "error": "capacity exceeded",
        }));
        let mut earlier = state.activities[0].clone();
        earlier.activity_id = "earlier".into();
        earlier.label = "Model inference round 2".into();
        earlier.kind = ::contracts::ActivityKind::Runtime;
        earlier.state = ActivityState::Completed;
        earlier.started_at = 10;
        earlier.updated_at = 100;
        earlier.progress = Some(serde_json::json!("provider response received"));
        state.activities.push(earlier);

        let rendered = rendered_text(140, 40, &state);
        let inference = rendered.find("Model inference round 2").unwrap();
        let spawn = rendered.find("agent_spawn").unwrap();
        assert!(
            inference < spawn,
            "work trace was not ordered by call start"
        );
        assert!(rendered.contains("capacity exceeded"));
        assert!(rendered.contains("Ctrl+G child Agents"));
    }

    #[test]
    fn u_tui_004_supported_terminal_sizes_keep_core_regions() {
        for (width, height) in [(80, 24), (120, 40), (200, 60)] {
            let rendered = rendered_text(width, height, &projected_state());
            assert!(
                rendered.contains("TASK"),
                "missing task at {width}x{height}"
            );
            assert!(
                rendered.contains("Conversation"),
                "missing conversation at {width}x{height}"
            );
            assert!(!rendered.contains("Activity timeline"));
        }
    }

    #[test]
    fn no_color_ascii_console_keeps_core_journey() {
        let mut caps = test_caps();
        caps.color = false;
        let workspace = WorkspacePolicy::from_resolved_roots(
            std::env::current_dir().expect("test cwd"),
            Vec::new(),
        )
        .expect("workspace policy");
        let area = Rect::new(0, 0, 80, 24);
        let mut buffer = Buffer::empty(area);
        TaskConsole {
            state: &projected_state(),
            caps: &caps,
            workspace_name: workspace
                .cwd()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workspace"),
            selected_activity: None,
            next_agent_runtime: None,
        }
        .render(area, &mut buffer);
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("TASK"));
        assert!(rendered.contains("Conversation"));
        assert!(rendered.contains("cargo check"));
        assert!(buffer
            .content
            .iter()
            .all(|cell| { cell.fg == Color::Reset && cell.bg == Color::Reset }));
    }

    #[test]
    fn u_tui_007_provider_failure_is_not_hidden_by_conversation() {
        let mut state = projected_state();
        state.tasks[0].phase = TaskPhase::Failed;
        state.last_error = Some("provider_rejected_request".into());
        let rendered = rendered_text(120, 40, &state);
        assert!(rendered.contains("failed"));
        assert!(rendered.contains("ERROR provider_rejected_request"));
    }

    #[test]
    fn u_robot_002_goal_action_verification_and_report_are_traceable() {
        let mut state = AppState::default();
        state.tasks.push(TaskSnapshot {
            task_id: "robot-task".into(),
            session_id: ::contracts::SessionId("robot-session".into()),
            goal: Some("让 kuavo-mujoco-01 站稳三秒".into()),
            phase: TaskPhase::Blocked,
            plan_revision: None,
            steps: vec![],
            active_turn_id: None,
            active_runtime_children: vec![],
            active_commands: vec![],
            pending_approvals: vec![],
            budget: None,
            checkpoint_head: None,
            checkpoint_review: None,
            settlement: Some(::contracts::TaskSettlement::Blocked),
            review_findings: vec![],
            runtime_facts: None,
        });
        let turn_id = ::contracts::TurnId::new();
        let stages = [
            ("Observe kuavo-mujoco-01 · biped-s53", "observe"),
            ("Plan governed VLA proposal", "plan"),
            ("Authorize hardware.command · attempt 1", "authorize"),
            ("Execute semantic skill · 1 attempt(s)", "execute"),
            ("Verify not_run · 3000ms", "verify"),
            ("Settle blocked · EpisodeReport", "settle"),
        ];
        for (index, (label, stage)) in stages.into_iter().enumerate() {
            let settle = stage == "settle";
            state.activities.push(ActivitySnapshot {
                activity_id: format!("robot:{index:02}-{stage}"),
                task_id: "robot-task".into(),
                turn_id,
                parent_activity_id: (index > 0)
                    .then(|| format!("robot:{:02}-{}", index - 1, stages[index - 1].1)),
                kind: ::contracts::ActivityKind::Robot,
                label: label.into(),
                state: if index < 2 {
                    ActivityState::Completed
                } else {
                    ActivityState::Blocked
                },
                started_at: 1_000,
                updated_at: 1_000 + index as u64,
                progress: Some(if settle {
                    serde_json::json!({
                        "stage": "settle",
                        "settlement": "blocked",
                        "device": "kuavo-mujoco-01",
                        "scene": "kuavo-mujoco/biped-s53",
                        "bridge_digest": "sha256:77e44869ba6a",
                        "report_sha256": "b307c5be1363fcb8",
                        "attempt_count": 1,
                        "evidence_refs": ["artifact://sha256/evidence"]
                    })
                } else {
                    serde_json::json!({"stage": stage})
                }),
                artifact_refs: if stage == "observe" || settle {
                    vec!["artifact://sha256/evidence".into()]
                } else {
                    Vec::new()
                },
                receipt_ref: Some("robot-episode:episode:sha256:b307c5be1363fcb8".into()),
            });
        }

        let rendered = rendered_text(200, 60, &state);
        assert!(rendered.contains("Activity timeline"));
        assert!(rendered.contains("Changes / diagnostics"));
        for label in [
            "Observe kuavo-mujoco-01",
            "Plan governed VLA proposal",
            "Authorize hardware.command",
            "Execute semantic skill",
            "Verify not_run",
            "Settle blocked",
        ] {
            assert!(rendered.contains(label), "missing Robot stage: {label}");
        }
        assert!(rendered.contains("blocked"));
        assert!(rendered.contains("ROBOT kuavo-mujoco-01"));
        assert!(rendered.contains("kuavo-mujoco/biped-s53"));
        assert!(rendered.contains("bridge sha256:77e44869ba6a"));
        assert!(rendered.contains("attempts 1"));
        assert!(rendered.contains("report b307c5be1363fcb8"));
        assert!(rendered.contains("evidence 1 ref(s)"));
        assert_eq!(
            rendered.matches("artifact://sha256/evidence").count(),
            1,
            "the same content-addressed Robot evidence must render only once"
        );
    }

    #[test]
    fn narrow_console_preserves_task_header_and_conversation() {
        let rendered = rendered_text(80, 24, &AppState::default());
        assert!(rendered.contains("TASK"));
        assert!(rendered.contains("Conversation"));
        assert!(!rendered.contains("Activity timeline"));
        assert!(!rendered.contains("Changes / diagnostics"));
    }

    #[test]
    fn runtime_identity_never_uses_cumulative_usage_as_context_occupancy() {
        let task = TaskSnapshot {
            task_id: "task".into(),
            session_id: ::contracts::SessionId("session".into()),
            goal: None,
            phase: TaskPhase::Active,
            plan_revision: None,
            steps: vec![],
            active_turn_id: None,
            active_runtime_children: vec![],
            active_commands: vec![],
            pending_approvals: vec![],
            budget: None,
            checkpoint_head: None,
            checkpoint_review: None,
            settlement: None,
            review_findings: vec![],
            runtime_facts: Some(::contracts::TaskRuntimeFacts {
                effective_provider: Some("provider".into()),
                effective_model: Some("model".into()),
                context_capacity_tokens: Some(1_000_000),
                active_context_occupancy_tokens: None,
                context_budget: None,
                cumulative_usage: ::contracts::InferenceUsage::default(),
                inference_rounds: 1,
                provider_retries: None,
                tool_calls: 0,
                terminal_tool_results: 0,
            }),
        };
        assert_eq!(
            task_runtime_identity(Some(&task)).2,
            "window 1,000k · history unknown (missing context budget projection)"
        );
    }

    #[test]
    fn typed_budget_renders_model_window_history_and_rollout_labels() {
        let mut state = projected_state();
        let (identity, metrics) = {
            let task = state.tasks.first_mut().unwrap();
            (
                task_runtime_identity(Some(task)).2,
                runtime_metrics(Some(task)),
            )
        };
        let diagnostic = state.context_diagnostic();
        assert!(identity.contains("window 1,000k"));
        assert!(identity.contains("profile input 1,000k"));
        assert!(identity.contains("history 83k / 915k"));
        assert!(metrics.contains("child limit 200k"));
        assert!(metrics.contains("root remaining unknown (no active Agent rollout)"));
        assert!(metrics.contains("task usage 10k in / 500 out"));
        assert!(diagnostic.contains("Window: 1,000k (source: runtime model capability"));
        assert!(diagnostic.contains("child limit 200k (source: effective admission config"));
    }

    fn projected_context_budget() -> ::contracts::ContextBudgetProjection {
        let runtime_source = ::contracts::ContextBudgetSource::new(
            ::contracts::ContextBudgetSourceKind::RuntimeModelCapability,
            "deepseek/deepseek-v4-flash[1m]",
        );
        let profile_source = ::contracts::ContextBudgetSource::new(
            ::contracts::ContextBudgetSourceKind::ActiveAgentProfile,
            "general",
        );
        let planner_source = ::contracts::ContextBudgetSource::new(
            ::contracts::ContextBudgetSourceKind::ContextBudgetPlanner,
            "ContextBudgetPlanner",
        );
        let admission_source = ::contracts::ContextBudgetSource::new(
            ::contracts::ContextBudgetSourceKind::EffectiveAdmissionConfig,
            "agent.admission.max_child_tokens",
        );
        let agent_source = ::contracts::ContextBudgetSource::new(
            ::contracts::ContextBudgetSourceKind::AgentRuntime,
            "current Agent rollout scope",
        );
        ::contracts::ContextBudgetProjection {
            model_spec: "deepseek-v4-flash[1m]".into(),
            model_context_tokens: 1_000_000.into(),
            profile_input_limit_tokens: 1_000_000.into(),
            reserved_output_tokens: 16_384.into(),
            system_and_skill_tokens: 10_000.into(),
            tool_schema_tokens: 8_000.into(),
            pending_input_tokens: 1_000.into(),
            safety_margin_tokens: 50_000.into(),
            current_history_tokens: 83_000.into(),
            admissible_history_tokens: 915_616.into(),
            compaction_threshold_tokens: 801_164.into(),
            model_source: runtime_source,
            profile_source,
            history_source: planner_source,
            rollout: ::contracts::RolloutBudgetProjection {
                root_remaining_tokens: ::contracts::RolloutBudgetValue::Unknown {
                    source: agent_source.clone(),
                    reason: ::contracts::BudgetMissingReason::NoActiveAgentRollout,
                },
                child_limit_tokens: ::contracts::RolloutBudgetValue::Known {
                    value: 200_000.into(),
                    source: admission_source,
                },
                current_agent_remaining_tokens: ::contracts::RolloutBudgetValue::Unknown {
                    source: agent_source,
                    reason: ::contracts::BudgetMissingReason::NoActiveAgentRollout,
                },
            },
        }
    }
}
