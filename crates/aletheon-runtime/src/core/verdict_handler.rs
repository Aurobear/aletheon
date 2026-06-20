//! Default verdict handler — maps SelfField verdicts to VerdictAction.

use aletheon_abi::context::Context;
use aletheon_abi::self_field::{Intent, Verdict, VerdictAction, VerdictHandler};
use serde_json::Value;

/// Callback for user confirmation. Returns true to proceed, false to deny.
pub type ConfirmCallback = Box<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// Default verdict handler that maps all 6 verdict types to actions.
///
/// - `Allow` -> `Proceed`
/// - `AllowWithModification` -> `Proceed` (modification noted but not parsed into Intent yet)
/// - `Deny` -> `ShortCircuit`
/// - `RequireConfirmation` -> calls callback if present, otherwise `ShortCircuit`
/// - `SandboxFirst` -> `SandboxThenProceed`
/// - `Delay` -> `ShortCircuit`
pub struct DefaultVerdictHandler {
    pub confirm_callback: Option<ConfirmCallback>,
}

impl DefaultVerdictHandler {
    /// Create a handler with no confirmation callback.
    /// `RequireConfirmation` verdicts will be auto-denied.
    pub fn new() -> Self {
        Self {
            confirm_callback: None,
        }
    }

    /// Create a handler with a confirmation callback.
    pub fn with_confirm_callback(callback: ConfirmCallback) -> Self {
        Self {
            confirm_callback: Some(callback),
        }
    }
}

impl Default for DefaultVerdictHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Merge a modification payload (partial overlay) into an existing Intent.
///
/// - `action` key overrides `intent.action`
/// - `parameters` key shallow-merges into `intent.parameters` (object values only)
/// - `description` key overrides `intent.description`
/// - `source` is never changed by modification
fn merge_intent(intent: &Intent, modification: &Value) -> Intent {
    let mut merged = intent.clone();

    if let Some(action) = modification.get("action").and_then(Value::as_str) {
        merged.action = action.to_string();
    }

    if let Some(new_params) = modification.get("parameters").and_then(|v| v.as_object()) {
        if let Some(existing) = merged.parameters.as_object_mut() {
            for (key, value) in new_params {
                existing.insert(key.clone(), value.clone());
            }
        } else {
            // intent.parameters was not an object — replace entirely
            merged.parameters = Value::Object(new_params.clone());
        }
    }

    if let Some(desc) = modification.get("description").and_then(Value::as_str) {
        merged.description = desc.to_string();
    }

    merged
}

impl VerdictHandler for DefaultVerdictHandler {
    fn handle(&self, verdict: &Verdict, _intent: &Intent, _ctx: &Context) -> VerdictAction {
        match verdict {
            Verdict::Allow => VerdictAction::Proceed {
                modified_intent: None,
            },
            Verdict::AllowWithModification { modification } => {
                let merged = merge_intent(_intent, modification);
                VerdictAction::Proceed {
                    modified_intent: Some(merged),
                }
            }
            Verdict::Deny { reason } => VerdictAction::ShortCircuit {
                response: format!("Denied by SelfField: {}", reason),
            },
            Verdict::RequireConfirmation { reason, risk_level } => {
                if let Some(ref cb) = self.confirm_callback {
                    if cb(reason, &format!("{:?}", risk_level)) {
                        VerdictAction::Proceed {
                            modified_intent: None,
                        }
                    } else {
                        VerdictAction::ShortCircuit {
                            response: format!("User declined: {}", reason),
                        }
                    }
                } else {
                    VerdictAction::ShortCircuit {
                        response: format!(
                            "Confirmation required (no handler): {}",
                            reason
                        ),
                    }
                }
            }
            Verdict::SandboxFirst { reason } => VerdictAction::SandboxThenProceed {
                reason: reason.clone(),
            },
            Verdict::Delay { reason, .. } => VerdictAction::ShortCircuit {
                response: format!("Delayed: {}", reason),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::self_field::{IntentSource, RiskLevel};
    use serde_json::json;

    fn test_intent() -> Intent {
        Intent {
            action: "test".to_string(),
            parameters: json!({}),
            source: IntentSource::User,
            description: "test intent".to_string(),
        }
    }

    fn test_ctx() -> Context {
        Context::new("test", std::path::PathBuf::from("/tmp"))
    }

    #[test]
    fn allow_returns_proceed() {
        let handler = DefaultVerdictHandler::new();
        let action = handler.handle(&Verdict::Allow, &test_intent(), &test_ctx());
        match action {
            VerdictAction::Proceed { modified_intent } => {
                assert!(modified_intent.is_none());
            }
            _ => panic!("expected Proceed"),
        }
    }

    #[test]
    fn allow_with_modification_returns_merged_intent() {
        let handler = DefaultVerdictHandler::new();
        let verdict = Verdict::AllowWithModification {
            modification: json!({"description": "modified description", "extra": "ignored"}),
        };
        let action = handler.handle(&verdict, &test_intent(), &test_ctx());
        match action {
            VerdictAction::Proceed { modified_intent } => {
                let merged = modified_intent.expect("expected merged intent");
                assert_eq!(merged.description, "modified description");
                // action and source unchanged
                assert_eq!(merged.action, "test");
                assert!(matches!(merged.source, IntentSource::User));
            }
            _ => panic!("expected Proceed"),
        }
    }

    #[test]
    fn merge_intent_overrides_action() {
        let intent = test_intent();
        let modification = json!({"action": "rewritten"});
        let merged = merge_intent(&intent, &modification);
        assert_eq!(merged.action, "rewritten");
        assert_eq!(merged.description, "test intent");
        assert!(matches!(merged.source, IntentSource::User));
    }

    #[test]
    fn merge_intent_shallow_merges_parameters() {
        let intent = Intent {
            action: "test".to_string(),
            parameters: json!({"key_a": "original", "key_b": 42}),
            source: IntentSource::User,
            description: "test intent".to_string(),
        };
        let modification = json!({"parameters": {"key_b": 99, "key_c": "new"}});
        let merged = merge_intent(&intent, &modification);
        let params = merged.parameters.as_object().unwrap();
        assert_eq!(params["key_a"], "original");
        assert_eq!(params["key_b"], 99);
        assert_eq!(params["key_c"], "new");
    }

    #[test]
    fn merge_intent_overrides_description() {
        let intent = test_intent();
        let modification = json!({"description": "new desc"});
        let merged = merge_intent(&intent, &modification);
        assert_eq!(merged.description, "new desc");
        assert_eq!(merged.action, "test");
    }

    #[test]
    fn merge_intent_preserves_source() {
        let intent = test_intent();
        let modification = json!({"source": "Brain", "action": "hijack"});
        let merged = merge_intent(&intent, &modification);
        // source must never change
        assert!(matches!(merged.source, IntentSource::User));
        // but action does
        assert_eq!(merged.action, "hijack");
    }

    #[test]
    fn merge_intent_all_keys() {
        let intent = test_intent();
        let modification = json!({
            "action": "new_action",
            "parameters": {"x": 1},
            "description": "new_desc"
        });
        let merged = merge_intent(&intent, &modification);
        assert_eq!(merged.action, "new_action");
        assert_eq!(merged.parameters["x"], 1);
        assert_eq!(merged.description, "new_desc");
    }

    #[test]
    fn merge_intent_empty_modification_is_noop() {
        let intent = test_intent();
        let modification = json!({});
        let merged = merge_intent(&intent, &modification);
        assert_eq!(merged.action, "test");
        assert_eq!(merged.description, "test intent");
    }

    #[test]
    fn deny_returns_short_circuit() {
        let handler = DefaultVerdictHandler::new();
        let verdict = Verdict::Deny {
            reason: "forbidden".to_string(),
        };
        let action = handler.handle(&verdict, &test_intent(), &test_ctx());
        match action {
            VerdictAction::ShortCircuit { response } => {
                assert!(response.contains("Denied by SelfField"));
                assert!(response.contains("forbidden"));
            }
            _ => panic!("expected ShortCircuit"),
        }
    }

    #[test]
    fn require_confirmation_with_approving_callback() {
        let handler = DefaultVerdictHandler::with_confirm_callback(Box::new(|_, _| true));
        let verdict = Verdict::RequireConfirmation {
            reason: "risky".to_string(),
            risk_level: RiskLevel::High,
        };
        let action = handler.handle(&verdict, &test_intent(), &test_ctx());
        match action {
            VerdictAction::Proceed { .. } => {}
            _ => panic!("expected Proceed from approving callback"),
        }
    }

    #[test]
    fn require_confirmation_with_denying_callback() {
        let handler = DefaultVerdictHandler::with_confirm_callback(Box::new(|_, _| false));
        let verdict = Verdict::RequireConfirmation {
            reason: "risky".to_string(),
            risk_level: RiskLevel::High,
        };
        let action = handler.handle(&verdict, &test_intent(), &test_ctx());
        match action {
            VerdictAction::ShortCircuit { response } => {
                assert!(response.contains("User declined"));
            }
            _ => panic!("expected ShortCircuit from denying callback"),
        }
    }

    #[test]
    fn require_confirmation_without_callback() {
        let handler = DefaultVerdictHandler::new();
        let verdict = Verdict::RequireConfirmation {
            reason: "needs approval".to_string(),
            risk_level: RiskLevel::Medium,
        };
        let action = handler.handle(&verdict, &test_intent(), &test_ctx());
        match action {
            VerdictAction::ShortCircuit { response } => {
                assert!(response.contains("no handler"));
                assert!(response.contains("needs approval"));
            }
            _ => panic!("expected ShortCircuit when no callback"),
        }
    }

    #[test]
    fn sandbox_first_returns_sandbox_then_proceed() {
        let handler = DefaultVerdictHandler::new();
        let verdict = Verdict::SandboxFirst {
            reason: "untested".to_string(),
        };
        let action = handler.handle(&verdict, &test_intent(), &test_ctx());
        match action {
            VerdictAction::SandboxThenProceed { reason } => {
                assert_eq!(reason, "untested");
            }
            _ => panic!("expected SandboxThenProceed"),
        }
    }

    #[test]
    fn delay_returns_short_circuit() {
        let handler = DefaultVerdictHandler::new();
        let verdict = Verdict::Delay {
            reason: "rate limited".to_string(),
            until: "cooldown".to_string(),
        };
        let action = handler.handle(&verdict, &test_intent(), &test_ctx());
        match action {
            VerdictAction::ShortCircuit { response } => {
                assert!(response.contains("Delayed"));
                assert!(response.contains("rate limited"));
            }
            _ => panic!("expected ShortCircuit for Delay"),
        }
    }
}
