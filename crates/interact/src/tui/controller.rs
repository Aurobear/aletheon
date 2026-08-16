//! TUI controller boundary.
//!
//! The controller is the only presentation-side owner of Gateway transports.
//! It does not mint canonical identities, construct repositories, derive
//! effective policy, or infer terminal state.  Command modules may borrow the
//! explicitly typed client slots while the final command adapter migration is
//! completed.

use gateway::client::{GatewayClient, UnixSocketTransport};

/// Presentation transport boundary for the typed Gateway.
pub(crate) struct TuiController {
    /// The sole production transport. It owns framing/correlation through
    /// `gateway-client`; the controller never opens a second socket.
    pub(crate) typed_gateway: Option<GatewayClient<UnixSocketTransport>>,
}

impl TuiController {
    pub(crate) fn new(typed_gateway: Option<GatewayClient<UnixSocketTransport>>) -> Self {
        Self { typed_gateway }
    }

    pub(crate) fn has_typed_gateway(&self) -> bool {
        self.typed_gateway.is_some()
    }
}
