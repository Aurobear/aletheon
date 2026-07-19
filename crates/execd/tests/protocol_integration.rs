use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    fn spawn(workspace: &Path) -> Self {
        let secret = "integration-test-secret";
        let mut child = Command::new(env!("CARGO_BIN_EXE_execd"))
            .env("ALETHEON_EXECD_SECRET", secret)
            .env(
                "ALETHEON_EXECD_WORKSPACE_ROOTS",
                serde_json::to_string(&[workspace]).unwrap(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut server = Self {
            child,
            input,
            output,
            next_id: 1,
        };
        let result = server.call("handshake", serde_json::json!({"secret": secret}));
        assert_eq!(result["protocol_version"], 1);
        server
    }

    fn call(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.input,
            "{}",
            serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        if let Some(error) = response.get("error") {
            panic!("{method} failed: {error}");
        }
        response["result"].clone()
    }

    fn start_shell(&mut self, workspace: &Path, command: &str) -> String {
        self.call(
            "process/start",
            serde_json::json!({
                "command":"/bin/sh",
                "args":["-c",command],
                "env":{},
                "working_dir":workspace,
            }),
        )["handle_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn read_until_terminal(&mut self, handle: &str) -> (Vec<serde_json::Value>, i64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut chunks = Vec::new();
        loop {
            assert!(Instant::now() < deadline, "terminal chunk timed out");
            let batch = self
                .call("process/read", serde_json::json!({"handle_id":handle}))
                .as_array()
                .unwrap()
                .clone();
            let terminal = batch.iter().find_map(|chunk| chunk["exit_code"].as_i64());
            chunks.extend(batch);
            if let Some(code) = terminal {
                return (chunks, code);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn real_process_read_streams_multiple_progress_chunks_and_one_terminal_exit() {
    let workspace = tempfile::tempdir().unwrap();
    let mut server = Server::spawn(workspace.path());
    let handle = server.start_shell(
        workspace.path(),
        "printf first; sleep 0.05; printf second; sleep 0.05; printf third >&2; exit 7",
    );
    let (chunks, code) = server.read_until_terminal(&handle);
    let data_chunks = chunks
        .iter()
        .filter(|chunk| !chunk["data"].as_str().unwrap_or("").is_empty())
        .count();
    let terminal_codes = chunks
        .iter()
        .filter_map(|chunk| chunk["exit_code"].as_i64())
        .collect::<Vec<_>>();
    assert!(
        data_chunks >= 3,
        "expected incremental progress: {chunks:?}"
    );
    assert!(!terminal_codes.is_empty());
    assert!(terminal_codes.iter().all(|terminal| *terminal == 7));
    assert_eq!(code, 7);
    server.call("process/kill", serde_json::json!({"handle_id":handle}));
}

#[test]
fn killed_server_can_be_reconnected_without_replaying_a_completed_command() {
    let workspace = tempfile::tempdir().unwrap();
    let marker = workspace.path().join("marker");
    let mut first = Server::spawn(workspace.path());
    let handle = first.start_shell(
        workspace.path(),
        &format!("printf x >> '{}'; exit 0", marker.display()),
    );
    assert_eq!(first.read_until_terminal(&handle).1, 0);
    first.child.kill().unwrap();
    first.child.wait().unwrap();

    let mut reconnected = Server::spawn(workspace.path());
    assert_eq!(
        reconnected.call("ping", serde_json::json!({}))["status"],
        "ok"
    );
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "x");
}

#[test]
fn terminate_is_bounded_and_normal_exit_code_remains_authoritative() {
    let workspace = tempfile::tempdir().unwrap();
    let mut server = Server::spawn(workspace.path());
    let sleeping = server.start_shell(workspace.path(), "exec sleep 30");
    let started = Instant::now();
    server.call(
        "process/terminate",
        serde_json::json!({"handle_id":sleeping}),
    );
    assert!(started.elapsed() < Duration::from_secs(2));

    let exited = server.start_shell(workspace.path(), "exit 23");
    assert_eq!(server.read_until_terminal(&exited).1, 23);
    server.call("process/kill", serde_json::json!({"handle_id":exited}));
}
