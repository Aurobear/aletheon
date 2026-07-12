//! Snapshot builder — aggregates runtime state into a one-page markdown summary.
//!
//! Used by `SessionGateway::handle_snapshot()` to produce a human/Claude-readable
//! overview of the agent's current "mental state".

use crate::core::config::ExecutiveConfig;
use cognit::harness::linear::circuit_breaker::CircuitBreakerStatus;
use cognit::harness::linear::goal_tracker::GoalTracker;
use fabric::kernel::debug_bus::PerfCounter;
use std::sync::Arc;
use std::time::Instant;

/// Builds a runtime snapshot as markdown.
///
/// All data sources are read-only — the snapshot is a point-in-time
/// representation of the agent's state.
pub struct SnapshotBuilder;

impl SnapshotBuilder {
    /// Build a markdown snapshot from the given runtime state.
    pub fn build(
        session_id: &str,
        goal_tracker: &GoalTracker,
        perf: &PerfCounter,
        config: &ExecutiveConfig,
        started_at: Instant,
        circuit_breaker_status: CircuitBreakerStatus,
        tool_budget_remaining: usize,
        tool_budget_max: usize,
        recent_tool_names: &[String],
        consecutive_errors: usize,
        iteration: usize,
        plan_mode: bool,
        message_count: usize,
        storm_breaker_failure_count: usize,
    ) -> String {
        let mut md = String::new();

        // Header
        md.push_str(&format!(
            "# Aletheon Runtime Snapshot — session: {}\n\n",
            session_id
        ));

        // Goal
        md.push_str("## Current Goal\n");
        match goal_tracker.current_goal_description() {
            Some(desc) => md.push_str(&format!("- {}\n", desc)),
            None => md.push_str("- *(no goal set)*\n"),
        }
        md.push('\n');

        // Plan (sub-goals)
        if goal_tracker.current_goal_description().is_some() {
            let ctx = goal_tracker.get_context();
            if ctx.contains("Sub-goals") {
                md.push_str("## Plan\n");
                let lines: Vec<&str> = ctx.lines().collect();
                for line in &lines {
                    if line.starts_with("  [") {
                        md.push_str(&format!("{}\n", line.trim()));
                    }
                }
                md.push('\n');
            }
        }

        // Mode
        md.push_str("## Mode\n");
        md.push_str(&format!(
            "- {}\n\n",
            if plan_mode { "plan" } else { "auto" }
        ));

        // Health
        let health_status = match &circuit_breaker_status {
            CircuitBreakerStatus::Ok => "HEALTHY",
            CircuitBreakerStatus::Warning(_) => "DEGRADED",
            CircuitBreakerStatus::Tripped(_) => "TRIPPED",
        };
        let uptime = started_at.elapsed();
        let perf_snapshot = perf.snapshot();

        md.push_str("## Health\n");
        md.push_str(&format!("- Status: {}\n", health_status));
        md.push_str(&format!("- Uptime: {}\n", format_duration(uptime)));
        md.push_str(&format!(
            "- Iteration: {}/{}\n",
            iteration, config.max_iterations
        ));
        md.push_str(&format!("- Consecutive errors: {}\n", consecutive_errors));

        match &circuit_breaker_status {
            CircuitBreakerStatus::Warning(msg) => md.push_str(&format!("- ⚠️ {}\n", msg)),
            CircuitBreakerStatus::Tripped(msg) => md.push_str(&format!("- 🛑 {}\n", msg)),
            CircuitBreakerStatus::Ok => {}
        }
        md.push('\n');

        // Recent Tool Activity
        md.push_str("## Recent Tool Activity\n");
        if recent_tool_names.is_empty() {
            md.push_str("- *(no tool calls yet)*\n");
        } else {
            for name in recent_tool_names.iter().rev().take(5) {
                md.push_str(&format!("- [tool] {}\n", name));
            }
        }
        md.push('\n');

        // Resource Usage
        let rss_kb = read_rss_kb().unwrap_or(0);
        md.push_str("## Resource Usage\n");
        md.push_str(&format!("- RSS: {:.1} MB\n", rss_kb as f64 / 1024.0));
        md.push_str(&format!(
            "- Tokens used: {} / {}\n",
            perf_snapshot.tokens_in + perf_snapshot.tokens_out,
            config.context_window_tokens
        ));
        md.push_str(&format!(
            "- Tool calls this turn: {} / {}\n",
            tool_budget_max - tool_budget_remaining,
            tool_budget_max
        ));
        md.push_str(&format!("- Messages: {}\n", message_count));
        md.push('\n');

        // Open Errors
        md.push_str("## Open Errors\n");
        if perf_snapshot.error_count > 0 {
            md.push_str(&format!(
                "- {} total errors recorded\n",
                perf_snapshot.error_count
            ));
        }
        if storm_breaker_failure_count > 0 {
            md.push_str(&format!(
                "- {} unique failure patterns tracked (StormBreaker)\n",
                storm_breaker_failure_count
            ));
        }
        if perf_snapshot.error_count == 0 && storm_breaker_failure_count == 0 {
            md.push_str("*(none)*\n");
        }
        md.push('\n');

        // Constraints
        let constraints = goal_tracker.get_constraints();
        if !constraints.is_empty() {
            md.push_str("## Active Constraints\n");
            for c in constraints {
                md.push_str(&format!("- {}\n", c));
            }
            md.push('\n');
        }

        // Config summary
        md.push_str("## Active Configuration\n");
        md.push_str(&format!("- Session: {}\n", session_id));
        md.push_str(&format!("- Max iterations: {}\n", config.max_iterations));
        md.push_str(&format!(
            "- Context window: {} tokens\n",
            config.context_window_tokens
        ));
        md.push_str(&format!(
            "- Compaction: {}\n",
            if config.compaction_enabled {
                "enabled"
            } else {
                "disabled"
            }
        ));
        md.push_str(&format!(
            "- Learning: {}\n",
            if config.learning_enabled {
                "enabled"
            } else {
                "disabled"
            }
        ));

        md
    }
}

fn read_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return parts[1].parse().ok();
            }
        }
    }
    None
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{}h {}m", h, m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_with_no_goal() {
        let goal_tracker = GoalTracker::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let perf = PerfCounter::default();
        let config = ExecutiveConfig::default();

        let md = SnapshotBuilder::build(
            "test-session",
            &goal_tracker,
            &perf,
            &config,
            Instant::now(),
            CircuitBreakerStatus::Ok,
            10,
            10,
            &[],
            0,
            0,
            false,
            3,
            0,
        );

        assert!(md.contains("HEALTHY"));
        assert!(md.contains("no goal set"));
        assert!(md.contains("no tool calls yet"));
    }

    #[test]
    fn snapshot_with_goal_and_errors() {
        let mut goal_tracker = GoalTracker::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        goal_tracker.set_goal("Fix all the bugs".into());
        goal_tracker.add_sub_goal("Find the bugs".into());
        goal_tracker.add_sub_goal("Fix the bugs".into());

        let perf = PerfCounter::default();
        // Simulate some activity
        perf.tokens_in
            .store(5000, std::sync::atomic::Ordering::SeqCst);
        perf.tokens_out
            .store(3000, std::sync::atomic::Ordering::SeqCst);
        perf.error_count
            .store(2, std::sync::atomic::Ordering::SeqCst);

        let config = ExecutiveConfig::default();
        let md = SnapshotBuilder::build(
            "test-session-2",
            &goal_tracker,
            &perf,
            &config,
            Instant::now(),
            CircuitBreakerStatus::Warning("test warning".into()),
            3,
            10,
            &["bash_exec".into(), "file_read".into(), "bash_exec".into()],
            2,
            5,
            false,
            12,
            3,
        );

        assert!(md.contains("Fix all the bugs"));
        assert!(md.contains("Find the bugs"));
        assert!(md.contains("DEGRADED"));
        assert!(md.contains("test warning"));
        assert!(md.contains("bash_exec"));
        assert!(md.contains("7 / 10"));
        assert!(md.contains("2 total errors"));
        assert!(md.contains("3 unique failure patterns"));
    }

    #[test]
    fn snapshot_with_circuit_tripped() {
        let goal_tracker = GoalTracker::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let perf = PerfCounter::default();
        let config = ExecutiveConfig::default();

        let md = SnapshotBuilder::build(
            "tripped",
            &goal_tracker,
            &perf,
            &config,
            Instant::now(),
            CircuitBreakerStatus::Tripped("Loop detected!".into()),
            0,
            10,
            &vec!["bash_exec".to_string(); 12],
            4,
            12,
            false,
            20,
            5,
        );

        assert!(md.contains("TRIPPED"));
        assert!(md.contains("Loop detected!"));
    }

    #[test]
    fn format_duration_test() {
        assert_eq!(format_duration(std::time::Duration::from_secs(30)), "30s");
        assert_eq!(
            format_duration(std::time::Duration::from_secs(90)),
            "1m 30s"
        );
        assert_eq!(
            format_duration(std::time::Duration::from_secs(3661)),
            "1h 1m"
        );
    }
}
