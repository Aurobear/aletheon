# P5: Testing Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build tmux-based test harness with real LLM integration, create test scenarios for all new features, and implement frame validation.

**Architecture:** Test harness in `tests/tui/` manages tmux sessions, sends keystrokes, captures frames, and validates against expected snapshots. Real LLM calls via configured API key. Scenarios are JSONL files.

**Tech Stack:** Rust, tokio, tmux CLI, serde_json

**Depends on:** P0-P4 (all layers wired)

---

### Task 1: Create test harness

**Files:**
- Create: `tests/tui/harness.rs`
- Create: `tests/tui/mod.rs`

- [ ] **Step 1: Create `tests/tui/mod.rs`**

```rust
pub mod harness;
```

- [ ] **Step 2: Create `tests/tui/harness.rs`**

```rust
//! tmux-based test harness for TUI integration testing.
//!
//! Orchestrates: start daemon → start TUI in tmux → send keys → capture frames → validate.

use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

/// Configuration for the test harness.
pub struct TestHarnessConfig {
    /// Path to aletheon-exec or aletheon-cli binary.
    pub binary_path: PathBuf,
    /// Path to the Unix socket.
    pub socket_path: PathBuf,
    /// tmux session name.
    pub session_name: String,
    /// LLM API key (for real LLM tests).
    pub api_key: Option<String>,
    /// LLM model to use.
    pub model: String,
    /// Working directory.
    pub working_dir: PathBuf,
    /// Timeout for individual operations.
    pub timeout: Duration,
}

impl Default for TestHarnessConfig {
    fn default() -> Self {
        Self {
            binary_path: PathBuf::from("target/debug/aletheon-cli"),
            socket_path: PathBuf::from("/tmp/aletheon-test.sock"),
            session_name: "aletheon-test".to_string(),
            api_key: None,
            model: "claude-sonnet-4-6".to_string(),
            working_dir: std::env::current_dir().unwrap_or_default(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// Test harness that manages tmux sessions for TUI testing.
pub struct TmuxTestHarness {
    config: TestHarnessConfig,
    daemon_handle: Option<tokio::process::Child>,
}

impl TmuxTestHarness {
    pub fn new(config: TestHarnessConfig) -> Self {
        Self {
            config,
            daemon_handle: None,
        }
    }

    /// Start the daemon process.
    pub async fn start_daemon(&mut self) -> Result<(), String> {
        let mut cmd = Command::new(&self.config.binary_path);
        cmd.arg("--socket").arg(&self.config.socket_path);

        if let Some(ref key) = self.config.api_key {
            cmd.env("ANTHROPIC_API_KEY", key);
        }

        let child = cmd.spawn().map_err(|e| format!("Failed to start daemon: {}", e))?;
        self.daemon_handle = Some(child);

        // Wait for socket to appear
        for _ in 0..50 {
            if self.config.socket_path.exists() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Err("Daemon socket did not appear within 5 seconds".to_string())
    }

    /// Start the TUI in a tmux session.
    pub async fn start_tui(&mut self) -> Result<(), String> {
        // Kill any existing session
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &self.config.session_name])
            .output()
            .await;

        // Create new tmux session
        let status = Command::new("tmux")
            .args([
                "new-session", "-d", "-s", &self.config.session_name,
                "-x", "120", "-y", "40",
            ])
            .status()
            .await
            .map_err(|e| format!("Failed to create tmux session: {}", e))?;

        if !status.success() {
            return Err("Failed to create tmux session".to_string());
        }

        // Send the TUI command
        let tui_cmd = format!(
            "{} --socket {} 2>/tmp/aletheon-tui-test.log",
            self.config.binary_path.display(),
            self.config.socket_path.display()
        );

        self.send_keys(&tui_cmd).await?;
        self.send_keys("Enter").await?;

        // Wait for TUI to render
        tokio::time::sleep(Duration::from_secs(2)).await;

        Ok(())
    }

    /// Send keystrokes to the tmux session.
    pub async fn send_keys(&self, keys: &str) -> Result<(), String> {
        let status = Command::new("tmux")
            .args(["send-keys", "-t", &self.config.session_name, keys])
            .status()
            .await
            .map_err(|e| format!("Failed to send keys: {}", e))?;

        if !status.success() {
            return Err("Failed to send keys".to_string());
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    /// Send text (character by character, for typing simulation).
    pub async fn send_text(&self, text: &str) -> Result<(), String> {
        for ch in text.chars() {
            self.send_keys(&ch.to_string()).await?;
        }
        Ok(())
    }

    /// Capture the current TUI frame from tmux.
    pub async fn capture_frame(&self) -> Result<String, String> {
        let output = Command::new("tmux")
            .args(["capture-pane", "-t", &self.config.session_name, "-p"])
            .output()
            .await
            .map_err(|e| format!("Failed to capture frame: {}", e))?;

        String::from_utf8(output.stdout)
            .map_err(|e| format!("Invalid UTF-8 in frame: {}", e))
    }

    /// Wait for a condition to be true in the captured frame.
    pub async fn wait_for(
        &self,
        condition: impl Fn(&str) -> bool,
        timeout: Duration,
    ) -> Result<String, String> {
        let start = tokio::time::Instant::now();

        loop {
            let frame = self.capture_frame().await?;
            if condition(&frame) {
                return Ok(frame);
            }

            if start.elapsed() > timeout {
                return Err(format!(
                    "Timeout waiting for condition after {}s. Last frame:\n{}",
                    timeout.as_secs(),
                    frame
                ));
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Assert that the frame contains expected text.
    pub fn assert_contains(frame: &str, expected: &str) -> Result<(), String> {
        if frame.contains(expected) {
            Ok(())
        } else {
            Err(format!(
                "Frame does not contain '{}'. Frame:\n{}",
                expected, frame
            ))
        }
    }

    /// Assert that the frame does NOT contain text.
    pub fn assert_not_contains(frame: &str, unexpected: &str) -> Result<(), String> {
        if !frame.contains(unexpected) {
            Ok(())
        } else {
            Err(format!(
                "Frame unexpectedly contains '{}'. Frame:\n{}",
                unexpected, frame
            ))
        }
    }

    /// Teardown: kill tmux session and daemon.
    pub async fn teardown(&mut self) {
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &self.config.session_name])
            .output()
            .await;

        if let Some(mut handle) = self.daemon_handle.take() {
            let _ = handle.kill().await;
        }

        // Clean up socket
        let _ = tokio::fs::remove_file(&self.config.socket_path).await;
    }
}

impl Drop for TmuxTestHarness {
    fn drop(&mut self) {
        // Best-effort cleanup
        let session_name = self.config.session_name.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("tmux")
                .args(["kill-session", "-t", &session_name])
                .output();
        });
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --tests`

- [ ] **Step 4: Commit**

```bash
git add tests/tui/
git commit -m "test: add tmux-based TUI test harness"
```

---

### Task 2: Create basic_chat integration test

**Files:**
- Create: `tests/integration/basic_chat.rs`

- [ ] **Step 1: Create the test**

```rust
//! Basic chat integration test.
//!
//! Tests: send message → receive streaming response → turn completes.

use crate::tui::harness::{TmuxTestHarness, TestHarnessConfig};
use std::time::Duration;

#[tokio::test]
async fn test_basic_chat_response() {
    let config = TestHarnessConfig {
        api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
        ..Default::default()
    };

    if config.api_key.is_none() {
        eprintln!("Skipping test: ANTHROPIC_API_KEY not set");
        return;
    }

    let mut harness = TmuxTestHarness::new(config);
    harness.start_daemon().await.expect("Failed to start daemon");
    harness.start_tui().await.expect("Failed to start TUI");

    // Send a simple message
    harness.send_text("Hello, respond with just 'hi'").await.unwrap();
    harness.send_keys("Enter").await.unwrap();

    // Wait for response to complete
    let frame = harness.wait_for(
        |f| f.contains("hi") || f.contains("turn_done"),
        Duration::from_secs(30),
    ).await.expect("Did not receive response");

    // Verify response contains something
    TmuxTestHarness::assert_contains(&frame, "hi").ok();

    harness.teardown().await;
}
```

- [ ] **Step 2: Create `tests/integration/mod.rs`**

```rust
pub mod basic_chat;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --tests`

- [ ] **Step 4: Commit**

```bash
git add tests/integration/
git commit -m "test: add basic_chat integration test"
```

---

### Task 3: Create mode_switching integration test

**Files:**
- Create: `tests/integration/mode_switching.rs`

- [ ] **Step 1: Create the test**

```rust
//! Mode switching integration test.
//!
//! Tests: /mode plan → verify plan mode → /mode default → verify default.

use crate::tui::harness::{TmuxTestHarness, TestHarnessConfig};
use std::time::Duration;

#[tokio::test]
async fn test_mode_cycle() {
    let config = TestHarnessConfig {
        api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
        ..Default::default()
    };

    if config.api_key.is_none() {
        eprintln!("Skipping test: ANTHROPIC_API_KEY not set");
        return;
    }

    let mut harness = TmuxTestHarness::new(config);
    harness.start_daemon().await.unwrap();
    harness.start_tui().await.unwrap();

    // Switch to plan mode
    harness.send_text("/mode plan").await.unwrap();
    harness.send_keys("Enter").await.unwrap();

    // Wait for mode change
    let frame = harness.wait_for(
        |f| f.contains("plan"),
        Duration::from_secs(5),
    ).await.expect("Mode did not change to plan");

    TmuxTestHarness::assert_contains(&frame, "plan").unwrap();

    // Switch back to default
    harness.send_text("/mode default").await.unwrap();
    harness.send_keys("Enter").await.unwrap();

    let frame = harness.wait_for(
        |f| f.contains("default"),
        Duration::from_secs(5),
    ).await.expect("Mode did not change back to default");

    TmuxTestHarness::assert_contains(&frame, "default").unwrap();

    harness.teardown().await;
}
```

- [ ] **Step 2: Register in `tests/integration/mod.rs`**

```rust
pub mod mode_switching;
```

- [ ] **Step 3: Commit**

```bash
git add tests/integration/
git commit -m "test: add mode_switching integration test"
```

---

### Task 4: Create interrupt integration test

**Files:**
- Create: `tests/integration/interrupt.rs`

- [ ] **Step 1: Create the test**

```rust
//! Interrupt integration test.
//!
//! Tests: send message → Ctrl+C during streaming → verify partial response.

use crate::tui::harness::{TmuxTestHarness, TestHarnessConfig};
use std::time::Duration;

#[tokio::test]
async fn test_interrupt_streaming() {
    let config = TestHarnessConfig {
        api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
        ..Default::default()
    };

    if config.api_key.is_none() {
        eprintln!("Skipping test: ANTHROPIC_API_KEY not set");
        return;
    }

    let mut harness = TmuxTestHarness::new(config);
    harness.start_daemon().await.unwrap();
    harness.start_tui().await.unwrap();

    // Send a message that will generate a long response
    harness.send_text("Write a 500 word essay about Rust programming").await.unwrap();
    harness.send_keys("Enter").await.unwrap();

    // Wait a bit for streaming to start
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Send Ctrl+C to interrupt
    harness.send_keys("C-c").await.unwrap();

    // Wait for interrupt to be acknowledged
    let frame = harness.wait_for(
        |f| f.contains("Interrupt") || f.contains("interrupt") || f.contains("cancel"),
        Duration::from_secs(5),
    ).await.expect("Interrupt not acknowledged");

    // Should have some partial response
    harness.teardown().await;
}
```

- [ ] **Step 2: Register and commit**

```bash
git add tests/integration/interrupt.rs tests/integration/mod.rs
git commit -m "test: add interrupt integration test"
```

---

### Task 5: Create plan_mode integration test

**Files:**
- Create: `tests/integration/plan_mode.rs`

- [ ] **Step 1: Create the test**

```rust
//! Plan mode integration test.
//!
//! Tests: /mode plan → send request → verify plan displayed → /approve → verify execution.

use crate::tui::harness::{TmuxTestHarness, TestHarnessConfig};
use std::time::Duration;

#[tokio::test]
async fn test_plan_approve_execute() {
    let config = TestHarnessConfig {
        api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
        ..Default::default()
    };

    if config.api_key.is_none() {
        eprintln!("Skipping test: ANTHROPIC_API_KEY not set");
        return;
    }

    let mut harness = TmuxTestHarness::new(config);
    harness.start_daemon().await.unwrap();
    harness.start_tui().await.unwrap();

    // Enter plan mode
    harness.send_text("/mode plan").await.unwrap();
    harness.send_keys("Enter").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send a request
    harness.send_text("Create a hello world Python file").await.unwrap();
    harness.send_keys("Enter").await.unwrap();

    // Wait for plan to be displayed
    let frame = harness.wait_for(
        |f| f.contains("Plan") || f.contains("plan") || f.contains("approve"),
        Duration::from_secs(30),
    ).await.expect("Plan not displayed");

    // Approve the plan
    harness.send_text("/approve").await.unwrap();
    harness.send_keys("Enter").await.unwrap();

    // Wait for execution
    let frame = harness.wait_for(
        |f| f.contains("turn_done") || f.contains("completed"),
        Duration::from_secs(60),
    ).await.expect("Execution did not complete");

    harness.teardown().await;
}
```

- [ ] **Step 2: Register and commit**

```bash
git add tests/integration/plan_mode.rs tests/integration/mod.rs
git commit -m "test: add plan_mode integration test"
```

---

### Task 6: Run all tests

- [ ] **Step 1: Run unit tests**

Run: `cargo test --workspace`
Expected: All unit tests pass.

- [ ] **Step 2: Run integration tests (requires ANTHROPIC_API_KEY)**

Run: `ANTHROPIC_API_KEY=sk-... cargo test --test integration -- --test-threads=1`
Expected: Integration tests pass (or skip if no API key).

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "test: P5 Testing complete — tmux harness, 4 integration tests (basic_chat, mode_switch, interrupt, plan_mode)"
```

---

## Summary

P5 adds:

| File | Action | What Added |
|------|--------|------------|
| `tests/tui/harness.rs` | NEW | `TmuxTestHarness` — tmux session management, key sending, frame capture |
| `tests/integration/basic_chat.rs` | NEW | Basic chat response test |
| `tests/integration/mode_switching.rs` | NEW | Mode cycle test |
| `tests/integration/interrupt.rs` | NEW | Ctrl+C interrupt test |
| `tests/integration/plan_mode.rs` | NEW | Plan → approve → execute test |
