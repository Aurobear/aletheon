use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use base::tool::PermissionLevel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub tool_pattern: String,
    pub level: PermissionLevel,
    pub action: PolicyAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyAction {
    Allow,
    Deny,
    RequireApproval,
}

#[derive(Debug)]
pub enum PolicyVerdict {
    Allow,
    Deny { reason: String },
    RequireApproval { reason: String },
}

pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
    #[allow(dead_code)]
    default_level: PermissionLevel,
}

impl PolicyEngine {
    pub fn with_defaults() -> Self {
        Self {
            rules: vec![
                // Dangerous commands require approval
                PolicyRule {
                    tool_pattern: "rm -rf *".into(),
                    level: PermissionLevel::L2,
                    action: PolicyAction::RequireApproval,
                },
                PolicyRule {
                    tool_pattern: "dd *".into(),
                    level: PermissionLevel::L2,
                    action: PolicyAction::RequireApproval,
                },
                PolicyRule {
                    tool_pattern: "mkfs *".into(),
                    level: PermissionLevel::L3,
                    action: PolicyAction::Deny,
                },
                PolicyRule {
                    tool_pattern: "systemctl stop *".into(),
                    level: PermissionLevel::L2,
                    action: PolicyAction::RequireApproval,
                },
                PolicyRule {
                    tool_pattern: "iptables *".into(),
                    level: PermissionLevel::L2,
                    action: PolicyAction::RequireApproval,
                },
            ],
            default_level: PermissionLevel::L1,
        }
    }

    pub fn check(&self, tool_name: &str, input: &serde_json::Value) -> PolicyVerdict {
        // Get tool's permission level from registry (simplified: check tool name)
        let tool_level = self.infer_tool_level(tool_name);

        // Check against rules
        for rule in &self.rules {
            if self.matches_pattern(&rule.tool_pattern, tool_name, input) {
                match rule.action {
                    PolicyAction::Allow => return PolicyVerdict::Allow,
                    PolicyAction::Deny => {
                        warn!(tool = tool_name, pattern = %rule.tool_pattern, "Tool call denied by policy");
                        return PolicyVerdict::Deny {
                            reason: format!("Blocked by policy rule: {}", rule.tool_pattern),
                        };
                    }
                    PolicyAction::RequireApproval => {
                        info!(tool = tool_name, pattern = %rule.tool_pattern, "Tool call requires approval");
                        return PolicyVerdict::RequireApproval {
                            reason: format!("Requires approval: {}", rule.tool_pattern),
                        };
                    }
                }
            }
        }

        // Default: allow based on permission level
        match tool_level {
            PermissionLevel::L3 => PolicyVerdict::Deny {
                reason: "L3 operations are forbidden by default".into(),
            },
            _ => PolicyVerdict::Allow,
        }
    }

    fn infer_tool_level(&self, tool_name: &str) -> PermissionLevel {
        match tool_name {
            "file_read" | "system_status" | "process_list" | "memory_search" => PermissionLevel::L0,
            "bash_exec" | "file_write" | "core_memory_append" | "core_memory_replace" => {
                PermissionLevel::L1
            }
            _ => PermissionLevel::L1,
        }
    }

    fn matches_pattern(&self, pattern: &str, tool_name: &str, input: &serde_json::Value) -> bool {
        // Simple pattern matching
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            // Check against tool name or command content
            if tool_name.starts_with(prefix) {
                return true;
            }
            // For bash_exec, check the command string
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                if cmd.starts_with(prefix) {
                    return true;
                }
            }
            false
        } else {
            tool_name == pattern
        }
    }
}
