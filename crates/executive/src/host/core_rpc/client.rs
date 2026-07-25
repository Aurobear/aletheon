use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures::stream;
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::application::inference_port::{
    CoreInferenceRequest, InferenceError, InferencePort, ModelCapabilities,
};
use fabric::{LlmResponse, LlmStream};

use super::protocol::{
    read_json_line, write_json_line, CoreFrame, CoreRequest, DEFAULT_MAX_FRAME_BYTES,
};

fn core_error(message: String) -> anyhow::Error {
    if message.ends_with("provider_unavailable") {
        cognit::inference::InferenceFailure::transient("provider_unavailable")
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

    async fn connect_and_send(&self, request: &CoreRequest) -> Result<UnixStream, InferenceError> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(anyhow::Error::from)?;
        write_json_line(&mut stream, request, self.max_frame_bytes)
            .await
            .map_err(InferenceError::from)?;
        Ok(stream)
    }
}

#[async_trait::async_trait]
impl InferencePort for CoreRpcClient {
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
            CoreFrame::Error { message, .. } => Err(anyhow::anyhow!(message).into()),
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
