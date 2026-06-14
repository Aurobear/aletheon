//! BoundaryLayer — pattern-matching rules engine.
//!
//! Like SELinux type enforcement: rules are evaluated in order,
//! first match wins. Uses glob patterns for action matching.

use aletheon_abi::{Intent, IntentSource, Verdict};
use aletheon_abi::self_field::RiskLevel;
use glob::Pattern;

/// Action taken when a boundary rule matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryAction {
    Deny,
    Sandbox,
    RequireConfirmation,
}

/// A boundary rule — matches intents by action pattern and optional source filter.
#[derive(Debug, Clone)]
pub struct BoundaryRule {
    /// Glob pattern for the action string (e.g., "rm *", "exec.*").
    pub action_pattern: String,
    /// If set, only match intents from this source.
    pub source_filter: Option<IntentSource>,
    /// What to do when matched.
    pub action: BoundaryAction,
    /// Risk level assigned to this rule's action.
    pub risk_level: RiskLevel,
    /// Human-readable description.
    pub description: String,
}

/// BoundaryLayer — the fast gate in the review() pipeline.
///
/// Evaluates rules in order; first match produces a verdict.
/// No match returns None (let downstream layers decide).
pub struct BoundaryLayer {
    rules: Vec<BoundaryRule>,
}

impl BoundaryLayer {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
        }
    }

    /// Add a rule. Rules are evaluated in insertion order (first match wins).
    pub fn add_rule(&mut self, rule: BoundaryRule) {
        self.rules.push(rule);
    }

    /// Bulk-load rules (replaces all existing rules).
    pub fn set_rules(&mut self, rules: Vec<BoundaryRule>) {
        self.rules = rules;
    }

    /// Return current rule count.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Check an intent against all rules. Returns Some(verdict) on first match.
    pub fn check(&self, intent: &Intent) -> Option<Verdict> {
        for rule in &self.rules {
            if !self.matches_source(rule, intent) {
                continue;
            }
            if !self.matches_pattern(rule, &intent.action) {
                continue;
            }
            return Some(self.rule_to_verdict(rule));
        }
        None
    }

    fn matches_source(&self, rule: &BoundaryRule, intent: &Intent) -> bool {
        match &rule.source_filter {
            Some(src) => std::mem::discriminant(src) == std::mem::discriminant(&intent.source),
            None => true,
        }
    }

    fn matches_pattern(&self, rule: &BoundaryRule, action: &str) -> bool {
        Pattern::new(&rule.action_pattern)
            .ok()
            .map(|p| p.matches(action))
            .unwrap_or(false)
    }

    fn rule_to_verdict(&self, rule: &BoundaryRule) -> Verdict {
        match rule.action {
            BoundaryAction::Deny => Verdict::Deny {
                reason: format!("Boundary rule: {}", rule.description),
            },
            BoundaryAction::Sandbox => Verdict::SandboxFirst {
                reason: format!("Boundary rule: {}", rule.description),
            },
            BoundaryAction::RequireConfirmation => Verdict::RequireConfirmation {
                reason: format!("Boundary rule: {}", rule.description),
                risk_level: rule.risk_level,
            },
        }
    }
}

impl Default for BoundaryLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_intent(action: &str, source: IntentSource) -> Intent {
        Intent {
            action: action.to_string(),
            parameters: json!({}),
            source,
            description: format!("test intent: {}", action),
        }
    }

    #[test]
    fn deny_pattern() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "rm *".to_string(),
            source_filter: None,
            action: BoundaryAction::Deny,
            risk_level: RiskLevel::Critical,
            description: "no rm allowed".to_string(),
        });

        let intent = make_intent("rm -rf /", IntentSource::User);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::Deny { .. })));
    }

    #[test]
    fn allow_overrides_deny() {
        let mut layer = BoundaryLayer::new();
        // First rule denies, but second matches with allow-ish action
        layer.add_rule(BoundaryRule {
            action_pattern: "exec.*".to_string(),
            source_filter: None,
            action: BoundaryAction::Deny,
            risk_level: RiskLevel::High,
            description: "deny exec".to_string(),
        });
        layer.add_rule(BoundaryRule {
            action_pattern: "exec.safe".to_string(),
            source_filter: None,
            action: BoundaryAction::Sandbox,
            risk_level: RiskLevel::Low,
            description: "sandbox safe exec".to_string(),
        });

        // "exec.safe" matches rule 1 first (glob "exec.*" matches "exec.safe")
        let intent = make_intent("exec.safe", IntentSource::Brain);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::Deny { .. })));
    }

    #[test]
    fn sandbox_high_risk() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "deploy.*".to_string(),
            source_filter: None,
            action: BoundaryAction::Sandbox,
            risk_level: RiskLevel::High,
            description: "sandbox deployments".to_string(),
        });

        let intent = make_intent("deploy.production", IntentSource::Brain);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::SandboxFirst { .. })));
    }

    #[test]
    fn no_match_returns_none() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "rm *".to_string(),
            source_filter: None,
            action: BoundaryAction::Deny,
            risk_level: RiskLevel::Critical,
            description: "no rm".to_string(),
        });

        let intent = make_intent("ls -la", IntentSource::User);
        assert!(layer.check(&intent).is_none());
    }

    #[test]
    fn source_filter_matches() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "*".to_string(),
            source_filter: Some(IntentSource::External),
            action: BoundaryAction::Deny,
            risk_level: RiskLevel::Critical,
            description: "deny all external".to_string(),
        });

        // External source matches
        let intent = make_intent("anything", IntentSource::External);
        assert!(layer.check(&intent).is_some());

        // User source does not match
        let intent = make_intent("anything", IntentSource::User);
        assert!(layer.check(&intent).is_none());
    }

    #[test]
    fn require_confirmation_verdict() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "write.*".to_string(),
            source_filter: None,
            action: BoundaryAction::RequireConfirmation,
            risk_level: RiskLevel::Medium,
            description: "confirm writes".to_string(),
        });

        let intent = make_intent("write.config", IntentSource::Brain);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::RequireConfirmation { .. })));
    }
}
