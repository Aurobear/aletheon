//! Turn-scoped repository-overview discovery guard.

use std::collections::{HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

use super::ToolContext;

const MAX_GUARDED_TURNS: usize = 1024;

#[derive(Default)]
struct GuardedTurns {
    order: VecDeque<String>,
    active: HashSet<String>,
}

fn turns() -> &'static Mutex<GuardedTurns> {
    static TURNS: OnceLock<Mutex<GuardedTurns>> = OnceLock::new();
    TURNS.get_or_init(|| Mutex::new(GuardedTurns::default()))
}

fn key(ctx: &ToolContext) -> Option<String> {
    ctx.approval_authority
        .as_ref()
        .map(|authority| format!("{}:{}", ctx.session_id, authority.turn_id.0))
}

pub fn mark(ctx: &ToolContext) {
    let Some(key) = key(ctx) else { return };
    let mut guarded = turns()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guarded.active.insert(key.clone()) {
        guarded.order.push_back(key);
    }
    while guarded.order.len() > MAX_GUARDED_TURNS {
        if let Some(expired) = guarded.order.pop_front() {
            guarded.active.remove(&expired);
        }
    }
}

pub fn active(ctx: &ToolContext) -> bool {
    let Some(key) = key(ctx) else { return false };
    turns()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .active
        .contains(&key)
}

pub fn broad_pattern(pattern: &str) -> bool {
    let normalized = pattern.replace('\\', "/");
    normalized.contains("**")
        || normalized
            .split('/')
            .rev()
            .skip(1)
            .any(|segment| segment.contains(['*', '?', '[']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_recursive_and_wildcard_directory_scopes() {
        assert!(broad_pattern("crates/executive/tests/**/*.rs"));
        assert!(broad_pattern("crates/*/Cargo.toml"));
        assert!(!broad_pattern("docs/design/architecture-overview.md"));
        assert!(!broad_pattern("scripts/*.sh"));
    }
}
