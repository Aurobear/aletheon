//! Request-handler compatibility entry points.
//!
//! Concrete construction lives exclusively in `daemon::bootstrap`; this module
//! exposes only request-facing accessors and notification wiring.

use std::sync::Arc;

use tokio::sync::mpsc;

use super::RequestHandler;
use crate::application::CapabilityService;
use crate::host::daemon::debug_handler::DebugHandler;

impl RequestHandler {
    pub fn debug_handler(&self) -> Arc<DebugHandler> {
        self.ports.debug.clone()
    }

    pub fn corpus_service(&self) -> Arc<dyn corpus::CorpusService> {
        self.ports.transport.corpus.clone()
    }

    pub fn corpus_grant(&self) -> corpus::ExtensionGrant {
        self.ports.transport.capabilities_grant.clone()
    }

    pub fn capability_service(&self) -> Arc<dyn CapabilityService> {
        self.ports.transport.capabilities.clone()
    }

    pub fn clock(&self) -> Arc<dyn fabric::Clock> {
        self.ports.transport.clock.clone()
    }

    pub async fn set_notify_channel(&mut self, tx: mpsc::Sender<String>) {
        self.notify_tx = Some(tx.clone());
        self.ports.turn.set_notify(tx).await;
    }

    pub async fn create_notify_channel(&mut self) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel(64);
        self.set_notify_channel(tx).await;
        rx
    }
}
