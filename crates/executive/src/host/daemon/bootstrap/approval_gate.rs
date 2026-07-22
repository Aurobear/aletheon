//! Workspace trust resolver and durable approval gate used during daemon bootstrap.

use std::sync::Arc;

use async_trait::async_trait;
use corpus::security::socket_approval::SocketApprovalGate;
use fabric::CanonicalEventBus;
use fabric::Clock;

pub(crate) fn bootstrap_workspace_trust_resolver(
    data_dir: &std::path::Path,
    folder_trust_enabled: bool,
    event_bus: Option<Arc<CanonicalEventBus>>,
) -> Arc<crate::application::workspace_trust::WorkspaceTrustResolver> {
    let mut resolver = crate::application::workspace_trust::WorkspaceTrustResolver::new(
        Arc::new(crate::application::workspace_trust::FileTrustStore::new(
            data_dir.join("workspace-trust-receipts.json"),
        )),
        Arc::new(crate::application::workspace_trust::KnownConfigDiscoverer::default()),
        folder_trust_enabled,
    );
    if let Some(bus) = event_bus {
        resolver = resolver.with_event_bus(bus);
    }
    Arc::new(resolver)
}

pub(crate) struct DurableSocketApprovalGate {
    pub(crate) socket: Arc<SocketApprovalGate>,
    pub(crate) repository: Arc<std::sync::Mutex<crate::r#impl::approval::ApprovalRepository>>,
    pub(crate) clock: Arc<dyn Clock>,
}

#[async_trait]
impl corpus::security::approval::ApprovalGate for DurableSocketApprovalGate {
    async fn request(
        &self,
        request: &corpus::security::approval::ApprovalRequest,
    ) -> corpus::security::approval::ApprovalDecision {
        let requested_at_ms = self.clock.wall_now().0;
        if self
            .repository
            .lock()
            .ok()
            .and_then(|repository| {
                repository
                    .record_external_request(
                        &request.call_id,
                        &request.tool,
                        &request.action_summary,
                        requested_at_ms,
                    )
                    .ok()
            })
            .is_none()
        {
            return corpus::security::approval::ApprovalDecision::Deny;
        }

        let decision = self.socket.request(request).await;
        let decision_wire = match decision {
            corpus::security::approval::ApprovalDecision::Approve => "approve",
            corpus::security::approval::ApprovalDecision::Deny => "deny",
            corpus::security::approval::ApprovalDecision::ApproveForSession => {
                "approve_for_session"
            }
        };
        if self
            .repository
            .lock()
            .ok()
            .and_then(|repository| {
                repository
                    .record_external_resolution(
                        &request.call_id,
                        decision_wire,
                        self.clock.wall_now().0,
                    )
                    .ok()
            })
            .is_none()
        {
            return corpus::security::approval::ApprovalDecision::Deny;
        }
        decision
    }
}
