//! Agent fork — lightweight context-sharing sub-agent.
//!
//! An `AgentFork` is a short-lived child spawned from a parent `AgentProcess`
//! via a `ForkDirective`. It inherits a fraction of the parent's token budget
//! and publishes a completion envelope when done.

use std::sync::Arc;

use base::agent::Pid;
use base::envelope::{Endpoint, Envelope, Pattern, Payload, Target};
use base::CommunicationBus;
use base::{ForkDirective, ForkResult, Priority};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::budget::TokenBudget;

// ---------------------------------------------------------------------------
// ForkState
// ---------------------------------------------------------------------------

/// Lifecycle state of an agent fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkState {
    Running,
    Completed,
    Failed(String),
}

// ---------------------------------------------------------------------------
// AgentFork
// ---------------------------------------------------------------------------

/// A lightweight sub-agent forked from a parent process.
pub struct AgentFork {
    pub pid: Pid,
    pub parent_pid: Pid,
    pub directive: String,
    pub budget: TokenBudget,
    pub state: ForkState,
    pub result: Option<ForkResult>,
    pub inbox: Option<mpsc::Receiver<Envelope>>,
    bus: Arc<CommunicationBus>,
}

// ---------------------------------------------------------------------------
// Completion payload (serialisable, sent as event payload)
// ---------------------------------------------------------------------------

/// Payload published when an `AgentFork` completes or fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentForkCompletedPayload {
    pub pid: u64,
    pub parent_pid: u64,
    pub success: bool,
}

// ---------------------------------------------------------------------------
// AgentFork implementation
// ---------------------------------------------------------------------------

impl AgentFork {
    /// Create a new fork from the given parent and directive.
    ///
    /// `parent_remaining` is the parent's *remaining* token budget at the
    /// moment of fork; the fork receives `parent_remaining * budget_ratio`.
    pub fn new(
        parent_pid: Pid,
        directive: ForkDirective,
        parent_remaining: u32,
        bus: Arc<CommunicationBus>,
    ) -> Self {
        let pid = Pid::new();
        let max_tokens = (parent_remaining as f64 * directive.budget_ratio) as u32;
        Self {
            pid,
            parent_pid,
            directive: directive.prompt,
            budget: TokenBudget::new(max_tokens),
            state: ForkState::Running,
            result: None,
            inbox: None,
            bus,
        }
    }

    /// Mark the fork as completed with the produced output and token usage.
    pub fn complete(&mut self, output: String, tokens_consumed: u32) {
        self.state = ForkState::Completed;
        self.result = Some(ForkResult {
            pid: self.pid,
            parent_pid: self.parent_pid,
            output,
            tokens_consumed,
            success: true,
        });
        self.publish_completed(true);
    }

    /// Mark the fork as failed with an error description.
    pub fn fail(&mut self, error: String) {
        self.state = ForkState::Failed(error.clone());
        self.result = Some(ForkResult {
            pid: self.pid,
            parent_pid: self.parent_pid,
            output: error,
            tokens_consumed: self.budget.total_consumed() as u32,
            success: false,
        });
        self.publish_completed(false);
    }

    /// Returns `true` while the fork has not yet completed or failed.
    pub fn is_running(&self) -> bool {
        self.state == ForkState::Running
    }

    // -- internal -----------------------------------------------------------

    /// Publish an `AgentForkCompleted` envelope onto the bus.
    fn publish_completed(&self, success: bool) {
        let payload = AgentForkCompletedPayload {
            pid: self.pid.as_u64(),
            parent_pid: self.parent_pid.as_u64(),
            success,
        };
        let envelope = Envelope::new(
            Endpoint::System,
            Target::Broadcast,
            Pattern::Publish,
            Payload::Json(serde_json::to_value(&payload).unwrap_or_default()),
        )
        .with_priority(Priority::Normal);
        let bus = self.bus.clone();
        tokio::spawn(async move {
            if let Err(e) = bus.publish(envelope).await {
                tracing::warn!(
                    source = "agent_fork",
                    error = %e,
                    "Failed to publish AgentForkCompleted envelope"
                );
            }
        });
    }
}
