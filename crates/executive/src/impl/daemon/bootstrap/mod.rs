//! Private daemon composition root.
//!
//! Construction code in this module is the only production code allowed to
//! know the concrete implementations behind the request and turn ports.

mod approval_gate;
mod channels;
mod extensions;
mod google;
mod params;
mod request;
mod request_ports;
mod runtime;
mod services;
mod storage;
mod turn_runtime;

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::core::config::GrokHardeningConfig;
use crate::r#impl::daemon::handler::ports::HandlerPorts;
use crate::r#impl::daemon::handler::RequestHandler;

/// Transient, non-cloneable result of daemon composition.
///
/// It is consumed immediately, so concrete construction state cannot become a
/// long-lived service locator.
pub(super) struct DaemonComposition {
    request: Arc<HandlerPorts>,
    active_connections: Arc<AtomicUsize>,
    thread_authority: Arc<crate::service::thread_authority::ThreadAuthorityStore>,
    grok_hardening: GrokHardeningConfig,
    workspace_trust: Arc<crate::service::workspace_trust::WorkspaceTrustResolver>,
    mcp: Option<Arc<corpus::tools::mcp::manager::McpManager>>,
}

impl DaemonComposition {
    fn into_handler(self) -> RequestHandler {
        RequestHandler {
            ports: self.request,
            notify_tx: None,
            active_connections: self.active_connections,
            thread_authority: self.thread_authority,
            grok_hardening: self.grok_hardening,
            workspace_trust: self.workspace_trust,
            mcp: self.mcp,
        }
    }
}
