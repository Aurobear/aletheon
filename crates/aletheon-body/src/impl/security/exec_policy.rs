//! Execution policy rules engine for bash command evaluation.
//!
//! Inspired by Codex execpolicy — supports prefix-based rule matching,
//! `.rules` file loading from `~/.aletheon/rules/` and `.aletheon/rules/`,
//! network rules, and runtime amendment (learning approved prefixes).
//!
//! ## Rule file format
//!
//! ```text
//! # Lines starting with # are comments
//! # Blank lines are ignored
//! # Format: action program [prefix1,prefix2,...]
//! deny   mkfs
//! ask    rm -rf
//! ask    dd
//! ask    systemctl stop
//! ask    iptables
//! allow  git  status,log,diff,add,commit,push,pull
//! allow  cargo  check,test,build,clippy
//! allow  ls,cat,grep,find,head,tail,wc,sort,uniq
//! allow  python3,python,node,npm
//! ```

use std::fs;
use std::path::Path;

use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyAction {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone)]
pub struct PolicyDecision {
    pub action: PolicyAction,
    pub reason: String,
    pub matched_rule: Option<String>,
}

impl PolicyDecision {
    fn allow(reason: impl Into<String>, rule: impl Into<String>) -> Self {
        Self {
            action: PolicyAction::Allow,
            reason: reason.into(),
            matched_rule: Some(rule.into()),
        }
    }

    fn deny(reason: impl Into<String>, rule: impl Into<String>) -> Self {
        Self {
            action: PolicyAction::Deny,
            reason: reason.into(),
            matched_rule: Some(rule.into()),
        }
    }

    fn ask(reason: impl Into<String>, rule: impl Into<String>) -> Self {
        Self {
            action: PolicyAction::Ask,
            reason: reason.into(),
            matched_rule: Some(rule.into()),
        }
    }

    fn default_ask(reason: impl Into<String>) -> Self {
        Self {
            action: PolicyAction::Ask,
            reason: reason.into(),
            matched_rule: None,
        }
    }
}

/// A rule loaded from a `.rules` file or added at runtime.
#[derive(Debug, Clone)]
pub struct PolicyRule {
    /// The program this rule applies to (e.g. "git", "rm", "mkfs").
    pub program: String,
    /// Allowed prefixes for this program (e.g. "git status", "git push").
    /// If empty, the rule applies to the bare program invocation.
    pub prefixes: Vec<String>,
    /// What to do when this rule matches.
    pub action: PolicyAction,
}

/// A pre-approved command prefix with its source.
#[derive(Debug, Clone)]
pub struct AllowPrefix {
    pub prefix: String,
    pub source: String,
}

/// Network access rule.
#[derive(Debug, Clone)]
pub struct NetworkRule {
    pub host_pattern: String,
    pub protocol: String,
    pub action: PolicyAction,
}

// ---------------------------------------------------------------------------
// ExecPolicyEngine
// ---------------------------------------------------------------------------

pub struct ExecPolicyEngine {
    rules: Vec<PolicyRule>,
    allow_prefixes: Vec<AllowPrefix>,
    network_rules: Vec<NetworkRule>,
}

impl ExecPolicyEngine {
    /// Create a new engine with default safe rules.
    pub fn new() -> Self {
        let rules = vec![
            // Deny dangerous filesystem commands
            PolicyRule {
                program: "mkfs".into(),
                prefixes: vec![],
                action: PolicyAction::Deny,
            },
            // Ask for destructive / high-privilege commands
            PolicyRule {
                program: "rm".into(),
                prefixes: vec!["-rf".into()],
                action: PolicyAction::Ask,
            },
            PolicyRule {
                program: "dd".into(),
                prefixes: vec![],
                action: PolicyAction::Ask,
            },
            PolicyRule {
                program: "systemctl".into(),
                prefixes: vec!["stop".into()],
                action: PolicyAction::Ask,
            },
            PolicyRule {
                program: "iptables".into(),
                prefixes: vec![],
                action: PolicyAction::Ask,
            },
        ];

        Self {
            rules,
            allow_prefixes: Vec::new(),
            network_rules: Vec::new(),
        }
    }

    /// Load `.rules` files from a directory.
    ///
    /// Each file is parsed line-by-line. Lines starting with `#` are comments;
    /// blank lines are ignored. Format: `action program [prefix,...]`
    pub fn load_from_dir(&mut self, dir: &Path) -> anyhow::Result<()> {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                debug!(dir = %dir.display(), error = %e, "Rules directory not found, skipping");
                return Ok(());
            }
        };

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rules") {
                continue;
            }
            self.load_rules_file(&path)?;
        }
        Ok(())
    }

    /// Parse and load a single `.rules` file.
    fn load_rules_file(&mut self, path: &Path) -> anyhow::Result<()> {
        let content = fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown")
            .to_string();

        for (line_no, raw_line) in content.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            if parts.len() < 2 {
                warn!(file = %file_name, line = line_no + 1, raw = %raw_line, "Skipping malformed rule line");
                continue;
            }

            let action = match parts[0].to_lowercase().as_str() {
                "allow" => PolicyAction::Allow,
                "deny" => PolicyAction::Deny,
                "ask" => PolicyAction::Ask,
                other => {
                    warn!(file = %file_name, line = line_no + 1, action = %other, "Unknown action, skipping");
                    continue;
                }
            };

            // The second token is a comma-separated list of programs.
            let programs: Vec<&str> = parts[1].split(',').map(|s| s.trim()).collect();

            // The optional third token is a comma-separated list of prefixes
            // that apply to *each* program.
            let prefixes: Vec<String> = parts
                .get(2)
                .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
                .unwrap_or_default();

            for prog in programs {
                if prog.is_empty() {
                    continue;
                }
                info!(
                    file = %file_name,
                    program = %prog,
                    prefixes = ?prefixes,
                    action = ?action,
                    "Loaded rule"
                );
                self.rules.push(PolicyRule {
                    program: prog.to_string(),
                    prefixes: prefixes.clone(),
                    action: action.clone(),
                });
            }
        }
        Ok(())
    }

    /// Evaluate a bash command against all rules.
    ///
    /// Evaluation order:
    /// 1. Parse command to extract program name (first token).
    /// 2. Check deny rules first (safety wins).
    /// 3. Check allow prefixes (user-approved or default-safe).
    /// 4. Check ask rules.
    /// 5. Default: Ask (unknown commands require approval).
    pub fn evaluate(&self, command: &str) -> PolicyDecision {
        let cmd = command.trim();
        if cmd.is_empty() {
            return PolicyDecision::default_ask("Empty command");
        }

        let tokens: Vec<&str> = cmd.split_whitespace().collect();
        let program = tokens[0];

        // --- 1. Deny rules (highest priority) ---
        for rule in &self.rules {
            if rule.action != PolicyAction::Deny {
                continue;
            }
            if !program_matches(&rule.program, program) {
                continue;
            }
            if rule.prefixes.is_empty() {
                return PolicyDecision::deny(
                    format!("Program '{}' is denied by rule", program),
                    &rule.program,
                );
            }
            for prefix in &rule.prefixes {
                let full_prefix = format!("{} {}", rule.program, prefix);
                if cmd.starts_with(&full_prefix) {
                    return PolicyDecision::deny(
                        format!("Command '{}' matches deny rule", cmd),
                        &full_prefix,
                    );
                }
            }
        }

        // --- 2. Allow prefixes (including runtime amendments) ---
        for ap in &self.allow_prefixes {
            if cmd.starts_with(&ap.prefix) {
                return PolicyDecision::allow(
                    format!("Matches approved prefix '{}'", ap.prefix),
                    &ap.prefix,
                );
            }
        }

        // --- 3. Allow rules from files ---
        for rule in &self.rules {
            if rule.action != PolicyAction::Allow {
                continue;
            }
            if !program_matches(&rule.program, program) {
                continue;
            }
            if rule.prefixes.is_empty() {
                // Bare program is allowed (e.g. "ls" matches any ls invocation)
                return PolicyDecision::allow(
                    format!("Program '{}' is allowed by rule", program),
                    &rule.program,
                );
            }
            for prefix in &rule.prefixes {
                let full_prefix = format!("{} {}", rule.program, prefix);
                if cmd.starts_with(&full_prefix) {
                    return PolicyDecision::allow(
                        format!("Command '{}' matches allow rule", cmd),
                        &full_prefix,
                    );
                }
            }
        }

        // --- 4. Ask rules ---
        for rule in &self.rules {
            if rule.action != PolicyAction::Ask {
                continue;
            }
            if !program_matches(&rule.program, program) {
                continue;
            }
            if rule.prefixes.is_empty() {
                return PolicyDecision::ask(
                    format!("Program '{}' requires approval", program),
                    &rule.program,
                );
            }
            for prefix in &rule.prefixes {
                let full_prefix = format!("{} {}", rule.program, prefix);
                if cmd.starts_with(&full_prefix) {
                    return PolicyDecision::ask(
                        format!("Command '{}' requires approval", cmd),
                        &full_prefix,
                    );
                }
            }
        }

        // --- 5. Default: unknown commands ask for approval ---
        PolicyDecision::default_ask(format!("Unknown command '{}'; approval required", program))
    }

    /// Add a user-approved allow prefix at runtime ("learning").
    pub fn amend(&mut self, prefix: &str, source: &str) {
        // Avoid duplicates
        if self
            .allow_prefixes
            .iter()
            .any(|ap| ap.prefix == prefix && ap.source == source)
        {
            debug!(prefix = %prefix, source = %source, "Allow prefix already exists, skipping");
            return;
        }
        info!(prefix = %prefix, source = %source, "Adding runtime allow prefix");
        self.allow_prefixes.push(AllowPrefix {
            prefix: prefix.to_string(),
            source: source.to_string(),
        });
    }

    /// Evaluate a network request against network rules.
    pub fn evaluate_network(&self, host: &str, protocol: &str) -> PolicyDecision {
        for rule in &self.network_rules {
            if !protocol_matches(&rule.protocol, protocol) {
                continue;
            }
            if host_matches(&rule.host_pattern, host) {
                return match rule.action {
                    PolicyAction::Allow => PolicyDecision::allow(
                        format!("Host '{}' matches network allow rule", host),
                        &rule.host_pattern,
                    ),
                    PolicyAction::Deny => PolicyDecision::deny(
                        format!("Host '{}' matches network deny rule", host),
                        &rule.host_pattern,
                    ),
                    PolicyAction::Ask => PolicyDecision::ask(
                        format!("Host '{}' matches network ask rule", host),
                        &rule.host_pattern,
                    ),
                };
            }
        }
        // Default: ask for unknown network destinations
        PolicyDecision::default_ask(format!(
            "No network rule for host '{}', approval required",
            host
        ))
    }

    // --- Accessors for testing / introspection ---

    /// Number of loaded rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Number of runtime-approved prefixes.
    pub fn prefix_count(&self) -> usize {
        self.allow_prefixes.len()
    }

    /// Add a network rule (useful for testing or programmatic setup).
    pub fn add_network_rule(&mut self, rule: NetworkRule) {
        self.network_rules.push(rule);
    }
}

impl Default for ExecPolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Match a rule program name against the actual program.
/// Handles variants like `mkfs` matching `mkfs.ext4`, `mkfs.xfs`, etc.
fn program_matches(rule_program: &str, actual_program: &str) -> bool {
    rule_program == actual_program
        || actual_program.starts_with(&format!("{}.", rule_program))
        || actual_program.starts_with(&format!(".{}", rule_program))
}

/// Match a protocol pattern against a protocol string.
/// `"any"` matches everything; otherwise case-insensitive exact match.
fn protocol_matches(pattern: &str, protocol: &str) -> bool {
    pattern.eq_ignore_ascii_case("any") || pattern.eq_ignore_ascii_case(protocol)
}

/// Match a host pattern against a hostname.
/// Supports `*.example.com` glob-style matching (prefix wildcard).
fn host_matches(pattern: &str, host: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        // "*.github.com" matches "api.github.com" and "github.com"
        host.ends_with(suffix) || host == suffix.trim_end_matches('.')
    } else {
        host == pattern
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    // -- Default rules --

    #[test]
    fn test_default_deny_mkfs() {
        let engine = ExecPolicyEngine::new();
        let decision = engine.evaluate("mkfs.ext4 /dev/sda1");
        assert_eq!(decision.action, PolicyAction::Deny);
        assert!(decision.reason.contains("mkfs"));
    }

    #[test]
    fn test_default_ask_rm_rf() {
        let engine = ExecPolicyEngine::new();
        let decision = engine.evaluate("rm -rf /tmp/junk");
        assert_eq!(decision.action, PolicyAction::Ask);
        assert!(decision.reason.contains("rm"));
    }

    #[test]
    fn test_default_ask_dd() {
        let engine = ExecPolicyEngine::new();
        let decision = engine.evaluate("dd if=/dev/zero of=/dev/sda");
        assert_eq!(decision.action, PolicyAction::Ask);
        assert!(decision.reason.contains("dd"));
    }

    #[test]
    fn test_default_ask_systemctl_stop() {
        let engine = ExecPolicyEngine::new();
        let decision = engine.evaluate("systemctl stop nginx");
        assert_eq!(decision.action, PolicyAction::Ask);
    }

    #[test]
    fn test_default_ask_iptables() {
        let engine = ExecPolicyEngine::new();
        let decision = engine.evaluate("iptables -L");
        assert_eq!(decision.action, PolicyAction::Ask);
    }

    #[test]
    fn test_unknown_command_defaults_to_ask() {
        let engine = ExecPolicyEngine::new();
        let decision = engine.evaluate("some_unknown_tool --flag");
        assert_eq!(decision.action, PolicyAction::Ask);
        assert!(decision.matched_rule.is_none());
    }

    // -- Edge cases --

    #[test]
    fn test_empty_command() {
        let engine = ExecPolicyEngine::new();
        let decision = engine.evaluate("");
        assert_eq!(decision.action, PolicyAction::Ask);
        assert!(decision.reason.contains("Empty"));
    }

    #[test]
    fn test_whitespace_only_command() {
        let engine = ExecPolicyEngine::new();
        let decision = engine.evaluate("   ");
        assert_eq!(decision.action, PolicyAction::Ask);
    }

    // -- .rules file loading --

    #[test]
    fn test_load_rules_file() {
        let dir = tempdir().unwrap();
        let rules_path = dir.path().join("test.rules");
        let mut f = fs::File::create(&rules_path).unwrap();
        writeln!(
            f,
            "# comment\nallow git status,log\ndeny mkfs\nask rm -rf\nallow cargo check,cargo test\n"
        )
        .unwrap();

        let mut engine = ExecPolicyEngine::new();
        engine.load_from_dir(dir.path()).unwrap();

        // git status should be allowed
        let d = engine.evaluate("git status");
        assert_eq!(d.action, PolicyAction::Allow);

        // git log should be allowed
        let d = engine.evaluate("git log --oneline");
        assert_eq!(d.action, PolicyAction::Allow);

        // cargo check should be allowed
        let d = engine.evaluate("cargo check");
        assert_eq!(d.action, PolicyAction::Allow);

        // mkfs should be denied
        let d = engine.evaluate("mkfs.ext4 /dev/sda");
        assert_eq!(d.action, PolicyAction::Deny);
    }

    #[test]
    fn test_load_rules_file_bare_program_allow() {
        let dir = tempdir().unwrap();
        let rules_path = dir.path().join("tools.rules");
        let mut f = fs::File::create(&rules_path).unwrap();
        writeln!(f, "allow ls,cat,grep").unwrap();

        let mut engine = ExecPolicyEngine::new();
        engine.load_from_dir(dir.path()).unwrap();

        let d = engine.evaluate("ls -la /tmp");
        assert_eq!(d.action, PolicyAction::Allow);

        let d = engine.evaluate("cat /etc/hosts");
        assert_eq!(d.action, PolicyAction::Allow);
    }

    #[test]
    fn test_load_nonexistent_dir() {
        let mut engine = ExecPolicyEngine::new();
        let result = engine.load_from_dir(Path::new("/nonexistent/path/xyz"));
        assert!(result.is_ok()); // Should succeed silently
    }

    #[test]
    fn test_load_dir_ignores_non_rules_files() {
        let dir = tempdir().unwrap();
        fs::File::create(dir.path().join("notes.txt")).unwrap();
        let rules_path = dir.path().join("valid.rules");
        let mut f = fs::File::create(&rules_path).unwrap();
        writeln!(f, "allow ls").unwrap();

        let mut engine = ExecPolicyEngine::new();
        let count_before = engine.rule_count();
        engine.load_from_dir(dir.path()).unwrap();
        // Only the .rules file should be loaded (+1 rule for "ls")
        assert_eq!(engine.rule_count(), count_before + 1);
    }

    // -- Amend (runtime learning) --

    #[test]
    fn test_amend_adds_allow_prefix() {
        let mut engine = ExecPolicyEngine::new();
        assert_eq!(engine.prefix_count(), 0);

        engine.amend("git status", "user-approved");
        assert_eq!(engine.prefix_count(), 1);

        let d = engine.evaluate("git status");
        assert_eq!(d.action, PolicyAction::Allow);
        assert!(d.reason.contains("approved prefix"));
    }

    #[test]
    fn test_amend_dedup() {
        let mut engine = ExecPolicyEngine::new();
        engine.amend("git push origin main", "user-approved");
        engine.amend("git push origin main", "user-approved");
        assert_eq!(engine.prefix_count(), 1);
    }

    #[test]
    fn test_amend_same_prefix_different_source() {
        let mut engine = ExecPolicyEngine::new();
        engine.amend("git status", "user-approved");
        engine.amend("git status", "rules-file");
        assert_eq!(engine.prefix_count(), 2);
    }

    // -- Network rules --

    #[test]
    fn test_network_rule_allow() {
        let mut engine = ExecPolicyEngine::new();
        engine.add_network_rule(NetworkRule {
            host_pattern: "*.github.com".into(),
            protocol: "tcp".into(),
            action: PolicyAction::Allow,
        });

        let d = engine.evaluate_network("api.github.com", "tcp");
        assert_eq!(d.action, PolicyAction::Allow);
    }

    #[test]
    fn test_network_rule_deny() {
        let mut engine = ExecPolicyEngine::new();
        engine.add_network_rule(NetworkRule {
            host_pattern: "*.malware.com".into(),
            protocol: "any".into(),
            action: PolicyAction::Deny,
        });

        let d = engine.evaluate_network("evil.malware.com", "tcp");
        assert_eq!(d.action, PolicyAction::Deny);
    }

    #[test]
    fn test_network_rule_protocol_mismatch() {
        let mut engine = ExecPolicyEngine::new();
        engine.add_network_rule(NetworkRule {
            host_pattern: "*.example.com".into(),
            protocol: "tcp".into(),
            action: PolicyAction::Allow,
        });

        // UDP should not match
        let d = engine.evaluate_network("api.example.com", "udp");
        assert_eq!(d.action, PolicyAction::Ask);
    }

    #[test]
    fn test_network_rule_any_protocol() {
        let mut engine = ExecPolicyEngine::new();
        engine.add_network_rule(NetworkRule {
            host_pattern: "*.example.com".into(),
            protocol: "any".into(),
            action: PolicyAction::Allow,
        });

        let d = engine.evaluate_network("api.example.com", "udp");
        assert_eq!(d.action, PolicyAction::Allow);
    }

    #[test]
    fn test_network_wildcard_match_all() {
        let mut engine = ExecPolicyEngine::new();
        engine.add_network_rule(NetworkRule {
            host_pattern: "*".into(),
            protocol: "any".into(),
            action: PolicyAction::Ask,
        });

        let d = engine.evaluate_network("anything.example.org", "tcp");
        assert_eq!(d.action, PolicyAction::Ask);
    }

    #[test]
    fn test_network_default_ask() {
        let engine = ExecPolicyEngine::new();
        let d = engine.evaluate_network("unknown.host.com", "tcp");
        assert_eq!(d.action, PolicyAction::Ask);
        assert!(d.matched_rule.is_none());
    }

    // -- Multi-word prefix matching --

    #[test]
    fn test_deny_rule_with_prefix() {
        let engine = ExecPolicyEngine::new();
        // "rm -rf" should match "rm -rf /tmp"
        let d = engine.evaluate("rm -rf /tmp/junk");
        assert_eq!(d.action, PolicyAction::Ask);

        // "rm" without "-rf" is not explicitly denied/asked → defaults to Ask
        let d = engine.evaluate("rm file.txt");
        assert_eq!(d.action, PolicyAction::Ask); // default unknown
    }

    // -- Priority ordering --

    #[test]
    fn test_deny_takes_priority_over_allow() {
        let dir = tempdir().unwrap();
        let rules_path = dir.path().join("conflict.rules");
        let mut f = fs::File::create(&rules_path).unwrap();
        writeln!(f, "deny rm\ndeny rm -rf\nallow rm -rf /safe/path").unwrap();

        let mut engine = ExecPolicyEngine::new();
        engine.load_from_dir(dir.path()).unwrap();

        // deny rule should win over allow
        let d = engine.evaluate("rm -rf /safe/path");
        assert_eq!(d.action, PolicyAction::Deny);
    }

    #[test]
    fn test_allow_prefix_takes_priority_over_allow_rule() {
        let mut engine = ExecPolicyEngine::new();
        // Runtime amend "git status" as allowed
        engine.amend("git status", "user-approved");

        // Should match the allow_prefix (step 2) before allow rules (step 3)
        let d = engine.evaluate("git status");
        assert_eq!(d.action, PolicyAction::Allow);
    }
}
