use std::sync::Arc;

use executive::application::inference_port::{
    CoreInferenceRequest, InferenceError, InferencePort, ModelCapabilities,
};
use executive::host::core_rpc::{
    CoreFrame, CorePeerPolicy, CoreRequest, CoreRpcClient, CoreRpcServer,
};
use fabric::{
    ContentBlock, InferenceUsage, LlmResponse, LlmStream, LocalOsPrincipal, StopReason, StreamChunk,
};
use futures::{stream, StreamExt};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

fn request() -> CoreInferenceRequest {
    CoreInferenceRequest {
        messages: vec![],
        tools: vec![],
        model_spec: String::new(),
    }
}

struct FakeInference;

#[async_trait::async_trait]
impl InferencePort for FakeInference {
    async fn provider_backpressure_metrics(
        &self,
    ) -> Result<
        std::collections::HashMap<String, cognit::inference::ProviderBackpressureSnapshot>,
        InferenceError,
    > {
        Ok(std::collections::HashMap::from([(
            "fixture-provider".into(),
            cognit::inference::ProviderBackpressureSnapshot {
                admitted: 7,
                available_permits: 2,
                ..Default::default()
            },
        )]))
    }

    async fn capabilities(&self, model_spec: &str) -> Result<ModelCapabilities, InferenceError> {
        Ok(ModelCapabilities {
            model_spec: format!("resolved/{model_spec}"),
            display_name: "fixture-model".into(),
            max_context_tokens: 1_000_000,
        })
    }

    async fn complete(
        &self,
        _request: CoreInferenceRequest,
    ) -> Result<LlmResponse, InferenceError> {
        Ok(LlmResponse {
            content: vec![ContentBlock::Text { text: "ok".into() }],
            stop_reason: StopReason::EndTurn,
            usage: InferenceUsage::default(),
        })
    }

    async fn stream(&self, _request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        Ok(Box::pin(stream::iter([
            Ok(StreamChunk::TextDelta {
                text: "streamed".into(),
            }),
            Ok(StreamChunk::Done {
                stop_reason: StopReason::EndTurn,
            }),
        ])))
    }
}

struct Harness {
    _directory: TempDir,
    socket: std::path::PathBuf,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    peers: Mutex<mpsc::Receiver<LocalOsPrincipal>>,
}

impl Harness {
    async fn start(limit: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("core.sock");
        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        let policy = CorePeerPolicy::new(uid, gid, [uid]);
        let (peer_tx, peer_rx) = mpsc::channel(8);
        let server = CoreRpcServer::bind_with_limit(
            &socket,
            Arc::new(FakeInference),
            policy,
            limit,
            Some(peer_tx),
        )
        .await
        .unwrap();
        let cancel = CancellationToken::new();
        let server_cancel = cancel.clone();
        let task = tokio::spawn(async move { server.run(server_cancel).await });
        Self {
            _directory: directory,
            socket,
            cancel,
            task,
            peers: Mutex::new(peer_rx),
        }
    }

    fn client(&self) -> CoreRpcClient {
        CoreRpcClient::new(self.socket.clone())
    }

    async fn observed_peer(&self) -> LocalOsPrincipal {
        self.peers.lock().await.recv().await.unwrap()
    }

    async fn shutdown(self) {
        self.cancel.cancel();
        self.task.await.unwrap().unwrap();
    }
}

#[test]
fn core_request_schema_has_no_authoritative_identity() {
    let value = serde_json::to_value(CoreRequest::complete(7, request())).unwrap();
    let wire = value.to_string();
    for forbidden in ["uid", "gid", "workspace", "working_dir"] {
        assert!(!wire.contains(forbidden), "request leaked {forbidden}");
    }
}

#[tokio::test]
async fn server_uses_peer_credentials_and_correlates_complete_and_stream_frames() {
    use std::os::unix::fs::PermissionsExt;

    let harness = Harness::start(8 * 1024 * 1024).await;
    assert_eq!(
        std::fs::metadata(&harness.socket)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o660
    );
    let client = harness.client();
    let provider_key = "https://provider.example::fixture";
    let first_provider_permit = client.acquire_provider_permit(provider_key).await.unwrap();
    assert_eq!(harness.observed_peer().await.uid, unsafe {
        libc::geteuid()
    });
    let second_provider_permit = client.acquire_provider_permit(provider_key).await.unwrap();
    assert_eq!(harness.observed_peer().await.uid, unsafe {
        libc::geteuid()
    });
    let third_acquire = tokio::spawn({
        let client = client.clone();
        async move { client.acquire_provider_permit(provider_key).await }
    });
    assert_eq!(harness.observed_peer().await.uid, unsafe {
        libc::geteuid()
    });
    assert!(
        !third_acquire.is_finished(),
        "the machine-core lease must retain the second configured permit"
    );
    drop(first_provider_permit);
    let third_provider_permit =
        tokio::time::timeout(std::time::Duration::from_secs(1), third_acquire)
            .await
            .expect("dropping the socket-backed lease releases the machine permit")
            .expect("provider permit task joins")
            .unwrap();
    drop(second_provider_permit);
    drop(third_provider_permit);
    client
        .observe_provider_retry_after(provider_key, Some(1))
        .await
        .unwrap();
    assert_eq!(harness.observed_peer().await.uid, unsafe {
        libc::geteuid()
    });

    let provider_metrics = client.provider_backpressure_metrics().await.unwrap();
    assert_eq!(provider_metrics["fixture-provider"].admitted, 7);
    assert_eq!(provider_metrics["fixture-provider"].available_permits, 2);
    assert_eq!(harness.observed_peer().await.uid, unsafe {
        libc::geteuid()
    });

    let capabilities = client.capabilities("fast").await.unwrap();
    assert_eq!(capabilities.model_spec, "resolved/fast");
    assert_eq!(capabilities.display_name, "fixture-model");
    assert_eq!(capabilities.max_context_tokens, 1_000_000);
    assert_eq!(harness.observed_peer().await.uid, unsafe {
        libc::geteuid()
    });

    let response = client.complete(request()).await.unwrap();
    assert_eq!(response.stop_reason, StopReason::EndTurn);
    assert_eq!(harness.observed_peer().await.uid, unsafe {
        libc::geteuid()
    });

    let chunks = client
        .stream(request())
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(
        chunks.last().unwrap().as_ref().unwrap(),
        StreamChunk::Done {
            stop_reason: StopReason::EndTurn
        }
    ));
    harness.shutdown().await;
}

#[test]
fn core_peer_policy_rejects_unlisted_users() {
    let policy = CorePeerPolicy::new(0, 991, [1001]);
    assert!(policy
        .authorize(LocalOsPrincipal {
            uid: 1001,
            gid: 100,
        })
        .is_ok());
    assert!(policy
        .authorize(LocalOsPrincipal {
            uid: 1002,
            gid: 100,
        })
        .is_err());
}

#[tokio::test]
async fn oversized_and_duplicate_frames_are_rejected() {
    let harness = Harness::start(128).await;

    let mut oversized = UnixStream::connect(&harness.socket).await.unwrap();
    oversized.write_all(&[b'x'; 129]).await.unwrap();
    let mut oversized_reader = BufReader::new(oversized);
    let mut line = String::new();
    oversized_reader.read_line(&mut line).await.unwrap();
    assert!(line.contains("frame exceeds 128 bytes"));
    harness.shutdown().await;

    let harness = Harness::start(1024).await;
    let mut stream = UnixStream::connect(&harness.socket).await.unwrap();
    let wire = serde_json::to_vec(&CoreRequest::complete(7, request())).unwrap();
    assert!(wire.len() <= 1024);
    stream.write_all(&wire).await.unwrap();
    stream.write_all(b"\n").await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut first = String::new();
    reader.read_line(&mut first).await.unwrap();
    assert!(matches!(
        serde_json::from_str::<CoreFrame>(&first).unwrap(),
        CoreFrame::Response { id: 7, .. }
    ));
    writer.write_all(&wire).await.unwrap();
    writer.write_all(b"\n").await.unwrap();
    let mut duplicate = String::new();
    reader.read_line(&mut duplicate).await.unwrap();
    assert!(matches!(
        serde_json::from_str::<CoreFrame>(&duplicate).unwrap(),
        CoreFrame::Error { id: 7, message } if message.contains("duplicate request id 7")
    ));
    harness.shutdown().await;
}
