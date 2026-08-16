//! Independent execution policy engine.
//!
//! Separates policy logic from the tool runner for testability and reuse.
//! Supports layered configuration (system > user > project) with overlay merge.

use serde::{Deserialize, Serialize};

/// Policy decision, ordered by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Decision {
    Allow,
    #[default]
    Prompt,
    Forbidden,
}

/// Result of checking a command against the policy.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub decision: Decision,
    pub matched_rules: Vec<String>,
}

/// A pattern token for prefix matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternToken {
    Exact(String),
    Alternatives(Vec<String>),
}

/// A single prefix-based policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixRule {
    pub program: String,
    pub decision: Decision,
    #[serde(default)]
    pub pattern: Vec<PatternToken>,
}

impl PrefixRule {
    pub fn new(program: &str, decision: Decision) -> Self {
        Self {
            program: program.to_string(),
            decision,
            pattern: Vec::new(),
        }
    }

    pub fn with_pattern(mut self, pattern: Vec<PatternToken>) -> Self {
        self.pattern = pattern;
        self
    }

    /// Check if this rule matches the given command.
    /// Returns Some(description) on match, None otherwise.
    pub fn matches(&self, cmd: &[String]) -> Option<String> {
        if cmd.is_empty() || cmd[0] != self.program {
            return None;
        }

        if self.pattern.is_empty() {
            // No pattern = match any invocation of this program
            return Some(format!("{} (any)", self.program));
        }

        // Match pattern tokens against command args
        let args = &cmd[1..];
        if args.len() < self.pattern.len() {
            return None;
        }

        for (i, token) in self.pattern.iter().enumerate() {
            match token {
                PatternToken::Exact(s) => {
                    if args.get(i).map(|a| a.as_str()) != Some(s.as_str()) {
                        return None;
                    }
                }
                PatternToken::Alternatives(alts) => {
                    let arg = args.get(i)?;
                    if !alts.iter().any(|a| a == arg) {
                        return None;
                    }
                }
            }
        }

        Some(format!("{} {}", self.program, self.pattern_desc()))
    }

    fn pattern_desc(&self) -> String {
        self.pattern
            .iter()
            .map(|t| match t {
                PatternToken::Exact(s) => s.clone(),
                PatternToken::Alternatives(alts) => format!("[{}]", alts.join("|")),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Network protocol for network rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProtocol {
    Http,
    Https,
    Any,
}

/// A network access rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRule {
    pub host: String,
    pub protocol: NetworkProtocol,
    pub decision: Decision,
}

/// The independent policy engine.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Policy {
    rules: Vec<PrefixRule>,
    network_rules: Vec<NetworkRule>,
}

impl Policy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rule(&mut self, rule: PrefixRule) {
        self.rules.push(rule);
    }

    pub fn add_network_rule(&mut self, rule: NetworkRule) {
        self.network_rules.push(rule);
    }

    /// Check a command against the policy.
    /// Returns the highest-severity decision from all matching rules.
    /// If no rules match, falls back to the provided heuristics function.
    pub fn check(&self, cmd: &[String], heuristics: fn(&[String]) -> Decision) -> Evaluation {
        if cmd.is_empty() {
            return Evaluation {
                decision: Decision::Prompt,
                matched_rules: vec!["empty command".into()],
            };
        }

        let mut matched = Vec::new();
        let mut max_decision = Decision::Allow;

        for rule in &self.rules {
            if let Some(desc) = rule.matches(cmd) {
                matched.push(desc);
                if rule.decision > max_decision {
                    max_decision = rule.decision;
                }
            }
        }

        if matched.is_empty() {
            // No explicit rule — use heuristics fallback
            let decision = heuristics(cmd);
            return Evaluation {
                decision,
                matched_rules: vec!["heuristics".into()],
            };
        }

        Evaluation {
            decision: max_decision,
            matched_rules: matched,
        }
    }

    /// Check network access against the policy.
    pub fn check_network(&self, host: &str, protocol: NetworkProtocol) -> Evaluation {
        for rule in &self.network_rules {
            if rule.host == host
                && (rule.protocol == NetworkProtocol::Any || rule.protocol == protocol)
            {
                return Evaluation {
                    decision: rule.decision,
                    matched_rules: vec![format!("network:{}", host)],
                };
            }
        }

        Evaluation {
            decision: Decision::Allow,
            matched_rules: vec!["default:allow".into()],
        }
    }

    /// Merge a higher-precedence overlay. Later rules override earlier ones.
    pub fn merge_overlay(&mut self, overlay: Policy) {
        self.rules.extend(overlay.rules);
        self.network_rules.extend(overlay.network_rules);
    }
}

/// Default heuristics for unmatched commands.
pub fn default_heuristics(cmd: &[String]) -> Decision {
    if cmd.is_empty() {
        return Decision::Prompt;
    }
    match cmd[0].as_str() {
        // Safe read-only
        "cat" | "ls" | "pwd" | "echo" | "which" | "whoami" | "head" | "tail" | "wc" => {
            Decision::Allow
        }
        // Dangerous
        "rm" | "rmdir" | "mkfs" | "dd" | "format" | "shutdown" | "reboot" => Decision::Forbidden,
        // Unknown
        _ => Decision::Prompt,
    }
}

/// Load a policy from a TOML string.
pub fn load_policy_from_str(toml_str: &str) -> Result<Policy, String> {
    #[derive(Deserialize)]
    struct PolicyConfig {
        #[serde(default)]
        rules: Vec<RuleConfig>,
        #[serde(default)]
        network_rules: Vec<NetworkRuleConfig>,
    }

    #[derive(Deserialize)]
    struct RuleConfig {
        program: String,
        decision: Decision,
        #[serde(default)]
        pattern: Vec<PatternToken>,
    }

    #[derive(Deserialize)]
    struct NetworkRuleConfig {
        host: String,
        protocol: NetworkProtocol,
        decision: Decision,
    }

    let config: PolicyConfig = toml::from_str(toml_str).map_err(|e| e.to_string())?;

    let mut policy = Policy::new();
    for rule in config.rules {
        policy.add_rule(PrefixRule {
            program: rule.program,
            decision: rule.decision,
            pattern: rule.pattern,
        });
    }
    for nr in config.network_rules {
        policy.add_network_rule(NetworkRule {
            host: nr.host,
            protocol: nr.protocol,
            decision: nr.decision,
        });
    }
    Ok(policy)
}

/// Load a policy from layered config files (system > user > project).
/// Later layers have higher precedence.
pub fn load_policy_layered(
    system: Option<&str>,
    user: Option<&str>,
    project: Option<&str>,
) -> Result<Policy, String> {
    let mut policy = Policy::new();

    if let Some(toml_str) = system {
        let overlay = load_policy_from_str(toml_str)?;
        policy.merge_overlay(overlay);
    }
    if let Some(toml_str) = user {
        let overlay = load_policy_from_str(toml_str)?;
        policy.merge_overlay(overlay);
    }
    if let Some(toml_str) = project {
        let overlay = load_policy_from_str(toml_str)?;
        policy.merge_overlay(overlay);
    }

    Ok(policy)
}
