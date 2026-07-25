//! gRPC embodiment provider over the vendor-neutral gateway contract.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use fabric::types::embodiment::{DeviceId, EmbodiedObservation, SkillDescriptor, SkillResult};
use tonic::transport::Channel;

use crate::grpc::convert;
use crate::grpc::error::map_error;
use crate::grpc::wire::embodiment_gateway_client::EmbodimentGatewayClient;
use crate::grpc::wire::{self, RequestMeta};
use crate::skill::SkillProgressSink;
use crate::{
    CancelAck, EmbodimentProvider, MonotonicClock, MonotonicInstant, ProviderError, StopReceipt,
    ValidatedSkillCommand,
};

/// Configuration for a gRPC embodiment provider.
#[derive(Debug, Clone)]
pub struct GrpcProviderConfig {
    /// gRPC endpoint URL (e.g. "http://127.0.0.1:50051").
    pub endpoint: String,
    /// Protocol version sent in every request header.
    pub protocol_version: String,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Per-RPC request timeout.
    pub request_timeout: Duration,
    /// Max gRPC message size in bytes.
    pub max_decoding_message_size: usize,
}

impl Default for GrpcProviderConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:50051".into(),
            protocol_version: "1.0".into(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            max_decoding_message_size: 16 * 1024 * 1024,
        }
    }
}

/// gRPC client provider implementing the EmbodimentProvider trait.
///
/// Connects on construction and performs a capabilities handshake.
/// All RPCs carry protocol version and deadline metadata.
pub struct GrpcEmbodimentProvider {
    client: EmbodimentGatewayClient<Channel>,
    config: GrpcProviderConfig,
    clock: Option<Arc<dyn MonotonicClock>>,
}

impl GrpcEmbodimentProvider {
    /// Connect to the bridge and perform a capabilities handshake.
    pub async fn connect(config: GrpcProviderConfig) -> Result<Self, ProviderError> {
        Self::connect_inner(config, None).await
    }

    /// Connect with the monotonic clock that issues permits and leases.
    ///
    /// A shared clock is required for skill execution because domain deadlines
    /// are process-local monotonic instants while the wire protocol carries
    /// Unix timestamps.
    pub async fn connect_with_clock(
        config: GrpcProviderConfig,
        clock: Arc<dyn MonotonicClock>,
    ) -> Result<Self, ProviderError> {
        Self::connect_inner(config, Some(clock)).await
    }

    async fn connect_inner(
        config: GrpcProviderConfig,
        clock: Option<Arc<dyn MonotonicClock>>,
    ) -> Result<Self, ProviderError> {
        let endpoint = tonic::transport::Endpoint::from_shared(config.endpoint.clone())
            .map_err(|e| ProviderError::Rejected(format!("invalid endpoint: {e}")))?
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout);

        let channel = endpoint
            .connect()
            .await
            .map_err(|_e| ProviderError::Disconnected)?;

        let mut client = EmbodimentGatewayClient::new(channel)
            .max_decoding_message_size(config.max_decoding_message_size);

        // Capabilities handshake
        let caps = client
            .get_capabilities(wire::GetCapabilitiesRequest {
                meta: Some(RequestMeta {
                    protocol_version: config.protocol_version.clone(),
                    ..Default::default()
                }),
            })
            .await
            .map_err(map_status)?
            .into_inner();

        if caps.protocol_version != config.protocol_version {
            return Err(ProviderError::Rejected(format!(
                "protocol version mismatch: client={} server={}",
                config.protocol_version, caps.protocol_version
            )));
        }

        Ok(Self {
            client,
            config,
            clock,
        })
    }

    fn build_meta(&self) -> RequestMeta {
        RequestMeta {
            protocol_version: self.config.protocol_version.clone(),
            ..Default::default()
        }
    }

    fn build_deadline_meta(&self, deadline_ms: u64) -> RequestMeta {
        RequestMeta {
            protocol_version: self.config.protocol_version.clone(),
            deadline_unix_ms: deadline_ms as i64,
            ..Default::default()
        }
    }

    fn deadline_unix_ms(&self, expires_at: MonotonicInstant) -> Result<u64, ProviderError> {
        let clock = self.clock.as_ref().ok_or_else(|| {
            ProviderError::Rejected("gRPC skill execution requires a shared monotonic clock".into())
        })?;
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ProviderError::Rejected("system clock precedes Unix epoch".into()))?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        Ok(monotonic_deadline_to_unix_ms(
            expires_at.0,
            clock.now().0,
            now_unix_ms,
        ))
    }
}

fn monotonic_deadline_to_unix_ms(expires_at: u64, now_mono: u64, now_unix: u64) -> u64 {
    now_unix.saturating_add(expires_at.saturating_sub(now_mono))
}

#[async_trait]
impl EmbodimentProvider for GrpcEmbodimentProvider {
    async fn observe(&self, device: &DeviceId) -> Result<Vec<EmbodiedObservation>, ProviderError> {
        let resp = self
            .client
            .clone()
            .snapshot(wire::SnapshotRequest {
                meta: Some(self.build_meta()),
                device_id: device.0.clone(),
            })
            .await
            .map_err(map_status)?
            .into_inner();

        if let Some(error) = resp.error {
            return Err(map_error(&error));
        }

        resp.observations
            .iter()
            .map(|o| convert::to_observation(o).map_err(ProviderError::Rejected))
            .collect()
    }

    async fn get_state(
        &self,
        device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, ProviderError> {
        let mut observations = self.observe(device).await?;
        Ok(observations.pop())
    }

    async fn list_skills(&self, device: &DeviceId) -> Result<Vec<SkillDescriptor>, ProviderError> {
        let resp = self
            .client
            .clone()
            .list_skills(wire::ListSkillsRequest {
                meta: Some(self.build_meta()),
                device_id: device.0.clone(),
            })
            .await
            .map_err(map_status)?
            .into_inner();

        if let Some(error) = resp.error {
            return Err(map_error(&error));
        }

        resp.skills
            .iter()
            .map(|s| convert::to_skill_descriptor(s).map_err(ProviderError::Rejected))
            .collect()
    }

    async fn execute_skill(
        &self,
        command: ValidatedSkillCommand<'_>,
        progress: Arc<dyn SkillProgressSink>,
    ) -> Result<SkillResult, ProviderError> {
        let request = command.request();
        let permit = command.permit();
        let lease = command.lease();

        let params = crate::grpc::convert::json_to_struct(&request.parameters);

        let permit_expires_unix_ms = self.deadline_unix_ms(permit.expires_at)?;
        let lease_expires_unix_ms = self.deadline_unix_ms(lease.expires_at)?;
        let wire_request = wire::ExecuteSkillRequest {
            meta: Some(self.build_deadline_meta(permit_expires_unix_ms)),
            operation_id: permit.operation.0.clone(),
            device_id: request.device.0.clone(),
            skill_id: request.skill.0.clone(),
            parameters: Some(params),
            lease_expires_unix_ms: lease_expires_unix_ms as i64,
        };

        let mut stream = self
            .client
            .clone()
            .execute_skill(wire_request)
            .await
            .map_err(map_status)?
            .into_inner();

        let mut final_result: Option<SkillResult> = None;

        while let Some(event) = stream
            .message()
            .await
            .map_err(|_s| ProviderError::Disconnected)?
        {
            match event.event {
                Some(wire::execute_skill_event::Event::Accepted(accepted)) => {
                    // Accepted — no domain equivalent to forward, just log
                    let _ = accepted.accepted_unix_ms;
                }
                Some(wire::execute_skill_event::Event::Progress(wp)) => {
                    if let Ok(dp) = convert::to_skill_progress(&wp) {
                        progress.progress(dp).await;
                    }
                }
                Some(wire::execute_skill_event::Event::Result(wr)) => {
                    let dr = convert::to_skill_result(&wr).map_err(ProviderError::Rejected)?;
                    // Validate identity
                    if dr.device != request.device {
                        return Err(ProviderError::Rejected("result device mismatch".into()));
                    }
                    if dr.skill != request.skill {
                        return Err(ProviderError::Rejected("result skill mismatch".into()));
                    }
                    final_result = Some(dr);
                    break; // terminal
                }
                Some(wire::execute_skill_event::Event::Error(error)) => {
                    return Err(map_error(&error));
                }
                None => {
                    // Empty event — continue
                }
            }
        }

        final_result.ok_or(ProviderError::Rejected(
            "stream ended without terminal result".into(),
        ))
    }

    async fn cancel(
        &self,
        device: &DeviceId,
        operation: &crate::OperationId,
    ) -> Result<CancelAck, ProviderError> {
        let resp = self
            .client
            .clone()
            .cancel(wire::CancelRequest {
                meta: Some(self.build_meta()),
                operation_id: operation.0.clone(),
                device_id: device.0.clone(),
            })
            .await
            .map_err(map_status)?
            .into_inner();

        if let Some(error) = resp.error {
            return Err(map_error(&error));
        }

        Ok(CancelAck {
            device: device.clone(),
        })
    }

    async fn safe_stop(&self, device: &DeviceId) -> Result<StopReceipt, ProviderError> {
        let resp = self
            .client
            .clone()
            .safe_stop(wire::SafeStopRequest {
                meta: Some(self.build_meta()),
                device_id: device.0.clone(),
                reason: "requested".into(),
            })
            .await
            .map_err(map_status)?
            .into_inner();

        if let Some(error) = resp.error {
            return Err(map_error(&error));
        }

        Ok(StopReceipt {
            device: device.clone(),
        })
    }
}

/// Map a tonic::Status to a ProviderError.
fn map_status(status: tonic::Status) -> ProviderError {
    match status.code() {
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => ProviderError::Timeout,
        tonic::Code::Unimplemented => ProviderError::Rejected(status.message().into()),
        _ => ProviderError::Rejected(format!(
            "gRPC {}: {}",
            status.code().description(),
            status.message()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::monotonic_deadline_to_unix_ms;

    #[test]
    fn converts_remaining_monotonic_duration_to_unix_deadline() {
        assert_eq!(
            monotonic_deadline_to_unix_ms(12_500, 10_000, 1_700_000_000_000),
            1_700_000_002_500
        );
    }

    #[test]
    fn expired_monotonic_deadline_maps_to_current_unix_time() {
        assert_eq!(
            monotonic_deadline_to_unix_ms(9_000, 10_000, 1_700_000_000_000),
            1_700_000_000_000
        );
    }
}
