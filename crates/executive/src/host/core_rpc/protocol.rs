use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::application::inference_port::CoreInferenceRequest;
use fabric::{LlmResponse, StreamChunk};

pub const DEFAULT_MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreRequest {
    Complete {
        id: u64,
        request: CoreInferenceRequest,
    },
    Stream {
        id: u64,
        request: CoreInferenceRequest,
    },
}

impl CoreRequest {
    pub fn complete(id: u64, request: CoreInferenceRequest) -> Self {
        Self::Complete { id, request }
    }

    pub fn stream(id: u64, request: CoreInferenceRequest) -> Self {
        Self::Stream { id, request }
    }

    pub fn id(&self) -> u64 {
        match self {
            Self::Complete { id, .. } | Self::Stream { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreFrame {
    Response { id: u64, response: LlmResponse },
    Chunk { id: u64, chunk: StreamChunk },
    Completed { id: u64 },
    Error { id: u64, message: String },
}

impl CoreFrame {
    pub fn id(&self) -> u64 {
        match self {
            Self::Response { id, .. }
            | Self::Chunk { id, .. }
            | Self::Completed { id }
            | Self::Error { id, .. } => *id,
        }
    }
}

pub(crate) async fn read_json_line<R, T>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> anyhow::Result<Option<T>>
where
    R: AsyncBufRead + Unpin,
    T: serde::de::DeserializeOwned,
{
    let mut bytes = Vec::new();
    let mut limited = reader.take((max_frame_bytes + 1) as u64);
    let read = limited.read_until(b'\n', &mut bytes).await?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if bytes.len() > max_frame_bytes {
        anyhow::bail!("frame exceeds {max_frame_bytes} bytes");
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(anyhow::Error::from)
}

pub(crate) async fn write_json_line<W, T>(
    writer: &mut W,
    value: &T,
    max_frame_bytes: usize,
) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > max_frame_bytes {
        anyhow::bail!("frame exceeds {max_frame_bytes} bytes");
    }
    writer.write_all(&bytes).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}
