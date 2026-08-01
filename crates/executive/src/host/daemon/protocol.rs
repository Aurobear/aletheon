//! Pure per-connection daemon protocol negotiation.

use fabric::protocol::client::{negotiate_protocol_version, ClientCapabilities, ClientRequest};

fn supported_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        item_events: true,
        cursors: true,
        memory_gateway_v1: true,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NegotiatedProtocol {
    pub(crate) protocol_version: u16,
    pub(crate) capabilities: ClientCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ConnectionProtocolState {
    New,
    AwaitingInitialized {
        negotiated: NegotiatedProtocol,
    },
    Ready {
        negotiated: Option<NegotiatedProtocol>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProtocolEvent {
    Initialize(NegotiatedProtocol),
    Initialized,
    Request,
    LegacyRequest,
}

pub(crate) enum ProtocolAction {
    InitializeResponse(NegotiatedProtocol),
    Initialized,
    Dispatch,
}

pub(crate) struct ProtocolTransition {
    pub(crate) next_state: ConnectionProtocolState,
    pub(crate) action: ProtocolAction,
}

pub(crate) fn reduce_protocol(
    state: &ConnectionProtocolState,
    event: ProtocolEvent,
) -> anyhow::Result<ProtocolTransition> {
    use ConnectionProtocolState::{AwaitingInitialized, New, Ready};
    use ProtocolEvent::{Initialize, Initialized, LegacyRequest, Request};
    match (state, event) {
        (New, Initialize(negotiated)) => Ok(ProtocolTransition {
            next_state: AwaitingInitialized {
                negotiated: negotiated.clone(),
            },
            action: ProtocolAction::InitializeResponse(negotiated),
        }),
        (New, LegacyRequest) => Ok(ProtocolTransition {
            next_state: Ready { negotiated: None },
            action: ProtocolAction::Dispatch,
        }),
        (New, _) => anyhow::bail!("connection must initialize before requests"),
        (AwaitingInitialized { negotiated }, Initialized) => Ok(ProtocolTransition {
            next_state: Ready {
                negotiated: Some(negotiated.clone()),
            },
            action: ProtocolAction::Initialized,
        }),
        (AwaitingInitialized { .. }, Initialize(_)) => {
            anyhow::bail!("connection initialization cannot be repeated")
        }
        (AwaitingInitialized { .. }, _) => {
            anyhow::bail!("connection must send initialized before requests")
        }
        (Ready { .. }, Initialize(_) | Initialized) => {
            anyhow::bail!("connection initialization cannot be repeated")
        }
        (
            Ready {
                negotiated: Some(_),
            },
            Request,
        ) => Ok(ProtocolTransition {
            next_state: state.clone(),
            action: ProtocolAction::Dispatch,
        }),
        (Ready { negotiated: None }, LegacyRequest) => Ok(ProtocolTransition {
            next_state: state.clone(),
            action: ProtocolAction::Dispatch,
        }),
        (Ready { negotiated: None }, Request) => {
            anyhow::bail!("legacy connections cannot send versioned requests")
        }
        (
            Ready {
                negotiated: Some(_),
            },
            LegacyRequest,
        ) => {
            anyhow::bail!("versioned connections cannot send legacy requests")
        }
    }
}

impl ConnectionProtocolState {
    pub(crate) fn accept(&mut self, request: &ClientRequest) -> anyhow::Result<ProtocolAction> {
        if request.requires_memory_gateway() {
            let enabled = matches!(
                self,
                Self::Ready {
                    negotiated: Some(NegotiatedProtocol { capabilities, .. })
                } if capabilities.memory_gateway_v1
            );
            anyhow::ensure!(enabled, "memory_gateway_v1 was not negotiated");
        }
        let event = match request {
            ClientRequest::Initialize(params) => ProtocolEvent::Initialize(NegotiatedProtocol {
                protocol_version: negotiate_protocol_version(&params.protocol_versions)?,
                capabilities: params.capabilities.intersect(&supported_capabilities()),
            }),
            ClientRequest::Initialized => ProtocolEvent::Initialized,
            _ => ProtocolEvent::Request,
        };
        self.apply(event)
    }

    pub(crate) fn accept_legacy(&mut self) -> anyhow::Result<ProtocolAction> {
        self.apply(ProtocolEvent::LegacyRequest)
    }

    /// The sole connection protocol mutation entry point.
    fn apply(&mut self, event: ProtocolEvent) -> anyhow::Result<ProtocolAction> {
        let transition = reduce_protocol(self, event)?;
        *self = transition.next_state;
        Ok(transition.action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn negotiated() -> NegotiatedProtocol {
        NegotiatedProtocol {
            protocol_version: 1,
            capabilities: ClientCapabilities {
                item_events: true,
                cursors: true,
                memory_gateway_v1: false,
            },
        }
    }

    #[test]
    fn characterizes_versioned_and_legacy_success_paths() {
        let waiting = reduce_protocol(
            &ConnectionProtocolState::New,
            ProtocolEvent::Initialize(negotiated()),
        )
        .unwrap()
        .next_state;
        let ready = reduce_protocol(&waiting, ProtocolEvent::Initialized)
            .unwrap()
            .next_state;
        assert!(reduce_protocol(&ready, ProtocolEvent::Request).is_ok());
        let legacy = reduce_protocol(&ConnectionProtocolState::New, ProtocolEvent::LegacyRequest)
            .unwrap()
            .next_state;
        assert!(reduce_protocol(&legacy, ProtocolEvent::LegacyRequest).is_ok());
    }

    #[test]
    fn rejects_out_of_order_repeated_and_cross_protocol_events() {
        let waiting = ConnectionProtocolState::AwaitingInitialized {
            negotiated: negotiated(),
        };
        let versioned = ConnectionProtocolState::Ready {
            negotiated: Some(negotiated()),
        };
        let legacy = ConnectionProtocolState::Ready { negotiated: None };
        assert!(reduce_protocol(&ConnectionProtocolState::New, ProtocolEvent::Request).is_err());
        assert!(reduce_protocol(&waiting, ProtocolEvent::Request).is_err());
        assert!(reduce_protocol(&waiting, ProtocolEvent::Initialize(negotiated())).is_err());
        assert!(reduce_protocol(&versioned, ProtocolEvent::Initialized).is_err());
        assert!(reduce_protocol(&versioned, ProtocolEvent::LegacyRequest).is_err());
        assert!(reduce_protocol(&legacy, ProtocolEvent::Request).is_err());
    }

    #[test]
    fn rejects_memory_requests_when_gateway_capability_was_not_negotiated() {
        let mut state = ConnectionProtocolState::New;
        state
            .accept(&ClientRequest::Initialize(
                fabric::protocol::client::InitializeParams {
                    client_version: "memory-client".into(),
                    protocol_versions: vec![1],
                    capabilities: ClientCapabilities {
                        item_events: true,
                        cursors: true,
                        memory_gateway_v1: false,
                    },
                },
            ))
            .unwrap();
        state.accept(&ClientRequest::Initialized).unwrap();

        assert!(state
            .accept(&ClientRequest::MemoryReceiptGet(
                fabric::protocol::memory::MemoryReceiptGetRequestV1 {
                    durable_intake_id: "intake-1".into(),
                },
            ))
            .is_err());
        assert!(state
            .accept(&ClientRequest::Snapshot(
                fabric::protocol::client::SnapshotRequest {
                    session_id: fabric::SessionId("session-1".into()),
                },
            ))
            .is_ok());
    }

    #[test]
    fn accepts_memory_requests_when_gateway_capability_was_negotiated() {
        let mut state = ConnectionProtocolState::New;
        state
            .accept(&ClientRequest::Initialize(
                fabric::protocol::client::InitializeParams {
                    client_version: "memory-client".into(),
                    protocol_versions: vec![1],
                    capabilities: ClientCapabilities {
                        item_events: true,
                        cursors: true,
                        memory_gateway_v1: true,
                    },
                },
            ))
            .unwrap();
        state.accept(&ClientRequest::Initialized).unwrap();

        assert!(state
            .accept(&ClientRequest::MemoryReceiptGet(
                fabric::protocol::memory::MemoryReceiptGetRequestV1 {
                    durable_intake_id: "intake-1".into(),
                },
            ))
            .is_ok());
    }
}
