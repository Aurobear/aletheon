use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use ::contracts::types::embodiment::{
    skill_request_digest, DeviceId, EmbodiedObservation, EmbodimentExecutionPort, RiskClass,
    SkillDescriptor, SkillId, SkillOutcome, SkillRequest, SkillResult,
};
use aletheon::wiring::embodiment::build_embodiment_invoker;
use aletheon::wiring::embodiment::EmbodimentService;
use async_trait::async_trait;
use hardware::approval::{HighRiskSkillApprovalPort, HighRiskSkillApprovalReceipt};
use hardware::progress_projection::RecordingEmbodimentProgress;
use hardware::{
    Broker, CancelAck, DeviceOperationId, EmbodimentProvider, ManualClock, ProviderError,
    ProviderRegistry, SimulatedEmbodiment, SkillProgressSink, StopReceipt, ValidatedSkillCommand,
};
use kernel::chronos::TestClock;

#[tokio::test]
async fn service_queries_executes_and_correlates_progress() {
    let kernel = Arc::new(kernel::KernelRuntime::with_clock(Arc::new(TestClock::new(
        0, 0,
    ))));
    let clock = Arc::new(ManualClock::new(0));
    let mut registry = ProviderRegistry::new();
    registry.register(
        DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot("bot", clock.clone())),
    );
    let broker = Arc::new(Broker::new(Arc::new(registry), clock));
    let progress = Arc::new(RecordingEmbodimentProgress::default());
    let (invoker, active) =
        build_embodiment_invoker(kernel.admission(), broker.clone(), progress.clone());
    let workspace = ::contracts::WorkspacePolicy::from_resolved_roots(
        std::path::PathBuf::from("/tmp/embodiment-test"),
        vec![],
    )
    .unwrap();
    let service = EmbodimentService::new(
        broker,
        invoker,
        active,
        ::contracts::ProcessId::new(),
        ::contracts::PrincipalId("operator".into()),
        workspace,
    );

    assert_eq!(
        service
            .observe(&DeviceId("bot".into()))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(service
        .get_state(&DeviceId("bot".into()))
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        service.list_skills(&DeviceId("bot".into())).await.unwrap()[0].skill,
        SkillId("navigate".into())
    );
    let result = service
        .execute_skill(SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({"x": 1.0, "y": 1.0}),
        })
        .await
        .unwrap();
    assert_eq!(result.outcome, SkillOutcome::Succeeded);
    let updates = progress.updates().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].operation_id, result.operation_id);
    service.safe_stop(&DeviceId("bot".into())).await.unwrap();
}

struct HighRiskProvider {
    inner: SimulatedEmbodiment,
    executions: AtomicUsize,
}

#[async_trait]
impl EmbodimentProvider for HighRiskProvider {
    async fn observe(&self, device: &DeviceId) -> Result<Vec<EmbodiedObservation>, ProviderError> {
        self.inner.observe(device).await
    }

    async fn get_state(
        &self,
        device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, ProviderError> {
        self.inner.get_state(device).await
    }

    async fn list_skills(&self, device: &DeviceId) -> Result<Vec<SkillDescriptor>, ProviderError> {
        let mut descriptors = self.inner.list_skills(device).await?;
        descriptors[0].risk = RiskClass::High;
        descriptors[0].input_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "x": {"type": "number", "minimum": -1.0, "maximum": 1.0},
                "y": {"type": "number", "minimum": -1.0, "maximum": 1.0}
            },
            "required": ["x", "y"],
            "additionalProperties": false
        });
        Ok(descriptors)
    }

    async fn execute_skill(
        &self,
        command: ValidatedSkillCommand<'_>,
        progress: Arc<dyn SkillProgressSink>,
    ) -> Result<SkillResult, ProviderError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        self.inner.execute_skill(command, progress).await
    }

    async fn cancel(
        &self,
        device: &DeviceId,
        operation: &DeviceOperationId,
    ) -> Result<CancelAck, ProviderError> {
        self.inner.cancel(device, operation).await
    }

    async fn safe_stop(&self, device: &DeviceId) -> Result<StopReceipt, ProviderError> {
        self.inner.safe_stop(device).await
    }
}

struct ExactOperatorApproval;

#[async_trait]
impl HighRiskSkillApprovalPort for ExactOperatorApproval {
    async fn authorize(
        &self,
        principal: &::contracts::PrincipalId,
        request: &SkillRequest,
        _descriptor: &SkillDescriptor,
    ) -> Result<HighRiskSkillApprovalReceipt, String> {
        Ok(HighRiskSkillApprovalReceipt {
            approval_id: "operator-approval-1".into(),
            principal: principal.clone(),
            device: request.device.clone(),
            skill: request.skill.clone(),
            request_digest: skill_request_digest(request)?,
            expires_at_unix_ms: i64::MAX,
        })
    }
}

fn high_risk_service(
    approval: Option<Arc<dyn HighRiskSkillApprovalPort>>,
) -> (EmbodimentService, Arc<HighRiskProvider>) {
    let kernel = Arc::new(kernel::KernelRuntime::with_clock(Arc::new(TestClock::new(
        0, 0,
    ))));
    let clock = Arc::new(ManualClock::new(0));
    let provider = Arc::new(HighRiskProvider {
        inner: SimulatedEmbodiment::mobile_robot("bot", clock.clone()),
        executions: AtomicUsize::new(0),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(DeviceId("bot".into()), provider.clone());
    let broker = Arc::new(Broker::new(Arc::new(registry), clock));
    let (invoker, active) = build_embodiment_invoker(
        kernel.admission(),
        broker.clone(),
        Arc::new(RecordingEmbodimentProgress::default()),
    );
    let service = EmbodimentService::new(
        broker,
        invoker,
        active,
        ::contracts::ProcessId::new(),
        ::contracts::PrincipalId("operator".into()),
        ::contracts::WorkspacePolicy::from_resolved_roots(
            std::path::PathBuf::from("/tmp/high-risk-embodiment-test"),
            vec![],
        )
        .unwrap(),
    );
    let service = match approval {
        Some(approval) => service.with_high_risk_approval(approval),
        None => service,
    };
    (service, provider)
}

fn navigate_request() -> SkillRequest {
    SkillRequest {
        skill: SkillId("navigate".into()),
        device: DeviceId("bot".into()),
        parameters: serde_json::json!({"x": 1.0, "y": 1.0}),
    }
}

#[tokio::test]
async fn high_risk_skill_fails_closed_without_explicit_operator_approval() {
    let (service, provider) = high_risk_service(None);
    let error = service.execute_skill(navigate_request()).await.unwrap_err();
    assert!(error.to_string().contains("explicit operator approval"));
    assert_eq!(provider.executions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn high_risk_skill_executes_only_with_request_bound_operator_receipt() {
    let (service, provider) = high_risk_service(Some(Arc::new(ExactOperatorApproval)));
    let result = service.execute_skill(navigate_request()).await.unwrap();
    assert_eq!(result.outcome, SkillOutcome::Succeeded);
    assert_eq!(provider.executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn aletheon_validator_rejects_out_of_range_before_approval_or_provider() {
    let (service, provider) = high_risk_service(Some(Arc::new(ExactOperatorApproval)));
    let error = service
        .execute_skill(SkillRequest {
            parameters: serde_json::json!({"x": 1.001, "y": 0.0}),
            ..navigate_request()
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("maximum"), "{error}");
    assert_eq!(provider.executions.load(Ordering::SeqCst), 0);
}
