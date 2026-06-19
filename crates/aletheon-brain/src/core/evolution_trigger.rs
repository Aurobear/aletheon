//! EvolutionTrigger — decides when and how to run behavior evolution.
//!
//! Monitors reflection history for triggers: consecutive impasses,
//! periodic intervals, and confidence drops. When triggered, delegates
//! to `ExperienceSummarizer` to produce an `EvolutionLogEntry`.

use aletheon_abi::brain::{
    EvolutionLogEntry, ReflectionEntry, ReflectionOutcome, ReflectionTrigger,
};
use chrono::{DateTime, Duration, Utc};

/// Configuration for the evolution trigger.
#[derive(Debug, Clone)]
pub struct EvolutionTriggerConfig {
    /// Number of consecutive failures (Impasse-triggered or Failure outcome)
    /// required to trigger evolution. Default: 3.
    pub failure_threshold: u32,
    /// Hours between automatic periodic evolution cycles. Default: 6.
    pub periodic_interval_hours: u64,
    /// Fractional drop (0.0-1.0) in average confidence that triggers
    /// a confidence-drop evolution. Default: 0.2 (20%).
    pub confidence_drop_threshold: f64,
}

impl Default for EvolutionTriggerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            periodic_interval_hours: 6,
            confidence_drop_threshold: 0.2,
        }
    }
}

/// Decision returned by `check_should_evolve`.
#[derive(Debug, Clone, PartialEq)]
pub enum EvolutionDecision {
    /// Evolution should run now.
    TriggerNow { reason: String },
    /// Not yet — check again at the given time.
    NotYet { next_check: DateTime<Utc> },
    /// An evolution cycle is already running.
    AlreadyRunning,
}

/// The evolution trigger — monitors reflection history and decides
/// when behavior evolution should run.
pub struct EvolutionTrigger {
    config: EvolutionTriggerConfig,
    /// Timestamp of the last completed evolution cycle.
    last_evolution: Option<DateTime<Utc>>,
    /// Whether an evolution cycle is currently in progress.
    running: bool,
}

impl EvolutionTrigger {
    pub fn new(config: EvolutionTriggerConfig) -> Self {
        Self {
            config,
            last_evolution: None,
            running: false,
        }
    }

    /// Update the last-evolution timestamp (e.g. after storing an EvolutionLogEntry).
    pub fn set_last_evolution(&mut self, time: DateTime<Utc>) {
        self.last_evolution = Some(time);
    }

    /// Mark the trigger as currently running (prevents re-entrance).
    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    /// Check whether evolution should be triggered based on recent reflections.
    pub fn check_should_evolve(&self, recent_reflections: &[ReflectionEntry]) -> EvolutionDecision {
        if self.running {
            return EvolutionDecision::AlreadyRunning;
        }

        // --- Condition 1: Consecutive impasse/failure threshold ---
        if let Some(reason) = self.check_consecutive_failures(recent_reflections) {
            return EvolutionDecision::TriggerNow { reason };
        }

        // --- Condition 2: Confidence drop ---
        if let Some(reason) = self.check_confidence_drop(recent_reflections) {
            return EvolutionDecision::TriggerNow { reason };
        }

        // --- Condition 3: Periodic interval ---
        if let Some(reason) = self.check_periodic() {
            return EvolutionDecision::TriggerNow { reason };
        }

        // Not yet — compute next check time
        let next_check = self
            .last_evolution
            .map(|t| t + Duration::hours(self.config.periodic_interval_hours as i64))
            .unwrap_or_else(|| {
                Utc::now() + Duration::hours(self.config.periodic_interval_hours as i64)
            });

        EvolutionDecision::NotYet { next_check }
    }

    /// Run a full evolution cycle: analyze patterns, generate adjustments,
    /// produce an EvolutionLogEntry.
    ///
    /// Uses the existing `ExperienceSummarizer::summarize` for pattern detection.
    pub fn run_evolution_cycle(
        &mut self,
        reflections: &[ReflectionEntry],
    ) -> Option<EvolutionLogEntry> {
        self.running = true;

        let trigger_reason = self
            .determine_trigger_reason(reflections)
            .unwrap_or_else(|| "manual".to_string());

        let result = match super::ExperienceSummarizer::summarize(reflections) {
            Some(mut entry) => {
                // Override the trigger field with the actual reason from EvolutionTrigger
                entry.trigger = trigger_reason;
                self.last_evolution = Some(Utc::now());
                Some(entry)
            }
            None => None,
        };

        self.running = false;
        result
    }

    // --- Private helpers ---

    fn check_consecutive_failures(&self, reflections: &[ReflectionEntry]) -> Option<String> {
        if reflections.is_empty() {
            return None;
        }

        let mut consecutive = 0u32;
        // reflections are assumed most-recent-first
        for r in reflections {
            let is_failure =
                r.outcome == ReflectionOutcome::Failure || r.trigger == ReflectionTrigger::Impasse;
            if is_failure {
                consecutive += 1;
            } else {
                break;
            }
        }

        if consecutive >= self.config.failure_threshold {
            Some(format!(
                "{} consecutive failures/impasses detected (threshold: {})",
                consecutive, self.config.failure_threshold
            ))
        } else {
            None
        }
    }

    fn check_confidence_drop(&self, reflections: &[ReflectionEntry]) -> Option<String> {
        if reflections.len() < 4 {
            return None; // need at least 2 batches of 2
        }

        let mid = reflections.len() / 2;
        let recent_batch = &reflections[..mid];
        let older_batch = &reflections[mid..];

        let avg_recent: f64 =
            recent_batch.iter().map(|r| r.confidence).sum::<f64>() / recent_batch.len() as f64;
        let avg_older: f64 =
            older_batch.iter().map(|r| r.confidence).sum::<f64>() / older_batch.len() as f64;

        if avg_older > 0.0 {
            let drop = (avg_older - avg_recent) / avg_older;
            if drop > self.config.confidence_drop_threshold {
                return Some(format!(
                    "Average confidence dropped {:.0}% (from {:.2} to {:.2}, threshold {:.0}%)",
                    drop * 100.0,
                    avg_older,
                    avg_recent,
                    self.config.confidence_drop_threshold * 100.0,
                ));
            }
        }

        None
    }

    fn check_periodic(&self) -> Option<String> {
        match self.last_evolution {
            Some(last) => {
                let elapsed = Utc::now() - last;
                let interval = Duration::hours(self.config.periodic_interval_hours as i64);
                if elapsed >= interval {
                    Some(format!(
                        "Periodic evolution: {} hours since last evolution (interval: {}h)",
                        elapsed.num_hours(),
                        self.config.periodic_interval_hours,
                    ))
                } else {
                    None
                }
            }
            // Never evolved before — trigger immediately
            None => Some("First evolution cycle — no previous evolution found".to_string()),
        }
    }

    fn determine_trigger_reason(&self, reflections: &[ReflectionEntry]) -> Option<String> {
        // Try each trigger in priority order
        self.check_consecutive_failures(reflections)
            .or_else(|| self.check_confidence_drop(reflections))
            .or_else(|| self.check_periodic())
    }
}

impl Default for EvolutionTrigger {
    fn default() -> Self {
        Self::new(EvolutionTriggerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(
        outcome: ReflectionOutcome,
        trigger: ReflectionTrigger,
        confidence: f64,
    ) -> ReflectionEntry {
        ReflectionEntry {
            id: format!("ref-{}", uuid::Uuid::new_v4()),
            timestamp: Utc::now(),
            trigger,
            task_summary: "test task".to_string(),
            outcome,
            what_worked: vec![],
            what_failed: vec![],
            learned: vec![],
            behavior_changes: vec![],
            confidence,
        }
    }

    fn failure_entry() -> ReflectionEntry {
        make_entry(ReflectionOutcome::Failure, ReflectionTrigger::Impasse, 0.1)
    }

    fn success_entry(confidence: f64) -> ReflectionEntry {
        make_entry(
            ReflectionOutcome::Success,
            ReflectionTrigger::TaskComplete,
            confidence,
        )
    }

    // --- check_should_evolve tests ---

    #[test]
    fn already_running_returns_already_running() {
        let mut trigger = EvolutionTrigger::default();
        trigger.set_running(true);

        let decision = trigger.check_should_evolve(&[]);
        assert_eq!(decision, EvolutionDecision::AlreadyRunning);
    }

    #[test]
    fn no_reflections_and_no_last_evolution_triggers_first_cycle() {
        let trigger = EvolutionTrigger::default();
        let decision = trigger.check_should_evolve(&[]);
        match decision {
            EvolutionDecision::TriggerNow { reason } => {
                assert!(reason.contains("First evolution"), "got: {}", reason);
            }
            other => panic!("Expected TriggerNow, got {:?}", other),
        }
    }

    #[test]
    fn consecutive_failures_triggers() {
        let mut trigger = EvolutionTrigger::default();
        // Set last evolution to the past so periodic doesn't fire first
        trigger.set_last_evolution(Utc::now());

        let reflections = vec![failure_entry(), failure_entry(), failure_entry()];

        let decision = trigger.check_should_evolve(&reflections);
        match decision {
            EvolutionDecision::TriggerNow { reason } => {
                assert!(reason.contains("consecutive failures"), "got: {}", reason);
            }
            other => panic!("Expected TriggerNow, got {:?}", other),
        }
    }

    #[test]
    fn two_failures_not_enough() {
        let mut trigger = EvolutionTrigger::default();
        trigger.set_last_evolution(Utc::now());

        let reflections = vec![failure_entry(), failure_entry(), success_entry(0.9)];

        let decision = trigger.check_should_evolve(&reflections);
        // Should not trigger on consecutive failures (only 2 consecutive at start)
        assert!(matches!(decision, EvolutionDecision::NotYet { .. }));
    }

    #[test]
    fn periodic_triggers_when_interval_exceeded() {
        let mut trigger = EvolutionTrigger::default();
        // Set last evolution to 7 hours ago
        trigger.set_last_evolution(Utc::now() - Duration::hours(7));

        let reflections = vec![success_entry(0.9)];
        let decision = trigger.check_should_evolve(&reflections);
        match decision {
            EvolutionDecision::TriggerNow { reason } => {
                assert!(reason.contains("Periodic"), "got: {}", reason);
            }
            other => panic!("Expected TriggerNow, got {:?}", other),
        }
    }

    #[test]
    fn periodic_does_not_trigger_within_interval() {
        let mut trigger = EvolutionTrigger::default();
        trigger.set_last_evolution(Utc::now() - Duration::hours(2));

        let reflections = vec![success_entry(0.9)];
        let decision = trigger.check_should_evolve(&reflections);
        assert!(matches!(decision, EvolutionDecision::NotYet { .. }));
    }

    #[test]
    fn confidence_drop_triggers() {
        let mut trigger = EvolutionTrigger::default();
        trigger.set_last_evolution(Utc::now());

        // Older batch (high confidence) then recent batch (low confidence)
        let reflections = vec![
            success_entry(0.2), // recent (index 0 = most recent)
            success_entry(0.3),
            success_entry(0.9), // older
            success_entry(0.85),
        ];

        let decision = trigger.check_should_evolve(&reflections);
        match decision {
            EvolutionDecision::TriggerNow { reason } => {
                assert!(reason.contains("confidence dropped"), "got: {}", reason);
            }
            other => panic!("Expected TriggerNow, got {:?}", other),
        }
    }

    #[test]
    fn confidence_drop_not_enough_data() {
        let mut trigger = EvolutionTrigger::default();
        trigger.set_last_evolution(Utc::now());

        // Only 3 entries — not enough for two batches
        let reflections = vec![success_entry(0.1), success_entry(0.9), success_entry(0.9)];

        let decision = trigger.check_should_evolve(&reflections);
        // No consecutive failures, no confidence drop (insufficient data), no periodic
        assert!(matches!(decision, EvolutionDecision::NotYet { .. }));
    }

    #[test]
    fn consecutive_failures_priority_over_periodic() {
        let mut trigger = EvolutionTrigger::default();
        // Recently evolved, so periodic won't fire
        trigger.set_last_evolution(Utc::now());

        let reflections = vec![failure_entry(), failure_entry(), failure_entry()];

        let decision = trigger.check_should_evolve(&reflections);
        match decision {
            EvolutionDecision::TriggerNow { reason } => {
                // Should be consecutive failures, not periodic
                assert!(reason.contains("consecutive"), "got: {}", reason);
            }
            other => panic!("Expected TriggerNow, got {:?}", other),
        }
    }

    // --- run_evolution_cycle tests ---

    #[test]
    fn evolution_cycle_produces_entry_with_consecutive_failures() {
        let mut trigger = EvolutionTrigger::default();

        let reflections = vec![failure_entry(), failure_entry(), failure_entry()];

        let entry = trigger.run_evolution_cycle(&reflections);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert!(entry.trigger.contains("consecutive"));
        assert_eq!(entry.basis.len(), 3);
    }

    #[test]
    fn evolution_cycle_empty_reflections_returns_none() {
        let mut trigger = EvolutionTrigger::default();
        let entry = trigger.run_evolution_cycle(&[]);
        assert!(entry.is_none());
    }

    #[test]
    fn evolution_cycle_single_reflection_returns_none() {
        let mut trigger = EvolutionTrigger::default();
        let reflections = vec![success_entry(0.9)];
        // ExperienceSummarizer::summarize returns None for a single entry with no pattern
        let entry = trigger.run_evolution_cycle(&reflections);
        assert!(entry.is_none());
    }

    #[test]
    fn evolution_cycle_clears_running_flag() {
        let mut trigger = EvolutionTrigger::default();
        let reflections = vec![success_entry(0.9)];

        // Even if summarize returns None, running should be cleared
        let _ = trigger.run_evolution_cycle(&reflections);
        assert!(!trigger.running);
    }

    #[test]
    fn evolution_cycle_updates_last_evolution() {
        let mut trigger = EvolutionTrigger::default();
        assert!(trigger.last_evolution.is_none());

        let reflections = vec![failure_entry(), failure_entry(), failure_entry()];

        trigger.run_evolution_cycle(&reflections);
        assert!(trigger.last_evolution.is_some());
    }

    #[test]
    fn evolution_cycle_with_high_failure_reflections() {
        let mut trigger = EvolutionTrigger::default();

        let reflections = vec![
            failure_entry(),
            failure_entry(),
            failure_entry(),
            success_entry(0.9),
        ];

        let entry = trigger.run_evolution_cycle(&reflections).unwrap();
        assert!(!entry.patterns_detected.is_empty() || !entry.adjustments.is_empty());
    }

    // --- config tests ---

    #[test]
    fn default_config_values() {
        let config = EvolutionTriggerConfig::default();
        assert_eq!(config.failure_threshold, 3);
        assert_eq!(config.periodic_interval_hours, 6);
        assert_eq!(config.confidence_drop_threshold, 0.2);
    }

    #[test]
    fn custom_config_threshold() {
        let config = EvolutionTriggerConfig {
            failure_threshold: 5,
            periodic_interval_hours: 12,
            confidence_drop_threshold: 0.3,
        };
        let trigger = EvolutionTrigger::new(config.clone());
        trigger.check_should_evolve(&[]);
        // Just verifying construction succeeds with custom values
        assert_eq!(config.failure_threshold, 5);
    }

    // --- EvolutionDecision equality ---

    #[test]
    fn evolution_decision_eq() {
        let d1 = EvolutionDecision::TriggerNow {
            reason: "test".to_string(),
        };
        let d2 = EvolutionDecision::TriggerNow {
            reason: "test".to_string(),
        };
        assert_eq!(d1, d2);

        let d3 = EvolutionDecision::AlreadyRunning;
        let d4 = EvolutionDecision::AlreadyRunning;
        assert_eq!(d3, d4);
    }

    #[test]
    fn evolution_decision_not_eq() {
        let d1 = EvolutionDecision::TriggerNow {
            reason: "a".to_string(),
        };
        let d2 = EvolutionDecision::TriggerNow {
            reason: "b".to_string(),
        };
        assert_ne!(d1, d2);
    }
}
