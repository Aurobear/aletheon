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

use fabric::protocol::client::{ActivitySnapshot, ActivityState, TaskPhase, TaskSnapshot};
use fabric::WorkspacePolicy;

use super::{
    state::{AppState, ContextDisplay},
    term_compat::TermCaps,
};

/// The primary work surface for an active Session.
pub struct TaskConsole<'a> {
    pub state: &'a AppState,
    pub caps: &'a TermCaps,
    pub workspace: &'a WorkspacePolicy,
    pub selected_activity: Option<usize>,
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
        render_task_header(chunks[0], buf, self.state, self.caps, self.workspace);

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
            activity.kind == fabric::ActivityKind::Robot
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
    workspace: &WorkspacePolicy,
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
    let project = workspace
        .cwd()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let activity = activity_summary(&state.activities);
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
                " session {session} · provider {provider} · model {model} · permission {permission}"
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
    let mut lines = state
        .items
        .values()
        .filter(|item| matches!(item.kind.as_str(), "user" | "assistant"))
        .map(|item| {
            let (prefix, color) = if item.kind == "user" {
                ("> ", theme.user_icon)
            } else {
                ("", theme.text)
            };
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(color)),
                Span::styled(item.content.clone(), Style::default().fg(color)),
            ])
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No projected conversation yet",
            Style::default().fg(theme.text_muted),
        )));
    }
    let visible = lines.len().saturating_sub(inner.height as usize);
    let lines = lines.into_iter().skip(visible).collect::<Vec<_>>();
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(inner, buf);
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
    activities.sort_by_key(|(_, activity)| activity.updated_at);
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
            fabric::CheckpointMutationCoverage::Full => "full",
            fabric::CheckpointMutationCoverage::BestEffort => "best-effort",
            fabric::CheckpointMutationCoverage::NonRollbackable => "non-rollbackable",
        };
        let rollback = match review.rollback_action {
            fabric::CheckpointRollbackAction::AutomaticAllowed => "automatic rollback available",
            fabric::CheckpointRollbackAction::ExplicitApprovalRequired => {
                "rollback requires explicit approval"
            }
            fabric::CheckpointRollbackAction::Unavailable => "rollback unavailable",
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
            fabric::CheckpointReviewSettlement::Partial
                | fabric::CheckpointReviewSettlement::Conflicted
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
        return ("—", "—", "unknown".into());
    };
    let provider = facts.effective_provider.as_deref().unwrap_or("—");
    let model = facts.effective_model.as_deref().unwrap_or("—");
    let context = match (
        facts.active_context_occupancy_tokens,
        facts.context_capacity_tokens,
    ) {
        (Some(used), Some(capacity)) => format!(
            "{}/{}",
            ContextDisplay::format_tokens(used),
            ContextDisplay::format_tokens(capacity)
        ),
        (_, Some(capacity)) => format!("—/{}", ContextDisplay::format_tokens(capacity)),
        _ => "unknown".into(),
    };
    (provider, model, context)
}

fn runtime_metrics(task: Option<&TaskSnapshot>) -> String {
    let Some(task) = task else {
        return "budget unknown · cache unknown".into();
    };
    let budget = task
        .budget
        .as_ref()
        .map(|value| bounded_value(value, 24))
        .unwrap_or_else(|| "unknown".into());
    let Some(facts) = task.runtime_facts.as_ref() else {
        return format!("budget {budget} · cache unknown");
    };
    let cache = match facts.cumulative_usage.cache_telemetry {
        fabric::CacheTelemetry::Reported => format!(
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
        fabric::CacheTelemetry::Unsupported => "unsupported".into(),
        fabric::CacheTelemetry::Unknown => "unknown".into(),
    };
    format!(
        "budget {budget} · cache {cache} · infer {} · retries {} · tools {}/{}",
        facts.inference_rounds,
        facts
            .provider_retries
            .map_or_else(|| "unknown".into(), |value| value.to_string()),
        facts.terminal_tool_results,
        facts.tool_calls
    )
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
    let progress = activity
        .progress
        .as_ref()
        .map(|value| format!(" · {}", bounded_value(value, 32)))
        .unwrap_or_default();
    let selection = if selected { ">" } else { " " };
    Line::from(vec![
        Span::styled(selection, Style::default().fg(caps.theme().accent)),
        Span::styled(format!(" {} ", caps.bullet()), Style::default().fg(color)),
        Span::styled(format!("{state:9}"), Style::default().fg(color)),
        Span::raw(format!("{}{progress}", activity.label)),
    ])
}

fn bounded_value(value: &serde_json::Value, limit: usize) -> String {
    let rendered = match value {
        serde_json::Value::String(value) => value.clone(),
        value => value.to_string(),
    };
    if rendered.chars().count() <= limit {
        rendered
    } else {
        format!("{}…", rendered.chars().take(limit).collect::<String>())
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

fn task_settlement(settlement: fabric::TaskSettlement) -> &'static str {
    match settlement {
        fabric::TaskSettlement::Accepted => "accepted",
        fabric::TaskSettlement::RepairRequired => "repair-required",
        fabric::TaskSettlement::Blocked => "blocked",
        fabric::TaskSettlement::Cancelled => "cancelled",
        fabric::TaskSettlement::RolledBack => "rolled-back",
        fabric::TaskSettlement::Failed => "failed",
    }
}

struct RobotSummary<'a> {
    device: &'a str,
    scene: &'a str,
    settlement: &'a str,
    bridge_digest: &'a str,
    report_sha256: &'a str,
    attempt_count: u64,
    evidence_count: usize,
}

fn robot_summary(activities: &[ActivitySnapshot]) -> Option<RobotSummary<'_>> {
    let progress = activities
        .iter()
        .rev()
        .find(|activity| {
            activity.kind == fabric::ActivityKind::Robot
                && activity
                    .progress
                    .as_ref()
                    .and_then(|progress| progress.get("stage"))
                    .and_then(serde_json::Value::as_str)
                    == Some("settle")
        })?
        .progress
        .as_ref()?;
    Some(RobotSummary {
        device: progress.get("device")?.as_str()?,
        scene: progress.get("scene")?.as_str()?,
        settlement: progress.get("settlement")?.as_str()?,
        bridge_digest: progress.get("bridge_digest")?.as_str()?,
        report_sha256: progress.get("report_sha256")?.as_str()?,
        attempt_count: progress.get("attempt_count")?.as_u64()?,
        evidence_count: progress
            .get("evidence_refs")?
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
    })
}

fn short_id(value: &str) -> &str {
    value.get(..12).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::{UiItem, UiItemStatus};

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
            workspace: &workspace,
            selected_activity,
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
            session_id: fabric::SessionId("session-1234567890".into()),
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
            runtime_facts: Some(fabric::TaskRuntimeFacts {
                effective_provider: Some("deepseek".into()),
                effective_model: Some("deepseek-v4-flash".into()),
                context_capacity_tokens: Some(1_000_000),
                active_context_occupancy_tokens: Some(8_000),
                cumulative_usage: fabric::InferenceUsage::reported(
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
            turn_id: fabric::TurnId::new(),
            parent_activity_id: None,
            kind: fabric::ActivityKind::Command,
            label: "cargo check".into(),
            state: ActivityState::Running,
            started_at: 1,
            updated_at: 2,
            progress: Some(serde_json::json!("42/100 lines")),
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
    fn non_robot_activity_does_not_take_permanent_screen_space() {
        let rendered = rendered_text(120, 40, &projected_state());
        assert!(rendered.contains("1 active"));
        assert!(!rendered.contains("42/100 lines"));
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
    fn u_tui_003_failed_runtime_does_not_pollute_conversation_text() {
        let mut state = projected_state();
        state.activities[0].kind = fabric::ActivityKind::Runtime;
        state.activities[0].state = ActivityState::Failed;
        state.activities[0].label = "sub-agent reviewer".into();
        let rendered = rendered_text(120, 40, &state);
        assert!(rendered.contains("1 recorded"));
        assert!(!rendered.contains("sub-agent reviewer"));
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
            workspace: &workspace,
            selected_activity: None,
        }
        .render(area, &mut buffer);
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("TASK"));
        assert!(rendered.contains("Conversation"));
        assert!(!rendered.contains("cargo check"));
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
            session_id: fabric::SessionId("robot-session".into()),
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
            settlement: Some(fabric::TaskSettlement::Blocked),
            review_findings: vec![],
            runtime_facts: None,
        });
        let turn_id = fabric::TurnId::new();
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
                kind: fabric::ActivityKind::Robot,
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
                artifact_refs: (stage == "observe" || settle)
                    .then(|| vec!["artifact://sha256/evidence".into()])
                    .unwrap_or_default(),
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
            session_id: fabric::SessionId("session".into()),
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
            runtime_facts: Some(fabric::TaskRuntimeFacts {
                effective_provider: Some("provider".into()),
                effective_model: Some("model".into()),
                context_capacity_tokens: Some(1_000_000),
                active_context_occupancy_tokens: None,
                cumulative_usage: fabric::InferenceUsage::default(),
                inference_rounds: 1,
                provider_retries: None,
                tool_calls: 0,
                terminal_tool_results: 0,
            }),
        };
        assert_eq!(task_runtime_identity(Some(&task)).2, "—/1M");
    }
}
