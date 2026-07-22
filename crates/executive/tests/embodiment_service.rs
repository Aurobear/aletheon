use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use executive::service::embodiment_authority::EmbodimentAuthorityPort;
use executive::service::embodiment_progress::RecordingEmbodimentProgress;
use executive::service::embodiment_service::EmbodimentService;
use fabric::{
    DeviceId, EmbodimentExecutionPort, OperationId, SkillDispatchError, SkillId, SkillOutcome,
    SkillRequest,
};
use hardware::{
    AuthorizedSkillRequest, Broker, ControlLease, ControlPermit, ManualClock, MonotonicInstant,
    OperationId as HardwareOperationId, PrincipalId, ProviderRegistry, SimulatedEmbodiment,
};

struct Authority {
    settlements: AtomicUsize,
}

#[async_trait]
impl EmbodimentAuthorityPort for Authority {
    async fn authorize(
        &self,
        operation_id: OperationId,
        request: &SkillRequest,
    ) -> Result<AuthorizedSkillRequest, SkillDispatchError> {
        let operation = HardwareOperationId(operation_id.0.to_string());
        let principal = PrincipalId("operator".into());
        let scope = std::collections::BTreeSet::from([request.skill.0.clone()]);
        Ok(AuthorizedSkillRequest {
            request: request.clone(),
            permit: ControlPermit {
                permit_id: "permit".into(),
                operation: operation.clone(),
                principal: principal.clone(),
                device: request.device.clone(),
                scope: scope.clone(),
                expires_at: MonotonicInstant(1_000),
                revoked: false,
            },
            lease: ControlLease {
                lease_id: "lease".into(),
                operation,
                device: request.device.clone(),
                holder: principal,
                scope,
                expires_at: MonotonicInstant(1_000),
                exclusive: true,
            },
        })
    }

    async fn settle(
        &self,
        _operation_id: OperationId,
        _succeeded: bool,
    ) -> Result<(), SkillDispatchError> {
        self.settlements.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn service_generates_identity_dispatches_and_settles_once() {
    let clock = Arc::new(ManualClock::new(0));
    let mut registry = ProviderRegistry::new();
    registry.register(
        DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot("bot", clock.clone())),
    );
    let broker = Arc::new(Broker::new(Arc::new(registry), clock));
    let authority = Arc::new(Authority {
        settlements: AtomicUsize::new(0),
    });
    let progress = Arc::new(RecordingEmbodimentProgress::default());
    let service = EmbodimentService::new(broker, authority.clone(), progress.clone());

    let observations = service.observe(&DeviceId("bot".into())).await.unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].schema, "pose");
    assert!(service
        .get_state(&DeviceId("bot".into()))
        .await
        .unwrap()
        .is_some());
    let skills = service.list_skills(&DeviceId("bot".into())).await.unwrap();
    assert_eq!(skills[0].skill, SkillId("navigate".into()));

    let result = service
        .execute_skill(SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({"x": 1.0, "y": 1.0}),
        })
        .await
        .unwrap();
    assert_eq!(result.outcome, SkillOutcome::Succeeded);
    assert_eq!(authority.settlements.load(Ordering::SeqCst), 1);
    let updates = progress.updates().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].operation_id, result.operation_id);
    service.safe_stop(&DeviceId("bot".into())).await.unwrap();
}
