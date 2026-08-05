//! Pure per-connection daemon protocol negotiation.

use fabric::protocol::client::{negotiate_protocol_version, ClientCapabilities, ClientRequest};

fn supported_capabilities(
    memory_maintenance_v1: bool,
    memory_admin_v1: bool,
) -> ClientCapabilities {
    ClientCapabilities {
        item_events: true,
        cursors: true,
        memory_gateway_v1: true,
        memory_maintenance_v1,
        memory_admin_v1,
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
    #[cfg(test)]
    pub(crate) fn accept(&mut self, request: &ClientRequest) -> anyhow::Result<ProtocolAction> {
        self.accept_with_capabilities(request, false, false)
    }

    pub(crate) fn accept_with_capabilities(
        &mut self,
        request: &ClientRequest,
        official_memory_agent: bool,
        official_memory_admin: bool,
    ) -> anyhow::Result<ProtocolAction> {
        // During the X5c presentation migration, legacy TUI connections may
        // consume the canonical read-only Session projection without gaining
        // access to any versioned mutation. This is deliberately limited to
        // snapshot/subscription reads and can be deleted once every TUI
        // command uses the versioned connection.
        if matches!(self, Self::Ready { negotiated: None })
            && matches!(
                request,
                ClientRequest::ReadSnapshot(_)
                    | ClientRequest::ReadSessions
                    | ClientRequest::ReadEvents(_)
                    | ClientRequest::Subscribe(_)
            )
        {
            return Ok(ProtocolAction::Dispatch);
        }
        if request.requires_memory_gateway() {
            let enabled = matches!(
                self,
                Self::Ready {
                    negotiated: Some(NegotiatedProtocol { capabilities, .. })
                } if capabilities.memory_gateway_v1
            );
            anyhow::ensure!(enabled, "memory_gateway_v1 was not negotiated");
        }
        if request.requires_memory_maintenance() {
            let enabled = matches!(
                self,
                Self::Ready {
                    negotiated: Some(NegotiatedProtocol { capabilities, .. })
                } if capabilities.memory_maintenance_v1
            );
            anyhow::ensure!(enabled, "memory_maintenance_v1 was not negotiated");
        }
        if request.requires_memory_admin() {
            let enabled = matches!(
                self,
                Self::Ready {
                    negotiated: Some(NegotiatedProtocol { capabilities, .. })
                } if capabilities.memory_admin_v1
            );
            anyhow::ensure!(enabled, "memory_admin_v1 was not negotiated");
        }
        let event = match request {
            ClientRequest::Initialize(params) => ProtocolEvent::Initialize(NegotiatedProtocol {
                protocol_version: negotiate_protocol_version(&params.protocol_versions)?,
                capabilities: params.capabilities.intersect(&supported_capabilities(
                    official_memory_agent,
                    official_memory_admin,
                )),
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
                memory_maintenance_v1: false,
                memory_admin_v1: false,
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
    fn legacy_connection_can_only_bridge_canonical_projection_reads() {
        let mut state = ConnectionProtocolState::New;
        state.accept_legacy().unwrap();
        let session_id = fabric::SessionId("session-1".into());
        assert!(state
            .accept(&ClientRequest::ReadSnapshot(
                fabric::protocol::client::SnapshotRequest {
                    session_id: session_id.clone(),
                },
            ))
            .is_ok());
        assert!(state.accept(&ClientRequest::ReadSessions).is_ok());
        assert!(state
            .accept(&ClientRequest::ReadEvents(
                fabric::protocol::client::EventSubscription {
                    session_id: fabric::SessionId("session-1".into()),
                    after: fabric::protocol::client::EventCursor::origin(),
                },
            ))
            .is_ok());
        assert!(state
            .accept(&ClientRequest::Subscribe(
                fabric::protocol::client::EventSubscription {
                    session_id,
                    after: fabric::protocol::client::EventCursor::origin(),
                },
            ))
            .is_ok());
        assert!(state
            .accept(&ClientRequest::Snapshot(
                fabric::protocol::client::SnapshotRequest {
                    session_id: fabric::SessionId("session-1".into()),
                },
            ))
            .is_err());
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
                        memory_maintenance_v1: false,
                        memory_admin_v1: false,
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
                        memory_maintenance_v1: false,
                        memory_admin_v1: false,
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

    #[test]
    fn maintenance_capability_is_connection_role_scoped() {
        let initialize = ClientRequest::Initialize(fabric::protocol::client::InitializeParams {
            client_version: "memory-agent".into(),
            protocol_versions: vec![1],
            capabilities: ClientCapabilities {
                item_events: false,
                cursors: false,
                memory_gateway_v1: false,
                memory_maintenance_v1: true,
                memory_admin_v1: false,
            },
        });
        let request = ClientRequest::MemoryMaintenanceStatus(
            fabric::protocol::memory_maintenance::MemoryMaintenanceStatusRequestV1 {
                request_id: "status-1".into(),
            },
        );

        let mut ordinary = ConnectionProtocolState::New;
        ordinary
            .accept_with_capabilities(&initialize, false, false)
            .unwrap();
        ordinary.accept(&ClientRequest::Initialized).unwrap();
        assert!(ordinary.accept(&request).is_err());

        let mut official = ConnectionProtocolState::New;
        official
            .accept_with_capabilities(&initialize, true, false)
            .unwrap();
        official.accept(&ClientRequest::Initialized).unwrap();
        assert!(official.accept(&request).is_ok());
    }

    #[test]
    fn memory_admin_capability_is_connection_role_scoped() {
        let initialize = ClientRequest::Initialize(fabric::protocol::client::InitializeParams {
            client_version: "memory-admin".into(),
            protocol_versions: vec![1],
            capabilities: ClientCapabilities {
                item_events: false,
                cursors: false,
                memory_gateway_v1: false,
                memory_maintenance_v1: false,
                memory_admin_v1: true,
            },
        });
        let request = ClientRequest::MemoryWorkspaceUnbind(
            fabric::protocol::memory::MemoryWorkspaceUnbindRequestV1 {
                working_dir: "/tmp".into(),
            },
        );

        let mut ordinary = ConnectionProtocolState::New;
        ordinary
            .accept_with_capabilities(&initialize, false, false)
            .unwrap();
        ordinary.accept(&ClientRequest::Initialized).unwrap();
        assert!(ordinary.accept(&request).is_err());

        let mut admin = ConnectionProtocolState::New;
        admin
            .accept_with_capabilities(&initialize, false, true)
            .unwrap();
        admin.accept(&ClientRequest::Initialized).unwrap();
        assert!(admin.accept(&request).is_ok());
    }
}
