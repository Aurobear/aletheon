use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream;
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use ::contracts::{LlmResponse, LlmStream};
use cognit::ports::inference::{
    CoreInferenceRequest, InferenceError, InferencePort, ModelCapabilities,
};

use super::protocol::{
    read_json_line, write_json_line, CoreFrame, CoreRequest, DEFAULT_MAX_FRAME_BYTES,
};

struct CoreRpcProviderPermit {
    // The machine core retains its semaphore permit until this authenticated
    // connection closes. Dropping the client-side reader releases authority.
    _connection: BufReader<UnixStream>,
}

fn core_error(message: String) -> anyhow::Error {
    if message.starts_with("provider_unavailable") {
        cognit::inference::InferenceFailure::transient("provider_unavailable").context(message)
    } else if message.starts_with("provider_rejected_request") {
        cognit::inference::InferenceFailure::terminal("provider_rejected_request").context(message)
    } else {
        anyhow::anyhow!(message)
    }
}

#[derive(Clone)]
pub struct CoreRpcClient {
    socket_path: PathBuf,
    max_frame_bytes: usize,
    next_request_id: Arc<AtomicU64>,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreRpcReadinessError {
    #[error(
        "machine core readiness timed out after {waited_ms}ms for socket {socket_path}: {last_error}"
    )]
    Timeout {
        socket_path: PathBuf,
        waited_ms: u64,
        last_error: String,
    },
    #[error("connecting to machine core socket {socket_path} during readiness probe: {source}")]
    Connect {
        socket_path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

const CORE_READINESS_RETRY_INTERVAL: Duration = Duration::from_millis(200);

impl CoreRpcClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            next_request_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn with_frame_limit(mut self, max_frame_bytes: usize) -> Self {
        self.max_frame_bytes = max_frame_bytes;
        self
    }

    fn next_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Wait for the machine-core listener during daemon bootstrap. Only the
    /// two expected startup races are retried; permissions and all other I/O
    /// failures remain immediate typed errors.
    pub async fn wait_until_ready(&self, timeout: Duration) -> Result<(), CoreRpcReadinessError> {
        let started = Instant::now();
        let mut last_error = "socket has not accepted a connection".to_owned();
        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(CoreRpcReadinessError::Timeout {
                    socket_path: self.socket_path.clone(),
                    waited_ms: duration_millis(timeout),
                    last_error,
                });
            }
            match tokio::time::timeout(remaining, UnixStream::connect(&self.socket_path)).await {
                Ok(Ok(stream)) => {
                    drop(stream);
                    return Ok(());
                }
                Ok(Err(error)) if is_core_startup_race(&error) => {
                    last_error = error.to_string();
                }
                Ok(Err(source)) => {
                    return Err(CoreRpcReadinessError::Connect {
                        socket_path: self.socket_path.clone(),
                        source,
                    });
                }
                Err(_) => {
                    return Err(CoreRpcReadinessError::Timeout {
                        socket_path: self.socket_path.clone(),
                        waited_ms: duration_millis(timeout),
                        last_error,
                    });
                }
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                continue;
            }
            tokio::time::sleep(CORE_READINESS_RETRY_INTERVAL.min(remaining)).await;
        }
    }

    async fn connect_and_send(&self, request: &CoreRequest) -> Result<UnixStream, InferenceError> {
        let operation = core_request_operation(request);
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "connecting to machine core socket {} for {operation}: {error}",
                    self.socket_path.display()
                )
            })?;
        write_json_line(&mut stream, request, self.max_frame_bytes)
            .await
            .map_err(InferenceError::from)?;
        Ok(stream)
    }
}

fn is_core_startup_race(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn core_request_operation(request: &CoreRequest) -> &'static str {
    match request {
        CoreRequest::AcquireProviderPermit { .. } => "provider permit request",
        CoreRequest::ObserveProviderRetryAfter { .. } => "provider cooldown observation",
        CoreRequest::ProviderBackpressureMetrics { .. } => "provider metrics request",
        CoreRequest::Capabilities { .. } => "capabilities request",
        CoreRequest::Complete { .. } => "completion request",
        CoreRequest::Stream { .. } => "stream request",
    }
}

#[async_trait::async_trait]
impl InferencePort for CoreRpcClient {
    async fn acquire_provider_permit(
        &self,
        provider_key: &str,
    ) -> Result<Box<dyn ::contracts::memory::ProviderRequestPermit>, InferenceError> {
        let id = self.next_id();
        let stream = self
            .connect_and_send(&CoreRequest::acquire_provider_permit(id, provider_key))
            .await?;
        let mut reader = BufReader::new(stream);
        let frame = read_json_line::<_, CoreFrame>(&mut reader, self.max_frame_bytes)
            .await
            .map_err(InferenceError::from)?
            .ok_or_else(|| {
                InferenceError::from(anyhow::anyhow!(
                    "core RPC closed before provider permit response"
                ))
            })?;
        if frame.id() != id {
            return Err(anyhow::anyhow!(
                "core RPC response id {} does not match request id {id}",
                frame.id()
            )
            .into());
        }
        match frame {
            CoreFrame::ProviderPermitAcquired { .. } => Ok(Box::new(CoreRpcProviderPermit {
                _connection: reader,
            })),
            CoreFrame::Error { message, .. } => Err(core_error(message).into()),
            other => Err(anyhow::anyhow!(
                "unexpected core RPC frame for provider permit request: {other:?}"
            )
            .into()),
        }
    }

    async fn observe_provider_retry_after(
        &self,
        provider_key: &str,
        retry_after_ms: Option<u64>,
    ) -> Result<(), InferenceError> {
        let id = self.next_id();
        let stream = self
            .connect_and_send(&CoreRequest::observe_provider_retry_after(
                id,
                provider_key,
                retry_after_ms,
            ))
            .await?;
        let mut reader = BufReader::new(stream);
        let frame = read_json_line::<_, CoreFrame>(&mut reader, self.max_frame_bytes)
            .await
            .map_err(InferenceError::from)?
            .ok_or_else(|| {
                InferenceError::from(anyhow::anyhow!(
                    "core RPC closed before provider cooldown response"
                ))
            })?;
        if frame.id() != id {
            return Err(anyhow::anyhow!(
                "core RPC response id {} does not match request id {id}",
                frame.id()
            )
            .into());
        }
        match frame {
            CoreFrame::ProviderCooldownObserved { .. } => Ok(()),
            CoreFrame::Error { message, .. } => Err(core_error(message).into()),
            other => Err(anyhow::anyhow!(
                "unexpected core RPC frame for provider cooldown request: {other:?}"
            )
            .into()),
        }
    }

    async fn provider_backpressure_metrics(
        &self,
    ) -> Result<HashMap<String, cognit::inference::ProviderBackpressureSnapshot>, InferenceError>
    {
        let id = self.next_id();
        let stream = self
            .connect_and_send(&CoreRequest::provider_backpressure_metrics(id))
            .await?;
        let mut reader = BufReader::new(stream);
        let frame = read_json_line::<_, CoreFrame>(&mut reader, self.max_frame_bytes)
            .await
            .map_err(InferenceError::from)?
            .ok_or_else(|| {
                InferenceError::from(anyhow::anyhow!(
                    "core RPC closed before provider metrics response"
                ))
            })?;
        if frame.id() != id {
            return Err(anyhow::anyhow!(
                "core RPC response id {} does not match request id {id}",
                frame.id()
            )
            .into());
        }
        match frame {
            CoreFrame::ProviderBackpressureMetrics { providers, .. } => Ok(providers),
            CoreFrame::Error { message, .. } => Err(core_error(message).into()),
            other => Err(anyhow::anyhow!(
                "unexpected core RPC frame for provider metrics request: {other:?}"
            )
            .into()),
        }
    }

    async fn capabilities(&self, model_spec: &str) -> Result<ModelCapabilities, InferenceError> {
        let id = self.next_id();
        let stream = self
            .connect_and_send(&CoreRequest::capabilities(id, model_spec))
            .await?;
        let mut reader = BufReader::new(stream);
        let frame = read_json_line::<_, CoreFrame>(&mut reader, self.max_frame_bytes)
            .await
            .map_err(InferenceError::from)?
            .ok_or_else(|| {
                InferenceError::from(anyhow::anyhow!(
                    "core RPC closed before capabilities response"
                ))
            })?;
        if frame.id() != id {
            return Err(anyhow::anyhow!(
                "core RPC response id {} does not match request id {id}",
                frame.id()
            )
            .into());
        }
        match frame {
            CoreFrame::Capabilities { capabilities, .. } => Ok(capabilities),
            CoreFrame::Error { message, .. } => Err(core_error(message).into()),
            other => Err(anyhow::anyhow!(
                "unexpected core RPC frame for capabilities request: {other:?}"
            )
            .into()),
        }
    }

    async fn complete(&self, request: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
        let id = self.next_id();
        let stream = self
            .connect_and_send(&CoreRequest::complete(id, request))
            .await?;
        let mut reader = BufReader::new(stream);
        let frame = read_json_line::<_, CoreFrame>(&mut reader, self.max_frame_bytes)
            .await
            .map_err(InferenceError::from)?
            .ok_or_else(|| {
                InferenceError::from(anyhow::anyhow!("core RPC closed before response"))
            })?;
        if frame.id() != id {
            return Err(anyhow::anyhow!(
                "core RPC response id {} does not match request id {id}",
                frame.id()
            )
            .into());
        }
        match frame {
            CoreFrame::Response { response, .. } => Ok(response),
            CoreFrame::Error { message, .. } => Err(core_error(message).into()),
            other => Err(anyhow::anyhow!(
                "unexpected core RPC frame for complete request: {other:?}"
            )
            .into()),
        }
    }

    async fn stream(&self, request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        let id = self.next_id();
        let stream = self
            .connect_and_send(&CoreRequest::stream(id, request))
            .await?;
        let max_frame_bytes = self.max_frame_bytes;
        let mut reader = BufReader::new(stream);
        let first = read_json_line::<_, CoreFrame>(&mut reader, max_frame_bytes)
            .await
            .map_err(InferenceError::from)?
            .ok_or_else(|| {
                InferenceError::from(anyhow::anyhow!(
                    "core RPC closed before stream establishment"
                ))
            })?;
        if first.id() != id {
            return Err(anyhow::anyhow!(
                "core RPC response id {} does not match request id {id}",
                first.id()
            )
            .into());
        }
        let first_chunk = match first {
            CoreFrame::Chunk { chunk, .. } => Some(chunk),
            CoreFrame::Completed { .. } => None,
            CoreFrame::Error { message, .. } => return Err(core_error(message).into()),
            other => {
                return Err(anyhow::anyhow!("unexpected core RPC stream frame: {other:?}").into())
            }
        };
        let (sender, receiver) = mpsc::channel(64);
        if let Some(chunk) = first_chunk {
            sender.send(Ok(chunk)).await.map_err(|_| {
                InferenceError::from(anyhow::anyhow!("core RPC stream receiver closed"))
            })?;
        } else {
            return Ok(Box::pin(stream::empty()));
        }
        tokio::spawn(async move {
            loop {
                let frame = match read_json_line::<_, CoreFrame>(&mut reader, max_frame_bytes).await
                {
                    Ok(Some(frame)) => frame,
                    Ok(None) => {
                        let _ = sender
                            .send(Err(anyhow::anyhow!(
                                "core RPC closed before stream completion"
                            )))
                            .await;
                        return;
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                };
                if frame.id() != id {
                    let _ = sender
                        .send(Err(anyhow::anyhow!(
                            "core RPC response id {} does not match request id {id}",
                            frame.id()
                        )))
                        .await;
                    return;
                }
                match frame {
                    CoreFrame::Chunk { chunk, .. } => {
                        if sender.send(Ok(chunk)).await.is_err() {
                            return;
                        }
                    }
                    CoreFrame::Completed { .. } => return,
                    CoreFrame::Error { message, .. } => {
                        let _ = sender.send(Err(core_error(message))).await;
                        return;
                    }
                    other => {
                        let _ = sender
                            .send(Err(anyhow::anyhow!(
                                "unexpected core RPC stream frame: {other:?}"
                            )))
                            .await;
                        return;
                    }
                }
            }
        });
        Ok(Box::pin(stream::unfold(receiver, |mut receiver| async {
            receiver.recv().await.map(|item| (item, receiver))
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    fn socket_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "aletheon-core-readiness-{label}-{}-{}.sock",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[tokio::test]
    async fn readiness_wait_recovers_when_socket_appears_within_deadline() {
        let socket = socket_path("delayed");
        let delayed = socket.clone();
        let server = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let listener = UnixListener::bind(&delayed).unwrap();
            let _ = listener.accept().await;
        });

        CoreRpcClient::new(socket.clone())
            .wait_until_ready(Duration::from_millis(500))
            .await
            .unwrap();
        server.await.unwrap();
        let _ = std::fs::remove_file(socket);
    }

    #[tokio::test]
    async fn readiness_timeout_names_machine_core_socket() {
        let socket = socket_path("missing");
        let error = CoreRpcClient::new(socket.clone())
            .wait_until_ready(Duration::from_millis(20))
            .await
            .unwrap_err();

        let diagnostic = error.to_string();
        assert!(matches!(error, CoreRpcReadinessError::Timeout { .. }));
        assert!(diagnostic.contains("machine core readiness timed out"));
        assert!(diagnostic.contains(&socket.display().to_string()));
    }

    #[test]
    fn readiness_retries_only_absent_or_refused_socket() {
        assert!(is_core_startup_race(&std::io::Error::from(
            std::io::ErrorKind::NotFound
        )));
        assert!(is_core_startup_race(&std::io::Error::from(
            std::io::ErrorKind::ConnectionRefused
        )));
        assert!(!is_core_startup_race(&std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        )));
    }
}
