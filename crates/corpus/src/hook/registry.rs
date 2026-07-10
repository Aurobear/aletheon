// crates/aletheon-runtime/src/impl/hooks/registry.rs

//! Hook registry — registers and executes lifecycle hooks.
//!
//! Hooks are registered for specific HookPoints and executed in
//! priority order (lower number = earlier execution).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tracing::warn;

use base::hook::{HookContext, HookPoint, HookResult};

/// A registered hook.
#[derive(Debug, Clone)]
pub struct RegisteredHook {
    /// Unique name (e.g. "git-workflow:validate").
    pub name: String,
    /// Origin: `"skill:<name>"` | `"builtin"` | `"config"`.
    pub source: String,
    /// Path to executable script (None for builtin hooks).
    pub script_path: Option<PathBuf>,
    /// Which lifecycle point this hook targets.
    pub point: HookPoint,
    /// Execution priority (lower = earlier).
    pub priority: i32,
}

/// Registry of lifecycle hooks.
pub struct HookRegistry {
    hooks: HashMap<HookPoint, Vec<RegisteredHook>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    /// Register a hook. Hooks are kept sorted by priority.
    pub fn register(&mut self, hook: RegisteredHook) {
        let entry = self.hooks.entry(hook.point).or_default();
        entry.push(hook);
        entry.sort_by_key(|h| h.priority);
    }

    /// List all registered hooks.
    pub fn list(&self) -> Vec<&RegisteredHook> {
        self.hooks.values().flat_map(|v| v.iter()).collect()
    }

    /// Unregister all hooks with the given name. Returns `true` if at least one was removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        let mut removed = false;
        for hooks in self.hooks.values_mut() {
            let before = hooks.len();
            hooks.retain(|h| h.name != name);
            if hooks.len() < before {
                removed = true;
            }
        }
        removed
    }

    /// Execute all hooks for a given point.
    ///
    /// Returns the aggregate result:
    /// - First `Block` wins (short-circuits).
    /// - First `ModifyInput` wins (short-circuits).
    /// - All `Inject` results are merged.
    /// - `Continue` is returned if no hooks modify behavior.
    pub async fn execute(&self, ctx: &HookContext) -> HookResult {
        let hooks = match self.hooks.get(&ctx.point) {
            Some(h) => h,
            None => return HookResult::Continue,
        };

        let mut injections = Vec::new();

        for hook in hooks {
            let result = self.execute_single(hook, ctx).await;
            match result {
                HookResult::Continue => {}
                HookResult::ModifyInput(v) => return HookResult::ModifyInput(v),
                HookResult::Block { reason } => return HookResult::Block { reason },
                HookResult::Inject(s) => injections.push(s),
            }
        }

        if injections.is_empty() {
            HookResult::Continue
        } else {
            HookResult::Inject(injections.join("\n"))
        }
    }

    /// Get the number of registered hooks for a point.
    pub fn count(&self, point: &HookPoint) -> usize {
        self.hooks.get(point).map_or(0, |h| h.len())
    }

    /// Get total registered hooks across all points.
    pub fn total_count(&self) -> usize {
        self.hooks.values().map(|h| h.len()).sum()
    }

    /// Execute a single hook.
    async fn execute_single(&self, hook: &RegisteredHook, ctx: &HookContext) -> HookResult {
        let script = match hook.script_path {
            Some(ref s) => s,
            None => return HookResult::Continue,
        };

        let ctx_json = serde_json::to_string(ctx).unwrap_or_default();

        let child = tokio::process::Command::new(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                warn!(hook = %hook.name, error = %e, "Hook spawn failed");
                return HookResult::Continue;
            }
        };

        // Write context to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(ctx_json.as_bytes()).await;
        }

        // Wait for the child with a 30-second timeout.
        // We use `child.wait()` so we retain ownership and can `kill()` on timeout.
        let deadline = tokio::time::timeout(Duration::from_secs(30), async {
            let stdout = child.stdout.take();
            let status = child.wait().await?;
            let mut out = Vec::new();
            if let Some(mut s) = stdout {
                use tokio::io::AsyncReadExt;
                let _ = s.read_to_end(&mut out).await;
            }
            Ok::<_, std::io::Error>((status, out))
        });

        match deadline.await {
            Ok(Ok((_status, stdout))) => parse_hook_output(&stdout),
            Ok(Err(e)) => {
                warn!(hook = %hook.name, error = %e, "Hook execution failed");
                HookResult::Continue
            }
            Err(_) => {
                warn!(hook = %hook.name, "Hook execution timed out after 30s");
                child.kill().await.ok();
                HookResult::Continue
            }
        }
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse hook script stdout into HookResult.
pub fn parse_hook_output(stdout: &[u8]) -> HookResult {
    let text = String::from_utf8_lossy(stdout).trim().to_string();
    if text.is_empty() {
        return HookResult::Continue;
    }

    // Try JSON structured response
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        match value.get("action").and_then(|v| v.as_str()) {
            Some("block") => {
                let reason = value
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Blocked by hook")
                    .to_string();
                return HookResult::Block { reason };
            }
            Some("inject") => {
                let content = value
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return HookResult::Inject(content);
            }
            Some("modify_input") => {
                if let Some(input) = value.get("input") {
                    return HookResult::ModifyInput(input.clone());
                }
            }
            _ => {}
        }
    }

    // Plain text -> inject
    HookResult::Inject(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_hook(name: &str, point: HookPoint, priority: i32) -> RegisteredHook {
        RegisteredHook {
            name: name.into(),
            source: "test".into(),
            script_path: None,
            point,
            priority,
        }
    }

    #[test]
    fn register_and_count() {
        let mut reg = HookRegistry::new();
        reg.register(make_hook("a", HookPoint::PreTool, 10));
        reg.register(make_hook("b", HookPoint::PreTool, 5));
        reg.register(make_hook("c", HookPoint::PostTool, 100));

        assert_eq!(reg.count(&HookPoint::PreTool), 2);
        assert_eq!(reg.count(&HookPoint::PostTool), 1);
        assert_eq!(reg.total_count(), 3);
    }

    #[test]
    fn priority_ordering() {
        let mut reg = HookRegistry::new();
        reg.register(make_hook("low", HookPoint::PreTool, 100));
        reg.register(make_hook("high", HookPoint::PreTool, 1));
        reg.register(make_hook("mid", HookPoint::PreTool, 50));

        let hooks = reg.hooks.get(&HookPoint::PreTool).unwrap();
        assert_eq!(hooks[0].name, "high");
        assert_eq!(hooks[1].name, "mid");
        assert_eq!(hooks[2].name, "low");
    }

    #[tokio::test]
    async fn execute_no_hooks_returns_continue() {
        let reg = HookRegistry::new();
        let ctx = HookContext {
            point: HookPoint::PreTool,
            session_id: "test".into(),
            turn_count: 0,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        assert!(matches!(reg.execute(&ctx).await, HookResult::Continue));
    }

    #[tokio::test]
    async fn execute_script_hook_inject() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("hook.sh");
        std::fs::write(&script, "#!/bin/bash\necho 'injected text'").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut reg = HookRegistry::new();
        reg.register(RegisteredHook {
            name: "test:inject".into(),
            source: "test".into(),
            script_path: Some(script),
            point: HookPoint::PostTurn,
            priority: 10,
        });

        let ctx = HookContext {
            point: HookPoint::PostTurn,
            session_id: "test".into(),
            turn_count: 1,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };

        match reg.execute(&ctx).await {
            HookResult::Inject(text) => assert_eq!(text, "injected text"),
            other => panic!("Expected Inject, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_script_hook_block() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("block.sh");
        std::fs::write(
            &script,
            "#!/bin/bash\necho '{\"action\":\"block\",\"reason\":\"not allowed\"}'",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut reg = HookRegistry::new();
        reg.register(RegisteredHook {
            name: "test:block".into(),
            source: "test".into(),
            script_path: Some(script),
            point: HookPoint::PreTool,
            priority: 10,
        });

        let ctx = HookContext {
            point: HookPoint::PreTool,
            session_id: "test".into(),
            turn_count: 0,
            tool_name: Some("bash_exec".into()),
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };

        match reg.execute(&ctx).await {
            HookResult::Block { reason } => assert!(reason.contains("not allowed")),
            other => panic!("Expected Block, got {:?}", other),
        }
    }

    #[test]
    fn parse_output_continue_on_empty() {
        assert!(matches!(parse_hook_output(b""), HookResult::Continue));
    }

    #[test]
    fn parse_output_inject_on_text() {
        match parse_hook_output(b"some context") {
            HookResult::Inject(s) => assert_eq!(s, "some context"),
            _ => panic!("Expected Inject"),
        }
    }

    #[test]
    fn parse_output_block_on_json() {
        let json = r#"{"action":"block","reason":"denied"}"#;
        match parse_hook_output(json.as_bytes()) {
            HookResult::Block { reason } => assert_eq!(reason, "denied"),
            _ => panic!("Expected Block"),
        }
    }

    #[test]
    fn parse_output_inject_on_json() {
        let json = r#"{"action":"inject","content":"extra info"}"#;
        match parse_hook_output(json.as_bytes()) {
            HookResult::Inject(s) => assert_eq!(s, "extra info"),
            _ => panic!("Expected Inject"),
        }
    }

    #[test]
    fn parse_output_modify_input_on_json() {
        let json = r#"{"action":"modify_input","input":{"key":"value"}}"#;
        match parse_hook_output(json.as_bytes()) {
            HookResult::ModifyInput(v) => assert_eq!(v["key"], "value"),
            _ => panic!("Expected ModifyInput"),
        }
    }

    #[test]
    fn unregister_hook() {
        let mut reg = HookRegistry::new();
        reg.register(make_hook("a", HookPoint::PreTool, 10));
        reg.register(make_hook("b", HookPoint::PreTool, 5));
        reg.register(make_hook("a", HookPoint::PostTool, 100));

        // Should remove both hooks named "a" across different points.
        assert!(reg.unregister("a"));
        assert_eq!(reg.count(&HookPoint::PreTool), 1);
        assert_eq!(reg.count(&HookPoint::PostTool), 0);
        assert_eq!(reg.total_count(), 1);

        // Unregistering a non-existent name returns false.
        assert!(!reg.unregister("nonexistent"));
    }

    #[tokio::test]
    async fn hook_timeout_kills_hanging_script() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("hanging.sh");
        // Script sleeps for 3600s -- far beyond the 30s timeout.
        std::fs::write(&script, "#!/bin/bash\nsleep 3600").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut reg = HookRegistry::new();
        reg.register(RegisteredHook {
            name: "test:timeout".into(),
            source: "test".into(),
            script_path: Some(script),
            point: HookPoint::PostTurn,
            priority: 10,
        });

        let ctx = HookContext {
            point: HookPoint::PostTurn,
            session_id: "test".into(),
            turn_count: 1,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };

        let start = std::time::Instant::now();
        let result = reg.execute(&ctx).await;
        let elapsed = start.elapsed();

        // Should return Continue (not hang for 3600s).
        assert!(matches!(result, HookResult::Continue));
        // Should complete in roughly 30s, with some tolerance.
        assert!(
            elapsed < std::time::Duration::from_secs(60),
            "Expected timeout ~30s, but took {:?}",
            elapsed
        );
    }
}
