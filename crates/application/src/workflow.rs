//! Owner-neutral workflow graph contracts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeKind {
    Agent { agent_id: String },
    Branch { condition: String },
    HumanApproval { prompt: String },
    SubGraph { graph_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
    Skipped,
    WaitingApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
    pub retry_policy: RetryPolicy,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: usize,
    pub backoff_ms: u64,
    pub on_exhausted: OnExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OnExhausted {
    FailGraph,
    SkipNode,
    Escalate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub condition: ConditionExpr,
}

#[derive(Debug, Clone)]
pub enum JoinStrategy {
    All,
    Any,
    FirstN(usize),
    TimeoutAll(Duration),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JoinStrategyDef {
    All,
    Any,
    FirstN(usize),
    TimeoutAll { millis: u64 },
}

impl From<&JoinStrategy> for JoinStrategyDef {
    fn from(strategy: &JoinStrategy) -> Self {
        match strategy {
            JoinStrategy::All => Self::All,
            JoinStrategy::Any => Self::Any,
            JoinStrategy::FirstN(count) => Self::FirstN(*count),
            JoinStrategy::TimeoutAll(duration) => Self::TimeoutAll {
                millis: duration.as_millis() as u64,
            },
        }
    }
}

impl From<&JoinStrategyDef> for JoinStrategy {
    fn from(strategy: &JoinStrategyDef) -> Self {
        match strategy {
            JoinStrategyDef::All => Self::All,
            JoinStrategyDef::Any => Self::Any,
            JoinStrategyDef::FirstN(count) => Self::FirstN(*count),
            JoinStrategyDef::TimeoutAll { millis } => {
                Self::TimeoutAll(Duration::from_millis(*millis))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub id: String,
    pub entry_node: String,
    pub join_strategy: JoinStrategyDef,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionExpr {
    Always,
    Equals(String, serde_json::Value),
    Exists(String),
    IsTruthy(String),
}

impl ConditionExpr {
    pub fn evaluate(&self, data: &HashMap<String, serde_json::Value>) -> bool {
        match self {
            Self::Always => true,
            Self::Equals(key, expected) => data.get(key) == Some(expected),
            Self::Exists(key) => data.contains_key(key),
            Self::IsTruthy(key) => data.get(key).is_some_and(|value| match value {
                serde_json::Value::Bool(value) => *value,
                serde_json::Value::Number(value) => {
                    value.as_f64().is_some_and(|number| number != 0.0)
                }
                serde_json::Value::String(value) => !value.is_empty(),
                serde_json::Value::Array(value) => !value.is_empty(),
                serde_json::Value::Object(value) => !value.is_empty(),
                serde_json::Value::Null => false,
            }),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphState {
    pub data: HashMap<String, serde_json::Value>,
    pub log: Vec<LogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub node_id: String,
    pub status: String,
    pub timestamp: String,
}

impl GraphState {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            log: Vec::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: serde_json::Value) {
        self.data.insert(key.to_string(), value);
    }

    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.data.get(key)
    }

    pub fn record(&mut self, node_id: &str, status: &str, clock: &dyn ::contracts::Clock) {
        self.log.push(LogEntry {
            node_id: node_id.to_string(),
            status: status.to_string(),
            timestamp: ::contracts::wall_to_datetime(clock.wall_now()).to_rfc3339(),
        });
    }
}

impl Default for GraphState {
    fn default() -> Self {
        Self::new()
    }
}
