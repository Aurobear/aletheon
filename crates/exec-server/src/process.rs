//! Process management for exec-server.
//!
//! Handles spawning, reading output, signaling, and terminating managed processes.
//! In-memory only — no persistence.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::protocol::*;

const MAX_HANDLES: usize = 128;
const MAX_BUFFER_BYTES: u64 = 1_048_576; // 1 MB
const TERMINATE_GRACE_MS: u64 = 500;
const READ_TIMEOUT_MS: u64 = 10; // short timeout so RPC loop is responsive

pub struct ProcessManager {
    processes: Arc<Mutex<HashMap<String, ManagedProcess>>>,
}

struct ManagedProcess {
    child: Child,
    owner: String,
    start_time: Instant,
    stdout: Option<BufReader<ChildStdout>>,
    stderr: Option<BufReader<ChildStderr>>,
    stdout_buf: Vec<u8>,
    stderr_buf: Vec<u8>,
    stdout_captured_bytes: u64,
    stderr_captured_bytes: u64,
    stdout_eof: bool,
    stderr_eof: bool,
    exited: bool,
    exit_code: Option<i32>,
    max_output_bytes: u64,
}

impl ProcessManager {
    pub fn new() -> Self {
        ProcessManager {
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn spawn(
        &self,
        owner: &str,
        req: &StartProcessRequest,
    ) -> Result<ProcessHandle, RpcError> {
        let mut procs = self.processes.lock().await;

        if procs.len() >= MAX_HANDLES {
            return Err(RpcError {
                code: SPAWN_FAILED,
                message: format!("Maximum concurrent handles reached ({})", MAX_HANDLES),
                data: None,
            });
        }

        let mut cmd = Command::new(&req.command);
        cmd.args(&req.args);

        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        if let Some(ref wd) = req.working_dir {
            cmd.current_dir(wd);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.stdin(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        // A fresh process group lets termination target the complete command
        // tree rather than only the immediate shell child.
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd.spawn().map_err(|e| RpcError {
            code: SPAWN_FAILED,
            message: format!("Failed to spawn process '{}': {}", req.command, e),
            data: None,
        })?;

        let pid = child.id().unwrap_or(0);
        let handle_id = format!("proc_{}", pid);

        let stdout = child.stdout.take().map(BufReader::new);
        let stderr = child.stderr.take().map(BufReader::new);

        let max_output_bytes = req.max_output_bytes.unwrap_or(MAX_BUFFER_BYTES);

        procs.insert(
            handle_id.clone(),
            ManagedProcess {
                child,
                owner: owner.to_owned(),
                start_time: Instant::now(),
                stdout,
                stderr,
                stdout_buf: Vec::new(),
                stderr_buf: Vec::new(),
                stdout_captured_bytes: 0,
                stderr_captured_bytes: 0,
                stdout_eof: false,
                stderr_eof: false,
                exited: false,
                exit_code: None,
                max_output_bytes,
            },
        );
        drop(procs);

        if let Some(timeout_secs) = req.timeout_secs {
            let processes = Arc::downgrade(&self.processes);
            let timed_handle = handle_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
                let Some(processes) = processes.upgrade() else {
                    return;
                };
                let process = processes.lock().await.remove(&timed_handle);
                if let Some(process) = process {
                    terminate_and_reap(process).await;
                }
            });
        }

        Ok(ProcessHandle { pid, handle_id })
    }

    pub async fn read(&self, handle_id: &str) -> Result<Vec<ReadChunk>, RpcError> {
        let mut procs = self.processes.lock().await;
        let proc = procs.get_mut(handle_id).ok_or_else(|| RpcError {
            code: PROCESS_NOT_FOUND,
            message: format!("Process handle not found: {}", handle_id),
            data: None,
        })?;

        let mut chunks = Vec::new();

        // Read stdout
        if !proc.stdout_eof {
            let mut got_data = false;
            let mut eof = false;

            if let Some(ref mut reader) = proc.stdout {
                let mut buf = [0u8; 8192];
                let read_fut = reader.read(&mut buf);
                match tokio::time::timeout(Duration::from_millis(READ_TIMEOUT_MS), read_fut).await {
                    Ok(Ok(0)) => {
                        eof = true;
                    }
                    Ok(Ok(n)) => {
                        let remaining = proc
                            .max_output_bytes
                            .saturating_sub(proc.stdout_captured_bytes);
                        let take = (n as u64).min(remaining) as usize;
                        if take > 0 {
                            proc.stdout_buf.extend_from_slice(&buf[..take]);
                            proc.stdout_captured_bytes =
                                proc.stdout_captured_bytes.saturating_add(take as u64);
                        }
                        got_data = true;
                    }
                    Ok(Err(_)) => {
                        eof = true;
                    }
                    Err(_timeout) => {
                        // No data available yet — that's fine
                    }
                }
            } else {
                eof = true;
            }

            if eof {
                proc.stdout_eof = true;
            }

            if !proc.stdout_buf.is_empty() {
                let data = String::from_utf8_lossy(&proc.stdout_buf).into_owned();
                proc.stdout_buf.clear();
                chunks.push(ReadChunk {
                    data,
                    stream: "stdout".into(),
                    eof: proc.stdout_eof,
                    exit_code: None,
                });
            } else if proc.stdout_eof && (got_data || proc.start_time.elapsed() > Duration::ZERO) {
                chunks.push(ReadChunk {
                    data: String::new(),
                    stream: "stdout".into(),
                    eof: true,
                    exit_code: None,
                });
            }
        }

        // Read stderr
        if !proc.stderr_eof {
            let mut got_data = false;
            let mut eof = false;

            if let Some(ref mut reader) = proc.stderr {
                let mut buf = [0u8; 8192];
                let read_fut = reader.read(&mut buf);
                match tokio::time::timeout(Duration::from_millis(READ_TIMEOUT_MS), read_fut).await {
                    Ok(Ok(0)) => {
                        eof = true;
                    }
                    Ok(Ok(n)) => {
                        let remaining = proc
                            .max_output_bytes
                            .saturating_sub(proc.stderr_captured_bytes);
                        let take = (n as u64).min(remaining) as usize;
                        if take > 0 {
                            proc.stderr_buf.extend_from_slice(&buf[..take]);
                            proc.stderr_captured_bytes =
                                proc.stderr_captured_bytes.saturating_add(take as u64);
                        }
                        got_data = true;
                    }
                    Ok(Err(_)) => {
                        eof = true;
                    }
                    Err(_timeout) => {
                        // No data available yet — that's fine
                    }
                }
            } else {
                eof = true;
            }

            if eof {
                proc.stderr_eof = true;
            }

            if !proc.stderr_buf.is_empty() {
                let data = String::from_utf8_lossy(&proc.stderr_buf).into_owned();
                proc.stderr_buf.clear();
                chunks.push(ReadChunk {
                    data,
                    stream: "stderr".into(),
                    eof: proc.stderr_eof,
                    exit_code: None,
                });
            } else if proc.stderr_eof && got_data {
                chunks.push(ReadChunk {
                    data: String::new(),
                    stream: "stderr".into(),
                    eof: true,
                    exit_code: None,
                });
            }
        }

        // Try to reap if both streams are at EOF
        if proc.stdout_eof && proc.stderr_eof && !proc.exited {
            if let Ok(Some(status)) = proc.child.try_wait() {
                proc.exited = true;
                proc.exit_code = status.code().or(Some(-1));
            }
        }

        if proc.exited {
            if chunks.is_empty() {
                chunks.push(ReadChunk {
                    data: String::new(),
                    stream: "stdout".into(),
                    eof: true,
                    exit_code: proc.exit_code,
                });
            } else {
                for chunk in &mut chunks {
                    chunk.exit_code = proc.exit_code;
                }
            }
        }

        Ok(chunks)
    }

    pub async fn write_stdin(&self, handle_id: &str, data: &str) -> Result<(), RpcError> {
        use tokio::io::AsyncWriteExt;

        let mut procs = self.processes.lock().await;
        let proc = procs.get_mut(handle_id).ok_or_else(|| RpcError {
            code: PROCESS_NOT_FOUND,
            message: format!("Process handle not found: {}", handle_id),
            data: None,
        })?;

        if let Some(ref mut stdin) = proc.child.stdin {
            stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| RpcError {
                    code: INTERNAL_ERROR,
                    message: format!("Failed to write to process stdin: {}", e),
                    data: None,
                })?;
        }

        Ok(())
    }

    pub async fn signal(&self, handle_id: &str, sig: i32) -> Result<(), RpcError> {
        let mut procs = self.processes.lock().await;
        let proc = procs.get_mut(handle_id).ok_or_else(|| RpcError {
            code: PROCESS_NOT_FOUND,
            message: format!("Process handle not found: {}", handle_id),
            data: None,
        })?;

        let pid = proc.child.id().unwrap_or(0);

        #[cfg(unix)]
        {
            if pid > 0 {
                unsafe {
                    libc::kill(
                        -(pid as i32),
                        if sig == 9 {
                            libc::SIGKILL
                        } else {
                            libc::SIGTERM
                        },
                    );
                }
            }
        }

        #[cfg(not(unix))]
        {
            let _ = proc.child.start_kill();
        }

        Ok(())
    }

    pub async fn terminate(&self, handle_id: &str) -> Result<(), RpcError> {
        let proc = {
            let mut procs = self.processes.lock().await;
            procs.remove(handle_id).ok_or_else(|| RpcError {
                code: PROCESS_NOT_FOUND,
                message: format!("Process handle not found: {}", handle_id),
                data: None,
            })?
        };
        terminate_and_reap(proc).await;
        Ok(())
    }

    /// Terminate and reap every foreground process owned by a disconnected
    /// transport connection. Other connections' processes are left intact.
    pub async fn cleanup_owner(&self, owner: &str) -> usize {
        let owned = {
            let mut procs = self.processes.lock().await;
            let handles: Vec<_> = procs
                .iter()
                .filter(|(_, process)| process.owner == owner)
                .map(|(handle, _)| handle.clone())
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| procs.remove(&handle))
                .collect::<Vec<_>>()
        };
        let count = owned.len();
        let mut tasks = tokio::task::JoinSet::new();
        for process in owned {
            tasks.spawn(terminate_and_reap(process));
        }
        while tasks.join_next().await.is_some() {}
        count
    }

    #[cfg(test)]
    async fn contains(&self, handle_id: &str) -> bool {
        self.processes.lock().await.contains_key(handle_id)
    }

    #[cfg(test)]
    async fn owner_of(&self, handle_id: &str) -> Option<String> {
        self.processes
            .lock()
            .await
            .get(handle_id)
            .map(|process| process.owner.clone())
    }

    #[cfg(test)]
    async fn pid_of(&self, handle_id: &str) -> Option<u32> {
        self.processes
            .lock()
            .await
            .get(handle_id)
            .and_then(|process| process.child.id())
    }

    /// Kill all managed processes on shutdown.
    pub async fn shutdown(&self) {
        let processes = {
            let mut procs = self.processes.lock().await;
            procs
                .drain()
                .map(|(_, process)| process)
                .collect::<Vec<_>>()
        };
        let mut tasks = tokio::task::JoinSet::new();
        for process in processes {
            tasks.spawn(terminate_and_reap(process));
        }
        while tasks.join_next().await.is_some() {}
    }
}

async fn terminate_and_reap(mut process: ManagedProcess) {
    let pid = process.child.id().unwrap_or(0);
    #[cfg(unix)]
    unsafe {
        if pid > 0 {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    let _ = process.child.start_kill();

    if tokio::time::timeout(
        Duration::from_millis(TERMINATE_GRACE_MS),
        process.child.wait(),
    )
    .await
    .is_err()
    {
        #[cfg(unix)]
        unsafe {
            if pid > 0 {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = process.child.start_kill();
        let _ = process.child.wait().await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sleeping_process() -> StartProcessRequest {
        StartProcessRequest {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "exec sleep 30".into()],
            env: HashMap::new(),
            working_dir: None,
            timeout_secs: None,
            max_output_bytes: None,
        }
    }

    fn process_is_gone(pid: u32) -> bool {
        let result = unsafe { libc::kill(pid as i32, 0) };
        result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    fn process_is_terminated(pid: u32) -> bool {
        if process_is_gone(pid) {
            return true;
        }
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(") ")
                    .map(|(_, rest)| rest.starts_with('Z'))
            })
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn spawned_process_records_connection_owner() {
        let manager = ProcessManager::new();
        let handle = manager
            .spawn("connection-a", &sleeping_process())
            .await
            .unwrap();

        assert_eq!(
            manager.owner_of(&handle.handle_id).await.as_deref(),
            Some("connection-a")
        );
        manager.cleanup_owner("connection-a").await;
    }

    #[tokio::test]
    async fn disconnect_cleanup_only_terminates_and_reaps_owned_children() {
        let manager = ProcessManager::new();
        let owned = manager
            .spawn("connection-a", &sleeping_process())
            .await
            .unwrap();
        let other = manager
            .spawn("connection-b", &sleeping_process())
            .await
            .unwrap();
        let owned_pid = manager.pid_of(&owned.handle_id).await.unwrap();

        assert_eq!(manager.cleanup_owner("connection-a").await, 1);
        assert!(!manager.contains(&owned.handle_id).await);
        assert!(manager.contains(&other.handle_id).await);
        assert!(
            process_is_gone(owned_pid),
            "owned child must be waited and reaped"
        );

        assert_eq!(manager.cleanup_owner("connection-b").await, 1);
    }

    #[tokio::test]
    async fn stdin_write_reaches_child_and_stream_limit_is_cumulative() {
        let manager = ProcessManager::new();
        let request = StartProcessRequest {
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "read line; printf '%s-abcdef' \"$line\"; printf 'stderr-abcdef' >&2".into(),
            ],
            env: HashMap::new(),
            working_dir: None,
            timeout_secs: None,
            max_output_bytes: Some(5),
        };
        let handle = manager.spawn("connection-a", &request).await.unwrap();
        manager
            .write_stdin(&handle.handle_id, "input\n")
            .await
            .unwrap();

        let mut stdout = String::new();
        let mut stderr = String::new();
        for _ in 0..20 {
            for chunk in manager.read(&handle.handle_id).await.unwrap() {
                match chunk.stream.as_str() {
                    "stdout" => stdout.push_str(&chunk.data),
                    "stderr" => stderr.push_str(&chunk.data),
                    _ => unreachable!(),
                }
            }
            if stdout.len() == 5 && stderr.len() == 5 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(stdout, "input");
        assert_eq!(stderr, "stder");
        manager.cleanup_owner("connection-a").await;
    }

    #[tokio::test]
    async fn owner_cleanup_terminates_the_complete_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let child_pid_path = temp.path().join("descendant.pid");
        let request = StartProcessRequest {
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "sleep 30 & child=$!; printf '%s' \"$child\" > {}; wait",
                    child_pid_path.display()
                ),
            ],
            env: HashMap::new(),
            working_dir: Some(temp.path().to_string_lossy().into_owned()),
            timeout_secs: None,
            max_output_bytes: None,
        };
        let manager = ProcessManager::new();
        let parent = manager.spawn("connection-a", &request).await.unwrap();
        let descendant_pid = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Ok(value) = std::fs::read_to_string(&child_pid_path) {
                    if let Ok(pid) = value.parse::<u32>() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("descendant pid must be reported");

        manager.cleanup_owner("connection-a").await;
        assert!(process_is_gone(parent.pid));
        for _ in 0..50 {
            if process_is_terminated(descendant_pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("descendant process survived process-group termination");
    }

    #[tokio::test]
    async fn configured_timeout_terminates_and_reaps_process() {
        let manager = ProcessManager::new();
        let mut request = sleeping_process();
        request.timeout_secs = Some(1);
        let handle = manager.spawn("connection-a", &request).await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.contains(&handle.handle_id).await || !process_is_gone(handle.pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("configured timeout must remove the managed process");
        assert!(process_is_gone(handle.pid));
    }

    #[tokio::test]
    async fn read_reports_authoritative_exit_code_after_both_streams_close() {
        let manager = ProcessManager::new();
        let request = StartProcessRequest {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf out; printf err >&2; exit 7".into()],
            env: HashMap::new(),
            working_dir: None,
            timeout_secs: None,
            max_output_bytes: None,
        };
        let handle = manager.spawn("connection-a", &request).await.unwrap();
        let exit_code = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(code) = manager
                    .read(&handle.handle_id)
                    .await
                    .unwrap()
                    .into_iter()
                    .find_map(|chunk| chunk.exit_code)
                {
                    break code;
                }
            }
        })
        .await
        .expect("exit code must become observable");
        assert_eq!(exit_code, 7);
        manager.terminate(&handle.handle_id).await.unwrap();
    }
}
