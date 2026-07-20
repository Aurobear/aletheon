//! Opt-in contract test against the pinned upstream Pi executable.
//!
//! CI installs the reviewed package release and enables this test explicitly.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);

#[test]
#[ignore = "requires the pinned real Pi package; enabled by the dedicated CI job"]
fn pinned_pi_rpc_get_state_obeys_reviewed_jsonl_contract() {
    let executable = std::env::var("ALETHEON_REAL_PI").expect("ALETHEON_REAL_PI must name Pi");
    let expected_version =
        std::env::var("ALETHEON_REAL_PI_VERSION").expect("version pin must be provided");

    let version = Command::new(&executable)
        .arg("--version")
        .output()
        .expect("execute Pi version probe");
    assert!(version.status.success(), "Pi version probe failed");
    let actual_version = String::from_utf8(version.stdout).expect("Pi version is UTF-8");
    assert!(
        actual_version.contains(&expected_version),
        "Pi build identity drift: expected {expected_version}, got {actual_version:?}"
    );

    let workspace = tempfile::tempdir().expect("isolated Pi contract workspace");
    let config = tempfile::tempdir().expect("isolated Pi config directory");
    let mut child = Command::new(&executable)
        .args([
            "--mode",
            "rpc",
            "--no-session",
            "--no-context-files",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-approve",
            "--offline",
        ])
        .current_dir(workspace.path())
        .env("PI_CODING_AGENT_DIR", config.path())
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Pi RPC process");

    child
        .stdin
        .as_mut()
        .expect("Pi stdin")
        .write_all(b"{\"id\":\"aletheon-contract\",\"type\":\"get_state\"}\n")
        .expect("write LF-framed Pi RPC request");
    let stdout = child.stdout.take().expect("Pi stdout");
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let line = receiver
        .recv_timeout(RESPONSE_TIMEOUT)
        .expect("Pi RPC response timed out")
        .expect("read Pi RPC response");
    let _ = child.kill();
    let _ = child.wait();

    assert!(line.ends_with('\n'), "Pi RPC response is not LF framed");
    assert!(!line[..line.len() - 1].contains('\n'));
    let response: Value = serde_json::from_str(&line).expect("Pi response is JSON");
    assert_eq!(response["type"], "response");
    assert_eq!(response["command"], "get_state");
    assert_eq!(response["id"], "aletheon-contract");
    assert_eq!(response["success"], true);
    assert_eq!(response["data"]["isStreaming"], false);
}
