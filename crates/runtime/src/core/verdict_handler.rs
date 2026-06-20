//! Default verdict handler — maps SelfField verdicts to VerdictAction.

use std::collections::HashMap;

use base::context::Context;
use base::self_field::{Intent, Verdict, VerdictAction, VerdictHandler};
use serde_json::Value;

/// Callback for user confirmation. Returns true to proceed, false to deny.
pub type ConfirmCallback = Box<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// Explicit specification of modifications to apply to an Intent.
///
/// Unlike the raw JSON overlay, this struct distinguishes between:
/// - **Overrides**: fields that replace the original value entirely
/// - **Merges**: parameter keys that overlay without removing unmentioned keys
/// - **Additions**: constraints that are appended to the intent metadata
#[derive(Debug, Clone, Default)]
pub struct Modifications {
    /// Replace `intent.action` if set.
    pub action: Option<String>,
    /// Parameter keys that replace specific values (partial override).
    pub parameter_overrides: HashMap<String, serde_json::Value>,
    /// Parameter keys that are deep-merged into the existing parameters.
    pub parameter_merges: HashMap<String, serde_json::Value>,
    /// Replace `intent.description` if set.
    pub description: Option<String>,
    /// Constraints to append (e.g., safety rails, scope limits).
    pub add_constraints: Vec<String>,
}

impl Modifications {
    /// Parse a `Modifications` from a raw JSON modification payload.
    ///
    /// Expected JSON shape:
    /// ```json
    /// {
    ///   "action": "new_action",
    ///   "parameters": { "key": "override_value" },
    ///   "parameter_merges": { "nested_key": { "sub": "merge" } },
    ///   "description": "new description",
    ///   "add_constraints": ["constraint1", "constraint2"]
    /// }
    /// ```
    pub fn from_value(value: &Value) -> Self {
        let action = value
            .get("action")
            .and_then(Value::as_str)
            .map(String::from);

        let description = value
            .get("description")
            .and_then(Value::as_str)
            .map(String::from);

        let parameter_overrides = value
            .get("parameters")
            .and_then(Value::as_object)
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let parameter_merges = value
            .get("parameter_merges")
            .and_then(Value::as_object)
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let add_constraints = value
            .get("add_constraints")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Modifications {
            action,
            parameter_overrides,
            parameter_merges,
            description,
            add_constraints,
        }
    }
}

/// Default verdict handler that maps all 6 verdict types to actions.
///
/// - `Allow` -> `Proceed`
/// - `AllowWithModification` -> `Proceed` (modification applied via deep merge)
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

/// Deep merge a `Modifications` into an existing Intent.
///
/// - `action`: replaced if present in modifications
/// - `parameters`: keys from `parameter_overrides` replace; keys from
///   `parameter_merges` are recursively merged (nested objects merged, scalars replace)
/// - `description`: replaced if present
/// - `add_constraints`: appended as a `_constraints` array in parameters
/// - `source`: never changed
pub fn merge_intent_deep(intent: &Intent, modifications: &Modifications) -> Intent {
    let mut merged = intent.clone();

    // Override action
    if let Some(ref action) = modifications.action {
        merged.action = action.clone();
    }

    // Override description
    if let Some(ref desc) = modifications.description {
        merged.description = desc.clone();
    }

    // Apply parameter overrides (direct key replacement)
    if !modifications.parameter_overrides.is_empty() {
        if let Some(existing) = merged.parameters.as_object_mut() {
            for (key, value) in &modifications.parameter_overrides {
                existing.insert(key.clone(), value.clone());
            }
        } else {
            // intent.parameters was not an object — replace entirely
            let mut obj = serde_json::Map::new();
            for (key, value) in &modifications.parameter_overrides {
                obj.insert(key.clone(), value.clone());
            }
            merged.parameters = Value::Object(obj);
        }
    }

    // Apply parameter merges (recursive deep merge for objects, direct replace for scalars)
    if !modifications.parameter_merges.is_empty() {
        if let Some(existing) = merged.parameters.as_object_mut() {
            for (key, value) in &modifications.parameter_merges {
                deep_merge_value(
                    existing.entry(key).or_insert(Value::Null),
                    value,
                );
            }
        } else {
            let mut obj = serde_json::Map::new();
            for (key, value) in &modifications.parameter_merges {
                obj.insert(key.clone(), value.clone());
            }
            merged.parameters = Value::Object(obj);
        }
    }

    // Append constraints
    if !modifications.add_constraints.is_empty() {
        if let Some(obj) = merged.parameters.as_object_mut() {
            let constraints = obj
                .entry("_constraints")
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(arr) = constraints.as_array_mut() {
                for c in &modifications.add_constraints {
                    arr.push(Value::String(c.clone()));
                }
            }
        }
    }

    merged
}

/// Recursively merge `overlay` into `base`.
///
/// - If both are objects, merge keys recursively (overlay keys win)
/// - Otherwise, `overlay` replaces `base`
fn deep_merge_value(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                deep_merge_value(
                    base_map.entry(key).or_insert(Value::Null),
                    value,
                );
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

/// Legacy merge from raw JSON modification (backward-compatible).
///
/// Delegates to `Modifications::from_value` + `merge_intent_deep`.
#[cfg(test)]
fn merge_intent(intent: &Intent, modification: &Value) -> Intent {
    let modifications = Modifications::from_value(modification);
    merge_intent_deep(intent, &modifications)
}

impl VerdictHandler for DefaultVerdictHandler {
    fn handle(&self, verdict: &Verdict, _intent: &Intent, _ctx: &Context) -> VerdictAction {
        match verdict {
            Verdict::Allow => VerdictAction::Proceed {
                modified_intent: None,
            },
            Verdict::AllowWithModification { modification } => {
                let modifications = Modifications::from_value(modification);
                let merged = merge_intent_deep(_intent, &modifications);
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
    use base::self_field::{IntentSource, RiskLevel};
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

    // --- VerdictHandler tests (existing) ---

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
                assert_eq!(merged.action, "test");
                assert!(matches!(merged.source, IntentSource::User));
            }
            _ => panic!("expected Proceed"),
        }
    }

    // --- merge_intent legacy tests (backward compatibility) ---

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
        assert!(matches!(merged.source, IntentSource::User));
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

    // --- Deep merge tests (new) ---

    #[test]
    fn deep_merge_action_override() {
        let intent = Intent {
            action: "original_action".to_string(),
            parameters: json!({"existing": "value"}),
            source: IntentSource::User,
            description: "original desc".to_string(),
        };
        let mods = Modifications {
            action: Some("replaced_action".to_string()),
            ..Default::default()
        };
        let merged = merge_intent_deep(&intent, &mods);
        assert_eq!(merged.action, "replaced_action");
        assert_eq!(merged.description, "original desc");
        assert_eq!(merged.parameters["existing"], "value");
        assert!(matches!(merged.source, IntentSource::User));
    }

    #[test]
    fn deep_merge_parameter_overrides_partial() {
        let intent = Intent {
            action: "act".to_string(),
            parameters: json!({"a": 1, "b": 2, "c": 3}),
            source: IntentSource::User,
            description: "desc".to_string(),
        };
        let mut overrides = HashMap::new();
        overrides.insert("b".to_string(), json!(99));
        overrides.insert("d".to_string(), json!(4));
        let mods = Modifications {
            parameter_overrides: overrides,
            ..Default::default()
        };
        let merged = merge_intent_deep(&intent, &mods);
        let params = merged.parameters.as_object().unwrap();
        assert_eq!(params["a"], 1);     // untouched
        assert_eq!(params["b"], 99);    // overridden
        assert_eq!(params["c"], 3);     // untouched
        assert_eq!(params["d"], 4);     // added
    }

    #[test]
    fn deep_merge_parameter_merges_nested_objects() {
        let intent = Intent {
            action: "act".to_string(),
            parameters: json!({"config": {"debug": false, "level": 1}}),
            source: IntentSource::User,
            description: "desc".to_string(),
        };
        let mut merges = HashMap::new();
        merges.insert("config".to_string(), json!({"debug": true, "verbose": true}));
        let mods = Modifications {
            parameter_merges: merges,
            ..Default::default()
        };
        let merged = merge_intent_deep(&intent, &mods);
        let config = &merged.parameters["config"];
        assert_eq!(config["debug"], true);      // overridden
        assert_eq!(config["level"], 1);         // preserved
        assert_eq!(config["verbose"], true);    // added
    }

    #[test]
    fn deep_merge_add_constraints() {
        let intent = Intent {
            action: "act".to_string(),
            parameters: json!({"existing": "val"}),
            source: IntentSource::User,
            description: "desc".to_string(),
        };
        let mods = Modifications {
            add_constraints: vec!["no_file_delete".to_string(), "read_only_mode".to_string()],
            ..Default::default()
        };
        let merged = merge_intent_deep(&intent, &mods);
        assert_eq!(merged.parameters["existing"], "val");
        let constraints = merged.parameters["_constraints"].as_array().unwrap();
        assert_eq!(constraints.len(), 2);
        assert_eq!(constraints[0], "no_file_delete");
        assert_eq!(constraints[1], "read_only_mode");
    }

    #[test]
    fn deep_merge_identity_no_modifications() {
        let intent = Intent {
            action: "act".to_string(),
            parameters: json!({"x": 1}),
            source: IntentSource::User,
            description: "desc".to_string(),
        };
        let mods = Modifications::default();
        let merged = merge_intent_deep(&intent, &mods);
        assert_eq!(merged.action, "act");
        assert_eq!(merged.parameters["x"], 1);
        assert_eq!(merged.description, "desc");
        assert!(matches!(merged.source, IntentSource::User));
    }

    #[test]
    fn deep_merge_all_fields() {
        let intent = Intent {
            action: "original".to_string(),
            parameters: json!({"a": 1, "nested": {"x": 10}}),
            source: IntentSource::User,
            description: "original desc".to_string(),
        };
        let mut overrides = HashMap::new();
        overrides.insert("a".to_string(), json!(2));
        let mut merges = HashMap::new();
        merges.insert("nested".to_string(), json!({"y": 20}));
        let mods = Modifications {
            action: Some("new_action".to_string()),
            parameter_overrides: overrides,
            parameter_merges: merges,
            description: Some("new desc".to_string()),
            add_constraints: vec!["safe_mode".to_string()],
        };
        let merged = merge_intent_deep(&intent, &mods);
        assert_eq!(merged.action, "new_action");
        assert_eq!(merged.description, "new desc");
        assert_eq!(merged.parameters["a"], 2);          // override
        assert_eq!(merged.parameters["nested"]["x"], 10); // preserved
        assert_eq!(merged.parameters["nested"]["y"], 20); // merged
        let c = merged.parameters["_constraints"].as_array().unwrap();
        assert_eq!(c[0], "safe_mode");
        assert!(matches!(merged.source, IntentSource::User));
    }

    #[test]
    fn deep_merge_empty_parameters_on_intent() {
        let intent = test_intent(); // parameters: {}
        let mut overrides = HashMap::new();
        overrides.insert("new_key".to_string(), json!("new_val"));
        let mods = Modifications {
            parameter_overrides: overrides,
            ..Default::default()
        };
        let merged = merge_intent_deep(&intent, &mods);
        assert_eq!(merged.parameters["new_key"], "new_val");
    }

    #[test]
    fn modifications_from_value_parses_all_fields() {
        let value = json!({
            "action": "parsed_action",
            "parameters": {"a": 1},
            "parameter_merges": {"b": {"nested": true}},
            "description": "parsed desc",
            "add_constraints": ["c1", "c2"]
        });
        let mods = Modifications::from_value(&value);
        assert_eq!(mods.action.as_deref(), Some("parsed_action"));
        assert_eq!(mods.parameter_overrides["a"], json!(1));
        assert_eq!(mods.parameter_merges["b"]["nested"], true);
        assert_eq!(mods.description.as_deref(), Some("parsed desc"));
        assert_eq!(mods.add_constraints, vec!["c1", "c2"]);
    }

    #[test]
    fn modifications_from_value_empty() {
        let value = json!({});
        let mods = Modifications::from_value(&value);
        assert!(mods.action.is_none());
        assert!(mods.description.is_none());
        assert!(mods.parameter_overrides.is_empty());
        assert!(mods.parameter_merges.is_empty());
        assert!(mods.add_constraints.is_empty());
    }

    // --- VerdictHandler tests (existing, unchanged) ---

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
