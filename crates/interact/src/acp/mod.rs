//! ACP (Agent Client Protocol) edge adapter.
//!
//! This module owns protocol translation and connection-local correlation only.
//! Executive remains authoritative for sessions, turns, cancellation, approvals,
//! and event history.

mod event_map;
mod gateway;
pub mod transport;

use std::{collections::VecDeque, path::PathBuf};

use fabric::{
    protocol::client::negotiate_protocol_version, ConnectionId, LocalOsPrincipal, PrincipalContext,
    PrincipalId, ThreadId, WorkspacePolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use event_map::{is_turn_terminal, map_client_event_to_acp};
pub use gateway::{
    run_transport_loop, AcpBackend, AcpEventSource, AcpServerFrame, AcpSessionEvent,
    AuthenticatedAcpConnection, CreatedAcpSession,
};

/// First-version ACP method subset. Unsupported methods are deliberately not
/// represented, so the edge cannot accidentally advertise unfinished features.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum AcpRequest {
    Initialize {
        #[serde(default)]
        client_capabilities: Value,
        protocol_versions: Vec<u16>,
    },
    NewSession {
        cwd: PathBuf,
    },
    Prompt {
        session_id: String,
        text: String,
    },
    Cancel {
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum AcpResponse {
    Initialized {
        protocol_version: u16,
        agent_capabilities: Value,
    },
    SessionCreated {
        session_id: String,
    },
    Accepted,
    Cancelled,
    Error {
        message: String,
    },
}

/// Host-minted binding for an ACP session. The session id is only a lookup key;
/// connection equality is checked again on every prompt/cancel operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpSessionBinding {
    pub connection_id: ConnectionId,
    pub thread_id: ThreadId,
}

/// Bounded connection-local correlation table.
///
/// Oldest bindings are evicted at the configured limit. Rebinding an existing
/// id refreshes its position without growing the table.
#[derive(Debug)]
pub struct AcpCorrelation {
    limit: usize,
    entries: VecDeque<(String, AcpSessionBinding)>,
}

impl AcpCorrelation {
    pub const DEFAULT_LIMIT: usize = 256;

    pub fn new(limit: usize) -> Result<Self, AcpError> {
        if limit == 0 {
            return Err(AcpError::InvalidCorrelationLimit);
        }
        Ok(Self {
            limit,
            entries: VecDeque::with_capacity(limit.min(Self::DEFAULT_LIMIT)),
        })
    }

    pub fn insert(&mut self, session_id: String, binding: AcpSessionBinding) {
        if let Some(index) = self.entries.iter().position(|(id, _)| id == &session_id) {
            self.entries.remove(index);
        }
        if self.entries.len() == self.limit {
            self.entries.pop_front();
        }
        self.entries.push_back((session_id, binding));
    }

    /// Resolve a client session id only inside its authenticated connection.
    pub fn resolve(
        &self,
        session_id: &str,
        connection_id: &ConnectionId,
    ) -> Result<&AcpSessionBinding, AcpError> {
        let binding = self
            .entries
            .iter()
            .find_map(|(id, binding)| (id == session_id).then_some(binding))
            .ok_or(AcpError::UnknownSession)?;
        if &binding.connection_id != connection_id {
            return Err(AcpError::SessionNotVisible);
        }
        Ok(binding)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for AcpCorrelation {
    fn default() -> Self {
        Self::new(Self::DEFAULT_LIMIT).expect("the default ACP correlation limit is non-zero")
    }
}

/// Protocol-only adapter state. It contains no session or turn domain state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcpMetrics {
    pub sessions_active: u64,
    pub prompt_total: u64,
    pub reconnect_total: u64,
    pub map_unmapped_event_total: u64,
}

#[derive(Debug, Default)]
pub struct AcpAdapter {
    correlation: AcpCorrelation,
    metrics: AcpMetrics,
}

impl AcpAdapter {
    pub fn with_correlation_limit(limit: usize) -> Result<Self, AcpError> {
        Ok(Self {
            correlation: AcpCorrelation::new(limit)?,
            metrics: AcpMetrics::default(),
        })
    }

    pub fn initialize(&self, protocol_versions: &[u16]) -> AcpResponse {
        match negotiate_protocol_version(protocol_versions) {
            Ok(protocol_version) => AcpResponse::Initialized {
                protocol_version,
                agent_capabilities: agent_capabilities(),
            },
            Err(error) => AcpResponse::Error {
                message: error.to_string(),
            },
        }
    }

    /// Record a binding only after Executive has successfully created the
    /// authoritative session.
    pub fn bind_created_session(
        &mut self,
        session_id: String,
        connection_id: ConnectionId,
        thread_id: ThreadId,
    ) -> AcpResponse {
        self.correlation.insert(
            session_id.clone(),
            AcpSessionBinding {
                connection_id,
                thread_id,
            },
        );
        AcpResponse::SessionCreated { session_id }
    }

    pub fn resolve_session(
        &self,
        session_id: &str,
        connection_id: &ConnectionId,
    ) -> Result<&AcpSessionBinding, AcpError> {
        self.correlation.resolve(session_id, connection_id)
    }

    pub fn metrics(&self) -> &AcpMetrics {
        &self.metrics
    }
}

impl fabric::Observable for AcpAdapter {
    fn status(&self) -> fabric::SubsystemStatus {
        fabric::SubsystemStatus {
            name: "acp-adapter".into(),
            running: true,
            status_line: format!("{} active session(s)", self.metrics.sessions_active),
            details: self.metrics().named().into_iter().collect(),
        }
    }

    fn metrics(&self) -> std::collections::HashMap<String, String> {
        self.metrics().named().into_iter().collect()
    }
}

impl AcpMetrics {
    /// Fixed-cardinality metric export. No session, principal, method, or
    /// workspace value is admitted as a label.
    pub fn named(&self) -> [(String, String); 4] {
        [
            (
                "acp_sessions_active".into(),
                self.sessions_active.to_string(),
            ),
            ("acp_prompt_total".into(), self.prompt_total.to_string()),
            (
                "acp_reconnect_total".into(),
                self.reconnect_total.to_string(),
            ),
            (
                "acp_map_unmapped_event_total".into(),
                self.map_unmapped_event_total.to_string(),
            ),
        ]
    }
}

/// Construct authority exclusively from host-authenticated connection facts.
/// No ACP-supplied session identifier participates in principal construction.
pub fn establish_principal(
    os_principal: LocalOsPrincipal,
    connection_id: ConnectionId,
    thread_id: ThreadId,
    workspace: WorkspacePolicy,
    permission_profile: fabric::PermissionProfileId,
    approval_policy: fabric::ApprovalPolicy,
) -> PrincipalContext {
    PrincipalContext::new(
        PrincipalId::local_uid(os_principal.uid),
        os_principal,
        connection_id,
        thread_id,
        workspace,
        permission_profile,
        approval_policy,
    )
}

fn agent_capabilities() -> Value {
    json!({
        "session": {
            "new": true,
            "prompt": true,
            "cancel": true,
            "load": false
        },
        "permissions": false,
        "clientFileSystem": false,
        "clientTerminal": false,
        "modes": false,
        "models": false
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum AcpError {
    InvalidCorrelationLimit,
    UnknownSession,
    SessionNotVisible,
    WorkspaceNotAuthorized,
    InvalidPrompt,
    Backend(String),
}

impl std::fmt::Display for AcpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCorrelationLimit => "ACP correlation limit must be non-zero",
            Self::UnknownSession => "unknown ACP session",
            Self::SessionNotVisible => "ACP session is not visible to this connection",
            Self::WorkspaceNotAuthorized => "ACP workspace is outside authenticated authority",
            Self::InvalidPrompt => "ACP prompt must not be empty",
            Self::Backend(message) => message,
        })
    }
}

impl std::error::Error for AcpError {}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::Observable;

    #[test]
    fn initialization_uses_fabric_negotiation_and_advertises_only_v1_subset() {
        let adapter = AcpAdapter::default();
        let response = adapter.initialize(&[99, fabric::CLIENT_PROTOCOL_VERSION]);
        let AcpResponse::Initialized {
            protocol_version,
            agent_capabilities,
        } = response
        else {
            panic!("expected initialized response")
        };
        assert_eq!(protocol_version, fabric::CLIENT_PROTOCOL_VERSION);
        assert_eq!(agent_capabilities["session"]["prompt"], true);
        assert_eq!(agent_capabilities["session"]["load"], false);
        assert_eq!(agent_capabilities["permissions"], false);
        assert!(matches!(
            adapter.initialize(&[99]),
            AcpResponse::Error { .. }
        ));
    }

    #[test]
    fn observable_exports_only_the_four_bounded_named_metrics() {
        let adapter = AcpAdapter::default();
        let metrics = Observable::metrics(&adapter);
        assert_eq!(metrics.len(), 4);
        assert_eq!(metrics["acp_sessions_active"], "0");
        assert_eq!(metrics["acp_prompt_total"], "0");
        assert_eq!(metrics["acp_reconnect_total"], "0");
        assert_eq!(metrics["acp_map_unmapped_event_total"], "0");
        assert_eq!(Observable::status(&adapter).details, metrics);
    }

    #[test]
    fn correlation_is_bounded_and_connection_scoped() {
        let mut table = AcpCorrelation::new(2).unwrap();
        let first_connection = ConnectionId::new();
        let other_connection = ConnectionId::new();
        table.insert(
            "one".into(),
            AcpSessionBinding {
                connection_id: first_connection.clone(),
                thread_id: ThreadId("t1".into()),
            },
        );
        table.insert(
            "two".into(),
            AcpSessionBinding {
                connection_id: first_connection.clone(),
                thread_id: ThreadId("t2".into()),
            },
        );
        assert_eq!(
            table.resolve("one", &other_connection),
            Err(AcpError::SessionNotVisible)
        );
        table.insert(
            "three".into(),
            AcpSessionBinding {
                connection_id: first_connection.clone(),
                thread_id: ThreadId("t3".into()),
            },
        );
        assert_eq!(table.len(), 2);
        assert_eq!(
            table.resolve("one", &first_connection),
            Err(AcpError::UnknownSession)
        );
        assert_eq!(
            table.resolve("three", &first_connection).unwrap().thread_id,
            ThreadId("t3".into())
        );
    }

    #[test]
    fn client_session_id_is_not_part_of_principal_authority() {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            WorkspacePolicy::from_resolved_roots(root.path().to_path_buf(), vec![]).unwrap();
        let connection_id = ConnectionId::new();
        let context = establish_principal(
            LocalOsPrincipal { uid: 501, gid: 20 },
            connection_id.clone(),
            ThreadId("host-minted-thread".into()),
            workspace,
            fabric::PermissionProfileId::workspace_write(),
            fabric::ApprovalPolicy::OnRequest,
        );
        assert_eq!(context.principal_id, PrincipalId::local_uid(501));
        assert_eq!(context.connection_id, connection_id);
        assert_eq!(context.thread_id, ThreadId("host-minted-thread".into()));
    }
}
