//! Workflow definition store: serialize a `DiGraph` DAG to disk and reload/run it.

use std::path::{Path, PathBuf};

use super::digraph::graph::{DiGraph, WorkflowDef};
use super::digraph::state::GraphState;
use super::registry::AgentRegistry;

// ---------------------------------------------------------------------------
// Filesystem workflow store
// ---------------------------------------------------------------------------

/// A filesystem-backed store of named workflow definitions (one JSON file each).
///
/// Mirrors the `~/.aletheon/` convention from `fabric::paths`; the default dir is
/// `~/.aletheon/workflows`.
pub struct WorkflowStore {
    dir: PathBuf,
}

impl WorkflowStore {
    /// Open (creating if needed) a store rooted at `dir`.
    pub fn new(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// The default store directory: `~/.aletheon/workflows`.
    pub fn default_dir() -> PathBuf {
        fabric::paths::config_dir().join("workflows")
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    /// Persist `graph` under `name` (overwrites an existing definition).
    pub fn save(&self, name: &str, graph: &DiGraph) -> anyhow::Result<()> {
        let def = graph.to_def();
        let json = serde_json::to_string_pretty(&def)?;
        std::fs::write(self.path_for(name), json)?;
        Ok(())
    }

    /// Load and reconstruct the executable graph stored under `name`.
    pub fn load(&self, name: &str) -> anyhow::Result<DiGraph> {
        let text = std::fs::read_to_string(self.path_for(name))
            .map_err(|e| anyhow::anyhow!("workflow '{name}' not found: {e}"))?;
        let def: WorkflowDef = serde_json::from_str(&text)?;
        Ok(DiGraph::from_def(&def))
    }

    /// List saved workflow names (sorted, `.json` extension stripped).
    pub fn list(&self) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        let dir_read = match std::fs::read_dir(&self.dir) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        for entry in dir_read {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    /// Delete a saved workflow by name.
    pub fn delete(&self, name: &str) -> anyhow::Result<()> {
        let path = self.path_for(name);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Load the workflow `name` and execute it against `registry`.
    pub async fn run(
        &self,
        name: &str,
        registry: &AgentRegistry,
        initial_state: GraphState,
    ) -> anyhow::Result<GraphState> {
        let graph = self.load(name)?;
        graph
            .execute(registry, initial_state)
            .await
            .map_err(|e| anyhow::anyhow!("workflow '{name}' execution failed: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::orchestration::digraph::edge::ConditionExpr;
    use crate::r#impl::orchestration::digraph::graph::{DiGraph, JoinStrategy};
    use crate::r#impl::orchestration::digraph::node::{Node, NodeKind, RetryPolicy};
    use crate::r#impl::orchestration::digraph::Edge;

    fn sample_node(id: &str, cond: &str) -> Node {
        Node {
            id: id.to_string(),
            name: id.to_string(),
            kind: NodeKind::Branch {
                condition: cond.to_string(),
            },
            retry_policy: RetryPolicy::default(),
            timeout: None,
        }
    }

    fn sample_graph() -> DiGraph {
        let mut g = DiGraph::new("wf-1", "a");
        g.join_strategy = JoinStrategy::FirstN(2);
        g.add_node(sample_node("a", "x"));
        g.add_node(sample_node("b", "y"));
        g.add_edge(Edge {
            from: "a".into(),
            to: "b".into(),
            condition: ConditionExpr::Always,
        });
        g
    }

    #[test]
    fn store_saves_lists_and_reloads_losslessly() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();

        assert!(store.list().unwrap().is_empty());

        store.save("greet", &sample_graph()).unwrap();
        store.save("deploy", &sample_graph()).unwrap();

        assert_eq!(
            store.list().unwrap(),
            vec!["deploy".to_string(), "greet".to_string()]
        );

        let g = store.load("greet").unwrap();
        assert_eq!(g.id, "wf-1");
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.topological_sort().unwrap(), vec!["a", "b"]);

        assert!(dir.path().join("greet.json").exists());
    }

    #[test]
    fn store_delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();

        store.save("test", &sample_graph()).unwrap();
        assert_eq!(store.list().unwrap(), vec!["test"]);

        store.delete("test").unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(!dir.path().join("test.json").exists());
    }

    #[test]
    fn store_delete_nonexistent_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();
        assert!(store.delete("nonexistent").is_ok());
    }

    #[test]
    fn store_load_missing_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();
        assert!(store.load("nonexistent").is_err());
    }

    #[tokio::test]
    async fn run_saved_workflow_reproduces_direct_execution() {
        use crate::r#impl::orchestration::digraph::state::GraphState;

        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();
        let registry = AgentRegistry::new();

        let direct = sample_graph()
            .execute(&registry, GraphState::new())
            .await
            .unwrap();
        let direct_trace: Vec<(String, String)> = direct
            .log
            .iter()
            .map(|e| (e.node_id.clone(), e.status.clone()))
            .collect();

        store.save("wf", &sample_graph()).unwrap();
        let replayed = store.run("wf", &registry, GraphState::new()).await.unwrap();
        let replayed_trace: Vec<(String, String)> = replayed
            .log
            .iter()
            .map(|e| (e.node_id.clone(), e.status.clone()))
            .collect();

        assert_eq!(
            direct_trace, replayed_trace,
            "reloaded run must reproduce the direct run"
        );
        assert!(!replayed_trace.is_empty());
    }
}
