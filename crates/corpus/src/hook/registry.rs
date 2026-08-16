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

use ::contracts::Clock;
use ::contracts::Timer;
use tracing::warn;

use crate::hook::{HookContext, HookPoint, HookResult};

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
    /// Optional package-declared execution timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// Registry of lifecycle hooks.
pub struct HookRegistry {
    hooks: HashMap<HookPoint, Vec<RegisteredHook>>,
    package_hook_owners: HashMap<String, String>,
    clock: Arc<dyn Clock>,
    event_bus: Option<Arc<runtime::event_projection::CanonicalEventBus>>,
    event_spine: Option<Arc<dyn runtime::EventSpine>>,
    execution_timeout: Duration,
}

impl HookRegistry {
    fn execution_failure(point: &HookPoint, reason: impl Into<String>) -> HookResult {
        if point.is_blocking() {
            HookResult::Block {
                reason: reason.into(),
            }
        } else {
            HookResult::Continue
        }
    }

    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            hooks: HashMap::new(),
            package_hook_owners: HashMap::new(),
            clock,
            event_bus: None,
            event_spine: None,
            execution_timeout: Duration::from_secs(30),
        }
    }

    pub fn with_event_bus(
        mut self,
        event_bus: Option<Arc<runtime::event_projection::CanonicalEventBus>>,
    ) -> Self {
        self.event_bus = event_bus;
        self
    }

    /// Attach the durable event spine used for terminal Hook receipts.
    pub fn with_event_spine(mut self, event_spine: Option<Arc<dyn runtime::EventSpine>>) -> Self {
        self.event_spine = event_spine;
        self
    }

    /// Attach or replace the durable event spine after composition completes.
    pub fn set_event_spine(&mut self, event_spine: Option<Arc<dyn runtime::EventSpine>>) {
        self.event_spine = event_spine;
    }

    /// Register a hook. Hooks are kept sorted by priority.
    pub fn register(&mut self, hook: RegisteredHook) {
        let entry = self.hooks.entry(hook.point).or_default();
        entry.push(hook);
        entry.sort_by_key(|h| h.priority);
    }

    /// Replace only hooks owned by the named extension package.
    pub fn replace_package_hooks(&mut self, owner: &str, mut hooks: Vec<RegisteredHook>) {
        self.remove_package_hooks(owner);
        for hook in &mut hooks {
            hook.source = format!("package:{owner}");
            self.package_hook_owners
                .insert(hook.name.clone(), owner.to_owned());
        }
        for hook in hooks {
            self.register(hook);
        }
    }

    /// Remove package-owned hooks without affecting built-in, configured, or
    /// legacy hooks that happen to share the same source conventions.
    pub fn remove_package_hooks(&mut self, owner: &str) {
        let names: std::collections::HashSet<_> = self
            .package_hook_owners
            .iter()
            .filter(|(_, registered_owner)| registered_owner.as_str() == owner)
            .map(|(name, _)| name.clone())
            .collect();
        if names.is_empty() {
            return;
        }
        for hooks in self.hooks.values_mut() {
            hooks.retain(|hook| !names.contains(&hook.name));
        }
        self.package_hook_owners
            .retain(|_, registered_owner| registered_owner != owner);
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
        if removed {
            self.package_hook_owners.remove(name);
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
                    ::contracts::SchemaId::from("aletheon.event.hook_triggered/v1"),
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
                let elapsed = started.elapsed();
                let result = HookResult::Continue;
                record_hook_metric(&hook.name, elapsed, false, false);
                self.publish_terminal_receipt(hook, ctx, "succeeded", &result, elapsed)
                    .await;
                return result;
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
                        ::contracts::SchemaId::from("aletheon.event.hook_restricted/v1"),
                        format!("session:{}", ctx.session_id),
                        serde_json::json!({
                            "hook": hook.name,
                            "hook_event_name": ctx.point.event_name(),
                            "reason": "untrusted_repository_hook",
                        }),
                    )
                    .await;
            }
            let elapsed = started.elapsed();
            let result = HookResult::Continue;
            record_hook_metric(&hook.name, elapsed, false, true);
            self.publish_terminal_receipt(hook, ctx, "restricted", &result, elapsed)
                .await;
            return result;
        }

        let ctx_json = hook_envelope_json(ctx, self.clock.wall_now().0);

        let mut command = tokio::process::Command::new(script);
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let child = crate::process_spawn::spawn_with_transient_retry(&mut command).await;

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                warn!(hook = %hook.name, error = %e, "Hook spawn failed");
                let elapsed = started.elapsed();
                let result = Self::execution_failure(
                    &ctx.point,
                    format!("hook '{}' could not start", hook.name),
                );
                record_hook_metric(&hook.name, elapsed, true, false);
                self.publish_terminal_receipt(hook, ctx, "failed", &result, elapsed)
                    .await;
                return result;
            }
        };

        // Write context to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(ctx_json.as_bytes()).await;
        }

        // Collect both output pipes while the child runs. Waiting before reading
        // can deadlock a hook that fills an OS pipe. `kill_on_drop` guarantees
        // that cancelling the timed future also terminates the child.
        let execution_timeout = hook
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(self.execution_timeout);
        let deadline =
            kernel::chronos::SystemTimer.timeout(execution_timeout, child.wait_with_output());

        match deadline.await {
            Ok(Ok(output)) => {
                let failed = !output.status.success();
                if failed {
                    warn!(hook = %hook.name, status = ?output.status, "Hook exited unsuccessfully");
                }
                let elapsed = started.elapsed();
                let result = parse_hook_output(&output.stdout);
                record_hook_metric(&hook.name, elapsed, failed, false);
                self.publish_terminal_receipt(
                    hook,
                    ctx,
                    if failed { "failed" } else { "succeeded" },
                    &result,
                    elapsed,
                )
                .await;
                result
            }
            Ok(Err(e)) => {
                warn!(hook = %hook.name, error = %e, "Hook execution failed");
                let elapsed = started.elapsed();
                let result = Self::execution_failure(
                    &ctx.point,
                    format!("hook '{}' execution failed", hook.name),
                );
                record_hook_metric(&hook.name, elapsed, true, false);
                self.publish_terminal_receipt(hook, ctx, "failed", &result, elapsed)
                    .await;
                result
            }
            Err(_) => {
                warn!(hook = %hook.name, timeout_ms = execution_timeout.as_millis(), "Hook execution timed out");
                let elapsed = started.elapsed();
                let result = Self::execution_failure(
                    &ctx.point,
                    format!("hook '{}' execution timed out", hook.name),
                );
                record_hook_metric(&hook.name, elapsed, true, false);
                self.publish_terminal_receipt(hook, ctx, "failed", &result, elapsed)
                    .await;
                result
            }
        }
    }

    async fn publish_terminal_receipt(
        &self,
        hook: &RegisteredHook,
        ctx: &HookContext,
        status: &str,
        result: &HookResult,
        elapsed: Duration,
    ) {
        if self.event_bus.is_none() && self.event_spine.is_none() {
            return;
        }
        let result_kind = match result {
            HookResult::Continue => "continue",
            HookResult::Block { .. } => "block",
            HookResult::ModifyInput(_) => "modify_input",
            HookResult::Inject(_) => "inject",
        };
        let payload = serde_json::json!({
            "hook": hook.name,
            "source": hook.source,
            "hook_event_name": ctx.point.event_name(),
            "session_id": ctx.session_id,
            "turn_count": ctx.turn_count,
            "status": status,
            "result_kind": result_kind,
            "elapsed_micros": elapsed.as_micros().try_into().unwrap_or(u64::MAX),
        });
        let event_id = runtime::EventId::new();
        let mut envelope = ::contracts::EnvelopeV2::new(
            ::contracts::SchemaId::from("aletheon.event.hook_completed/v1"),
            ::contracts::EnvelopeV2Target(format!("session:{}", ctx.session_id)),
            ::contracts::EnvelopeV2Target("broadcast".into()),
            ::contracts::EnvelopeV2Delivery::FanOut,
            ::contracts::NamespaceId("default".into()),
            payload.clone(),
        );
        envelope.id = ::contracts::MessageId(event_id.0);
        if let Some(spine) = &self.event_spine {
            let _ = spine.append(runtime::UnsequencedEvent {
                tree_id: runtime::EventTreeId::for_root_session(&ctx.session_id),
                event_id,
                parent: None,
                identity: runtime::EventIdentity {
                    root_session_id: ctx.session_id.clone(),
                    session_id: ctx.session_id.clone(),
                    agent_id: None,
                },
                envelope: envelope.clone(),
                visibility: runtime::EventVisibility::Control,
                payload: runtime::EventPayload::Inline {
                    value: payload.clone(),
                },
            });
        }
        if let Some(bus) = &self.event_bus {
            let _ = bus.publish(envelope).await;
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

    #[derive(Default)]
    struct RecordingSpine(Mutex<Vec<runtime::UnsequencedEvent>>);

    impl runtime::EventSpine for RecordingSpine {
        fn append(&self, event: runtime::UnsequencedEvent) -> anyhow::Result<runtime::SpineEvent> {
            event.validate()?;
            let sequence = {
                let mut events = self
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                events.push(event.clone());
                events.len() as u64
            };
            Ok(runtime::SpineEvent {
                position: runtime::EventPosition {
                    tree_id: event.tree_id,
                    event_id: event.event_id,
                    parent: event.parent,
                    sequence: runtime::TreeSequence(sequence),
                },
                identity: event.identity,
                schema: event.envelope.schema.clone(),
                visibility: event.visibility,
                envelope: event.envelope,
                payload: event.payload,
            })
        }
    }

    fn make_hook(name: &str, point: HookPoint, priority: i32) -> RegisteredHook {
        RegisteredHook {
            name: name.into(),
            source: "test".into(),
            script_path: None,
            point,
            priority,
            timeout_ms: None,
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
            timeout_ms: None,
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
        let bus = Arc::new(runtime::event_projection::CanonicalEventBus::new(8));
        let mut receipts = bus.subscribe_channel(::contracts::SchemaId::from(
            "aletheon.event.hook_restricted/v1",
        ));
        let mut registry = HookRegistry::default().with_event_bus(Some(bus));
        registry.register(RegisteredHook {
            name: "repo-hook".into(),
            source: "skill:repo".into(),
            script_path: Some(script),
            point: HookPoint::PreTool,
            priority: 0,
            timeout_ms: None,
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
    async fn completed_hook_emits_broadcast_and_durable_terminal_receipt() {
        let bus = Arc::new(runtime::event_projection::CanonicalEventBus::new(8));
        let mut receipts = bus.subscribe_channel(::contracts::SchemaId::from(
            "aletheon.event.hook_completed/v1",
        ));
        let spine = Arc::new(RecordingSpine::default());
        let mut registry = HookRegistry::default()
            .with_event_bus(Some(bus))
            .with_event_spine(Some(spine.clone()));
        registry.register(make_hook("package:post-turn", HookPoint::PostTurn, 0));
        let context = HookContext {
            point: HookPoint::PostTurn,
            session_id: "session-a".into(),
            turn_count: 3,
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
        let receipt = receipts.recv().await.unwrap();
        assert_eq!(receipt.payload["hook"], "package:post-turn");
        assert_eq!(receipt.payload["status"], "succeeded");
        assert_eq!(receipt.payload["result_kind"], "continue");
        let events = spine
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].envelope.schema.0,
            "aletheon.event.hook_completed/v1"
        );
        assert_eq!(events[0].identity.session_id, "session-a");
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

        let spine = Arc::new(RecordingSpine::default());
        let mut reg = HookRegistry::default().with_event_spine(Some(spine.clone()));
        reg.register(RegisteredHook {
            name: "test:inject".into(),
            source: "test".into(),
            script_path: Some(script),
            point: HookPoint::PostTurn,
            priority: 10,
            timeout_ms: None,
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
            other => {
                let events = spine
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                panic!(
                    "Expected Inject, got {other:?}; receipt={:?}",
                    events.last().map(|event| &event.payload)
                );
            }
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
            timeout_ms: None,
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
            HookResult::Block { .. } => {}
            other => panic!("Expected Block, got {other:?}"),
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
            timeout_ms: None,
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
            timeout_ms: None,
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

    #[test]
    fn package_hook_replacement_preserves_non_package_entries() {
        let mut reg = HookRegistry::default();
        reg.register(make_hook("builtin:audit", HookPoint::PostTurn, 10));
        reg.replace_package_hooks(
            "pkg.one",
            vec![make_hook("pkg.one:audit", HookPoint::PostTurn, 20)],
        );
        assert_eq!(reg.total_count(), 2);

        reg.replace_package_hooks(
            "pkg.one",
            vec![make_hook("pkg.one:review", HookPoint::PreTool, 5)],
        );
        let names: std::collections::HashSet<_> = reg
            .list()
            .into_iter()
            .map(|hook| hook.name.as_str())
            .collect();
        assert!(names.contains("builtin:audit"));
        assert!(names.contains("pkg.one:review"));
        assert!(!names.contains("pkg.one:audit"));

        reg.remove_package_hooks("pkg.one");
        assert_eq!(reg.total_count(), 1);
        assert_eq!(reg.list()[0].name, "builtin:audit");
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

        let mut reg = HookRegistry {
            execution_timeout: Duration::from_millis(100),
            ..HookRegistry::default()
        };
        reg.register(RegisteredHook {
            name: "test:timeout".into(),
            source: "test".into(),
            script_path: Some(script),
            point: HookPoint::PostTurn,
            priority: 10,
            timeout_ms: None,
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
        // The test uses a short injected timeout rather than holding the whole
        // library suite open for the production 30-second bound.
        assert!(
            elapsed < 5_000,
            "Expected bounded timeout, but took {elapsed} ms"
        );
    }
}
