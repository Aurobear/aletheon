use std::collections::BTreeSet;
use std::sync::Arc;

use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionRequest, CapabilityId, CapabilityScope, LeaseRequest,
    PrincipalId as KernelPrincipalId, SandboxRequirement,
};
use hardware::{
    CommandDecision, CommandSequence, ControlLease, ControlPermit, DeviceId, ManualClock,
    MonotonicInstant, OperationId, PrincipalId, RejectionReason, SimulatedDevice, TypedCommand,
};
use kernel::chronos::TestClock;

fn project_permit(
    request: &AdmissionRequest,
    permit: &fabric::ExecutionPermit,
    device: &DeviceId,
) -> Result<ControlPermit, String> {
    if permit.operation_id != request.operation_id
        || permit.capability.0 != "hardware.command"
        || !permit
            .granted_scope
            .allowed_targets
            .iter()
            .any(|target| target == &format!("device:{}", device.0))
    {
        return Err("Kernel permit scope does not authorize this hardware operation".into());
    }
    Ok(ControlPermit {
        permit_id: permit.id.0.to_string(),
        operation: OperationId(permit.operation_id.0.to_string()),
        principal: PrincipalId(request.principal.0.clone()),
        device: device.clone(),
        scope: permit.granted_scope.allowed_paths.iter().cloned().collect(),
        expires_at: MonotonicInstant(permit.expires_at.0 .0),
        revoked: false,
    })
}

fn verifies_operation(expected: &OperationId, receipt: &hardware::CommandReceipt) -> bool {
    receipt.operation == *expected && receipt.accepted()
}

#[tokio::test]
async fn kernel_permit_lease_navigate_stop_and_receipt_settlement_are_correlated() {
    let kernel_clock = Arc::new(TestClock::new(0, 100));
    let kernel = kernel::KernelRuntime::with_clock(kernel_clock.clone());
    let request = AdmissionRequest {
        operation_id: fabric::OperationId::new(),
        process_id: fabric::ProcessId::new(),
        principal: KernelPrincipalId("operator".into()),
        capability: CapabilityId("hardware.command".into()),
        action: "navigate_then_stop".into(),
        input_summary: "simulated mobile robot".into(),
        risk: RiskLevel::SystemModify,
        requested_scope: CapabilityScope {
            allowed_paths: vec!["navigate".into(), "stop".into()],
            allowed_targets: vec!["device:bot".into()],
            max_runtime_ms: Some(5_000),
            max_output_bytes: Some(16 * 1024),
        },
        budget: None,
        lease: Some(LeaseRequest {
            resource: "hardware:bot".into(),
            duration_ms: 5_000,
        }),
        sandbox: SandboxRequirement::NotRequired,
    };
    let execution_permit = kernel.admission().admit(request.clone()).await.unwrap();
    let device = DeviceId("bot".into());
    let permit = project_permit(&request, &execution_permit, &device).unwrap();
    let operation = permit.operation.clone();
    let principal = permit.principal.clone();
    let hardware_clock = Arc::new(ManualClock::new(100));
    let mut robot = SimulatedDevice::mobile_robot("bot", hardware_clock.clone());
    robot
        .grant_lease(ControlLease {
            lease_id: execution_permit.lease.unwrap().0.to_string(),
            operation: operation.clone(),
            device: device.clone(),
            holder: principal.clone(),
            scope: BTreeSet::from(["navigate".into()]),
            expires_at: MonotonicInstant(5_100),
            exclusive: true,
        })
        .unwrap();
    let navigate = TypedCommand {
        command_id: "navigate-1".into(),
        operation: operation.clone(),
        principal: principal.clone(),
        sequence: CommandSequence(1),
        device: device.clone(),
        schema: "navigate".into(),
        payload: serde_json::json!({"x":2.0,"y":3.0}),
        deadline: MonotonicInstant(1_000),
    };
    let navigate_receipt = robot.execute(&navigate, Some(&permit));
    assert!(verifies_operation(&operation, &navigate_receipt));
    let stop = TypedCommand {
        command_id: "stop-2".into(),
        sequence: CommandSequence(2),
        schema: "stop".into(),
        payload: serde_json::json!({}),
        ..navigate
    };
    let stop_receipt = robot.execute(&stop, Some(&permit));
    assert!(matches!(
        stop_receipt.decision,
        CommandDecision::FailSafeApplied
    ));
    assert!(verifies_operation(&operation, &stop_receipt));
    assert!(!verifies_operation(
        &OperationId("other".into()),
        &stop_receipt
    ));

    let mut revoked = permit.clone();
    kernel
        .admission()
        .revoke(
            execution_permit.id,
            fabric::RevokeReason::OperationCancelled,
        )
        .await
        .unwrap();
    revoked.revoked = true;
    let rejected = robot.execute(
        &TypedCommand {
            command_id: "after-revoke".into(),
            sequence: CommandSequence(3),
            ..stop
        },
        Some(&revoked),
    );
    assert_eq!(
        rejected.decision,
        CommandDecision::Rejected(RejectionReason::RevokedPermit)
    );
}

#[tokio::test]
async fn missing_mismatched_and_expired_authority_fail_closed() {
    let clock = Arc::new(ManualClock::new(10));
    let mut robot = SimulatedDevice::mobile_robot("bot", clock.clone());
    let command = TypedCommand {
        command_id: "c".into(),
        operation: OperationId("op".into()),
        principal: PrincipalId("p".into()),
        sequence: CommandSequence(1),
        device: DeviceId("bot".into()),
        schema: "navigate".into(),
        payload: serde_json::json!({"x":1.0,"y":1.0}),
        deadline: MonotonicInstant(20),
    };
    assert_eq!(
        robot.execute(&command, None).decision,
        CommandDecision::Rejected(RejectionReason::MissingPermit)
    );
    let permit = ControlPermit {
        permit_id: "p".into(),
        operation: OperationId("other".into()),
        principal: command.principal.clone(),
        device: command.device.clone(),
        scope: BTreeSet::from(["navigate".into()]),
        expires_at: MonotonicInstant(20),
        revoked: false,
    };
    assert_eq!(
        robot.execute(&command, Some(&permit)).decision,
        CommandDecision::Rejected(RejectionReason::PermitOperationMismatch)
    );
    clock.advance_to(20).unwrap();
    let mut expired = permit;
    expired.operation = command.operation.clone();
    assert_eq!(
        robot.execute(&command, Some(&expired)).decision,
        CommandDecision::Rejected(RejectionReason::ExpiredPermit)
    );
}
