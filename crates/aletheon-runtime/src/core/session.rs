//! Session management for TUI ↔ Runtime communication.
//!
//! Each TUI connection creates a Session that tracks mode, context state,
//! and sub-agents. Sessions persist to JSONL for resume.

use std::collections::HashMap;
use std::time::Instant;
use aletheon_abi::ui_event::CollaborationMode;
use aletheon_abi::permission::PermissionMode;

/// Context window usage tracking.
#[derive(Debug, Clone)]
pub struct ContextState {
    pub used_tokens: usize,
    pub max_tokens: usize,
    pub compaction_count: usize,
    pub last_compaction: Option<Instant>,
}

impl ContextState {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            used_tokens: 0,
            max_tokens,
            compaction_count: 0,
            last_compaction: None,
        }
    }

    pub fn usage_percent(&self) -> f64 {
        if self.max_tokens == 0 {
            return 0.0;
        }
        (self.used_tokens as f64 / self.max_tokens as f64) * 100.0
    }

    pub fn is_near_limit(&self) -> bool {
        self.usage_percent() > 80.0
    }
}

/// A single session's state.
#[derive(Debug)]
pub struct Session {
    pub id: String,
    pub mode: CollaborationMode,
    pub context_state: ContextState,
    pub model_override: Option<String>,
    pub created_at: Instant,
    pub turn_count: usize,
}

impl Session {
    pub fn new(id: String, max_context_tokens: usize) -> Self {
        Self {
            id,
            mode: CollaborationMode::Default,
            context_state: ContextState::new(max_context_tokens),
            model_override: None,
            created_at: Instant::now(),
            turn_count: 0,
        }
    }

    /// Get the effective permission mode for the current collaboration mode.
    pub fn effective_permission_mode(&self) -> PermissionMode {
        match self.mode {
            CollaborationMode::Default => PermissionMode::Default,
            CollaborationMode::Plan => PermissionMode::Plan,
            CollaborationMode::Auto => PermissionMode::BypassAll,
            CollaborationMode::Sandbox => PermissionMode::Default,
        }
    }

    /// Check if a tool is allowed in the current mode.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        match self.mode {
            CollaborationMode::Plan => {
                // Only allow read-only tools in plan mode
                matches!(tool_name, "glob" | "grep" | "read" | "web_fetch" | "web_search" | "status")
            }
            _ => true,
        }
    }
}

/// Manages multiple sessions.
#[derive(Debug)]
pub struct TuiSessionManager {
    sessions: HashMap<String, Session>,
    active_session: Option<String>,
    max_context_tokens: usize,
}

impl TuiSessionManager {
    pub fn new(max_context_tokens: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            active_session: None,
            max_context_tokens,
        }
    }

    /// Create a new session and set it as active.
    pub fn create_session(&mut self, id: String) -> String {
        let session = Session::new(id.clone(), self.max_context_tokens);
        self.sessions.insert(id.clone(), session);
        self.active_session = Some(id.clone());
        id
    }

    /// Get the active session.
    pub fn active(&self) -> Option<&Session> {
        self.active_session.as_ref().and_then(|id| self.sessions.get(id))
    }

    /// Get the active session (mutable).
    pub fn active_mut(&mut self) -> Option<&mut Session> {
        self.active_session.as_ref().and_then(|id| {
            // Workaround for borrow checker
            let id = id.clone();
            self.sessions.get_mut(&id)
        })
    }

    /// Switch the active session.
    pub fn switch_to(&mut self, id: &str) -> bool {
        if self.sessions.contains_key(id) {
            self.active_session = Some(id.to_string());
            true
        } else {
            false
        }
    }

    /// List all session IDs.
    pub fn list_sessions(&self) -> Vec<&str> {
        self.sessions.keys().map(|s| s.as_str()).collect()
    }

    /// Remove a session.
    pub fn remove(&mut self, id: &str) -> bool {
        let removed = self.sessions.remove(id).is_some();
        if self.active_session.as_deref() == Some(id) {
            self.active_session = self.sessions.keys().next().cloned();
        }
        removed
    }
}
