//! Lifecycle hooks — trait-based hook system with session distillation.

pub mod session_distiller;

use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

// ── Hook Events and Results ──────────────────────────────────────────────────

/// Events that trigger hooks.
#[derive(Debug, Clone)]
pub enum HookEvent {
    UserPromptSubmit {
        prompt: String,
    },
    PreToolUse {
        tool_name: String,
        args: Value,
    },
    PostToolWrite {
        file_path: PathBuf,
    },
    SessionStop {
        transcript_path: PathBuf,
        session_id: String,
    },
}

/// Result of a hook execution.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// Allow the operation to continue.
    Allow,
    /// Block the operation with a reason.
    Block { reason: String },
    /// Inject additional context.
    Inject { context: String },
    /// No-op.
    Noop,
}

// ── Hook Trait ───────────────────────────────────────────────────────────────

pub trait Hook: Send + Sync {
    fn name(&self) -> &str;
    fn handle(&self, event: &HookEvent) -> Result<HookResult>;
}

// ── HookRunner ───────────────────────────────────────────────────────────────

pub struct HookRunner {
    hooks: Vec<Box<dyn Hook>>,
}

impl HookRunner {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, hook: Box<dyn Hook>) {
        self.hooks.push(hook);
    }

    /// Run all hooks for an event, collecting results.
    ///
    /// - Block takes priority: if any hook blocks, return Block.
    /// - Inject contexts are merged (joined by newlines).
    /// - If no hooks produce a meaningful result, return Allow.
    pub fn run(&self, event: &HookEvent) -> Result<HookResult> {
        let mut injections = Vec::new();

        for hook in &self.hooks {
            let result = hook.handle(event)?;
            match result {
                HookResult::Block { reason } => return Ok(HookResult::Block { reason }),
                HookResult::Inject { context } => injections.push(context),
                _ => {}
            }
        }

        if injections.is_empty() {
            Ok(HookResult::Allow)
        } else {
            Ok(HookResult::Inject {
                context: injections.join("\n"),
            })
        }
    }
}

impl Default for HookRunner {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Parked — placeholder Hook impl for future lifecycle tests.
    #[allow(dead_code)]
    struct AllowHook;
    impl Hook for AllowHook {
        fn name(&self) -> &str {
            "allow"
        }
        fn handle(&self, _event: &HookEvent) -> Result<HookResult> {
            Ok(HookResult::Allow)
        }
    }

    struct BlockHook {
        reason: String,
    }
    impl Hook for BlockHook {
        fn name(&self) -> &str {
            "block"
        }
        fn handle(&self, _event: &HookEvent) -> Result<HookResult> {
            Ok(HookResult::Block {
                reason: self.reason.clone(),
            })
        }
    }

    struct InjectHook {
        context: String,
    }
    impl Hook for InjectHook {
        fn name(&self) -> &str {
            "inject"
        }
        fn handle(&self, _event: &HookEvent) -> Result<HookResult> {
            Ok(HookResult::Inject {
                context: self.context.clone(),
            })
        }
    }

    #[test]
    fn test_hook_runner_allow() {
        let runner = HookRunner::new();
        let event = HookEvent::UserPromptSubmit {
            prompt: "hello".into(),
        };
        let result = runner.run(&event).unwrap();
        assert!(matches!(result, HookResult::Allow));
    }

    #[test]
    fn test_hook_runner_block() {
        let mut runner = HookRunner::new();
        runner.register(Box::new(BlockHook {
            reason: "denied".into(),
        }));
        let event = HookEvent::UserPromptSubmit {
            prompt: "hello".into(),
        };
        let result = runner.run(&event).unwrap();
        match result {
            HookResult::Block { reason } => assert_eq!(reason, "denied"),
            _ => panic!("Expected Block"),
        }
    }

    #[test]
    fn test_hook_runner_inject_merge() {
        let mut runner = HookRunner::new();
        runner.register(Box::new(InjectHook {
            context: "context A".into(),
        }));
        runner.register(Box::new(InjectHook {
            context: "context B".into(),
        }));
        let event = HookEvent::UserPromptSubmit {
            prompt: "hello".into(),
        };
        let result = runner.run(&event).unwrap();
        match result {
            HookResult::Inject { context } => {
                assert!(context.contains("context A"));
                assert!(context.contains("context B"));
            }
            _ => panic!("Expected Inject"),
        }
    }
}
