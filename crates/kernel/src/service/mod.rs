//! Service Ports — Phase 6A.
//!
//! Bundles kernel services behind clean interfaces so the daemon/exec handler
//! layers depend on service ports rather than individual kernel tables.
//!
//! This is the first step of CoreSystems contraction: new code uses
//! `ServicePorts`; old code gradually migrates its fields from `CoreSystems`
//! into `ServicePorts` (tracked as RFC-018 D5).
//!
//! # Migration state
//!
//! | Service | In ServicePorts | Still in CoreSystems |
//! |---|---|---|
//! | ProcessTable | ✅ | — |
//! | OperationTable | ✅ | — |
//! | Clock | ✅ | — |
//! | SupervisorTree | ✅ | — |
//! | MailboxService | ✅ | — |
//! | AdmissionController | ✅ | — |
//! | AgoraOps | ✅ | ✅ (transitional; prefer ports) |
//! | BudgetController | ✅ | — |
//! | ResourceLeaseManager | ✅ | — |

use std::sync::Arc;

use fabric::ipc::mailbox::InProcessMailboxService;
use fabric::{AdmissionController, AgoraOps, Clock};

use crate::admission::budget::InMemoryBudgetController;
use crate::admission::lease::InMemoryResourceLeaseManager;
use crate::admission::ProductionAdmissionController;
use crate::chronos::SystemClock;
use crate::operation::OperationTable;
use crate::process::ProcessTable;
use crate::space::InMemorySpaceManager;
use crate::supervision::SupervisorTree;
use tokio::sync::Mutex as TokioMutex;

/// Bundled kernel service ports.
///
/// Provides the canonical access points for all kernel primitives.
/// New handler code should depend on `ServicePorts` rather than constructing
/// tables individually.
///
/// # Migration note
///
/// Fields are intentionally `Arc<…>` so they can be shared with `CoreSystems`
/// during the transitional period. Once all consumers route through
/// `ServicePorts`, the individual table fields in `CoreSystems` can be removed.
pub struct ServicePorts {
    /// Authoritative process lifecycle table.
    pub process_table: Arc<ProcessTable>,
    /// Authoritative operation tree with cancellation propagation.
    pub operation_table: Arc<OperationTable>,
    /// Monotonic/wall clock for deadline enforcement.
    pub clock: Arc<dyn Clock>,
    /// Process supervision with restart policies.
    /// Shared via Arc<Mutex<>> so both ServicePorts tests and the daemon
    /// orchestrator can share the same supervisor instance.
    pub supervisor: Arc<TokioMutex<SupervisorTree>>,
    /// Inter-process mailbox routing (EnvelopeV2).
    pub mailbox_service: Arc<InProcessMailboxService>,
    /// Admission controller for capability gating.
    pub admission: Arc<dyn AdmissionController>,
    /// Shared cognitive workspace (RFC-014).
    ///
    /// Session-isolated working memory. When set, consumers should prefer
    /// this over the `agora` field in `CoreSystems`.
    pub agora: Option<Arc<dyn AgoraOps>>,
    /// Per-principal budget tracking (Phase 5B).
    pub budget: Arc<InMemoryBudgetController>,
    /// Resource lease manager (Phase 5B).
    pub leases: Arc<InMemoryResourceLeaseManager>,
    /// Context space manager (fork/attach — Phase 3A).
    pub space_manager: InMemorySpaceManager,
}

impl ServicePorts {
    /// Create production service ports backed by real kernel primitives.
    ///
    /// Uses `SystemClock` and `ProductionAdmissionController` for capability gating.
    /// Agora is set to `None` and should be wired via `with_agora()`.
    pub fn new() -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let process_table = Arc::new(ProcessTable::new(clock.clone()));
        let operation_table = Arc::new(OperationTable::new(clock.clone()));
        let supervisor = Arc::new(TokioMutex::new(SupervisorTree::new()));
        let mailbox_service = Arc::new(InProcessMailboxService::new());
        let budget = Arc::new(InMemoryBudgetController::new());
        let leases = Arc::new(InMemoryResourceLeaseManager::new());
        let admission: Arc<dyn AdmissionController> =
            Arc::new(ProductionAdmissionController::new(clock.clone())
                .with_budget(budget.clone())
                .with_leases(leases.clone()));
        let space_manager = InMemorySpaceManager::new();

        Self {
            process_table,
            operation_table,
            clock,
            supervisor,
            mailbox_service,
            admission,
            agora: None,
            budget,
            leases,
            space_manager,
        }
    }

    /// Attach an Agora workspace to the service ports.
    ///
    /// This should be called after `AgoraRegistry` is constructed and before
    /// any turn processing begins.
    pub fn with_agora(mut self, agora: Arc<dyn AgoraOps>) -> Self {
        self.agora = Some(agora);
        self
    }

    /// Create service ports for testing with a deterministic clock.
    pub fn for_testing(clock: Arc<dyn Clock>, admission: Arc<dyn AdmissionController>) -> Self {
        let process_table = Arc::new(ProcessTable::new(clock.clone()));
        let operation_table = Arc::new(OperationTable::new(clock.clone()));
        let supervisor = Arc::new(TokioMutex::new(SupervisorTree::new()));
        let mailbox_service = Arc::new(InProcessMailboxService::new());
        let budget = Arc::new(InMemoryBudgetController::new());
        let leases = Arc::new(InMemoryResourceLeaseManager::new());
        let space_manager = InMemorySpaceManager::new();

        Self {
            process_table,
            operation_table,
            clock,
            supervisor,
            mailbox_service,
            admission,
            agora: None,
            budget,
            leases,
            space_manager,
        }
    }
}

impl Default for ServicePorts {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ServicePorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServicePorts")
            .field("has_agora", &self.agora.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chronos::TestClock;
    use fabric::{
        AdmissionError, AdmissionRequest, ExecutionPermit, PermitId, ProcessManager, RevokeReason,
        SpawnSpec, UsageReport,
    };

    /// Minimal admission controller for testing service port wiring.
    struct TestAdmission;
    #[async_trait::async_trait]
    impl AdmissionController for TestAdmission {
        async fn admit(
            &self,
            request: AdmissionRequest,
        ) -> Result<ExecutionPermit, AdmissionError> {
            Ok(ExecutionPermit {
                id: PermitId::new(),
                operation_id: request.operation_id,
                process_id: request.process_id,
                capability: request.capability,
                granted_scope: request.requested_scope,
                expires_at: fabric::MonoDeadline::after(fabric::MonoTime(0), 60_000),
                sandbox: fabric::SandboxDecision::NotApplicable,
                budget_reservation: None,
                lease: None,
            })
        }

        async fn settle(
            &self,
            _permit_id: PermitId,
            _usage: UsageReport,
        ) -> Result<(), AdmissionError> {
            Ok(())
        }

        async fn revoke(
            &self,
            _permit_id: PermitId,
            _reason: RevokeReason,
        ) -> Result<(), AdmissionError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn service_ports_spawn_and_route() {
        let clock: Arc<dyn Clock> = Arc::new(TestClock::default());
        let admission: Arc<dyn AdmissionController> = Arc::new(TestAdmission);
        let ports = ServicePorts::for_testing(clock, admission);

        // Spawn a process.
        let handle = ports
            .process_table
            .spawn(SpawnSpec::default())
            .await
            .unwrap();

        let snapshot = ports.process_table.inspect(handle.id).await.unwrap();
        assert_eq!(snapshot.state, fabric::ProcessState::Created);
    }

    #[tokio::test]
    async fn service_ports_mailbox_routing() {
        use fabric::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId, Target};
        use fabric::ipc::mailbox::{InProcessMailbox, Mailbox, MailboxService};
        use fabric::types::process::NamespaceId;

        let clock: Arc<dyn Clock> = Arc::new(TestClock::default());
        let admission: Arc<dyn AdmissionController> = Arc::new(TestAdmission);
        let ports = ServicePorts::for_testing(clock, admission);

        let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
        ports
            .mailbox_service
            .register(Target::from("agent-1"), mb.clone())
            .await
            .unwrap();

        let env = EnvelopeV2::new(
            SchemaId::from("aletheon.test/v1"),
            Target::from("kernel"),
            Target::from("agent-1"),
            DeliveryPattern::Direct,
            NamespaceId("test".into()),
            serde_json::json!({"msg": "hi"}),
        );

        let receipt = ports.mailbox_service.route(env).await;
        assert!(receipt.is_ok());

        let received = mb.recv().await.unwrap();
        assert_eq!(received.payload["msg"], "hi");
    }

    #[tokio::test]
    async fn service_ports_has_budget_and_lease_controllers() {
        let ports = ServicePorts::new();
        // Budget controller starts empty (no principals configured).
        let budget_id = ports
            .budget
            .reserve(
                "test-agent",
                &fabric::BudgetRequest {
                    max_tokens: Some(100),
                    max_cost_micro: None,
                },
            )
            .await;
        assert!(budget_id.is_ok());

        // Lease manager can acquire resources.
        let lease_id = ports
            .leases
            .acquire(
                "test-agent",
                &fabric::LeaseRequest {
                    resource: "gpu-0".into(),
                    duration_ms: 5_000,
                },
                0,
            )
            .await;
        assert!(lease_id.is_ok());
    }

    #[tokio::test]
    async fn service_ports_with_agora() {
        use fabric::AgoraOperation;
        use std::sync::Arc;

        // Create a minimal AgoraOps stub.
        struct StubAgora;
        #[async_trait::async_trait]
        impl AgoraOps for StubAgora {
            async fn publish(
                &self,
                _s: &str,
                _k: &str,
                _v: serde_json::Value,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn recall(
                &self,
                _s: &str,
                _k: &str,
            ) -> anyhow::Result<Option<serde_json::Value>> {
                Ok(None)
            }
            async fn update(&self, _s: &str, _p: serde_json::Value) -> anyhow::Result<()> {
                Ok(())
            }
            async fn snapshot(&self, _s: &str) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::Value::Null)
            }
            async fn clear(&self, _s: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn trace(&self, _s: &str, _k: &str, _c: serde_json::Value) -> anyhow::Result<()> {
                Ok(())
            }
            async fn propose(
                &self,
                _s: &str,
                _b: u64,
                _op: AgoraOperation,
            ) -> Result<fabric::AgoraProposal, String> {
                Err("not implemented".into())
            }
            async fn commit(
                &self,
                _s: &str,
                _id: uuid::Uuid,
            ) -> Result<fabric::AgoraCommit, String> {
                Err("not implemented".into())
            }
        }

        let ports = ServicePorts::new().with_agora(Arc::new(StubAgora));
        assert!(ports.agora.is_some());
    }
}
