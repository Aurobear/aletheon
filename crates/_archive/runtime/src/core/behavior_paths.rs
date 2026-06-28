use base::self_field::{Intent, Verdict};

/// Which behavior path to take.
#[derive(Debug, Clone, PartialEq)]
pub enum BehaviorPath {
    /// Emergency: Event -> BodyRuntime.execute() (no Brain involved).
    Reflex,
    /// Normal: Intent -> BrainCore.think() -> Plan -> BodyRuntime.execute().
    Cognitive,
    /// Self-modification: Intent -> SelfField.review -> BrainCore.think -> execute.
    Volitional,
}

/// Routes intents through the appropriate behavior path.
pub struct BehaviorPathRouter;

impl BehaviorPathRouter {
    pub fn new() -> Self {
        Self
    }

    /// Determine which path to take based on the intent and verdict.
    pub fn select_path(intent: &Intent, verdict: &Verdict) -> BehaviorPath {
        if Self::is_emergency(intent) {
            return BehaviorPath::Reflex;
        }

        match verdict {
            Verdict::Deny { .. } => {
                // Denied actions don't go through any path; caller should short-circuit.
                BehaviorPath::Reflex
            }
            Verdict::Allow => BehaviorPath::Cognitive,
            Verdict::AllowWithModification { .. } => BehaviorPath::Cognitive,
            Verdict::SandboxFirst { .. } => BehaviorPath::Volitional,
            Verdict::RequireConfirmation { .. } => BehaviorPath::Volitional,
            Verdict::Delay { .. } => BehaviorPath::Volitional,
        }
    }

    /// Check if an intent is an emergency that should bypass BrainCore.
    pub fn is_emergency(intent: &Intent) -> bool {
        intent.action.starts_with("emergency_")
            || intent.action == "abort"
            || intent.action == "kill_process"
    }
}

impl Default for BehaviorPathRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::self_field::{IntentSource, RiskLevel};
    use serde_json::json;

    fn make_intent(action: &str) -> Intent {
        Intent {
            action: action.to_string(),
            parameters: json!({}),
            source: IntentSource::User,
            description: "test".to_string(),
        }
    }

    #[test]
    fn test_emergency_detection() {
        assert!(BehaviorPathRouter::is_emergency(&make_intent(
            "emergency_stop"
        )));
        assert!(BehaviorPathRouter::is_emergency(&make_intent("abort")));
        assert!(BehaviorPathRouter::is_emergency(&make_intent(
            "kill_process"
        )));
        assert!(!BehaviorPathRouter::is_emergency(&make_intent(
            "walk_forward"
        )));
    }

    #[test]
    fn test_select_path_allow() {
        let intent = make_intent("walk_forward");
        assert_eq!(
            BehaviorPathRouter::select_path(&intent, &Verdict::Allow),
            BehaviorPath::Cognitive
        );
    }

    #[test]
    fn test_select_path_deny() {
        let intent = make_intent("delete_system");
        assert_eq!(
            BehaviorPathRouter::select_path(
                &intent,
                &Verdict::Deny {
                    reason: "no".to_string()
                }
            ),
            BehaviorPath::Reflex
        );
    }

    #[test]
    fn test_select_path_sandbox() {
        let intent = make_intent("try_new_behavior");
        assert_eq!(
            BehaviorPathRouter::select_path(
                &intent,
                &Verdict::SandboxFirst {
                    reason: "untested".to_string()
                }
            ),
            BehaviorPath::Volitional
        );
    }

    #[test]
    fn test_select_path_require_confirmation() {
        let intent = make_intent("modify_boundary");
        assert_eq!(
            BehaviorPathRouter::select_path(
                &intent,
                &Verdict::RequireConfirmation {
                    reason: "risky".to_string(),
                    risk_level: RiskLevel::High,
                }
            ),
            BehaviorPath::Volitional
        );
    }

    #[test]
    fn test_select_path_allow_with_modification() {
        let intent = make_intent("speak");
        assert_eq!(
            BehaviorPathRouter::select_path(
                &intent,
                &Verdict::AllowWithModification {
                    modification: json!({"tone": "gentler"})
                }
            ),
            BehaviorPath::Cognitive
        );
    }

    #[test]
    fn test_select_path_delay() {
        let intent = make_intent("deploy");
        assert_eq!(
            BehaviorPathRouter::select_path(
                &intent,
                &Verdict::Delay {
                    reason: "waiting".to_string(),
                    until: "approval".to_string(),
                }
            ),
            BehaviorPath::Volitional
        );
    }

    #[test]
    fn test_emergency_overrides_verdict() {
        let intent = make_intent("emergency_stop");
        // Even with Allow verdict, emergency intent should go Reflex
        assert_eq!(
            BehaviorPathRouter::select_path(&intent, &Verdict::Allow),
            BehaviorPath::Reflex
        );
    }
}
