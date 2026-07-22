//! Fail-closed authority validation and provider routing.

use std::sync::Arc;

use fabric::{DeviceId, EmbodiedObservation, SkillDescriptor, SkillResult};

use crate::{
    AuthorizedSkillRequest, EmbodimentProvider, MonotonicClock, OperationId, ProviderError,
    ProviderRegistry, SkillProgressSink, ValidatedSkillCommand,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrokerError {
    #[error("no provider registered for device {0}")]
    NoProvider(String),
    #[error("invalid projected authority: {0}")]
    InvalidAuthority(String),
    #[error(transparent)]
    Provider(ProviderError),
}

pub struct Broker {
    registry: Arc<ProviderRegistry>,
    clock: Arc<dyn MonotonicClock>,
}

impl Broker {
    pub fn new(registry: Arc<ProviderRegistry>, clock: Arc<dyn MonotonicClock>) -> Self {
        Self { registry, clock }
    }

    pub async fn execute(
        &self,
        authorized: AuthorizedSkillRequest,
        progress: Arc<dyn SkillProgressSink>,
    ) -> Result<SkillResult, BrokerError> {
        validate_projected_authority(&authorized, self.clock.now())?;
        let device = authorized.request.device.clone();
        let provider = self.provider(&device)?;
        provider
            .execute_skill(ValidatedSkillCommand(&authorized), progress)
            .await
            .map_err(BrokerError::Provider)
    }

    pub async fn list_skills(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<SkillDescriptor>, BrokerError> {
        self.provider(device)?
            .list_skills(device)
            .await
            .map_err(BrokerError::Provider)
    }

    pub async fn observe(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<EmbodiedObservation>, BrokerError> {
        self.provider(device)?
            .observe(device)
            .await
            .map_err(BrokerError::Provider)
    }

    pub async fn get_state(
        &self,
        device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, BrokerError> {
        self.provider(device)?
            .get_state(device)
            .await
            .map_err(BrokerError::Provider)
    }

    pub async fn cancel(
        &self,
        device: &DeviceId,
        operation: &OperationId,
    ) -> Result<(), BrokerError> {
        self.provider(device)?
            .cancel(device, operation)
            .await
            .map(|_| ())
            .map_err(BrokerError::Provider)
    }

    pub async fn safe_stop(&self, device: &DeviceId) -> Result<(), BrokerError> {
        self.provider(device)?
            .safe_stop(device)
            .await
            .map(|_| ())
            .map_err(BrokerError::Provider)
    }

    fn provider(&self, device: &DeviceId) -> Result<Arc<dyn EmbodimentProvider>, BrokerError> {
        self.registry
            .provider(device)
            .ok_or_else(|| BrokerError::NoProvider(device.0.clone()))
    }
}

fn validate_projected_authority(
    authorized: &AuthorizedSkillRequest,
    now: crate::MonotonicInstant,
) -> Result<(), BrokerError> {
    let request = &authorized.request;
    let permit = &authorized.permit;
    let lease = &authorized.lease;
    let reject = |reason: &str| Err(BrokerError::InvalidAuthority(reason.into()));

    if permit.revoked {
        return reject("permit revoked");
    }
    if now >= permit.expires_at {
        return reject("permit expired");
    }
    if now >= lease.expires_at {
        return reject("lease expired");
    }
    if permit.device != request.device || lease.device != request.device {
        return reject("device mismatch");
    }
    if permit.operation != lease.operation {
        return reject("operation mismatch");
    }
    if permit.principal != lease.holder {
        return reject("principal mismatch");
    }
    if !permit.scope.contains(&request.skill.0) || !lease.scope.contains(&request.skill.0) {
        return reject("skill outside authority scope");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use fabric::{SkillId, SkillOutcome, SkillProgress, SkillRequest};

    use super::*;
    use crate::{skill::authorized_fixture, CancelAck, ManualClock, StopReceipt};

    struct Sink;
    #[async_trait]
    impl SkillProgressSink for Sink {
        async fn progress(&self, _update: SkillProgress) {}
    }

    struct Provider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EmbodimentProvider for Provider {
        async fn observe(
            &self,
            _device: &DeviceId,
        ) -> Result<Vec<EmbodiedObservation>, ProviderError> {
            Ok(vec![])
        }

        async fn get_state(
            &self,
            _device: &DeviceId,
        ) -> Result<Option<EmbodiedObservation>, ProviderError> {
            Ok(None)
        }

        async fn list_skills(
            &self,
            _device: &DeviceId,
        ) -> Result<Vec<SkillDescriptor>, ProviderError> {
            Ok(vec![])
        }

        async fn execute_skill(
            &self,
            command: ValidatedSkillCommand<'_>,
            _progress: Arc<dyn SkillProgressSink>,
        ) -> Result<SkillResult, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let operation_id = command.permit().operation.0.parse().unwrap();
            Ok(SkillResult {
                operation_id,
                skill: command.request().skill.clone(),
                device: command.request().device.clone(),
                outcome: SkillOutcome::Succeeded,
                duration_ms: 1,
                evidence: vec![],
            })
        }

        async fn cancel(
            &self,
            device: &DeviceId,
            _operation: &OperationId,
        ) -> Result<CancelAck, ProviderError> {
            Ok(CancelAck {
                device: device.clone(),
            })
        }

        async fn safe_stop(&self, device: &DeviceId) -> Result<StopReceipt, ProviderError> {
            Ok(StopReceipt {
                device: device.clone(),
            })
        }
    }

    fn setup() -> (Broker, Arc<AtomicUsize>, AuthorizedSkillRequest) {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ProviderRegistry::new();
        registry.register(
            DeviceId("bot".into()),
            Arc::new(Provider {
                calls: calls.clone(),
            }),
        );
        let request = SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({}),
        };
        (
            Broker::new(Arc::new(registry), Arc::new(ManualClock::new(0))),
            calls,
            authorized_fixture(request),
        )
    }

    #[tokio::test]
    async fn routes_only_valid_authority() {
        let (broker, calls, authorized) = setup();
        assert_eq!(
            broker
                .execute(authorized, Arc::new(Sink))
                .await
                .unwrap()
                .outcome,
            SkillOutcome::Succeeded
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mismatch_revocation_scope_and_expiry_never_reach_provider() {
        let mutations: Vec<Box<dyn Fn(&mut AuthorizedSkillRequest)>> = vec![
            Box::new(|a| a.permit.revoked = true),
            Box::new(|a| a.permit.device = DeviceId("other".into())),
            Box::new(|a| a.lease.operation = OperationId("other".into())),
            Box::new(|a| a.permit.principal = crate::PrincipalId("other".into())),
            Box::new(|a| a.permit.scope.clear()),
            Box::new(|a| a.permit.expires_at = crate::MonotonicInstant(0)),
            Box::new(|a| a.lease.expires_at = crate::MonotonicInstant(0)),
        ];
        for mutate in mutations {
            let (broker, calls, mut authorized) = setup();
            mutate(&mut authorized);
            assert!(matches!(
                broker.execute(authorized, Arc::new(Sink)).await,
                Err(BrokerError::InvalidAuthority(_))
            ));
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }
}
