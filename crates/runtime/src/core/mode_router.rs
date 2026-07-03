//! Collaboration mode routing.
//!
//! Maps CollaborationMode to SelfField verdicts and tool filtering.

use base::self_field::{Intent, Verdict};
use base::ui_event::CollaborationMode;
use std::collections::HashSet;

/// Routes intents through different behavior paths based on collaboration mode.
#[derive(Debug)]
pub struct ModeRouter {
    current_mode: CollaborationMode,
    /// Tools that are read-only (allowed in Plan mode).
    read_only_tools: HashSet<String>,
}

impl ModeRouter {
    pub fn new() -> Self {
        let mut read_only_tools = HashSet::new();
        for name in &[
            "glob",
            "grep",
            "read",
            "web_fetch",
            "web_search",
            "status",
            "file_read",
        ] {
            read_only_tools.insert(name.to_string());
        }
        Self {
            current_mode: CollaborationMode::Default,
            read_only_tools,
        }
    }

    pub fn current_mode(&self) -> CollaborationMode {
        self.current_mode
    }

    pub fn set_mode(&mut self, mode: CollaborationMode) {
        self.current_mode = mode;
    }

    /// Check if a tool is allowed in the current mode.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        match self.current_mode {
            CollaborationMode::Plan => self.read_only_tools.contains(tool_name),
            _ => true,
        }
    }

    /// Get the system prompt suffix for the current mode.
    pub fn system_prompt_suffix(&self) -> &'static str {
        self.current_mode.system_prompt_suffix()
    }

    /// Map the current mode to a SelfField verdict override for a given intent.
    /// Returns None if the mode doesn't override verdicts (use normal SelfField flow).
    pub fn verdict_override(&self, intent: &Intent) -> Option<Verdict> {
        match self.current_mode {
            CollaborationMode::Default => None, // Use normal SelfField flow
            CollaborationMode::Plan => {
                // Deny all mutations in plan mode
                if self.read_only_tools.contains(&intent.action) {
                    None // Allow normal flow for read-only tools
                } else {
                    Some(Verdict::Deny {
                        reason: "Plan mode: mutations not allowed".to_string(),
                    })
                }
            }
            CollaborationMode::Auto => Some(Verdict::Allow), // Bypass all
            CollaborationMode::Sandbox => {
                // Force sandbox for side-effect tools
                if self.read_only_tools.contains(&intent.action) {
                    None
                } else {
                    Some(Verdict::SandboxFirst {
                        reason: "Sandbox mode: side-effects sandboxed".to_string(),
                    })
                }
            }
        }
    }
}

impl Default for ModeRouter {
    fn default() -> Self {
        Self::new()
    }
}
