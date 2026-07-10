//! Attention — the workspace's current focus and priority ordering (RFC-014).

use serde::{Deserialize, Serialize};

/// Attention state: the current focus and a ranked list of foci.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Attention {
    /// The single current focus, if any.
    pub focus: Option<String>,
    /// Ranked foci, highest priority first.
    pub priorities: Vec<String>,
}

impl Attention {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the current focus and push it to the front of the priority list.
    pub fn set_focus(&mut self, focus: impl Into<String>) {
        let f = focus.into();
        self.priorities.retain(|p| p != &f);
        self.priorities.insert(0, f.clone());
        self.focus = Some(f);
    }

    /// Clear the current focus (priorities are retained).
    pub fn clear_focus(&mut self) {
        self.focus = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_focus_updates_priorities() {
        let mut a = Attention::new();
        a.set_focus("task-a");
        a.set_focus("task-b");
        assert_eq!(a.focus.as_deref(), Some("task-b"));
        assert_eq!(
            a.priorities,
            vec!["task-b".to_string(), "task-a".to_string()]
        );
    }

    #[test]
    fn refocus_dedups() {
        let mut a = Attention::new();
        a.set_focus("x");
        a.set_focus("y");
        a.set_focus("x");
        assert_eq!(a.priorities, vec!["x".to_string(), "y".to_string()]);
    }
}
