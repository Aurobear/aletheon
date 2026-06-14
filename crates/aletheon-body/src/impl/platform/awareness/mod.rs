//! Agent awareness: cross-process discovery and conflict detection.
//!
//! Phase 1 scope: L2 local discovery only (Unix socket scan at `/var/run/aletheon/*.sock`).
//! L3/L4 (mDNS, WAN) are deferred to future phases.

pub mod communication;
pub mod conflict;
pub mod discovery;
pub mod lifecycle;

pub use communication::{AgentCommunication, JsonRpcRequest, JsonRpcResponse};
pub use conflict::{ConflictDetector, ConflictReport, ConflictResolution, ConflictType};
pub use discovery::AgentDiscovery;
pub use lifecycle::{AgentLifecycle, AgentStatus, StateTransition};

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use uuid::Uuid;

/// Unique agent identifier based on UUID v4.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// Generate a new random agent ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Parse from string representation.
    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Ok(Self(Uuid::parse_str(s)?))
    }

    /// Get the inner UUID.
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The kind of agent in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    /// Primary orchestrator agent.
    Main,
    /// Background worker agent.
    Worker,
    /// Long-running daemon agent.
    Daemon,
    /// Plugin-provided agent.
    Plugin,
}

/// Trust level assigned to a discovered agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Fully trusted, can modify shared resources.
    Full,
    /// Partially trusted, allowed operations are restricted.
    Partial,
    /// Limited trust, read-only access to shared state.
    Limited,
    /// Untrusted, isolated execution only.
    Untrusted,
}

impl TrustLevel {
    /// Returns whether the trust level allows writing shared resources.
    pub fn can_write_shared(&self) -> bool {
        matches!(self, TrustLevel::Full)
    }

    /// Returns whether the trust level allows reading shared state.
    pub fn can_read_shared(&self) -> bool {
        !matches!(self, TrustLevel::Untrusted)
    }
}

/// A capability advertised by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// Capability name (e.g., "file-management", "network-scan").
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Human-readable description.
    pub description: String,
}

impl Capability {
    /// Create a new capability.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: description.into(),
        }
    }
}

/// Communication endpoint for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endpoint {
    /// Unix domain socket at the given path.
    UnixSocket(PathBuf),
    /// TCP socket at the given address.
    TcpSocket(SocketAddr),
}

impl Endpoint {
    /// Returns the Unix socket path, if this is a UnixSocket endpoint.
    pub fn unix_path(&self) -> Option<&PathBuf> {
        match self {
            Endpoint::UnixSocket(p) => Some(p),
            _ => None,
        }
    }
}

/// Full information about a discovered agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Unique agent identifier.
    pub id: AgentId,
    /// The kind of agent.
    pub kind: AgentKind,
    /// Advertised capabilities.
    pub capabilities: Vec<Capability>,
    /// Communication endpoint.
    pub endpoint: Endpoint,
    /// Current status.
    pub status: AgentStatus,
    /// Assigned trust level.
    pub trust_level: TrustLevel,
}

impl AgentInfo {
    /// Create a new AgentInfo with defaults for status and trust.
    pub fn new(id: AgentId, kind: AgentKind, endpoint: Endpoint) -> Self {
        Self {
            id,
            kind,
            capabilities: Vec::new(),
            endpoint,
            status: AgentStatus::Starting,
            trust_level: TrustLevel::Untrusted,
        }
    }

    /// Check whether this agent has a given capability by name.
    pub fn has_capability(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c.name == name)
    }
}
