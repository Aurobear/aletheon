//! Phase 5 budget/quota/lease admission tests.

use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionController, AdmissionError, AdmissionRequest, BudgetRequest, CapabilityId,
    CapabilityScope, LeaseRequest, PrincipalId, RevokeReason, SandboxRequirement, UsageReport,
};
use kernel::admission::{
    InMemoryBudgetController, InMemoryResourceLeaseManager, ProductionAdmissionController,
};
use kernel::chronos::TestClock;
use std::sync::Arc;

fn request(principal: &str) -> AdmissionRequest {
    AdmissionRequest {
        operation_id: fabric::OperationId::new(),
        process_id: fabric::ProcessId::new(),
        principal: PrincipalId(principal.into()),
        capability: CapabilityId("test.tool".into()),
        action: "execute".into(),
        input_summary: "test".into(),
        risk: RiskLevel::ReadOnly,
        requested_scope: CapabilityScope::default(),
        budget: None,
        lease: None,
        sandbox: SandboxRequirement::NotRequired,
    }
}

#[tokio::test]
async fn budget_quota_lease_settle_releases_unused_budget_and_lease() {
    let clock = Arc::new(TestClock::new(0, 0));
    let budget = Arc::new(InMemoryBudgetController::new());
    budget.set_budget("agent-a", Some(100), None).await;
    let leases = Arc::new(InMemoryResourceLeaseManager::new());
    let admission = ProductionAdmissionController::new(clock)
        .with_budget(budget.clone())
        .with_leases(leases.clone());

    let permit = admission
        .admit(AdmissionRequest {
            budget: Some(BudgetRequest {
                max_tokens: Some(80),
                max_cost_micro: None,
            }),
            lease: Some(LeaseRequest {
                resource: "sandbox-1".into(),
                duration_ms: 30_000,
            }),
            ..request("agent-a")
        })
        .await
        .unwrap();

    assert_eq!(budget.remaining_tokens("agent-a").await, Some(Some(20)));
    assert!(leases.is_leased("sandbox-1", 0).await);

    admission
        .settle(
            permit.id,
            UsageReport {
                permit_id: permit.id,
                tokens_used: 30,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(budget.remaining_tokens("agent-a").await, Some(Some(70)));
    assert!(!leases.is_leased("sandbox-1", 0).await);
}

#[tokio::test]
async fn budget_quota_lease_lease_failure_rolls_back_budget_reservation() {
    let clock = Arc::new(TestClock::new(0, 0));
    let budget = Arc::new(InMemoryBudgetController::new());
    budget.set_budget("agent-a", Some(100), None).await;
    budget.set_budget("agent-b", Some(100), None).await;
    let leases = Arc::new(InMemoryResourceLeaseManager::new());
    let admission = ProductionAdmissionController::new(clock)
        .with_budget(budget.clone())
        .with_leases(leases.clone());

    let first = admission
        .admit(AdmissionRequest {
            lease: Some(LeaseRequest {
                resource: "gpu-0".into(),
                duration_ms: 30_000,
            }),
            ..request("agent-a")
        })
        .await
        .unwrap();
    assert!(first.lease.is_some());

    let err = admission
        .admit(AdmissionRequest {
            budget: Some(BudgetRequest {
                max_tokens: Some(40),
                max_cost_micro: None,
            }),
            lease: Some(LeaseRequest {
                resource: "gpu-0".into(),
                duration_ms: 30_000,
            }),
            ..request("agent-b")
        })
        .await
        .unwrap_err();

    assert!(matches!(err, AdmissionError::LeaseUnavailable));
    assert_eq!(budget.remaining_tokens("agent-b").await, Some(Some(100)));
}

#[tokio::test]
async fn budget_quota_lease_operation_revoke_releases_active_resources() {
    let clock = Arc::new(TestClock::new(0, 0));
    let budget = Arc::new(InMemoryBudgetController::new());
    budget.set_budget("agent-a", Some(100), None).await;
    let leases = Arc::new(InMemoryResourceLeaseManager::new());
    let admission = ProductionAdmissionController::new(clock)
        .with_budget(budget.clone())
        .with_leases(leases.clone());

    let permit = admission
        .admit(AdmissionRequest {
            budget: Some(BudgetRequest {
                max_tokens: Some(60),
                max_cost_micro: None,
            }),
            lease: Some(LeaseRequest {
                resource: "db-0".into(),
                duration_ms: 30_000,
            }),
            ..request("agent-a")
        })
        .await
        .unwrap();

    admission
        .revoke(permit.id, RevokeReason::OperationCancelled)
        .await
        .unwrap();

    assert_eq!(budget.remaining_tokens("agent-a").await, Some(Some(100)));
    assert!(!leases.is_leased("db-0", 0).await);
    assert!(matches!(
        admission.settle(permit.id, UsageReport::default()).await,
        Err(AdmissionError::AlreadySettled)
    ));
}

#[tokio::test]
async fn budget_quota_lease_concurrent_budget_reservations_do_not_overspend() {
    let clock = Arc::new(TestClock::new(0, 0));
    let budget = Arc::new(InMemoryBudgetController::new());
    budget.set_budget("agent-a", Some(100), None).await;
    let admission = Arc::new(ProductionAdmissionController::new(clock).with_budget(budget.clone()));

    let mut tasks = Vec::new();
    for _ in 0..10 {
        let admission = admission.clone();
        tasks.push(tokio::spawn(async move {
            admission
                .admit(AdmissionRequest {
                    budget: Some(BudgetRequest {
                        max_tokens: Some(30),
                        max_cost_micro: None,
                    }),
                    ..request("agent-a")
                })
                .await
                .is_ok()
        }));
    }

    let mut granted = 0;
    for task in tasks {
        if task.await.unwrap() {
            granted += 1;
        }
    }

    assert_eq!(granted, 3);
    assert_eq!(budget.remaining_tokens("agent-a").await, Some(Some(10)));
}
