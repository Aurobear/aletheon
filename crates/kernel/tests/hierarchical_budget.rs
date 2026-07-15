use aletheon_kernel::admission::InMemoryBudgetController;
use aletheon_kernel::KernelRuntime;
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionRequest, BudgetRequest, BudgetScopeKind, CapabilityId, CapabilityScope, OperationKind,
    OperationRequest, PermitId, PrincipalId, SandboxRequirement, SpawnSpec, UsageReport,
};
use std::sync::Arc;

fn tokens(value: u64) -> BudgetRequest {
    BudgetRequest {
        max_tokens: Some(value),
        max_cost_micro: None,
    }
}

#[tokio::test]
async fn hierarchy_enforces_levels_parent_capacity_and_atomic_sibling_contention() {
    let budget = Arc::new(InMemoryBudgetController::new());
    let root = budget.create_root("rollout:r1", tokens(100)).await;
    assert!(budget
        .reserve_child(root, BudgetScopeKind::Operation, "orphan-op", tokens(1))
        .await
        .is_err());

    let left = {
        let budget = budget.clone();
        tokio::spawn(async move {
            budget
                .reserve_child(root, BudgetScopeKind::Process, "process:left", tokens(60))
                .await
        })
    };
    let right = {
        let budget = budget.clone();
        tokio::spawn(async move {
            budget
                .reserve_child(root, BudgetScopeKind::Process, "process:right", tokens(60))
                .await
        })
    };
    let outcomes = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(budget.remaining(root).await.unwrap().max_tokens, Some(40));
}

#[tokio::test]
async fn settlement_propagates_unused_capacity_once_at_each_ancestor() {
    let budget = InMemoryBudgetController::new();
    let root = budget.create_root("rollout:r1", tokens(100)).await;
    let process = budget
        .reserve_child(root, BudgetScopeKind::Process, "process:p1", tokens(80))
        .await
        .unwrap();
    let operation = budget
        .reserve_child(
            process.scope_id,
            BudgetScopeKind::Operation,
            "operation:o1",
            tokens(50),
        )
        .await
        .unwrap();
    let capability = budget
        .reserve_child(
            operation.scope_id,
            BudgetScopeKind::Capability,
            "permit:c1",
            tokens(40),
        )
        .await
        .unwrap();

    let usage = UsageReport {
        tokens_used: 10,
        ..Default::default()
    };
    budget
        .settle_reservation(capability.reservation_id, &usage)
        .await
        .unwrap();
    budget
        .settle_reservation(operation.reservation_id, &usage)
        .await
        .unwrap();
    budget
        .settle_reservation(process.reservation_id, &usage)
        .await
        .unwrap();

    assert_eq!(budget.remaining(root).await.unwrap().max_tokens, Some(90));
    assert!(budget
        .settle_reservation(process.reservation_id, &usage)
        .await
        .is_err());
}

#[tokio::test]
async fn recursive_revocation_restores_every_hold_and_is_idempotent() {
    let budget = InMemoryBudgetController::new();
    let root = budget.create_root("rollout:r1", tokens(100)).await;
    let process = budget
        .reserve_child(root, BudgetScopeKind::Process, "process:p1", tokens(80))
        .await
        .unwrap();
    let operation = budget
        .reserve_child(
            process.scope_id,
            BudgetScopeKind::Operation,
            "operation:o1",
            tokens(60),
        )
        .await
        .unwrap();
    budget
        .reserve_child(
            operation.scope_id,
            BudgetScopeKind::Capability,
            "permit:c1",
            tokens(40),
        )
        .await
        .unwrap();

    budget.revoke_scope_tree(process.scope_id).await.unwrap();
    budget.revoke_scope_tree(process.scope_id).await.unwrap();
    assert_eq!(budget.remaining(root).await.unwrap().max_tokens, Some(100));
    assert_eq!(budget.active_reservation_count().await, 0);
}

#[tokio::test]
async fn runtime_binds_each_allocation_to_process_operation_and_permit_identity() {
    let runtime = KernelRuntime::new();
    let process = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    let operation = runtime
        .submit_operation(OperationRequest {
            owner: process.id,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    let root = runtime
        .create_rollout_budget("rollout:r1", tokens(100))
        .await;
    let process_budget = runtime
        .reserve_process_budget(root, process.id, tokens(80))
        .await
        .unwrap();
    let operation_budget = runtime
        .reserve_operation_budget(process_budget.scope_id, operation.id, tokens(60))
        .await
        .unwrap();
    let permit = PermitId::new();
    let capability_budget = runtime
        .reserve_capability_budget(operation_budget.scope_id, permit, tokens(40))
        .await
        .unwrap();

    assert_eq!(
        runtime
            .budget_controller()
            .scope(capability_budget.scope_id)
            .await
            .unwrap()
            .owner,
        format!("permit:{}", permit.0)
    );
}

#[tokio::test]
async fn production_admission_nests_capability_under_the_turn_operation() {
    let runtime = KernelRuntime::new();
    let process = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    let operation = runtime
        .submit_operation(OperationRequest {
            owner: process.id,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    let before = runtime.budget_controller().active_reservation_count().await;
    let admission = runtime.admission();
    let permit = admission
        .admit(AdmissionRequest {
            operation_id: operation.id,
            process_id: process.id,
            principal: PrincipalId("user".into()),
            capability: CapabilityId("tool".into()),
            action: "read".into(),
            input_summary: String::new(),
            risk: RiskLevel::ReadOnly,
            requested_scope: CapabilityScope::default(),
            budget: Some(tokens(20)),
            lease: None,
            sandbox: SandboxRequirement::NotRequired,
        })
        .await
        .unwrap();
    assert_eq!(
        runtime.budget_controller().active_reservation_count().await,
        before + 1
    );
    admission
        .settle(
            permit.id,
            UsageReport {
                tokens_used: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        runtime.budget_controller().active_reservation_count().await,
        before
    );
}
