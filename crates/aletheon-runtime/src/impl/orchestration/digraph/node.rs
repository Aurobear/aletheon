use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Kind of node in the workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeKind {
    /// Execute an agent.
    Agent { agent_id: String },
    /// Conditional branch.
    Branch { condition: String },
    /// Human approval gate.
    HumanApproval { prompt: String },
    /// Sub-graph execution.
    SubGraph { graph_id: String },
}

/// Status of a node execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
    Skipped,
    WaitingApproval,
}

/// A node in the workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
    pub retry_policy: RetryPolicy,
    pub timeout: Option<Duration>,
}

/// Retry policy for a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: usize,
    pub backoff_ms: u64,
    pub on_exhausted: OnExhausted,
}

/// What to do when retries are exhausted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OnExhausted {
    FailGraph,
    SkipNode,
    Escalate,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff_ms: 1000,
            on_exhausted: OnExhausted::FailGraph,
        }
    }
}
