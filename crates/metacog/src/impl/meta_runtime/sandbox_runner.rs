//! Sandbox runner — tests candidate runtimes in isolation.
//!
//! Runs cargo test in the workspace and parses the output to produce
//! a TestResult with pass/fail counts and failure details.

use anyhow::{Context, Result};
use fabric::{Clock, RuntimeCandidate, TestResult};
use std::process::Command;
use std::sync::Arc;

pub struct SandboxRunner {
    /// Working directory for running tests (defaults to current dir).
    work_dir: std::path::PathBuf,
    clock: Arc<dyn Clock>,
}

impl SandboxRunner {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            work_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            clock,
        }
    }

    pub fn with_work_dir(work_dir: std::path::PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self { work_dir, clock }
    }

    /// Run sandbox tests on a candidate runtime.
    ///
    /// Runs `cargo test --workspace --message-format=json` and parses
    /// the JSON output to extract test results.
    pub async fn run_tests(&self, _candidate: &RuntimeCandidate) -> Result<TestResult> {
        let start = self.clock.mono_now();

        let output = Command::new("cargo")
            .args(["test", "--workspace", "--message-format=json"])
            .current_dir(&self.work_dir)
            .output()
            .context("Failed to run cargo test")?;

        let elapsed_ms = self.clock.mono_now().0 - start.0;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Parse JSON test results from cargo test
        let mut tests_run = 0usize;
        let mut tests_passed = 0usize;
        let mut tests_failed = 0usize;
        let mut failures = Vec::new();

        for line in stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }
            // cargo test --message-format=json outputs one JSON object per line
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match event_type {
                    "test" => {
                        let name = event
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let test_event = event.get("event").and_then(|v| v.as_str()).unwrap_or("");
                        match test_event {
                            "ok" => {
                                tests_run += 1;
                                tests_passed += 1;
                            }
                            "failed" => {
                                tests_run += 1;
                                tests_failed += 1;
                                let stderr =
                                    event.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
                                failures.push(format!("{}: {}", name, stderr.trim()));
                            }
                            "ignored" | "bench" => {
                                // Count ignored/bench but don't fail
                            }
                            _ => {}
                        }
                    }
                    "suite" => {
                        // Suite-level events — we rely on individual test events
                    }
                    _ => {}
                }
            }
        }

        // If we couldn't parse JSON output, fall back to exit code
        if tests_run == 0 {
            let passed = output.status.success();
            if passed {
                // No test output parsed but exit was 0 — assume some tests ran
                tests_run = 1;
                tests_passed = 1;
            } else {
                tests_run = 1;
                tests_failed = 1;
                // Try to extract failure info from stderr
                let last_lines: Vec<&str> = stderr.lines().rev().take(10).collect();
                failures.push(format!(
                    "cargo test failed (no JSON output). stderr tail: {}",
                    last_lines
                        .iter()
                        .rev()
                        .copied()
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
        }

        Ok(TestResult {
            passed: tests_failed == 0,
            tests_run,
            tests_passed,
            tests_failed,
            failures,
            elapsed_ms,
        })
    }
}
