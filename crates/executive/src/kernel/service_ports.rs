//! Service Ports — Phase 6A.
//!
//! Bundles kernel services behind clean interfaces so the daemon/exec handler
//! layers depend on service ports rather than individual kernel tables.
//!
//! This is the first step of CoreSystems contraction: new code uses
//! `ServicePorts`; old code gradually migrates its fields from `CoreSystems`
//! into `ServicePorts` (tracked as RFC-018 D5).

use crate::kernel::admission::ProductionAdmissionController;
use crate::kernel::chronos::SystemClock;
use crate::kernel::operation::OperationTable;
use crate::kernel::process::ProcessTable;
use crate::kernel::supervision::SupervisorTree;
use fabric::ipc::mailbox::InProcessMailboxService;
use fabric::{AdmissionController, Clock};
use std::sync::Arc;

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
    pub supervisor: SupervisorTree,
    /// Inter-process mailbox routing (EnvelopeV2).
    pub mailbox_service: Arc<InProcessMailboxService>,
    /// Admission controller for capability gating.
    pub admission: Arc<dyn AdmissionController>,
}

impl ServicePorts {
    /// Create production service ports backed by real kernel primitives.
    ///
    /// Uses `SystemClock` and `ProductionAdmissionController` for capability gating.
    pub fn new() -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let process_table = Arc::new(ProcessTable::new(clock.clone()));
        let operation_table = Arc::new(OperationTable::new(clock.clone()));
        let supervisor = SupervisorTree::new();
        let mailbox_service = Arc::new(InProcessMailboxService::new());
        let admission: Arc<dyn AdmissionController> =
            Arc::new(ProductionAdmissionController::new(clock.clone()));

        Self {
            process_table,
            operation_table,
            clock,
            supervisor,
            mailbox_service,
            admission,
        }
    }

    /// Create service ports for testing with a deterministic clock.
    pub fn for_testing(clock: Arc<dyn Clock>, admission: Arc<dyn AdmissionController>) -> Self {
        let process_table = Arc::new(ProcessTable::new(clock.clone()));
        let operation_table = Arc::new(OperationTable::new(clock.clone()));
        let supervisor = SupervisorTree::new();
        let mailbox_service = Arc::new(InProcessMailboxService::new());

        Self {
            process_table,
            operation_table,
            clock,
            supervisor,
            mailbox_service,
            admission,
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
        f.debug_struct("ServicePorts").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::chronos::TestClock;
    use fabric::{AdmissionError, AdmissionRequest, ExecutionPermit, PermitId, RevokeReason};
    use fabric::{ProcessManager, SpawnSpec, UsageReport};

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
            SchemaId::from("test/v1"),
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
}
