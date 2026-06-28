//! Pre-reflective awareness signals — idea ideae in the hot path.
//!
//! Lightweight, rule-based state detectors that emit `AwarenessSignal`
//! during the cognitive loop. No LLM calls — pure functions that detect
//! states like confusion, uncertainty, confidence, and goal shifts from
//! observable loop metrics and response text.
//!
//! These signals are the "idea of an idea" — awareness that arises
//! inherently in the act of thinking, not as a separate reflection.

use base::self_field::{AwarenessCore, AwarenessExtension, SelfAwareness, SelfState};
use base::ui_event::AwarenessLevel;
use chrono::{DateTime, Utc};

/// Lightweight awareness signal emitted during cognitive loop.
/// No LLM call — pure rule-based state detection.
#[derive(Debug, Clone)]
pub struct AwarenessSignal {
    pub step: StepType,
    pub action: String,
    pub detected_state: Option<SelfState>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum StepType {
    LoopStart,
    ThinkingComplete,
    ToolCallStart,
    ToolCallEnd,
    FinalResponse,
}

/// Detect impasse from consecutive errors and iteration count.
///
/// Returns `Confused` if:
/// - 3 or more consecutive tool errors, OR
/// - iteration exceeds half of max_iterations
pub fn detect_impasse(
    consecutive_errors: usize,
    iteration: usize,
    max_iterations: usize,
) -> Option<SelfState> {
    if consecutive_errors >= 3 || iteration > max_iterations / 2 {
        Some(SelfState::Confused)
    } else {
        None
    }
}

/// Detect uncertainty from hedging language in LLM response.
///
/// Scans for hedging patterns like "not sure", "it depends", etc.
/// Returns `Hesitant` if any pattern is found.
pub fn detect_uncertainty(response: &str) -> Option<SelfState> {
    let hedging_patterns = [
        "not sure",
        "it depends",
        "possibly",
        "might be",
        "unclear",
        "i think",
        "perhaps",
        "hard to say",
        "不确定",
        "可能",
        "也许",
    ];
    let lower = response.to_lowercase();
    if hedging_patterns.iter().any(|p| lower.contains(p)) {
        Some(SelfState::Hesitant)
    } else {
        None
    }
}

/// Detect confidence from plan critique results.
///
/// Returns `Hesitant` if critical issues were found, `Confident` otherwise.
pub fn detect_confidence(has_critical_issues: bool) -> Option<SelfState> {
    if has_critical_issues {
        Some(SelfState::Hesitant)
    } else {
        Some(SelfState::Confident)
    }
}

/// Detect goal shift from tool name changes.
///
/// Compares the domain prefix (before first `_`) of the last two tools.
/// Returns `Curious` if domains differ, suggesting a goal shift.
pub fn detect_goal_shift(tool_sequence: &[String]) -> Option<SelfState> {
    if tool_sequence.len() >= 2 {
        let last = tool_sequence.last().unwrap();
        let prev = &tool_sequence[tool_sequence.len() - 2];
        let last_domain = last.split('_').next().unwrap_or(last);
        let prev_domain = prev.split('_').next().unwrap_or(prev);
        if last_domain != prev_domain {
            Some(SelfState::Curious)
        } else {
            None
        }
    } else {
        None
    }
}

/// Convert a `SelfState` to an `AwarenessLevel` for TUI display.
pub fn self_state_to_awareness_level(state: &SelfState) -> AwarenessLevel {
    match state {
        SelfState::Confident => AwarenessLevel::Confident,
        SelfState::Hesitant => AwarenessLevel::Hesitant,
        SelfState::Confused => AwarenessLevel::Confused,
        SelfState::Curious => AwarenessLevel::Curious,
        SelfState::Focused => AwarenessLevel::Confident,
        SelfState::Other(_) => AwarenessLevel::Confident,
    }
}

/// Convert awareness signals to UiEvent pairs for TUI display.
///
/// Filters out signals with no detected state, then maps each to
/// an `(AwarenessLevel, context_string)` pair.
pub fn signals_to_ui_events(
    signals: &[AwarenessSignal],
) -> Vec<(AwarenessLevel, String)> {
    signals
        .iter()
        .filter_map(|s| {
            let state = s.detected_state.as_ref()?;
            let level = self_state_to_awareness_level(state);
            let context = match state {
                SelfState::Confused => format!("Impasse detected at step {:?}", s.step),
                SelfState::Hesitant => format!("Uncertainty detected at step {:?}", s.step),
                SelfState::Curious => format!("Goal shift detected: {}", s.action),
                SelfState::Confident => format!("Confident at step {:?}", s.step),
                _ => return None,
            };
            Some((level, context))
        })
        .collect()
}

/// Convert collected signals into SelfAwareness entries for storage.
///
/// Each signal becomes a `(action, SelfAwareness)` pair suitable for
/// `EpisodicMemory::store_awareness()`.
pub fn signals_to_awareness(
    signals: &[AwarenessSignal],
) -> Vec<(String, SelfAwareness)> {
    signals
        .iter()
        .map(|s| {
            let core = AwarenessCore {
                action: s.action.clone(),
                aware: true,
            };
            let extensions = match &s.detected_state {
                Some(state) => vec![AwarenessExtension::SelfState {
                    state: state.clone(),
                }],
                None => vec![],
            };
            let awareness = SelfAwareness { core, extensions };
            (s.action.clone(), awareness)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to assert SelfState variant matches
    macro_rules! assert_state {
        ($expr:expr, $variant:pat) => {
            assert!(matches!($expr, $variant), "expected {}, got {:?}", stringify!($variant), $expr);
        };
    }

    #[test]
    fn detect_impasse_three_consecutive_errors() {
        assert_state!(detect_impasse(3, 1, 10), Some(SelfState::Confused));
    }

    #[test]
    fn detect_impasse_many_consecutive_errors() {
        assert_state!(detect_impasse(5, 1, 10), Some(SelfState::Confused));
    }

    #[test]
    fn detect_impasse_past_half_iterations() {
        // iteration 6 > max_iterations/2 = 5
        assert_state!(detect_impasse(0, 6, 10), Some(SelfState::Confused));
    }

    #[test]
    fn detect_impasse_few_errors_no_impasse() {
        assert!(detect_impasse(1, 1, 10).is_none());
        assert!(detect_impasse(2, 1, 10).is_none());
    }

    #[test]
    fn detect_impasse_early_iteration_no_impasse() {
        assert!(detect_impasse(0, 3, 10).is_none());
    }

    #[test]
    fn detect_impasse_exact_half_no_impasse() {
        // iteration == max_iterations / 2 is not >, so no impasse
        assert!(detect_impasse(0, 5, 10).is_none());
    }

    #[test]
    fn detect_uncertainty_with_hedging_english() {
        assert_state!(
            detect_uncertainty("I'm not sure about this approach"),
            Some(SelfState::Hesitant)
        );
        assert_state!(
            detect_uncertainty("It depends on the context"),
            Some(SelfState::Hesitant)
        );
        assert_state!(
            detect_uncertainty("This might be the wrong path"),
            Some(SelfState::Hesitant)
        );
        assert_state!(
            detect_uncertainty("Perhaps we should reconsider"),
            Some(SelfState::Hesitant)
        );
    }

    #[test]
    fn detect_uncertainty_with_hedging_chinese() {
        assert_state!(
            detect_uncertainty("我不确定这个方案"),
            Some(SelfState::Hesitant)
        );
        assert_state!(
            detect_uncertainty("这可能是对的"),
            Some(SelfState::Hesitant)
        );
    }

    #[test]
    fn detect_uncertainty_case_insensitive() {
        assert_state!(
            detect_uncertainty("NOT SURE if this works"),
            Some(SelfState::Hesitant)
        );
    }

    #[test]
    fn detect_uncertainty_confident_text_none() {
        assert!(detect_uncertainty("This is the correct solution").is_none());
        assert!(detect_uncertainty("Execute the plan successfully").is_none());
        assert!(detect_uncertainty("All steps completed").is_none());
    }

    #[test]
    fn detect_uncertainty_empty_text_none() {
        assert!(detect_uncertainty("").is_none());
    }

    #[test]
    fn detect_confidence_no_issues() {
        assert_state!(detect_confidence(false), Some(SelfState::Confident));
    }

    #[test]
    fn detect_confidence_with_issues() {
        assert_state!(detect_confidence(true), Some(SelfState::Hesitant));
    }

    #[test]
    fn detect_goal_shift_different_domains() {
        let tools = vec!["file_read".to_string(), "shell_execute".to_string()];
        assert_state!(detect_goal_shift(&tools), Some(SelfState::Curious));
    }

    #[test]
    fn detect_goal_shift_same_domain() {
        let tools = vec!["file_read".to_string(), "file_write".to_string()];
        assert!(detect_goal_shift(&tools).is_none());
    }

    #[test]
    fn detect_goal_shift_single_tool() {
        let tools = vec!["file_read".to_string()];
        assert!(detect_goal_shift(&tools).is_none());
    }

    #[test]
    fn detect_goal_shift_empty() {
        let tools: Vec<String> = vec![];
        assert!(detect_goal_shift(&tools).is_none());
    }

    #[test]
    fn detect_goal_shift_no_underscore() {
        let tools = vec!["bash".to_string(), "grep".to_string()];
        assert_state!(detect_goal_shift(&tools), Some(SelfState::Curious));
    }

    #[test]
    fn signals_to_awareness_converts_correctly() {
        let signals = vec![
            AwarenessSignal {
                step: StepType::LoopStart,
                action: "thinking".to_string(),
                detected_state: Some(SelfState::Focused),
                timestamp: Utc::now(),
            },
            AwarenessSignal {
                step: StepType::ToolCallEnd,
                action: "tool_exec".to_string(),
                detected_state: None,
                timestamp: Utc::now(),
            },
        ];

        let result = signals_to_awareness(&signals);
        assert_eq!(result.len(), 2);

        // First signal has SelfState extension
        assert_eq!(result[0].0, "thinking");
        assert!(result[0].1.core.aware);
        assert_eq!(result[0].1.extensions.len(), 1);
        assert!(matches!(
            &result[0].1.extensions[0],
            AwarenessExtension::SelfState {
                state: SelfState::Focused
            }
        ));

        // Second signal has no extensions
        assert_eq!(result[1].0, "tool_exec");
        assert!(result[1].1.core.aware);
        assert!(result[1].1.extensions.is_empty());
    }

    #[test]
    fn signals_to_awareness_empty_input() {
        let signals: Vec<AwarenessSignal> = vec![];
        let result = signals_to_awareness(&signals);
        assert!(result.is_empty());
    }

    #[test]
    fn signals_to_awareness_preserves_all_states() {
        let states: Vec<(SelfState, &str)> = vec![
            (SelfState::Focused, "Focused"),
            (SelfState::Confused, "Confused"),
            (SelfState::Confident, "Confident"),
            (SelfState::Hesitant, "Hesitant"),
            (SelfState::Curious, "Curious"),
        ];

        for (state, expected_name) in states {
            let signals = vec![AwarenessSignal {
                step: StepType::LoopStart,
                action: "test".to_string(),
                detected_state: Some(state),
                timestamp: Utc::now(),
            }];
            let result = signals_to_awareness(&signals);
            assert!(matches!(
                &result[0].1.extensions[0],
                AwarenessExtension::SelfState { .. }
            ), "expected SelfState extension for {}", expected_name);
        }
    }
}
