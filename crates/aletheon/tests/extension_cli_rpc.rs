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
        let mut method_tx = Some(method_tx);
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();
        for index in 0..3 {
            let line = lines.next_line().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let id = request["id"].clone();
            let result = match index {
                0 => json!({"protocol_version":1}),
                1 => json!({"status":"ready"}),
                _ => {
                    method_tx
                        .take()
                        .unwrap()
                        .send(request["method"].as_str().unwrap().to_owned())
                        .unwrap();
                    json!({
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
                    })
                }
            };
            let response = json!({"jsonrpc":"2.0", "id":id, "result":result});
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
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
    assert_eq!(method_rx.await.unwrap(), "extension.enable");
    assert!(String::from_utf8_lossy(&output.stdout).contains("local:1000"));
    server.await.unwrap();
}
