//! Deterministic mock sandbox backend for integration tests.
//!
//! Implements `fabric::SandboxBackend` with pre-configured FIFO result queues per
//! tool name. Records all executed commands for post-turn assertion.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use fabric::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};

/// A pre-configured result for a single tool invocation.
pub struct MockToolResult {
    pub output: String,
    pub is_error: bool,
}

/// Record of a single sandbox execution for post-turn assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockExecutionRecord {
    pub cmd: String,
    pub config_workspace_cwd: String,
}

/// Deterministic mock sandbox backend for integration tests.
///
/// Pre-configured with FIFO result queues per tool name. The mock identifies
/// "tool calls" by scanning the command string (the sandbox's `execute()` receives
/// a shell command string, not a structured tool call). Tests should configure
/// results with the tool name as the key.
///
/// If no result is configured for a given tool name, the mock panics — the test
/// under-specified its expected tool calls.
pub struct MockSandbox {
    /// FIFO queue of results per tool name.
    results: Mutex<HashMap<String, Vec<MockToolResult>>>,
    /// All execute calls for post-turn assertion.
    execution_log: Mutex<Vec<MockExecutionRecord>>,
}

impl MockSandbox {
    /// Create a mock sandbox with pre-configured tool results.
    ///
    /// Keys should match the tool name as it appears in the command string
    /// (e.g. "bash" for bash commands, "read" for file reads).
    pub fn new(results: HashMap<String, Vec<MockToolResult>>) -> Self {
        Self {
            results: Mutex::new(results),
            execution_log: Mutex::new(Vec::new()),
        }
    }

    /// Create a mock sandbox with no pre-configured results.
    /// Useful when sandbox execution is not expected in a test scenario.
    pub fn empty() -> Self {
        Self::new(HashMap::new())
    }

    /// All execution records for assertion.
    pub fn execution_log(&self) -> Vec<MockExecutionRecord> {
        self.execution_log.lock().unwrap().clone()
    }

    /// Number of times execute() was called.
    pub fn execution_count(&self) -> usize {
        self.execution_log.lock().unwrap().len()
    }

    /// Pop the next result for a given tool name. Panics if no results configured.
    fn next_result(&self, tool_name: &str) -> MockToolResult {
        let mut results = self.results.lock().unwrap();
        let queue = results.get_mut(tool_name).unwrap_or_else(|| {
            panic!(
                "MockSandbox: no results configured for tool '{tool_name}'. \
                 Test must configure results for every expected tool call."
            )
        });
        if queue.is_empty() {
            panic!(
                "MockSandbox: result queue exhausted for tool '{tool_name}'. \
                 Expected more invocations of this tool."
            );
        }
        queue.remove(0)
    }
}

#[async_trait]
impl SandboxBackend for MockSandbox {
    fn name(&self) -> &str {
        "mock"
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::None
    }

    fn is_available(&self) -> bool {
        true
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: false,
            network_isolation: false,
            resource_limits: false,
            seccomp_filter: false,
            limitations: vec!["mock sandbox — no real isolation".into()],
        }
    }

    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        _timeout: Duration,
    ) -> anyhow::Result<SandboxResult> {
        // Record the execution for assertion.
        self.execution_log.lock().unwrap().push(MockExecutionRecord {
            cmd: cmd.to_string(),
            config_workspace_cwd: config.working_dir().display().to_string(),
        });

        // Match tool name from the command. The sandbox receives a shell command string;
        // we look for known tool names as substrings. Tests should use distinct tool names.
        let result = if let Some(tool_name) = guess_tool_name(cmd) {
            self.next_result(tool_name)
        } else {
            // Return a success with empty output for unrecognized commands.
            MockToolResult {
                output: String::new(),
                is_error: false,
            }
        };

        Ok(SandboxResult {
            stdout: result.output.clone(),
            stderr: String::new(),
            exit_code: if result.is_error { 1 } else { 0 },
            backend_used: "mock".into(),
            isolation_level: IsolationLevel::None,
            elapsed_ms: 0,
        })
    }
}

/// Guess the tool name from the command string.
/// The sandbox receives shell commands like "bash -c 'ls'" or "file_read path".
/// Returns the tool name if recognized, or None for unrecognized commands.
fn guess_tool_name(cmd: &str) -> Option<&str> {
    let known_tools = ["bash", "read", "write", "edit", "glob", "grep"];
    for tool in known_tools {
        if cmd.starts_with(tool) || cmd.contains(&format!("{tool} ")) {
            return Some(tool);
        }
    }
    // If the command starts with a known binary name, use the first word.
    cmd.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_preconfigured_result() {
        let mut results = HashMap::new();
        results.insert(
            "bash".into(),
            vec![MockToolResult {
                output: "file.txt".into(),
                is_error: false,
            }],
        );
        let sandbox = MockSandbox::new(results);
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                std::path::PathBuf::from("/tmp"),
                vec![],
            )
            .unwrap(),
            environment: Default::default(),
            policy: None,
        };

        let result = sandbox
            .execute("bash -c ls", &config, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result.stdout, "file.txt");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn records_execution_log() {
        let mut results = HashMap::new();
        results.insert(
            "bash".into(),
            vec![MockToolResult {
                output: "ok".into(),
                is_error: false,
            }],
        );
        let sandbox = MockSandbox::new(results);
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                std::path::PathBuf::from("/tmp"),
                vec![],
            )
            .unwrap(),
            environment: Default::default(),
            policy: None,
        };

        sandbox
            .execute("bash -c ls", &config, Duration::from_secs(1))
            .await
            .unwrap();

        let log = sandbox.execution_log();
        assert_eq!(log.len(), 1);
        assert!(log[0].cmd.contains("bash"));
    }

    #[tokio::test]
    #[should_panic(expected = "exhausted")]
    async fn panics_when_queue_exhausted() {
        let mut results = HashMap::new();
        results.insert(
            "bash".into(),
            vec![MockToolResult {
                output: "only".into(),
                is_error: false,
            }],
        );
        let sandbox = MockSandbox::new(results);
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                std::path::PathBuf::from("/tmp"),
                vec![],
            )
            .unwrap(),
            environment: Default::default(),
            policy: None,
        };

        // First call succeeds.
        sandbox
            .execute("bash -c ls", &config, Duration::from_secs(1))
            .await
            .unwrap();

        // Second call panics.
        sandbox
            .execute("bash -c pwd", &config, Duration::from_secs(1))
            .await
            .unwrap();
    }

    #[tokio::test]
    #[should_panic(expected = "no results configured")]
    async fn panics_when_no_results_for_tool() {
        let sandbox = MockSandbox::empty();
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                std::path::PathBuf::from("/tmp"),
                vec![],
            )
            .unwrap(),
            environment: Default::default(),
            policy: None,
        };

        sandbox
            .execute("bash -c ls", &config, Duration::from_secs(1))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reports_error_results() {
        let mut results = HashMap::new();
        results.insert(
            "bash".into(),
            vec![MockToolResult {
                output: "permission denied".into(),
                is_error: true,
            }],
        );
        let sandbox = MockSandbox::new(results);
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                std::path::PathBuf::from("/tmp"),
                vec![],
            )
            .unwrap(),
            environment: Default::default(),
            policy: None,
        };

        let result = sandbox
            .execute("bash -c rm /important", &config, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.stdout, "permission denied");
    }
}
