use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufStream};

#[tokio::test]
async fn client_negotiates_capability_and_reads_committed_run_receipt() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("memory.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufStream::new(stream);

        let initialize = read(&mut stream).await;
        assert_eq!(initialize["method"], "initialize");
        assert_eq!(
            initialize["params"]["payload"]["data"]["capabilities"]["memory_maintenance_v1"],
            true
        );
        write(
            &mut stream,
            serde_json::json!({
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "protocol_version":1,
                    "payload":{
                        "type":"initialize_response",
                        "data":{
                            "protocol_version":1,
                            "server_capabilities":{
                                "item_events":false,
                                "cursors":false,
                                "memory_gateway_v1":false,
                                "memory_maintenance_v1":true
                            },
                            "connection_id":"00000000-0000-0000-0000-000000000001",
                            "principal_id":"local-uid:1000",
                            "os_principal":{"uid":1000,"gid":1000},
                            "runtime_version":"0.1.0"
                        }
                    },
                    "extensions":{}
                }
            }),
        )
        .await;

        let initialized = read(&mut stream).await;
        assert_eq!(initialized["method"], "initialized");
        write(
            &mut stream,
            serde_json::json!({"jsonrpc":"2.0","id":2,"result":{"status":"ready"}}),
        )
        .await;

        let run = read(&mut stream).await;
        assert_eq!(run["method"], "memory.maintenance.run/v1");
        assert_eq!(run["params"]["payload"]["data"]["max_items"], 3);
        write(
            &mut stream,
            serde_json::json!({
                "jsonrpc":"2.0",
                "id":3,
                "result":{
                    "protocol_version":1,
                    "payload":{
                        "type":"memory_maintenance_run_receipt",
                        "data":{
                            "request_id":"run-a",
                            "dry_run":false,
                            "claimed":1,
                            "promoted_local":1,
                            "rejected":0,
                            "deferred":0,
                            "receipts":[],
                            "reason_codes":[]
                        }
                    },
                    "extensions":{}
                }
            }),
        )
        .await;
    });

    let mut client = interact::memory_client::MemoryAgentClient::connect_official(Some(socket))
        .await
        .unwrap();
    let receipt = client.run("run-a", 3, false).await.unwrap();
    assert_eq!(receipt.claimed, 1);
    assert_eq!(receipt.promoted_local, 1);
    server.await.unwrap();
}

async fn read(stream: &mut BufStream<tokio::net::UnixStream>) -> serde_json::Value {
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    serde_json::from_str(line.trim()).unwrap()
}

async fn write(stream: &mut BufStream<tokio::net::UnixStream>, value: serde_json::Value) {
    stream
        .write_all(value.to_string().as_bytes())
        .await
        .unwrap();
    stream.write_all(b"\n").await.unwrap();
    stream.flush().await.unwrap();
}
