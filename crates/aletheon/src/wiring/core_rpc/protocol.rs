use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use ::contracts::{LlmResponse, StreamChunk};
use cognit::ports::inference::{CoreInferenceRequest, ModelCapabilities};
use std::collections::HashMap;

pub const DEFAULT_MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreRequest {
    AcquireProviderPermit {
        id: u64,
        provider_key: String,
    },
    ObserveProviderRetryAfter {
        id: u64,
        provider_key: String,
        retry_after_ms: Option<u64>,
    },
    ProviderBackpressureMetrics {
        id: u64,
    },
    Capabilities {
        id: u64,
        model_spec: String,
    },
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
    pub fn acquire_provider_permit(id: u64, provider_key: impl Into<String>) -> Self {
        Self::AcquireProviderPermit {
            id,
            provider_key: provider_key.into(),
        }
    }

    pub fn observe_provider_retry_after(
        id: u64,
        provider_key: impl Into<String>,
        retry_after_ms: Option<u64>,
    ) -> Self {
        Self::ObserveProviderRetryAfter {
            id,
            provider_key: provider_key.into(),
            retry_after_ms,
        }
    }

    pub fn provider_backpressure_metrics(id: u64) -> Self {
        Self::ProviderBackpressureMetrics { id }
    }

    pub fn capabilities(id: u64, model_spec: impl Into<String>) -> Self {
        Self::Capabilities {
            id,
            model_spec: model_spec.into(),
        }
    }

    pub fn complete(id: u64, request: CoreInferenceRequest) -> Self {
        Self::Complete { id, request }
    }

    pub fn stream(id: u64, request: CoreInferenceRequest) -> Self {
        Self::Stream { id, request }
    }

    pub fn id(&self) -> u64 {
        match self {
            Self::AcquireProviderPermit { id, .. }
            | Self::ObserveProviderRetryAfter { id, .. }
            | Self::ProviderBackpressureMetrics { id }
            | Self::Capabilities { id, .. }
            | Self::Complete { id, .. }
            | Self::Stream { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreFrame {
    ProviderPermitAcquired {
        id: u64,
    },
    ProviderCooldownObserved {
        id: u64,
    },
    ProviderBackpressureMetrics {
        id: u64,
        providers: HashMap<String, cognit::inference::ProviderBackpressureSnapshot>,
    },
    Capabilities {
        id: u64,
        capabilities: ModelCapabilities,
    },
    Response {
        id: u64,
        response: LlmResponse,
    },
    Chunk {
        id: u64,
        chunk: StreamChunk,
    },
    Completed {
        id: u64,
    },
    Error {
        id: u64,
        message: String,
    },
}

impl CoreFrame {
    pub fn id(&self) -> u64 {
        match self {
            Self::ProviderPermitAcquired { id }
            | Self::ProviderCooldownObserved { id }
            | Self::ProviderBackpressureMetrics { id, .. }
            | Self::Capabilities { id, .. }
            | Self::Response { id, .. }
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
