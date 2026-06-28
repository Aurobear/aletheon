# Phase 1: Blocking Fixes Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the 4 blocking issues that prevent the basic agent loop from functioning correctly.

**Architecture:** Four independent fixes that can be parallelized. Each task modifies 1-3 files and includes its own tests. No cross-dependencies between tasks.

**Tech Stack:** Rust, tokio, serde_json, anyhow, tracing

---

## Task 1: Rewrite L1+ tool execution in security runner

**Problem:** `ToolRunnerWithGuard::execute_tool()` at `crates/aletheon-body/src/impl/security/runner.rs:130-163` extracts `input["command"]` for ALL L1+ tools, then runs it through the sandbox. This breaks `file_write` (uses `path`+`content`), `apply_patch` (uses `patch`+`base_dir`), and any non-bash L1+ tool.

**Files:**
- Modify: `crates/aletheon-body/src/impl/security/runner.rs:130-163`

- [ ] **Step 1: Add `tool_name` parameter to distinguish tool types**

The fix is to check `tool.name()` and dispatch accordingly:
- `bash_exec` → sandbox with `input["command"]` (existing behavior, correct)
- All other L1+ tools → call `tool.execute(input, ctx)` directly (bypass sandbox)

This is the minimal correct fix: the sandbox is only meaningful for shell commands. File operations and patches have their own execution logic that must be called.

- [ ] **Step 2: Replace the L1+ execution block**

Replace lines 130-163 in `runner.rs` with:

```rust
        // 3. Execute tool (with optional sandbox for bash_exec L1+)
        let result = if tool.permission_level() >= PermissionLevel::L1 && tool.name() == "bash_exec" {
            // bash_exec: route through sandbox with the command field
            let cmd = input.get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let sandbox_config = SandboxConfig {
                working_dir: ctx.working_dir.to_string_lossy().to_string(),
                env_vars: std::collections::HashMap::new(),
            };

            match self.sandbox.run(cmd, &sandbox_config, Duration::from_secs(30)).await {
                Ok(sandbox_result) => ToolResult {
                    content: format!("{}\n{}", sandbox_result.stdout, sandbox_result.stderr)
                        .trim()
                        .to_string(),
                    is_error: sandbox_result.exit_code != 0,
                    metadata: ToolResultMeta {
                        execution_time_ms: sandbox_result.elapsed_ms,
                        truncated: false,
                    },
                },
                Err(e) => ToolResult {
                    content: format!("Sandbox execution failed: {}", e),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: 0,
                        truncated: false,
                    },
                },
            }
        } else if tool.permission_level() >= PermissionLevel::L1 {
            // Non-bash L1+ tools: call their own execute() method
            // The tool handles its own logic (file I/O, patching, etc.)
            tool.execute(input.clone(), ctx).await
        } else {
            // Direct execution for L0 tools
            tool.execute(input.clone(), ctx).await
        };
```

- [ ] **Step 3: Verify existing tests still pass**

Run: `cargo test -p aletheon-body`
Expected: All existing tests pass (the change only affects the guarded execution path, not direct tool tests).

- [ ] **Step 4: Add integration test for file_write through security runner**

Add to the test module at the bottom of `runner.rs`:

```rust
#[cfg(test)]
mod runner_tests {
    use super::*;
    use crate::r#impl::tools::file_write::FileWriteTool;
    use crate::r#impl::tools::bash_exec::BashExecTool;
    use aletheon_abi::tool::ToolContext;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_file_write_through_security_runner() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("test_output.txt");
        let ctx = ToolContext {
            working_dir: tmp.path().to_path_buf(),
            session_id: "test-session".to_string(),
        };

        let audit_logger = AuditLogger::new(None);
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger);
        runner.on_new_turn("test-turn");

        let tool = FileWriteTool;
        let input = serde_json::json!({
            "path": file_path.to_string_lossy(),
            "content": "hello from security runner"
        });

        let result = runner.execute_tool(&tool, input, &ctx, "test-turn").await;
        assert!(result.is_ok(), "file_write should succeed through runner: {:?}", result.err());
        let result = result.unwrap();
        assert!(!result.is_error, "file_write should not be an error: {}", result.content);

        // Verify the file was actually written
        let written = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(written, "hello from security runner");
    }

    #[tokio::test]
    async fn test_apply_patch_through_security_runner() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("existing.txt");
        tokio::fs::write(&file_path, "line one\nline two\nline three\n").await.unwrap();

        let ctx = ToolContext {
            working_dir: tmp.path().to_path_buf(),
            session_id: "test-session".to_string(),
        };

        let audit_logger = AuditLogger::new(None);
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger);
        runner.on_new_turn("test-turn");

        let tool = ApplyPatchTool;
        let input = serde_json::json!({
            "patch": "--- a/existing.txt\n+++ b/existing.txt\n@@ -1,3 +1,3 @@\n line one\n-line two\n+line TWO\n line three\n"
        });

        let result = runner.execute_tool(&tool, input, &ctx, "test-turn").await;
        assert!(result.is_ok(), "apply_patch should succeed through runner: {:?}", result.err());
        let result = result.unwrap();
        assert!(!result.is_error, "apply_patch should not be an error: {}", result.content);

        // Verify the patch was applied
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "line one\nline TWO\nline three\n");
    }

    #[tokio::test]
    async fn test_bash_exec_still_uses_sandbox() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext {
            working_dir: tmp.path().to_path_buf(),
            session_id: "test-session".to_string(),
        };

        let audit_logger = AuditLogger::new(None);
        let mut runner = ToolRunnerWithGuard::with_default_sandbox(audit_logger);
        runner.on_new_turn("test-turn");

        let tool = BashExecTool;
        let input = serde_json::json!({
            "command": "echo hello"
        });

        let result = runner.execute_tool(&tool, input, &ctx, "test-turn").await;
        // bash_exec should succeed (through sandbox or direct, depending on backend)
        assert!(result.is_ok(), "bash_exec should succeed: {:?}", result.err());
    }
}
```

- [ ] **Step 5: Run the new tests**

Run: `cargo test -p aletheon-body runner_tests`
Expected: All 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-body/src/impl/security/runner.rs
git commit -m "fix(security): dispatch L1+ tools by type, not universal bash assumption

The security runner previously extracted input[\"command\"] for ALL L1+
tools, breaking file_write, apply_patch, and any non-bash tool. Now:
- bash_exec → sandbox (unchanged)
- Other L1+ tools → tool.execute() directly

Closes the P0-corpus issue from the June 22 audit."
```

---

## Task 2: Fix socket path to user-space default

**Problem:** Default socket path is `/run/aletheon/aletheon.sock` which requires root. Three locations hardcode this.

**Files:**
- Modify: `crates/aletheon-abi/src/paths.rs:11`
- Modify: `crates/aletheon-runtime/src/core/config.rs:219`
- Modify: `crates/aletheon-brain/src/config/mod.rs:193`

- [ ] **Step 1: Add a user-space socket path function to aletheon-abi**

In `crates/aletheon-abi/src/paths.rs`, replace line 11:

```rust
/// System socket directory: /var/run/aletheon/
pub const SOCKET_DIR: &str = "/var/run/aletheon";
```

With:

```rust
/// System socket directory (for systemd service units only).
pub const SYSTEM_SOCKET_DIR: &str = "/var/run/aletheon";

/// User-space socket directory: $XDG_RUNTIME_DIR/aletheon or ~/.aletheon/
pub fn user_socket_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(xdg).join("aletheon")
    } else {
        config_dir()
    }
}

/// Default socket path for user-mode daemon.
pub fn default_socket_path() -> PathBuf {
    user_socket_dir().join("aletheon.sock")
}
```

Keep `SOCKET_DIR` as `SYSTEM_SOCKET_DIR` for backward compatibility with systemd units.

- [ ] **Step 2: Update aletheon-runtime config default**

In `crates/aletheon-runtime/src/core/config.rs`, replace line 219:

```rust
fn default_daemon_socket_path() -> String { "/run/aletheon/aletheon.sock".to_string() }
```

With:

```rust
fn default_daemon_socket_path() -> String {
    aletheon_abi::paths::default_socket_path().to_string_lossy().to_string()
}
```

- [ ] **Step 3: Update aletheon-brain config default**

In `crates/aletheon-brain/src/config/mod.rs`, replace line 193:

```rust
fn default_daemon_socket_path() -> String { "/run/aletheon/aletheon.sock".to_string() }
```

With:

```rust
fn default_daemon_socket_path() -> String {
    aletheon_abi::paths::default_socket_path().to_string_lossy().to_string()
}
```

- [ ] **Step 4: Add tests for socket path resolution**

In `crates/aletheon-abi/src/paths.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_socket_path_not_system() {
        let path = default_socket_path();
        let path_str = path.to_string_lossy();
        // Should NOT be in /run/ or /var/run/
        assert!(!path_str.starts_with("/run/"), "socket path should not be in /run/: {}", path_str);
        assert!(!path_str.starts_with("/var/run/"), "socket path should not be in /var/run/: {}", path_str);
        assert!(path_str.ends_with("aletheon.sock"), "should end with aletheon.sock: {}", path_str);
    }

    #[test]
    fn test_default_socket_path_xdg() {
        // When XDG_RUNTIME_DIR is set, should use it
        let path = default_socket_path();
        // At minimum, should be a valid path
        assert!(path.parent().is_some(), "socket path should have a parent directory");
    }

    #[test]
    fn test_system_socket_dir_still_available() {
        // System socket dir should still be available for systemd units
        assert_eq!(SYSTEM_SOCKET_DIR, "/var/run/aletheon");
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p aletheon-abi paths`
Run: `cargo test -p aletheon-runtime config`
Run: `cargo test -p aletheon-brain config`
Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-abi/src/paths.rs crates/aletheon-runtime/src/core/config.rs crates/aletheon-brain/src/config/mod.rs
git commit -m "fix(config): default socket path to user-space (~/.aletheon/)

Previously defaulted to /run/aletheon/aletheon.sock which requires root.
Now uses $XDG_RUNTIME_DIR/aletheon/ or ~/.aletheon/ for user-mode daemon.
System socket dir renamed to SYSTEM_SOCKET_DIR for systemd service units."
```

---

## Task 3: API key fail fast

**Problem:** `resolve_api_key()` returns `""` when no key is found. The error only surfaces on the first API call as a cryptic 401.

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/provider_factory.rs:104-114`
- Modify: `crates/aletheon-brain/src/impl/provider_registry.rs:139-145`

- [ ] **Step 1: Change provider_factory resolve_api_key to return Result**

In `crates/aletheon-brain/src/impl/llm/provider_factory.rs`, replace lines 104-114:

```rust
/// Resolve API key: config value first, then env var `<NAME>_API_KEY`.
fn resolve_api_key(config: &ProviderConfig) -> String {
    if !config.api_key.is_empty() {
        return config.api_key.clone();
    }
    let env_name = format!(
        "{}_API_KEY",
        config.name.to_uppercase().replace('-', "_")
    );
    std::env::var(&env_name).unwrap_or_default()
}
```

With:

```rust
/// Resolve API key: config value first, then env var `<NAME>_API_KEY`.
///
/// Returns an error if no key is found and the provider requires one.
/// Ollama (local) is exempt — it doesn't need an API key.
fn resolve_api_key(config: &ProviderConfig) -> Result<String> {
    if !config.api_key.is_empty() {
        return Ok(config.api_key.clone());
    }
    let env_name = format!(
        "{}_API_KEY",
        config.name.to_uppercase().replace('-', "_")
    );
    match std::env::var(&env_name) {
        Ok(key) if !key.is_empty() => Ok(key),
        _ => {
            // Ollama doesn't need an API key
            let base_lower = config.base_url.to_lowercase();
            if base_lower.contains("localhost:11434") || base_lower.contains("127.0.0.1:11434") {
                Ok(String::new())
            } else {
                anyhow::bail!(
                    "API key not found for provider '{}'. \
                     Set {} in your environment or add api_key to config.",
                    config.name,
                    env_name
                )
            }
        }
    }
}
```

- [ ] **Step 2: Update create_provider to propagate the error**

In the same file, `create_provider()` at line 38 calls `resolve_api_key(config)`. Since it now returns `Result`, update:

```rust
pub fn create_provider(config: &ProviderConfig, model: &str) -> Result<Arc<dyn LlmProvider>> {
    let api_key = resolve_api_key(config)?;
    // ... rest unchanged
}
```

And `create_provider_by_kind()` at line 80:

```rust
pub fn create_provider_by_kind(
    kind: &str,
    config: &ProviderConfig,
    model: &str,
) -> Result<Arc<dyn LlmProvider>> {
    let api_key = resolve_api_key(config)?;
    // ... rest unchanged
}
```

- [ ] **Step 3: Apply the same fix to provider_registry**

In `crates/aletheon-brain/src/impl/provider_registry.rs`, find the `resolve_api_key` method (around line 139). Apply the same pattern: return `Result<String>`, bail on missing key (except Ollama).

- [ ] **Step 4: Update tests**

In `provider_factory.rs` tests, update `test_resolve_api_key_from_config` to handle the new `Result` return type. Add a test for missing key:

```rust
    #[test]
    fn test_resolve_api_key_missing_returns_error() {
        let config = ProviderConfig {
            name: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: String::new(),
            transport: Transport::Auto,
            models: vec![],
        };
        // Remove env var if set
        std::env::remove_var("ANTHROPIC_API_KEY");
        let result = resolve_api_key(&config);
        assert!(result.is_err(), "should fail when API key is missing");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("ANTHROPIC_API_KEY"), "error should mention the env var name: {}", err_msg);
    }

    #[test]
    fn test_resolve_api_key_ollama_no_key_ok() {
        let config = ProviderConfig {
            name: "ollama".to_string(),
            base_url: "http://localhost:11434".to_string(),
            api_key: String::new(),
            transport: Transport::Auto,
            models: vec![],
        };
        let result = resolve_api_key(&config);
        assert!(result.is_ok(), "ollama should not require API key");
        assert_eq!(result.unwrap(), "");
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p aletheon-brain provider_factory`
Run: `cargo test -p aletheon-brain provider_registry`
Expected: All pass.

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/provider_factory.rs crates/aletheon-brain/src/impl/provider_registry.rs
git commit -m "fix(brain): API key missing now fails fast with clear error

Previously resolve_api_key() returned empty string on missing key,
deferring the error to the first API call (cryptic 401). Now returns
Result<String> with a message naming the expected env var. Ollama
(local provider) is exempt — it doesn't need an API key."
```

---

## Task 4: Fix Anthropic URL auto-detection

**Problem:** `detect_provider_kind()` and `detect_transport()` use `ends_with("/anthropic")` which doesn't match `https://api.anthropic.com`.

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/provider_factory.rs:17-26`
- Modify: `crates/aletheon-brain/src/impl/provider_registry.rs:20-27`

- [ ] **Step 1: Fix detect_provider_kind in provider_factory**

In `crates/aletheon-brain/src/impl/llm/provider_factory.rs`, replace lines 17-26:

```rust
fn detect_provider_kind(base_url: &str) -> &str {
    let normalized = base_url.trim().to_lowercase();
    if normalized.ends_with("/anthropic") {
        "anthropic"
    } else if normalized.contains("localhost:11434") || normalized.contains("127.0.0.1:11434") {
        "ollama"
    } else {
        "openai"
    }
}
```

With:

```rust
fn detect_provider_kind(base_url: &str) -> &str {
    let normalized = base_url.trim().to_lowercase();
    if normalized.contains("anthropic.com") || normalized.ends_with("/anthropic") {
        "anthropic"
    } else if normalized.contains("localhost:11434") || normalized.contains("127.0.0.1:11434") {
        "ollama"
    } else {
        "openai"
    }
}
```

- [ ] **Step 2: Fix detect_transport in provider_registry**

In `crates/aletheon-brain/src/impl/provider_registry.rs`, replace lines 20-27:

```rust
pub fn detect_transport(base_url: &str) -> ResolvedTransport {
    let normalized = base_url.trim().to_lowercase();
    if normalized.ends_with("/anthropic") {
        ResolvedTransport::Anthropic
    } else {
        ResolvedTransport::OpenAi
    }
}
```

With:

```rust
pub fn detect_transport(base_url: &str) -> ResolvedTransport {
    let normalized = base_url.trim().to_lowercase();
    if normalized.contains("anthropic.com") || normalized.ends_with("/anthropic") {
        ResolvedTransport::Anthropic
    } else {
        ResolvedTransport::OpenAi
    }
}
```

- [ ] **Step 3: Add test for the official Anthropic URL**

In `provider_factory.rs` tests, add:

```rust
    #[test]
    fn test_detect_provider_kind_anthropic_official_url() {
        assert_eq!(
            detect_provider_kind("https://api.anthropic.com"),
            "anthropic"
        );
    }

    #[test]
    fn test_detect_provider_kind_anthropic_with_path() {
        assert_eq!(
            detect_provider_kind("https://api.example.com/anthropic"),
            "anthropic"
        );
    }
```

In `provider_registry.rs` tests (if a test module exists), add:

```rust
    #[test]
    fn test_detect_transport_anthropic_official() {
        assert_eq!(
            detect_transport("https://api.anthropic.com"),
            ResolvedTransport::Anthropic
        );
    }

    #[test]
    fn test_detect_transport_anthropic_proxy() {
        assert_eq!(
            detect_transport("https://proxy.example.com/anthropic"),
            ResolvedTransport::Anthropic
        );
    }

    #[test]
    fn test_detect_transport_openai() {
        assert_eq!(
            detect_transport("https://api.openai.com"),
            ResolvedTransport::OpenAi
        );
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p aletheon-brain provider_factory`
Run: `cargo test -p aletheon-brain provider_registry`
Expected: All pass, including the new Anthropic URL tests.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/provider_factory.rs crates/aletheon-brain/src/impl/provider_registry.rs
git commit -m "fix(brain): detect Anthropic URLs by host, not just path suffix

Previously only matched URLs ending in /anthropic, which missed the
official https://api.anthropic.com. Now checks for 'anthropic.com' in
the host portion as well."
```

---

## Verification

After all 4 tasks are complete:

- [ ] **Full workspace check:** `cargo check --workspace`
- [ ] **Full workspace test:** `cargo test --workspace`
- [ ] **Clippy:** `cargo clippy --workspace -- -D warnings`

## Summary

| Task | Files | Effort | Risk |
|------|-------|--------|------|
| 1. L1+ tool dispatch | 1 file | 1-2h | Medium (core execution path) |
| 2. Socket path | 3 files | 30min | Low |
| 3. API key fail fast | 2 files | 1h | Low |
| 4. Anthropic URL | 2 files | 30min | Low |

Total estimated effort: 3-4 hours for all 4 tasks.
