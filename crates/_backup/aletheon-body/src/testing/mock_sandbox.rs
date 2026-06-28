use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use aletheon_abi::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};

/// Mock sandbox with an in-memory virtual file system.
///
/// Supports registering canned command outputs and tracking executed commands.
pub struct MockSandbox {
    /// Maps command string -> canned result.
    responses: Mutex<HashMap<String, SandboxResult>>,
    /// Log of all executed commands.
    pub execution_log: Mutex<Vec<(String, SandboxConfig)>>,
    /// In-memory filesystem: path -> content.
    fs: Mutex<HashMap<String, String>>,
}

impl MockSandbox {
    pub fn new() -> Self {
        Self {
            responses: Mutex::new(HashMap::new()),
            execution_log: Mutex::new(Vec::new()),
            fs: Mutex::new(HashMap::new()),
        }
    }

    /// Register a canned response for a specific command.
    pub fn register_command(&self, cmd: impl Into<String>, stdout: impl Into<String>) {
        let mut map = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(
            cmd.into(),
            SandboxResult {
                stdout: stdout.into(),
                stderr: String::new(),
                exit_code: 0,
                backend_used: "mock".to_string(),
                isolation_level: IsolationLevel::None,
                elapsed_ms: 1,
            },
        );
    }

    /// Register a canned response with custom exit code and stderr.
    pub fn register_command_full(
        &self,
        cmd: impl Into<String>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        exit_code: i32,
    ) {
        let mut map = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(
            cmd.into(),
            SandboxResult {
                stdout: stdout.into(),
                stderr: stderr.into(),
                exit_code,
                backend_used: "mock".to_string(),
                isolation_level: IsolationLevel::None,
                elapsed_ms: 1,
            },
        );
    }

    /// Write a file to the in-memory filesystem.
    pub fn write_file(&self, path: impl Into<String>, content: impl Into<String>) {
        self.fs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.into(), content.into());
    }

    /// Read a file from the in-memory filesystem.
    pub fn read_file(&self, path: &str) -> Option<String> {
        self.fs.lock().unwrap_or_else(|e| e.into_inner()).get(path).cloned()
    }

    /// List all files in the in-memory filesystem.
    pub fn list_files(&self) -> Vec<String> {
        self.fs.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }
}

impl Default for MockSandbox {
    fn default() -> Self {
        Self::new()
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
            limitations: vec!["Mock sandbox -- no real isolation".into()],
        }
    }

    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        _timeout: Duration,
    ) -> Result<SandboxResult> {
        self.execution_log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((cmd.to_string(), config.clone()));

        // Handle built-in cat command for in-memory FS
        if let Some(path) = cmd.strip_prefix("cat ") {
            let path = path.trim();
            if let Some(content) = self.fs.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
                return Ok(SandboxResult {
                    stdout: content.clone(),
                    stderr: String::new(),
                    exit_code: 0,
                    backend_used: "mock".to_string(),
                    isolation_level: IsolationLevel::None,
                    elapsed_ms: 0,
                });
            }
        }

        // Handle built-in echo redirect: echo "..." > path
        if cmd.starts_with("echo ") {
            if let Some(redirect_pos) = cmd.find(" > ") {
                let content_part = &cmd[5..redirect_pos].trim();
                let content = content_part.trim_matches('"');
                let path = cmd[redirect_pos + 3..].trim();
                self.fs
                    .lock()
                    .unwrap()
                    .insert(path.to_string(), content.to_string());
                return Ok(SandboxResult {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                    backend_used: "mock".to_string(),
                    isolation_level: IsolationLevel::None,
                    elapsed_ms: 0,
                });
            }
        }

        // Look up canned response
        let map = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(result) = map.get(cmd) {
            return Ok(result.clone());
        }

        // Default: return empty success
        Ok(SandboxResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            backend_used: "mock".to_string(),
            isolation_level: IsolationLevel::None,
            elapsed_ms: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_sandbox_canned_response() {
        let mock = MockSandbox::new();
        mock.register_command("echo hello", "hello\n");

        let config = SandboxConfig::default();
        let result = mock
            .execute("echo hello", &config, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(result.stdout, "hello\n");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_mock_sandbox_filesystem() {
        let mock = MockSandbox::new();
        mock.write_file("/tmp/test.txt", "file content");

        let config = SandboxConfig::default();
        let result = mock
            .execute("cat /tmp/test.txt", &config, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(result.stdout, "file content");
    }

    #[tokio::test]
    async fn test_mock_sandbox_echo_redirect() {
        let mock = MockSandbox::new();
        let config = SandboxConfig::default();

        mock.execute(
            r#"echo "data" > /tmp/out.txt"#,
            &config,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(mock.read_file("/tmp/out.txt").unwrap(), "data");
    }

    #[tokio::test]
    async fn test_mock_sandbox_execution_log() {
        let mock = MockSandbox::new();
        mock.register_command("ls", "file1\nfile2\n");

        let config = SandboxConfig {
            working_dir: "/home".to_string(),
            env_vars: HashMap::new(),
        };
        mock.execute("ls", &config, Duration::from_secs(5))
            .await
            .unwrap();

        let log = mock.execution_log.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "ls");
        assert_eq!(log[0].1.working_dir, "/home");
    }

    #[tokio::test]
    async fn test_mock_sandbox_error_response() {
        let mock = MockSandbox::new();
        mock.register_command_full("bad_cmd", "", "command not found", 127);

        let config = SandboxConfig::default();
        let result = mock
            .execute("bad_cmd", &config, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 127);
        assert_eq!(result.stderr, "command not found");
    }
}
