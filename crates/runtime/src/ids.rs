//! Runtime identity owner (RA-01).
//!
//! These are the canonical aggregate IDs **assigned by the Runtime** when it
//! creates/advances Session/Turn/AgentRun.  A caller never mints a canonical
//! aggregate ID: `CreateSessionCommand` / `StartTurnCommand` /
//! `SpawnAgentRunCommand` accept correlation references, not new aggregate
//! IDs.  `UiOverlayId` lives in the Presentation layer and never enters the
//! Runtime.  Caller correlation and `LegacySessionAlias` carry no authority.

use serde::{Deserialize, Serialize};

/// Runtime-assigned session identity.  Created by the Runtime on
/// `CreateSession`; a client consumes the receipt, never constructs this.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// Runtime-assigned turn identity.  Minted by the Runtime reducer when a turn
/// starts; the client only ever observes it in a `TurnStarted`/`TurnSettled`
/// event.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnId(pub String);

/// Runtime-assigned child agent run identity.  Minted by the Runtime when a
/// delegate is spawned.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentRunId(pub String);

/// Runtime-assigned generation/epoch for a session's writer generation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Generation(pub u64);
