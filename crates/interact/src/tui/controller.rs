//! TUI controller boundary.
//!
//! The controller is the only presentation-side owner of Gateway transports.
//! It does not mint canonical identities, construct repositories, derive
//! effective policy, or infer terminal state.  Command modules may borrow the
//! explicitly typed client slots while the final command adapter migration is
//! completed.

use gateway::client::{GatewayClient, ReconnectPolicy, UnixSocketTransport};
use gateway::protocol::ProtocolError;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentationConnectionState {
    Connected,
    Recovering,
    Disconnected,
}

/// Presentation transport boundary for the typed Gateway.
pub(crate) struct TuiController {
    /// The sole production transport. It owns framing/correlation through
    /// `gateway-client`; the controller never opens a second socket.
    pub(crate) typed_gateway: Option<GatewayClient<UnixSocketTransport>>,
    pub(crate) connection_state: PresentationConnectionState,
    reconnect_task: Option<
        tokio::task::JoinHandle<(
            GatewayClient<UnixSocketTransport>,
            Result<(), ProtocolError>,
        )>,
    >,
}

impl TuiController {
    pub(crate) fn new(typed_gateway: Option<GatewayClient<UnixSocketTransport>>) -> Self {
        let connection_state = if typed_gateway.is_some() {
            PresentationConnectionState::Connected
        } else {
            PresentationConnectionState::Disconnected
        };
        Self {
            typed_gateway,
            connection_state,
            reconnect_task: None,
        }
    }

    pub(crate) fn has_typed_gateway(&self) -> bool {
        self.typed_gateway.is_some()
    }

    /// Move the transport off the render/input task while bounded reconnect
    /// attempts are in flight. Calling this repeatedly is idempotent.
    pub(crate) fn begin_reconnect(&mut self) -> bool {
        if self.connection_state == PresentationConnectionState::Recovering {
            return false;
        }
        let Some(mut client) = self.typed_gateway.take() else {
            self.connection_state = PresentationConnectionState::Disconnected;
            return false;
        };
        self.connection_state = PresentationConnectionState::Recovering;
        self.reconnect_task = Some(tokio::spawn(async move {
            let result = client
                .reconnect(ReconnectPolicy {
                    max_attempts: 40,
                    backoff: Duration::from_millis(250),
                })
                .await;
            (client, result)
        }));
        true
    }

    /// Poll a background reconnect without awaiting socket progress on the TUI
    /// task. The JoinHandle is awaited only after it reports completion.
    pub(crate) async fn poll_reconnect(&mut self) -> Option<Result<(), ProtocolError>> {
        if !self
            .reconnect_task
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
        {
            return None;
        }
        let task = self.reconnect_task.take().expect("finished task checked");
        match task.await {
            Ok((client, result)) => {
                self.typed_gateway = Some(client);
                self.connection_state = if result.is_ok() {
                    PresentationConnectionState::Connected
                } else {
                    PresentationConnectionState::Disconnected
                };
                Some(result)
            }
            Err(error) => {
                self.connection_state = PresentationConnectionState::Disconnected;
                Some(Err(ProtocolError::Server(format!(
                    "presentation reconnect task failed: {error}"
                ))))
            }
        }
    }
}
