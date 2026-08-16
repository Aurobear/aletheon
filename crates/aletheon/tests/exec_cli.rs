use std::io::Write;
use std::process::{Command, Stdio};

fn command(
    home: &std::path::Path,
    workspace: &std::path::Path,
    config: &std::path::Path,
) -> Command {
    let runtime = home.join("run");
    std::fs::create_dir_all(&runtime).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_aletheon"));
    command
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_RUNTIME_DIR", runtime)
        .args([
            "-C",
            workspace.to_str().unwrap(),
            "exec",
            "--output",
            "jsonl",
            "--config",
            config.to_str().unwrap(),
            "--idempotency-key",
            "ci-replay-1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(mut command: Command) -> std::process::Output {
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"read the prompt from stdin")
        .unwrap();
    child.wait_with_output().unwrap()
}

fn run_empty(mut command: Command) -> std::process::Output {
    let mut child = command.spawn().unwrap();
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

#[test]
fn u_cli_003_jsonl_is_ordered_terminal_and_idempotently_replayable() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = temp.path().join("invalid.toml");
    std::fs::write(&config, "[invalid\n").unwrap();

    let first = run(command(temp.path(), &workspace, &config));
    assert_eq!(first.status.code(), Some(24));
    assert!(first.stderr.is_empty(), "protocol leaked to stderr");
    let lines = String::from_utf8(first.stdout).unwrap();
    let events = lines
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["schema_version"], 1);
    assert_eq!(events[0]["sequence"], 1);
    assert_eq!(events[0]["type"], "terminal");
    assert_eq!(events[0]["status"], "validation_failed");
    for id in ["session_id", "task_id", "turn_id", "operation_id"] {
        assert!(events[0][id]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
    }

    std::fs::remove_file(&config).unwrap();
    let replay = run(command(temp.path(), &workspace, &config));
    // The first run failed before producing a durable terminal receipt, so a
    // second run with the same idempotency key is blocked rather than
    // replayed. The protocol still emits exactly one typed terminal event on
    // stdout and never leaks to stderr.
    assert_eq!(replay.status.code(), Some(20));
    assert!(replay.stderr.is_empty());
    let replay_events = String::from_utf8(replay.stdout).unwrap();
    let replay_event: serde_json::Value =
        serde_json::from_str(replay_events.trim()).unwrap();
    assert_eq!(replay_event["type"], "terminal");
    assert_eq!(replay_event["status"], "blocked");
}

#[test]
fn u_cli_003_invalid_stdin_uses_the_versioned_protocol_and_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = temp.path().join("unused.toml");

    let output = run_empty(command(temp.path(), &workspace, &config));
    assert_eq!(output.status.code(), Some(24));
    assert!(output.stderr.is_empty());
    let events = String::from_utf8(output.stdout).unwrap();
    let event: serde_json::Value = serde_json::from_str(events.trim()).unwrap();
    assert_eq!(event["schema_version"], 1);
    assert_eq!(event["sequence"], 1);
    assert_eq!(event["type"], "terminal");
    assert_eq!(event["status"], "validation_failed");
    assert_eq!(event["error_code"], "validation_failed");
}
