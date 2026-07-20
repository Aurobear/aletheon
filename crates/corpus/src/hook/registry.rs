// crates/aletheon-runtime/src/impl/hooks/registry.rs

//! Hook registry — registers and executes lifecycle hooks.
//!
//! Hooks are registered for specific HookPoints and executed in
//! priority order (lower number = earlier execution).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use std::time::Instant;

use fabric::Clock;
use fabric::Timer;
use tracing::warn;

use fabric::hook::{HookContext, HookPoint, HookResult};

const MAX_HOOK_ENVELOPE_BYTES: usize = 128 * 1024;
const MAX_HOOK_METRIC_SERIES: usize = 256;
const MAX_HOOK_METRIC_NAME_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HookMetricSnapshot {
    pub executions_total: u64,
    pub failed_total: u64,
    pub restricted_total: u64,
    pub latency_micros_total: u64,
}

static HOOK_METRICS: LazyLock<Mutex<HashMap<String, HookMetricSnapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Process-local, bounded metrics keyed by the host-registered hook name.
pub fn hook_metrics(name: &str) -> HookMetricSnapshot {
    HOOK_METRICS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&bounded_metric_name(name))
        .copied()
        .unwrap_or_default()
}

fn bounded_metric_name(name: &str) -> String {
    let mut end = name.len().min(MAX_HOOK_METRIC_NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

fn record_hook_metric(name: &str, elapsed: Duration, failed: bool, restricted: bool) {
    let name = bounded_metric_name(name);
    let mut metrics = HOOK_METRICS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !metrics.contains_key(&name) && metrics.len() == MAX_HOOK_METRIC_SERIES {
        return;
    }
    let metric = metrics.entry(name).or_default();
    metric.executions_total = metric.executions_total.saturating_add(1);
    metric.failed_total = metric.failed_total.saturating_add(u64::from(failed));
    metric.restricted_total = metric
        .restricted_total
        .saturating_add(u64::from(restricted));
    metric.latency_micros_total = metric
        .latency_micros_total
        .saturating_add(elapsed.as_micros().try_into().unwrap_or(u64::MAX));
}

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
    clock: Arc<dyn Clock>,
    event_bus: Option<Arc<fabric::CanonicalEventBus>>,
}

impl HookRegistry {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            hooks: HashMap::new(),
            clock,
            event_bus: None,
        }
    }

    pub fn with_event_bus(mut self, event_bus: Option<Arc<fabric::CanonicalEventBus>>) -> Self {
        self.event_bus = event_bus;
        self
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
        if let Some(bus) = &self.event_bus {
            let _ = bus
                .publish_event(
                    fabric::SchemaId::from("aletheon.event.hook_triggered/v1"),
                    format!("session:{}", ctx.session_id),
                    serde_json::json!({
                        "hook_event_name": ctx.point.event_name(),
                        "turn_count": ctx.turn_count,
                    }),
                )
                .await;
        }
        let hooks = match self.hooks.get(&ctx.point) {
            Some(h) => h,
            None => return HookResult::Continue,
        };

        let mut injections = Vec::new();

        for hook in hooks {
            let result = self.execute_single(hook, ctx).await;
            match result {
                HookResult::Continue => {}
                HookResult::ModifyInput(v) if ctx.point.is_blocking() => {
                    return HookResult::ModifyInput(v)
                }
                HookResult::Block { reason } if ctx.point.is_blocking() => {
                    return HookResult::Block { reason }
                }
                HookResult::ModifyInput(_) | HookResult::Block { .. } => {
                    warn!(
                        hook = %hook.name,
                        point = ctx.point.event_name(),
                        "Ignoring blocking hook result at non-blocking lifecycle point"
                    );
                }
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
        let started = Instant::now();
        let script = match hook.script_path {
            Some(ref s) => s,
            None => {
                record_hook_metric(&hook.name, started.elapsed(), false, false);
                return HookResult::Continue;
            }
        };

        if is_restricted_repo_hook(hook, script, ctx) {
            warn!(
                hook = %hook.name,
                point = ctx.point.event_name(),
                "Skipping untrusted repository hook"
            );
            if let Some(bus) = &self.event_bus {
                let _ = bus
                    .publish_event(
                        fabric::SchemaId::from("aletheon.event.hook_restricted/v1"),
                        format!("session:{}", ctx.session_id),
                        serde_json::json!({
                            "hook": hook.name,
                            "hook_event_name": ctx.point.event_name(),
                            "reason": "untrusted_repository_hook",
                        }),
                    )
                    .await;
            }
            record_hook_metric(&hook.name, started.elapsed(), false, true);
            return HookResult::Continue;
        }

        let ctx_json = hook_envelope_json(ctx, self.clock.wall_now().0);

        let child = tokio::process::Command::new(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                warn!(hook = %hook.name, error = %e, "Hook spawn failed");
                record_hook_metric(&hook.name, started.elapsed(), true, false);
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
        let deadline = kernel::chronos::SystemTimer.timeout(Duration::from_secs(30), async {
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
            Ok(Ok((status, stdout))) => {
                let failed = !status.success();
                if failed {
                    warn!(hook = %hook.name, ?status, "Hook exited unsuccessfully");
                }
                record_hook_metric(&hook.name, started.elapsed(), failed, false);
                parse_hook_output(&stdout)
            }
            Ok(Err(e)) => {
                warn!(hook = %hook.name, error = %e, "Hook execution failed");
                record_hook_metric(&hook.name, started.elapsed(), true, false);
                HookResult::Continue
            }
            Err(_) => {
                warn!(hook = %hook.name, "Hook execution timed out after 30s");
                child.kill().await.ok();
                record_hook_metric(&hook.name, started.elapsed(), true, false);
                HookResult::Continue
            }
        }
    }
}

fn is_restricted_repo_hook(
    hook: &RegisteredHook,
    script: &std::path::Path,
    ctx: &HookContext,
) -> bool {
    if hook.source == "config" {
        return false;
    }
    let Some(workspace_root) = ctx.metadata.get("workspace_root") else {
        return false;
    };
    let workspace_root = std::fs::canonicalize(workspace_root)
        .unwrap_or_else(|_| std::path::PathBuf::from(workspace_root));
    let script = std::fs::canonicalize(script).unwrap_or_else(|_| script.to_path_buf());
    script.starts_with(workspace_root)
        && ctx.metadata.get("repo_hooks_trusted").map(String::as_str) != Some("true")
}

fn hook_envelope_json(ctx: &HookContext, timestamp_ms: i64) -> String {
    let workspace_root = ctx.metadata.get("workspace_root").cloned();
    let full = serde_json::json!({
        "hook_event_name": ctx.point.event_name(),
        "timestamp_ms": timestamp_ms,
        "session_id": ctx.session_id,
        "turn_count": ctx.turn_count,
        "workspace_root": workspace_root,
        "tool_name": ctx.tool_name,
        "tool_input": ctx.tool_input,
        "tool_result": ctx.tool_result,
        "message": ctx.message,
        "metadata": ctx.metadata,
        "payload_truncated": false,
    });
    let encoded = serde_json::to_string(&full).unwrap_or_default();
    if encoded.len() <= MAX_HOOK_ENVELOPE_BYTES {
        return encoded;
    }

    // Keep the authority/scope fields intact and carry a UTF-8-safe bounded
    // rendering of the original detail. Re-serialize while shrinking because
    // JSON escaping can make the encoded representation larger than the text.
    let mut keep = MAX_HOOK_ENVELOPE_BYTES / 2;
    loop {
        let detail = truncate_utf8(&encoded, keep);
        let bounded = serde_json::json!({
            "hook_event_name": ctx.point.event_name(),
            "timestamp_ms": timestamp_ms,
            "session_id": ctx.session_id,
            "turn_count": ctx.turn_count,
            "workspace_root": workspace_root,
            "payload_truncated": true,
            "truncated_detail": detail,
        });
        let bounded = serde_json::to_string(&bounded).unwrap_or_default();
        if bounded.len() <= MAX_HOOK_ENVELOPE_BYTES || keep == 0 {
            return bounded;
        }
        keep /= 2;
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new(Arc::new(kernel::chronos::TestClock::default()))
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
        let mut reg = HookRegistry::default();
        reg.register(make_hook("a", HookPoint::PreTool, 10));
        reg.register(make_hook("b", HookPoint::PreTool, 5));
        reg.register(make_hook("c", HookPoint::PostTool, 100));

        assert_eq!(reg.count(&HookPoint::PreTool), 2);
        assert_eq!(reg.count(&HookPoint::PostTool), 1);
        assert_eq!(reg.total_count(), 3);
    }

    #[test]
    fn priority_ordering() {
        let mut reg = HookRegistry::default();
        reg.register(make_hook("low", HookPoint::PreTool, 100));
        reg.register(make_hook("high", HookPoint::PreTool, 1));
        reg.register(make_hook("mid", HookPoint::PreTool, 50));

        let hooks = reg.hooks.get(&HookPoint::PreTool).unwrap();
        assert_eq!(hooks[0].name, "high");
        assert_eq!(hooks[1].name, "mid");
        assert_eq!(hooks[2].name, "low");
    }

    #[test]
    fn command_envelope_is_stable_and_utf8_bounded() {
        let mut metadata = HashMap::new();
        metadata.insert("workspace_root".into(), "/tmp/project".into());
        let context = HookContext {
            point: HookPoint::PostToolFailure,
            session_id: "session-a".into(),
            turn_count: 4,
            tool_name: Some("bash_exec".into()),
            tool_input: Some(serde_json::json!({"command": "x".repeat(200_000)})),
            tool_result: None,
            message: Some("界".repeat(100_000)),
            metadata,
        };

        let encoded = hook_envelope_json(&context, 1234);
        assert!(encoded.len() <= MAX_HOOK_ENVELOPE_BYTES);
        let envelope: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(envelope["hook_event_name"], "post_tool_failure");
        assert_eq!(envelope["timestamp_ms"], 1234);
        assert_eq!(envelope["workspace_root"], "/tmp/project");
        assert_eq!(envelope["payload_truncated"], true);
    }

    #[test]
    fn repository_hook_requires_explicit_host_trust_but_config_is_exempt() {
        let directory = TempDir::new().unwrap();
        let script = directory.path().join("hook.sh");
        std::fs::write(&script, "#!/bin/sh").unwrap();
        let mut metadata = HashMap::from([(
            "workspace_root".to_string(),
            directory.path().display().to_string(),
        )]);
        let context = HookContext {
            point: HookPoint::PreTool,
            session_id: "session-a".into(),
            turn_count: 0,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: metadata.clone(),
        };
        let mut hook = RegisteredHook {
            name: "repo-hook".into(),
            source: "skill:repo".into(),
            script_path: Some(script.clone()),
            point: HookPoint::PreTool,
            priority: 0,
        };
        assert!(is_restricted_repo_hook(&hook, &script, &context));

        metadata.insert("repo_hooks_trusted".into(), "true".into());
        let trusted = HookContext {
            metadata,
            ..context
        };
        assert!(!is_restricted_repo_hook(&hook, &script, &trusted));
        hook.source = "config".into();
        assert!(!is_restricted_repo_hook(&hook, &script, &trusted));
    }

    #[tokio::test]
    async fn restricted_repository_hook_emits_canonical_receipt() {
        let directory = TempDir::new().unwrap();
        let script = directory.path().join("hook.sh");
        std::fs::write(&script, "#!/bin/sh\necho should-not-run").unwrap();
        let bus = Arc::new(fabric::CanonicalEventBus::new(8));
        let mut receipts =
            bus.subscribe_channel(fabric::SchemaId::from("aletheon.event.hook_restricted/v1"));
        let mut registry = HookRegistry::default().with_event_bus(Some(bus));
        registry.register(RegisteredHook {
            name: "repo-hook".into(),
            source: "skill:repo".into(),
            script_path: Some(script),
            point: HookPoint::PreTool,
            priority: 0,
        });
        let context = HookContext {
            point: HookPoint::PreTool,
            session_id: "session-a".into(),
            turn_count: 0,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::from([(
                "workspace_root".into(),
                directory.path().display().to_string(),
            )]),
        };

        assert!(matches!(
            registry.execute(&context).await,
            HookResult::Continue
        ));
        let receipt = receipts.recv().await.unwrap();
        assert_eq!(receipt.source.0, "session:session-a");
        assert_eq!(receipt.payload["reason"], "untrusted_repository_hook");
    }

    #[tokio::test]
    async fn execute_no_hooks_returns_continue() {
        let reg = HookRegistry::default();
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

        let mut reg = HookRegistry::default();
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

        let mut reg = HookRegistry::default();
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

    #[tokio::test]
    async fn block_is_ignored_outside_pre_tool() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("block.sh");
        std::fs::write(
            &script,
            "#!/bin/bash\necho '{\"action\":\"block\",\"reason\":\"late\"}'",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut registry = HookRegistry::default();
        registry.register(RegisteredHook {
            name: "late-block".into(),
            source: "config".into(),
            script_path: Some(script),
            point: HookPoint::PostTurn,
            priority: 0,
        });
        let context = HookContext {
            point: HookPoint::PostTurn,
            session_id: "session".into(),
            turn_count: 1,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        assert!(matches!(
            registry.execute(&context).await,
            HookResult::Continue
        ));
    }

    #[tokio::test]
    async fn named_hook_metrics_record_latency_and_failure() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("fail.sh");
        std::fs::write(&script, "#!/bin/bash\nexit 7").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let name = "metrics:known-failure";
        let before = hook_metrics(name);
        let mut registry = HookRegistry::default();
        registry.register(RegisteredHook {
            name: name.into(),
            source: "config".into(),
            script_path: Some(script),
            point: HookPoint::PostTurn,
            priority: 0,
        });
        registry
            .execute(&HookContext {
                point: HookPoint::PostTurn,
                session_id: "metrics-session".into(),
                turn_count: 1,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: None,
                metadata: HashMap::new(),
            })
            .await;
        let after = hook_metrics(name);
        assert_eq!(after.executions_total, before.executions_total + 1);
        assert_eq!(after.failed_total, before.failed_total + 1);
        assert!(after.latency_micros_total >= before.latency_micros_total);
    }

    #[test]
    fn unregister_hook() {
        let mut reg = HookRegistry::default();
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
        std::fs::write(&script, "#!/bin/bash\nexec sleep 3600").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut reg = HookRegistry::default();
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

        let start = reg.clock.mono_now();
        let result = reg.execute(&ctx).await;
        let elapsed = reg.clock.mono_now().0.saturating_sub(start.0);

        // Should return Continue (not hang for 3600s).
        assert!(matches!(result, HookResult::Continue));
        // Should complete in roughly 30s, with some tolerance.
        assert!(
            elapsed < 60_000,
            "Expected timeout ~30s, but took {} ms",
            elapsed
        );
    }
}
