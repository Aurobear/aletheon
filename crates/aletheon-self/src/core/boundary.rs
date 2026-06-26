//! BoundaryLayer — pattern-matching rules engine.
//!
//! Like SELinux type enforcement: rules are evaluated in order,
//! first match wins. Uses glob patterns for action matching.

use aletheon_abi::{Intent, IntentSource, Verdict};
use aletheon_abi::self_field::RiskLevel;
use anyhow::Result;
use glob::Pattern;
use serde::{Deserialize, Serialize};

/// Action taken when a boundary rule matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundaryAction {
    Deny,
    Sandbox,
    RequireConfirmation,
}

/// A boundary rule — matches intents by action pattern and optional source filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// If true, this rule cannot be relaxed or tightened by evolution.
    pub immutable: bool,
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

    /// Add a rule with individual parameters. Returns the index of the new rule.
    pub fn add_rule_params(
        &mut self,
        pattern: &str,
        verdict: BoundaryAction,
        immutable: bool,
        origin: &str,
    ) -> usize {
        let rule = BoundaryRule {
            action_pattern: pattern.to_string(),
            source_filter: None,
            action: verdict,
            risk_level: RiskLevel::Medium,
            description: origin.to_string(),
            immutable,
        };
        let idx = self.rules.len();
        self.rules.push(rule);
        idx
    }

    /// Relax a rule: change Deny to Sandbox.
    ///
    /// Returns `true` if the rule was found, not immutable, and changed.
    /// Returns `false` if not found, immutable, or already Sandbox.
    pub fn relax_rule(&mut self, pattern: &str) -> bool {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.action_pattern == pattern) {
            if rule.immutable {
                return false;
            }
            if rule.action == BoundaryAction::Deny {
                rule.action = BoundaryAction::Sandbox;
                return true;
            }
        }
        false
    }

    /// Tighten a rule: change Sandbox to Deny.
    ///
    /// Returns `true` if the rule was found, not immutable, and changed.
    /// Returns `false` if not found, immutable, or already Deny.
    pub fn tighten_rule(&mut self, pattern: &str) -> bool {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.action_pattern == pattern) {
            if rule.immutable {
                return false;
            }
            if rule.action == BoundaryAction::Sandbox {
                rule.action = BoundaryAction::Deny;
                return true;
            }
        }
        false
    }

    /// Bulk-load rules (replaces all existing rules).
    pub fn set_rules(&mut self, rules: Vec<BoundaryRule>) {
        self.rules = rules;
    }

    /// Return current rule count.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Return count of immutable rules.
    pub fn immutable_rule_count(&self) -> usize {
        self.rules.iter().filter(|r| r.immutable).count()
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

    /// Persist all boundary rules to the SQLite store.
    pub fn save_to_store(&self, store: &crate::core::store::SelfFieldStore) -> Result<()> {
        let conn = store.conn();

        conn.execute("DELETE FROM boundary_rules", [])?;

        let mut stmt = conn.prepare(
            "INSERT INTO boundary_rules (action_pattern, source_filter, action, risk_level, description, immutable) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for rule in &self.rules {
            let action_json = serde_json::to_string(&rule.action)?;
            let risk_json = serde_json::to_string(&rule.risk_level)?;
            let source_json = rule
                .source_filter
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            stmt.execute(rusqlite::params![
                rule.action_pattern,
                source_json,
                action_json,
                risk_json,
                rule.description,
                rule.immutable as i32,
            ])?;
        }
        Ok(())
    }

    /// Load boundary rules from the SQLite store, replacing current rules.
    pub fn load_from_store(&mut self, store: &crate::core::store::SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        let mut stmt = conn.prepare(
            "SELECT action_pattern, source_filter, action, risk_level, description, immutable FROM boundary_rules",
        )?;

        let loaded: Vec<BoundaryRule> = stmt
            .query_map([], |row| {
                let action_json: String = row.get(2)?;
                let risk_json: String = row.get(3)?;
                let source_json: Option<String> = row.get(1)?;
                let immutable: i32 = row.get(5)?;

                let action: BoundaryAction =
                    serde_json::from_str(&action_json).unwrap_or(BoundaryAction::Deny);
                let risk_level: RiskLevel =
                    serde_json::from_str(&risk_json).unwrap_or(RiskLevel::Medium);
                let source_filter: Option<IntentSource> = source_json
                    .and_then(|s| serde_json::from_str(&s).ok());

                Ok(BoundaryRule {
                    action_pattern: row.get(0)?,
                    source_filter,
                    action,
                    risk_level,
                    description: row.get(4)?,
                    immutable: immutable != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if !loaded.is_empty() {
            self.rules = loaded;
        }
        Ok(())
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
            immutable: false,
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
            immutable: false,
        });
        layer.add_rule(BoundaryRule {
            action_pattern: "exec.safe".to_string(),
            source_filter: None,
            action: BoundaryAction::Sandbox,
            risk_level: RiskLevel::Low,
            description: "sandbox safe exec".to_string(),
            immutable: false,
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
            immutable: false,
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
            immutable: false,
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
            immutable: false,
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
            immutable: false,
        });

        let intent = make_intent("write.config", IntentSource::Brain);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::RequireConfirmation { .. })));
    }

    #[test]
    fn add_rule_params_basic() {
        let mut layer = BoundaryLayer::new();
        let idx = layer.add_rule_params("test.*", BoundaryAction::Deny, true, "test origin");
        assert_eq!(idx, 0);
        assert_eq!(layer.rule_count(), 1);
    }

    #[test]
    fn relax_rule_deny_to_sandbox() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "danger.*".to_string(),
            source_filter: None,
            action: BoundaryAction::Deny,
            risk_level: RiskLevel::High,
            description: "test".to_string(),
            immutable: false,
        });
        assert!(layer.relax_rule("danger.*"));
        // Verify it's now Sandbox
        let intent = make_intent("danger.critical", IntentSource::User);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::SandboxFirst { .. })));
    }

    #[test]
    fn tighten_rule_sandbox_to_deny() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "risky.*".to_string(),
            source_filter: None,
            action: BoundaryAction::Sandbox,
            risk_level: RiskLevel::High,
            description: "test".to_string(),
            immutable: false,
        });
        assert!(layer.tighten_rule("risky.*"));
        // Verify it's now Deny
        let intent = make_intent("risky.thing", IntentSource::User);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::Deny { .. })));
    }

    #[test]
    fn relax_immutable_rule_fails() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "rm *".to_string(),
            source_filter: None,
            action: BoundaryAction::Deny,
            risk_level: RiskLevel::Critical,
            description: "immutable rm".to_string(),
            immutable: true,
        });
        assert!(!layer.relax_rule("rm *"));
        // Still Deny
        let intent = make_intent("rm -rf /", IntentSource::User);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::Deny { .. })));
    }

    #[test]
    fn tighten_immutable_rule_fails() {
        let mut layer = BoundaryLayer::new();
        layer.add_rule(BoundaryRule {
            action_pattern: "watch.*".to_string(),
            source_filter: None,
            action: BoundaryAction::Sandbox,
            risk_level: RiskLevel::Low,
            description: "immutable watch".to_string(),
            immutable: true,
        });
        assert!(!layer.tighten_rule("watch.*"));
        // Still Sandbox
        let intent = make_intent("watch.logs", IntentSource::User);
        let verdict = layer.check(&intent);
        assert!(matches!(verdict, Some(Verdict::SandboxFirst { .. })));
    }

    #[test]
    fn relax_nonexistent_rule() {
        let mut layer = BoundaryLayer::new();
        assert!(!layer.relax_rule("nope.*"));
    }

    #[test]
    fn tighten_nonexistent_rule() {
        let mut layer = BoundaryLayer::new();
        assert!(!layer.tighten_rule("nope.*"));
    }
}
