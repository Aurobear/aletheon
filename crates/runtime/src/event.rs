//! Runtime typed events (RA-01).
//!
//! Events the Runtime emits.  Terminal is authoritative here: a client/projection
//! must never infer terminal from text, spinner, EOF or socket state — it may
//! only consume a typed `TurnSettled` event.

use crate::ids::{AgentRunId, SessionId, TurnId};
use serde::{Deserialize, Serialize};

/// Versioned AgentStream envelope. The physical EventSpine position remains
/// the journal's global cursor; this sequence is the logical sequence within
/// the Runtime Agent writer and is therefore carried in the event payload.
pub const AGENT_STREAM_SCHEMA_VERSION: u32 = 1;
pub const TURN_STREAM_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStreamEvent {
    pub schema_version: u32,
    pub sequence: u64,
    pub digest: String,
    pub event: Box<RuntimeEvent>,
}

impl AgentStreamEvent {
    pub fn new(sequence: u64, event: RuntimeEvent) -> Self {
        let encoded = serde_json::to_vec(&(AGENT_STREAM_SCHEMA_VERSION, sequence, &event))
            .unwrap_or_default();
        let digest = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(encoded))
        };
        Self {
            schema_version: AGENT_STREAM_SCHEMA_VERSION,
            sequence,
            digest,
            event: Box::new(event),
        }
    }

    /// Read a legacy unwrapped RuntimeEvent without weakening the new stream
    /// validation. Used only for additive replay of pre-envelope data.
    pub fn legacy(sequence: u64, event: RuntimeEvent) -> Self {
        Self::new(sequence.max(1), event)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != AGENT_STREAM_SCHEMA_VERSION {
            return Err(format!(
                "unsupported AgentStream schema version {}",
                self.schema_version
            ));
        }
        if self.sequence == 0 {
            return Err("AgentStream sequence must be nonzero".into());
        }
        let expected = Self::new(self.sequence, (*self.event).clone()).digest;
        if self.digest != expected {
            return Err("AgentStream payload digest mismatch".into());
        }
        Ok(())
    }

    pub fn into_event(self) -> RuntimeEvent {
        *self.event
    }
}

/// Versioned TurnStream envelope. This is intentionally a distinct type from
/// AgentStream: progress/mailbox and turn terminal events have different
/// replay and ownership semantics even when their wire shapes overlap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStreamEvent {
    pub schema_version: u32,
    pub sequence: u64,
    pub digest: String,
    pub event: Box<RuntimeEvent>,
}

impl TurnStreamEvent {
    pub fn new(sequence: u64, event: RuntimeEvent) -> Self {
        let encoded =
            serde_json::to_vec(&(TURN_STREAM_SCHEMA_VERSION, sequence, &event)).unwrap_or_default();
        let digest = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(encoded))
        };
        Self {
            schema_version: TURN_STREAM_SCHEMA_VERSION,
            sequence,
            digest,
            event: Box::new(event),
        }
    }

    pub fn legacy(sequence: u64, event: RuntimeEvent) -> Self {
        Self::new(sequence.max(1), event)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != TURN_STREAM_SCHEMA_VERSION {
            return Err(format!(
                "unsupported TurnStream schema version {}",
                self.schema_version
            ));
        }
        if self.sequence == 0 {
            return Err("TurnStream sequence must be nonzero".into());
        }
        if !matches!(
            self.event.as_ref(),
            RuntimeEvent::TurnStarted { .. }
                | RuntimeEvent::TurnSettled { .. }
                | RuntimeEvent::TurnObserved { .. }
        ) {
            return Err("TurnStream contains a non-turn event".into());
        }
        let expected = Self::new(self.sequence, (*self.event).clone()).digest;
        if self.digest != expected {
            return Err("TurnStream payload digest mismatch".into());
        }
        Ok(())
    }

    pub fn into_event(self) -> RuntimeEvent {
        *self.event
    }
}

/// A turn reached a terminal state.  This is the only terminal source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnTerminal {
    Completed,
    Interrupted,
    Failed { message: String },
}

/// Durable delivery state for a Runtime-owned Agent mailbox receipt.  This is
/// deliberately distinct from the public Fabric mailbox schema: it records
/// the Runtime lifecycle observation and never carries user message content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentMailboxDelivery {
    Pending,
    Delivered,
    Rejected,
}

/// Typed Runtime events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeEvent {
    SessionCreated {
        session: SessionId,
    },
    /// Admission receipt written before the host SQL projection.  It proves
    /// that Runtime assigned the AgentRun identity; it is not a process start.
    AgentRunAccepted {
        session: SessionId,
        agent_run: AgentRunId,
        generation: crate::ids::Generation,
        #[serde(default)]
        backend: Option<String>,
    },
    TurnStarted {
        session: SessionId,
        turn: TurnId,
    },
    TurnSettled {
        session: SessionId,
        turn: TurnId,
        terminal: TurnTerminal,
    },
    TurnObserved {
        session: SessionId,
        turn: TurnId,
        kind: String,
    },
    AgentRunStarted {
        session: SessionId,
        agent_run: AgentRunId,
        /// Optional for replay compatibility with the first Runtime Agent
        /// event schema. New writers always persist the generation.
        #[serde(default)]
        generation: Option<crate::ids::Generation>,
        /// Stable backend identity selected at admission. This is metadata,
        /// never an authority supplied by the backend receipt.
        #[serde(default)]
        backend: Option<String>,
    },
    AgentRunSettled {
        session: SessionId,
        agent_run: AgentRunId,
        terminal: TurnTerminal,
        /// Optional for replay compatibility with pre-generation events.
        #[serde(default)]
        generation: Option<crate::ids::Generation>,
    },
    /// A mailbox delivery observation. The payload intentionally carries no
    /// user content; runtime progress and public Agent messages use distinct
    /// schemas and the host mailbox remains the delivery authority.
    AgentRunMessage {
        session: SessionId,
        agent_run: AgentRunId,
        kind: String,
        #[serde(default)]
        correlation: Option<String>,
    },
    /// Recovery decisions are Runtime-owned evidence.  The host projection
    /// may retain the richer Fabric receipt, but it cannot be the authority.
    AgentRunRecovery {
        session: SessionId,
        agent_run: AgentRunId,
        generation: crate::ids::Generation,
        decision: String,
    },
    /// Mailbox delivery receipt with no message content.  Content remains in
    /// the host projection; Runtime owns only delivery lifecycle/fencing.
    AgentRunMailbox {
        session: SessionId,
        agent_run: AgentRunId,
        delivery_id: String,
        kind: String,
        #[serde(default)]
        correlation: Option<String>,
        delivery: AgentMailboxDelivery,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> RuntimeEvent {
        RuntimeEvent::AgentRunStarted {
            session: SessionId("s1".into()),
            agent_run: AgentRunId("run-1".into()),
            generation: Some(crate::ids::Generation(1)),
            backend: Some("native".into()),
        }
    }

    #[test]
    fn agent_stream_event_has_verified_digest_and_sequence() {
        let event = AgentStreamEvent::new(7, started());
        assert_eq!(event.schema_version, AGENT_STREAM_SCHEMA_VERSION);
        assert_eq!(event.sequence, 7);
        assert!(event.validate().is_ok());

        let mut tampered = event.clone();
        tampered.sequence = 8;
        assert!(tampered.validate().is_err());
        tampered = event;
        tampered.schema_version = AGENT_STREAM_SCHEMA_VERSION + 1;
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn turn_stream_rejects_agent_events() {
        let event = TurnStreamEvent::new(
            1,
            RuntimeEvent::AgentRunMessage {
                session: SessionId("s1".into()),
                agent_run: AgentRunId("run-1".into()),
                kind: "progress".into(),
                correlation: None,
            },
        );
        assert!(event.validate().is_err());
    }
}
