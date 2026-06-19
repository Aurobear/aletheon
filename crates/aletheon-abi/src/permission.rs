//! Permission model for tool execution control.
//!
//! Defines the permission mode, per-tool rules, and a context that resolves
//! whether a tool invocation should be allowed, denied, or prompted to the user.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// High-level permission mode for the session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Default mode: dangerous actions require user approval.
    #[default]
    Default,
    /// Accept all edits without prompting (still blocks dangerous by default).
    AcceptEdits,
    /// Plan-only: no execution, only produce plans.
    Plan,
    /// Bypass all permission checks (for fully automated / trusted environments).
    BypassAll,
}

/// The outcome of a permission check.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
}

/// A single rule that matches a tool (optionally with a glob pattern) and
/// specifies the resulting behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PermissionRule {
    /// Tool name to match (e.g. "bash", "write_file").
    pub tool: String,
    /// Optional glob pattern matched against the action summary.
    /// A trailing `*` acts as a prefix wildcard.
    pub pattern: Option<String>,
    /// Behavior when this rule matches.
    pub behavior: PermissionBehavior,
}

impl PermissionRule {
    /// Returns `true` if this rule matches the given tool name and action summary.
    ///
    /// The rule matches when:
    /// - `self.tool` equals `tool`, AND
    /// - `self.pattern` is `None`, OR
    /// - `self.pattern` is `Some(p)` where `p` is a prefix of `action_summary`
    ///   after stripping a trailing `*` (if present).
    pub fn matches(&self, tool: &str, action_summary: &str) -> bool {
        if self.tool != tool {
            return false;
        }
        match &self.pattern {
            None => true,
            Some(p) => {
                if let Some(prefix) = p.strip_suffix('*') {
                    action_summary.starts_with(prefix)
                } else {
                    action_summary == p
                }
            }
        }
    }
}

/// Aggregated permission context for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionContext {
    /// Current permission mode.
    pub mode: PermissionMode,
    /// Ordered list of per-tool rules (first match wins).
    pub rules: Vec<PermissionRule>,
    /// Set of tool:action_summary strings that have been approved in this session.
    pub session_approvals: HashSet<String>,
}

impl PermissionContext {
    /// Resolve the permission behavior for a tool invocation.
    ///
    /// Resolution order:
    /// 1. If the exact `tool:action_summary` key is in `session_approvals` → Allow.
    /// 2. First matching rule → rule's behavior.
    /// 3. Mode default:
    ///    - `BypassAll` → Allow
    ///    - `Plan` → Deny
    ///    - `AcceptEdits` → Allow (unless `is_dangerous` → Ask)
    ///    - `Default` → Allow (unless `is_dangerous` → Ask)
    pub fn resolve(
        &self,
        tool: &str,
        action_summary: &str,
        is_dangerous: bool,
    ) -> PermissionBehavior {
        // 1. Session approvals
        let key = format!("{}:{}", tool, action_summary);
        if self.session_approvals.contains(&key) {
            return PermissionBehavior::Allow;
        }

        // 2. First matching rule
        for rule in &self.rules {
            if rule.matches(tool, action_summary) {
                return rule.behavior;
            }
        }

        // 3. Mode defaults
        match self.mode {
            PermissionMode::BypassAll => PermissionBehavior::Allow,
            PermissionMode::Plan => PermissionBehavior::Deny,
            PermissionMode::AcceptEdits => {
                if is_dangerous {
                    PermissionBehavior::Ask
                } else {
                    PermissionBehavior::Allow
                }
            }
            PermissionMode::Default => {
                if is_dangerous {
                    PermissionBehavior::Ask
                } else {
                    PermissionBehavior::Allow
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_default_asks_for_dangerous() {
        let ctx = PermissionContext::default(); // Default mode
        assert_eq!(
            ctx.resolve("bash", "rm -rf /", true),
            PermissionBehavior::Ask
        );
        // Non-dangerous is allowed
        assert_eq!(
            ctx.resolve("bash", "ls -la", false),
            PermissionBehavior::Allow
        );
    }

    #[test]
    fn rule_matches_glob_pattern() {
        let rule = PermissionRule {
            tool: "git".to_string(),
            pattern: Some("git *".to_string()),
            behavior: PermissionBehavior::Allow,
        };
        // "git status" starts with "git " → matches
        assert!(rule.matches("git", "git status"));
        // "git push origin main" also matches
        assert!(rule.matches("git", "git push origin main"));
        // "gitstatus" does NOT match (no space after "git")
        assert!(!rule.matches("git", "gitstatus"));
        // Wrong tool → no match
        assert!(!rule.matches("rm", "rm -rf /"));
    }

    #[test]
    fn session_approval_overrides_rules() {
        let mut ctx = PermissionContext {
            mode: PermissionMode::Default,
            rules: vec![PermissionRule {
                tool: "bash".to_string(),
                pattern: None,
                behavior: PermissionBehavior::Deny,
            }],
            ..Default::default()
        };
        ctx.session_approvals.insert("bash:reboot".to_string());
        assert_eq!(
            ctx.resolve("bash", "reboot", true),
            PermissionBehavior::Allow
        );
    }
}
