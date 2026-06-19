//! Pre-processing script runner for automation pipelines.

use anyhow::{Context, Result};
use std::path::Path;

/// Stateless script executor.
pub struct ScriptRunner;

impl ScriptRunner {
    /// Run a script file with the given environment variables and return its
    /// stdout as a `String`.
    ///
    /// The script is executed via `/bin/sh -c <path>` so it may be any format
    /// accepted by the system shell (bash, python, etc.).
    pub async fn run(script: &Path, env_vars: &[(String, String)]) -> Result<String> {
        let script_path = script
            .to_str()
            .context("Script path contains invalid UTF-8")?;

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(script_path);

        // Clear inherited env and inject only provided vars + PATH.
        cmd.env_clear();
        for (key, value) in env_vars {
            cmd.env(key, value);
        }
        // Preserve PATH so `sh` can be found and scripts can call common tools.
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }

        let output = cmd.output().await.context("Failed to execute script")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Script '{}' exited with {}: {}",
                script_path,
                output.status,
                stderr.trim()
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Write a script body to a file in a temp directory and make it executable.
    /// Returns the path (the directory is dropped on scope exit, cleaning up).
    fn make_executable_script(body: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.sh");
        fs::write(&path, body).unwrap();
        // Set owner execute bit.
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn run_simple_script() {
        let (_dir, path) = make_executable_script("#!/bin/sh\necho hello\n");

        let result = ScriptRunner::run(&path, &[]).await.unwrap();
        assert_eq!(result.trim(), "hello");
    }

    #[tokio::test]
    async fn run_with_env_vars() {
        let (_dir, path) = make_executable_script("#!/bin/sh\necho $MY_VAR\n");

        let env = vec![("MY_VAR".to_string(), "custom_value".to_string())];
        let result = ScriptRunner::run(&path, &env).await.unwrap();
        assert_eq!(result.trim(), "custom_value");
    }

    #[tokio::test]
    async fn run_script_failure() {
        let (_dir, path) = make_executable_script("#!/bin/sh\nexit 1\n");

        let result = ScriptRunner::run(&path, &[]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_multiline_output() {
        let (_dir, path) = make_executable_script("#!/bin/sh\necho line1\necho line2\n");

        let result = ScriptRunner::run(&path, &[]).await.unwrap();
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines, vec!["line1", "line2"]);
    }
}
