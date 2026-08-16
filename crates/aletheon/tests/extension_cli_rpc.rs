use std::path::PathBuf;

use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::oneshot;

fn aletheon_binary() -> PathBuf {
    let test_bin = std::env::current_exe().expect("cannot find current exe");
    test_bin
        .parent()
        .and_then(|path| path.parent())
        .expect("cannot resolve target directory")
        .join("aletheon")
}

#[tokio::test]
async fn enable_cli_uses_official_socket_not_direct_store() {
    let temp = TempDir::new().unwrap();
    let socket = temp.path().join("aletheon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (method_tx, method_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        // The typed protocol carries the extension operation inside the
        // Command::ManageExtension envelope. Assert the CLI sent the Enable
        // mutation for the requested package id.
        method_tx
            .send(format!(
                "enable:{}",
                request["body"]["Command"]["ManageExtension"]["Enable"]["id"]
                    .as_str()
                    .unwrap()
            ))
            .unwrap();
        // The CLI speaks the typed Gateway protocol (WireRequest/WireResponse),
        // not legacy JSON-RPC. The daemon responds with a typed Command outcome.
        let response = json!({
            "version": 1,
            "request_id": request["request_id"],
            "body": {
                "Command": {
                    "ExtensionResult": {
                        "result": {
                            "schema_version":1,
                            "operation":"enable",
                            "actor":"local:1000",
                            "package_id":"aurb.core",
                            "package_version":"1.0.0",
                            "package_hash":"aa",
                            "previous_snapshot_digest":"old",
                            "snapshot_digest":"new",
                            "permission_approved":true,
                            "health":"healthy",
                            "evidence_references":[]
                        }
                    }
                }
            }
        });
        writer
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });

    let output = tokio::process::Command::new(aletheon_binary())
        .args([
            "--socket",
            socket.to_str().unwrap(),
            "extension",
            "enable",
            "aurb.core",
            "--approve-permissions",
        ])
        .output()
        .await
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(method_rx.await.unwrap(), "enable:aurb.core");
    assert!(String::from_utf8_lossy(&output.stdout).contains("local:1000"));
    server.await.unwrap();
}
