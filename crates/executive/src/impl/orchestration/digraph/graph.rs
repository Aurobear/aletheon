use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;
use tracing::{debug, info, warn};

use fabric::{Clock, Timer};
use kernel::chronos::SystemTimer;

use super::super::registry::AgentRegistry;
use super::edge::Edge;
use super::node::{Node, NodeKind, NodeStatus, OnExhausted};
use super::state::GraphState;

/// Join strategy for parallel fan-out.
#[derive(Debug, Clone)]
pub enum JoinStrategy {
    /// Wait for all branches.
    All,
    /// Wait for any branch.
    Any,
    /// Wait for first N branches.
    FirstN(usize),
    /// Wait for all with timeout.
    TimeoutAll(std::time::Duration),
}

// ---------------------------------------------------------------------------
// Serde mirror types — JoinStrategy holds Duration (non-serde), so we
// provide a serializable mirror for filesystem persistence.
// ---------------------------------------------------------------------------

/// Serde-friendly mirror of [`JoinStrategy`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JoinStrategyDef {
    All,
    Any,
    FirstN(usize),
    /// Timeout in milliseconds (avoids `Duration` in the serialized form).
    TimeoutAll {
        millis: u64,
    },
}

impl From<&JoinStrategy> for JoinStrategyDef {
    fn from(j: &JoinStrategy) -> Self {
        match j {
            JoinStrategy::All => JoinStrategyDef::All,
            JoinStrategy::Any => JoinStrategyDef::Any,
            JoinStrategy::FirstN(n) => JoinStrategyDef::FirstN(*n),
            JoinStrategy::TimeoutAll(d) => JoinStrategyDef::TimeoutAll {
                millis: d.as_millis() as u64,
            },
        }
    }
}

impl From<&JoinStrategyDef> for JoinStrategy {
    fn from(j: &JoinStrategyDef) -> Self {
        match j {
            JoinStrategyDef::All => JoinStrategy::All,
            JoinStrategyDef::Any => JoinStrategy::Any,
            JoinStrategyDef::FirstN(n) => JoinStrategy::FirstN(*n),
            JoinStrategyDef::TimeoutAll { millis } => {
                JoinStrategy::TimeoutAll(Duration::from_millis(*millis))
            }
        }
    }
}

/// A serializable, on-disk representation of a [`DiGraph`] workflow.
///
/// Nodes are stored as a sorted `Vec` (not the runtime `HashMap`) so JSON is
/// deterministic. `Node` / `Edge` already derive serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub id: String,
    pub entry_node: String,
    pub join_strategy: JoinStrategyDef,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// A directed acyclic graph workflow.
pub struct DiGraph {
    pub id: String,
    pub nodes: HashMap<String, Node>,
    pub edges: Vec<Edge>,
    pub entry_node: String,
    pub join_strategy: JoinStrategy,
}

impl DiGraph {
    pub fn new(id: &str, entry_node: &str) -> Self {
        Self {
            id: id.to_string(),
            nodes: HashMap::new(),
            edges: Vec::new(),
            entry_node: entry_node.to_string(),
            join_strategy: JoinStrategy::All,
        }
    }

    pub fn add_node(&mut self, node: Node) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn add_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
    }

    /// Get nodes that have no incoming edges (except entry).
    pub fn get_roots(&self) -> Vec<&str> {
        let has_incoming: HashSet<&str> = self.edges.iter().map(|e| e.to.as_str()).collect();
        self.nodes
            .keys()
            .filter(|id| !has_incoming.contains(id.as_str()) || **id == self.entry_node)
            .map(|s| s.as_str())
            .collect()
    }

    /// Get outgoing edges from a node.
    pub fn outgoing(&self, node_id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.from == node_id).collect()
    }

    /// Get incoming edges to a node.
    pub fn incoming(&self, node_id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.to == node_id).collect()
    }

    /// Topological sort for execution order.
    pub fn topological_sort(&self) -> Result<Vec<String>, String> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        for node_id in self.nodes.keys() {
            in_degree.entry(node_id.as_str()).or_insert(0);
        }
        for edge in &self.edges {
            *in_degree.entry(edge.to.as_str()).or_insert(0) += 1;
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut result = Vec::new();
        while let Some(node_id) = queue.pop_front() {
            result.push(node_id.to_string());
            for edge in self.outgoing(node_id) {
                let deg = in_degree
                    .get_mut(edge.to.as_str())
                    .ok_or_else(|| format!("edge target '{}' not found in graph", edge.to))?;
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(&edge.to);
                }
            }
        }

        if result.len() != self.nodes.len() {
            return Err("Graph has cycles".to_string());
        }

        Ok(result)
    }

    /// Execute the graph with the given agent registry.
    pub async fn execute(
        &self,
        registry: &AgentRegistry,
        initial_state: GraphState,
        clock: &dyn Clock,
    ) -> Result<GraphState, String> {
        let mut state = initial_state;
        let mut node_statuses: HashMap<String, NodeStatus> = HashMap::new();

        // Initialize all nodes as pending
        for node_id in self.nodes.keys() {
            node_statuses.insert(node_id.clone(), NodeStatus::Pending);
        }

        // Execute in topological order
        let order = self.topological_sort()?;

        for node_id in &order {
            let node = self
                .nodes
                .get(node_id)
                .ok_or_else(|| format!("node '{}' not found", node_id))?;

            // Check if all incoming edges are satisfied
            let incoming = self.incoming(node_id);
            let can_execute = incoming.iter().all(|edge| {
                let from_status = node_statuses.get(&edge.from);
                match from_status {
                    Some(NodeStatus::Completed) => edge.condition.evaluate(&state.data),
                    _ => false,
                }
            });

            if !can_execute && !incoming.is_empty() {
                debug!(
                    node = node_id.as_str(),
                    "Skipping node (dependencies not met)"
                );
                node_statuses.insert(node_id.clone(), NodeStatus::Skipped);
                state.record(node_id, "skipped", clock);
                continue;
            }

            // Execute node
            info!(node = node_id.as_str(), kind = ?node.kind, "Executing node");
            node_statuses.insert(node_id.clone(), NodeStatus::Running);

            let result = self.execute_node(node, registry, &mut state).await;

            match result {
                Ok(()) => {
                    node_statuses.insert(node_id.clone(), NodeStatus::Completed);
                    state.record(node_id, "completed", clock);
                    info!(node = node_id.as_str(), "Node completed");
                }
                Err(e) => {
                    // Handle retry
                    let retried = self
                        .handle_retry(node, registry, &mut state, &e, clock)
                        .await;
                    if retried {
                        node_statuses.insert(node_id.clone(), NodeStatus::Completed);
                        state.record(node_id, "completed_after_retry", clock);
                    } else {
                        match node.retry_policy.on_exhausted {
                            OnExhausted::FailGraph => {
                                node_statuses
                                    .insert(node_id.clone(), NodeStatus::Failed(e.clone()));
                                state.record(node_id, "failed", clock);
                                return Err(format!("Node '{}' failed: {}", node_id, e));
                            }
                            OnExhausted::SkipNode => {
                                node_statuses.insert(node_id.clone(), NodeStatus::Skipped);
                                state.record(node_id, "skipped_after_failure", clock);
                                warn!(node = node_id.as_str(), error = %e, "Node failed, skipping");
                            }
                            OnExhausted::Escalate => {
                                node_statuses
                                    .insert(node_id.clone(), NodeStatus::Failed(e.clone()));
                                state.record(node_id, "escalated", clock);
                                return Err(format!(
                                    "Node '{}' needs human intervention: {}",
                                    node_id, e
                                ));
                            }
                        }
                    }
                }
            }
        }

        Ok(state)
    }

    /// Capture this graph into a serializable definition for persistence.
    pub fn to_def(&self) -> WorkflowDef {
        let mut nodes: Vec<Node> = self.nodes.values().cloned().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        WorkflowDef {
            id: self.id.clone(),
            entry_node: self.entry_node.clone(),
            join_strategy: JoinStrategyDef::from(&self.join_strategy),
            nodes,
            edges: self.edges.clone(),
        }
    }

    /// Reconstruct an executable graph from a serialized definition.
    pub fn from_def(def: &WorkflowDef) -> Self {
        let mut g = DiGraph::new(&def.id, &def.entry_node);
        g.join_strategy = JoinStrategy::from(&def.join_strategy);
        for n in &def.nodes {
            g.add_node(n.clone());
        }
        for e in &def.edges {
            g.add_edge(e.clone());
        }
        g
    }

    async fn execute_node(
        &self,
        node: &Node,
        registry: &AgentRegistry,
        state: &mut GraphState,
    ) -> Result<(), String> {
        match &node.kind {
            NodeKind::Agent { agent_id } => {
                let agent = registry
                    .get(agent_id)
                    .await
                    .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;

                let agent = agent.write().await;
                let msg = fabric::Message::user(format!(
                    "Execute task for graph node '{}'. Current state: {:?}",
                    node.id, state.data
                ));

                let response = agent.on_message(msg).await.map_err(|e| e.to_string())?;

                state.set(
                    &format!("{}_result", node.id),
                    serde_json::json!(response.content),
                );
                Ok(())
            }
            NodeKind::Branch { condition } => {
                // Evaluate condition and set result in state
                let result = state
                    .data
                    .get(condition)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                state.set(&format!("{}_branch", node.id), result);
                Ok(())
            }
            NodeKind::HumanApproval { prompt } => {
                // In automated mode, auto-approve
                warn!(
                    node = node.id.as_str(),
                    prompt = prompt.as_str(),
                    "Auto-approving (human approval not implemented)"
                );
                state.set(&format!("{}_approved", node.id), serde_json::json!(true));
                Ok(())
            }
            NodeKind::SubGraph { graph_id } => {
                // Sub-graph execution not implemented yet
                warn!(
                    graph_id = graph_id.as_str(),
                    "Sub-graph execution not implemented"
                );
                Ok(())
            }
        }
    }

    async fn handle_retry(
        &self,
        node: &Node,
        registry: &AgentRegistry,
        state: &mut GraphState,
        _error: &str,
        _clock: &dyn Clock,
    ) -> bool {
        for attempt in 0..node.retry_policy.max_retries {
            warn!(
                node = node.id.as_str(),
                attempt = attempt,
                max = node.retry_policy.max_retries,
                "Retrying node"
            );

            SystemTimer
                .sleep(std::time::Duration::from_millis(
                    node.retry_policy.backoff_ms * (attempt as u64 + 1),
                ))
                .await;

            if self.execute_node(node, registry, state).await.is_ok() {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::edge::ConditionExpr;
    use super::super::node::{Node, NodeKind, RetryPolicy};
    use super::*;

    fn make_node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.to_string(),
            name: id.to_string(),
            kind,
            retry_policy: RetryPolicy::default(),
            timeout: None,
        }
    }

    #[test]
    fn test_topological_sort() {
        let mut graph = DiGraph::new("test", "a");
        graph.add_node(make_node(
            "a",
            NodeKind::Branch {
                condition: "x".into(),
            },
        ));
        graph.add_node(make_node(
            "b",
            NodeKind::Branch {
                condition: "y".into(),
            },
        ));
        graph.add_node(make_node(
            "c",
            NodeKind::Branch {
                condition: "z".into(),
            },
        ));

        graph.add_edge(Edge {
            from: "a".into(),
            to: "b".into(),
            condition: ConditionExpr::Always,
        });
        graph.add_edge(Edge {
            from: "a".into(),
            to: "c".into(),
            condition: ConditionExpr::Always,
        });
        graph.add_edge(Edge {
            from: "b".into(),
            to: "c".into(),
            condition: ConditionExpr::Always,
        });

        let order = graph.topological_sort().unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = DiGraph::new("test", "a");
        graph.add_node(make_node(
            "a",
            NodeKind::Branch {
                condition: "x".into(),
            },
        ));
        graph.add_node(make_node(
            "b",
            NodeKind::Branch {
                condition: "y".into(),
            },
        ));

        graph.add_edge(Edge {
            from: "a".into(),
            to: "b".into(),
            condition: ConditionExpr::Always,
        });
        graph.add_edge(Edge {
            from: "b".into(),
            to: "a".into(),
            condition: ConditionExpr::Always,
        });

        assert!(graph.topological_sort().is_err());
    }

    #[test]
    fn test_outgoing_edges() {
        let mut graph = DiGraph::new("test", "a");
        graph.add_node(make_node(
            "a",
            NodeKind::Branch {
                condition: "x".into(),
            },
        ));
        graph.add_node(make_node(
            "b",
            NodeKind::Branch {
                condition: "y".into(),
            },
        ));
        graph.add_node(make_node(
            "c",
            NodeKind::Branch {
                condition: "z".into(),
            },
        ));

        graph.add_edge(Edge {
            from: "a".into(),
            to: "b".into(),
            condition: ConditionExpr::Always,
        });
        graph.add_edge(Edge {
            from: "a".into(),
            to: "c".into(),
            condition: ConditionExpr::Always,
        });

        let outgoing = graph.outgoing("a");
        assert_eq!(outgoing.len(), 2);
    }

    // --- WorkflowDef round-trip tests ---

    fn sample_graph() -> DiGraph {
        let mut g = DiGraph::new("wf-1", "a");
        g.join_strategy = JoinStrategy::FirstN(2);
        g.add_node(make_node(
            "a",
            NodeKind::Branch {
                condition: "x".into(),
            },
        ));
        g.add_node(make_node(
            "b",
            NodeKind::Branch {
                condition: "y".into(),
            },
        ));
        g.add_edge(Edge {
            from: "a".into(),
            to: "b".into(),
            condition: ConditionExpr::Always,
        });
        g
    }

    #[test]
    fn workflow_def_round_trips_through_json() {
        let g = sample_graph();
        let def = WorkflowDef {
            id: g.id.clone(),
            entry_node: g.entry_node.clone(),
            join_strategy: JoinStrategyDef::from(&g.join_strategy),
            nodes: g.nodes.values().cloned().collect(),
            edges: g.edges.clone(),
        };
        let json = serde_json::to_string_pretty(&def).unwrap();
        let back: WorkflowDef = serde_json::from_str(&json).unwrap();
        let g2 = DiGraph::from_def(&back);

        assert_eq!(g2.id, "wf-1");
        assert_eq!(g2.entry_node, "a");
        assert_eq!(g2.nodes.len(), 2);
        assert_eq!(g2.edges.len(), 1);
        assert_eq!(g2.edges[0].from, "a");
        assert!(matches!(g2.join_strategy, JoinStrategy::FirstN(2)));
        assert_eq!(g2.topological_sort().unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn workflow_def_round_trips_via_to_from_def() {
        let g = sample_graph();
        let def = g.to_def();
        let g2 = DiGraph::from_def(&def);

        assert_eq!(g2.id, g.id);
        assert_eq!(g2.entry_node, g.entry_node);
        assert_eq!(g2.nodes.len(), g.nodes.len());
        assert_eq!(g2.edges.len(), g.edges.len());
        assert_eq!(
            g2.topological_sort().unwrap(),
            g.topological_sort().unwrap()
        );
    }

    #[test]
    fn join_strategy_def_round_trip_all_variants() {
        // All
        let js = JoinStrategy::All;
        let def = JoinStrategyDef::from(&js);
        let back = JoinStrategy::from(&def);
        assert!(matches!(back, JoinStrategy::All));

        // Any
        let js = JoinStrategy::Any;
        let def = JoinStrategyDef::from(&js);
        let back = JoinStrategy::from(&def);
        assert!(matches!(back, JoinStrategy::Any));

        // FirstN
        let js = JoinStrategy::FirstN(3);
        let def = JoinStrategyDef::from(&js);
        let back = JoinStrategy::from(&def);
        assert!(matches!(back, JoinStrategy::FirstN(3)));

        // TimeoutAll
        let js = JoinStrategy::TimeoutAll(Duration::from_millis(5000));
        let def = JoinStrategyDef::from(&js);
        let back = JoinStrategy::from(&def);
        assert!(matches!(back, JoinStrategy::TimeoutAll(d) if d == Duration::from_millis(5000)));
    }
}
