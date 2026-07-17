use cognit::config::AgentAdmissionConfig;
use executive::service::agent_control::{
    AgentAdmissionPort, AgentAdmissionRequest, AgentStorageRequest, BoundedAgentAdmission,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentControlErrorKind, AgentId, AgentProfileId,
    AgentSpawnRequest, RuntimeId,
};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn config_accepts_nonzero_bounded_tree_budget_and_storage_limits() {
    AgentAdmissionConfig::default().validate().unwrap();
    let parsed: executive::AppConfig =
        toml::from_str(include_str!("../../../config/default.toml")).unwrap();
    parsed.agent.admission.validate().unwrap();
}

#[test]
fn config_rejects_zero_overflow_and_child_allowance_above_root() {
    let config = AgentAdmissionConfig {
        max_running_agents: 0,
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());

    let config = AgentAdmissionConfig {
        max_queued_per_root: AgentAdmissionConfig::default().max_agents_per_root + 1,
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());

    let config = AgentAdmissionConfig {
        max_child_tokens: AgentAdmissionConfig::default().root_max_tokens + 1,
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());

    let config = AgentAdmissionConfig {
        root_max_cost_micro: Some(10),
        max_child_cost_micro: Some(11),
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn config_rejects_finite_child_cost_under_unbounded_root_representation() {
    let config = AgentAdmissionConfig {
        root_max_cost_micro: None,
        max_child_cost_micro: Some(1),
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());
}

fn spawn(root: AgentId, parent: Option<AgentId>, profile: &str) -> AgentSpawnRequest {
    AgentSpawnRequest {
        root_agent_id: root,
        parent_agent_id: parent,
        parent_process_id: parent.map(|_| fabric::ProcessId::new()),
        profile_id: AgentProfileId(profile.into()),
        runtime_id: RuntimeId("native-cognit".into()),
        task: "bounded work".into(),
        context: AgentContextFork::None,
        broadcast_refs: vec![],
        allowed_tools: vec![],
        budget: AgentBudget {
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_elapsed_ms: 1_000,
            max_cost_usd: None,
            max_depth: 4,
        },
    }
}

async fn wait_for_resident(admission: &BoundedAgentAdmission, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if admission.metrics().resident_agents == expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("admission metrics did not reach expected resident count");
}

#[tokio::test]
async fn reservation_is_atomic_and_fifo_under_concurrent_sibling_contention() {
    let config = AgentAdmissionConfig {
        max_agents_per_root: 3,
        max_running_agents: 1,
        max_queued_per_root: 3,
        ..AgentAdmissionConfig::default()
    };
    let admission = Arc::new(
        BoundedAgentAdmission::with_budget(
            config,
            Arc::new(aletheon_kernel::admission::InMemoryBudgetController::new()),
        )
        .unwrap(),
    );
    let root = AgentId::new();
    let first_request = spawn(root, Some(root), "worker");
    let mut first = admission
        .reserve(AgentAdmissionRequest::new(
            &first_request,
            1,
            None,
            AgentStorageRequest::default(),
        ))
        .await
        .unwrap();

    let second_admission = admission.clone();
    let mut second = tokio::spawn(async move {
        let request = spawn(root, Some(root), "worker");
        second_admission
            .reserve(AgentAdmissionRequest::new(
                &request,
                1,
                None,
                AgentStorageRequest::default(),
            ))
            .await
    });
    wait_for_resident(&admission, 2).await;

    let third_admission = admission.clone();
    let mut third = tokio::spawn(async move {
        let request = spawn(root, Some(root), "worker");
        third_admission
            .reserve(AgentAdmissionRequest::new(
                &request,
                1,
                None,
                AgentStorageRequest::default(),
            ))
            .await
    });
    wait_for_resident(&admission, 3).await;
    assert_eq!(admission.available_permits(), 0);
    assert!(!second.is_finished());
    assert!(!third.is_finished());

    first.revoke().await.unwrap();
    let mut second_lease = tokio::time::timeout(Duration::from_secs(1), &mut second)
        .await
        .expect("second sibling was not admitted after release")
        .unwrap()
        .unwrap();
    assert!(!third.is_finished(), "third sibling bypassed FIFO order");

    second_lease.revoke().await.unwrap();
    let mut third_lease = tokio::time::timeout(Duration::from_secs(1), &mut third)
        .await
        .expect("third sibling was not admitted after release")
        .unwrap()
        .unwrap();
    third_lease.revoke().await.unwrap();
    assert_eq!(admission.metrics().resident_agents, 0);
}

#[tokio::test]
async fn cancelled_waiter_releases_topology_and_does_not_block_fifo() {
    // Constrain execution separately while retaining room for queued siblings.
    let config = AgentAdmissionConfig {
        max_agents_per_root: 3,
        max_running_agents: 1,
        max_queued_per_root: 3,
        ..AgentAdmissionConfig::default()
    };
    let admission = Arc::new(
        BoundedAgentAdmission::with_budget(
            config,
            Arc::new(aletheon_kernel::admission::InMemoryBudgetController::new()),
        )
        .unwrap(),
    );
    let root = AgentId::new();
    let first_request = spawn(root, Some(root), "worker");
    let mut first = admission
        .reserve(AgentAdmissionRequest::new(
            &first_request,
            1,
            None,
            AgentStorageRequest::default(),
        ))
        .await
        .unwrap();

    let cancelled_admission = admission.clone();
    let cancelled = tokio::spawn(async move {
        let request = spawn(root, Some(root), "worker");
        cancelled_admission
            .reserve(AgentAdmissionRequest::new(
                &request,
                1,
                None,
                AgentStorageRequest::default(),
            ))
            .await
    });
    wait_for_resident(&admission, 2).await;
    cancelled.abort();
    assert!(matches!(cancelled.await, Err(error) if error.is_cancelled()));
    wait_for_resident(&admission, 1).await;

    let next_admission = admission.clone();
    let next = tokio::spawn(async move {
        let request = spawn(root, Some(root), "worker");
        next_admission
            .reserve(AgentAdmissionRequest::new(
                &request,
                1,
                None,
                AgentStorageRequest::default(),
            ))
            .await
    });
    wait_for_resident(&admission, 2).await;
    first.revoke().await.unwrap();
    let mut next_lease = tokio::time::timeout(Duration::from_secs(1), next)
        .await
        .expect("successor remained blocked behind cancelled waiter")
        .unwrap()
        .unwrap();
    next_lease.revoke().await.unwrap();
    assert_eq!(admission.metrics().resident_agents, 0);
}

#[tokio::test]
async fn timed_out_waiter_releases_every_reserved_counter() {
    let config = AgentAdmissionConfig {
        max_agents_per_root: 2,
        max_running_agents: 1,
        max_queued_per_root: 2,
        ..AgentAdmissionConfig::default()
    };
    let admission = BoundedAgentAdmission::with_budget(
        config,
        Arc::new(aletheon_kernel::admission::InMemoryBudgetController::new()),
    )
    .unwrap();
    let root = AgentId::new();
    let first_request = spawn(root, Some(root), "worker");
    let mut first = admission
        .reserve(AgentAdmissionRequest::new(
            &first_request,
            1,
            None,
            AgentStorageRequest::default(),
        ))
        .await
        .unwrap();
    let mut timed_request = spawn(root, Some(root), "worker");
    timed_request.budget.max_elapsed_ms = 20;
    let error = admission
        .reserve(AgentAdmissionRequest::new(
            &timed_request,
            1,
            None,
            AgentStorageRequest {
                bytes: 64,
                items: 1,
            },
        ))
        .await
        .err()
        .expect("waiter should time out while the execution slot is occupied");
    assert_eq!(error.kind, AgentControlErrorKind::Timeout);
    let metrics = admission.metrics();
    assert_eq!(metrics.resident_agents, 1);
    assert_eq!(metrics.queued_agents, 1);
    assert_eq!(metrics.reserved_storage_bytes, 0);
    assert_eq!(metrics.reserved_storage_items, 0);
    first.revoke().await.unwrap();
}

#[tokio::test]
async fn shared_root_rollout_releases_capacity_for_a_later_sibling() {
    let config = AgentAdmissionConfig {
        max_agents_per_root: 2,
        max_running_agents: 2,
        max_queued_per_root: 2,
        root_max_tokens: 300,
        max_child_tokens: 200,
        ..AgentAdmissionConfig::default()
    };
    let admission = BoundedAgentAdmission::with_budget(
        config,
        Arc::new(aletheon_kernel::admission::InMemoryBudgetController::new()),
    )
    .unwrap();
    let root = AgentId::new();
    let first_request = spawn(root, Some(root), "worker");
    let second_request = spawn(root, Some(root), "worker");
    let context = |spawn| AgentAdmissionRequest {
        spawn,
        depth: 1,
        parent_profile: None,
        storage: AgentStorageRequest::default(),
    };
    let mut first = admission.reserve(context(&first_request)).await.unwrap();
    assert!(admission.reserve(context(&second_request)).await.is_err());
    first.revoke().await.unwrap();
    let second = admission.reserve(context(&second_request)).await;
    assert!(second.is_ok());
}

#[tokio::test]
async fn lease_transitions_settle_once_and_expose_content_free_metrics() {
    let admission = BoundedAgentAdmission::new(2).unwrap();
    let request = spawn(AgentId::new(), None, "worker");
    let mut lease = admission
        .reserve(AgentAdmissionRequest {
            spawn: &request,
            depth: 0,
            parent_profile: None,
            storage: AgentStorageRequest {
                bytes: 4096,
                items: 1,
            },
        })
        .await
        .unwrap();
    assert_eq!(admission.metrics().queued_agents, 1);
    lease.mark_running().await.unwrap();
    let running = admission.metrics();
    assert_eq!(running.running_agents, 1);
    assert_eq!(running.reserved_storage_bytes, 4096);
    lease
        .settle(&fabric::AttemptUsage {
            input_tokens: 10,
            output_tokens: 5,
            cost_usd: None,
            elapsed_ms: 20,
        })
        .await
        .unwrap();
    assert_eq!(admission.metrics().resident_agents, 0);
    assert!(lease
        .settle(&fabric::AttemptUsage::default())
        .await
        .is_err());
}

#[tokio::test]
async fn policy_rejects_depth_internal_delegation_and_storage_before_resources() {
    let config = AgentAdmissionConfig {
        max_storage_bytes: 1024,
        max_storage_items: 1,
        max_depth: 2,
        ..AgentAdmissionConfig::default()
    };
    let admission = BoundedAgentAdmission::with_budget(
        config,
        Arc::new(aletheon_kernel::admission::InMemoryBudgetController::new()),
    )
    .unwrap();
    let root = AgentId::new();
    let request = spawn(root, Some(root), "worker");
    assert!(admission
        .reserve(AgentAdmissionRequest {
            spawn: &request,
            depth: 3,
            parent_profile: None,
            storage: AgentStorageRequest::default(),
        })
        .await
        .is_err());
    let memory = AgentProfileId("mnemosyne-consolidator".into());
    assert!(admission
        .reserve(AgentAdmissionRequest {
            spawn: &request,
            depth: 1,
            parent_profile: Some(&memory),
            storage: AgentStorageRequest::default(),
        })
        .await
        .is_err());
    assert!(admission
        .reserve(AgentAdmissionRequest {
            spawn: &request,
            depth: 1,
            parent_profile: None,
            storage: AgentStorageRequest {
                bytes: 1025,
                items: 1
            },
        })
        .await
        .is_err());
    assert_eq!(admission.metrics().resident_agents, 0);
}

#[test]
fn service_reserves_policy_before_creating_any_child_resource() {
    let source = include_str!("../src/service/agent_control/mod.rs");
    let reserve = source.find(".reserve(AgentAdmissionRequest").unwrap();
    let process = source.find(".spawn_process(SpawnSpec").unwrap();
    let operation = source.find(".submit_operation(OperationRequest").unwrap();
    let mailbox = source.find(".register_process_mailbox(").unwrap();
    assert!(reserve < process && process < operation && operation < mailbox);
}
