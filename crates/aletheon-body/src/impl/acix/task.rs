use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::r#impl::driver::types::{Key, ScrollDirection};

use super::aci::Aci;

/// Task node status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
    Skipped,
}

/// Task action
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskAction {
    /// ACI actions
    Click { x: i32, y: i32 },
    Type { text: String },
    Hotkey { keys: Vec<String> },
    Screenshot,
    Observe,
    Scroll { x: i32, y: i32, direction: ScrollDirection, amount: i32 },
    /// Compound action
    Composite(Vec<TaskAction>),
    /// Wait (ms)
    Wait(u64),
}

/// Task node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: usize,
    pub description: String,
    pub action: TaskAction,
    pub dependencies: Vec<usize>,
    pub status: TaskStatus,
}

/// DAG task graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGraph {
    pub nodes: Vec<TaskNode>,
    pub goal: String,
}

impl TaskGraph {
    /// Create a new task graph
    pub fn new(goal: &str) -> Self {
        Self {
            nodes: Vec::new(),
            goal: goal.to_string(),
        }
    }

    /// Add a node
    pub fn add_node(&mut self, description: &str, action: TaskAction, deps: Vec<usize>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(TaskNode {
            id,
            description: description.to_string(),
            action,
            dependencies: deps,
            status: TaskStatus::Pending,
        });
        id
    }

    /// Set the status of a node
    pub fn set_status(&mut self, id: usize, status: TaskStatus) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.status = status;
        }
    }

    /// Topological sort, returns execution order
    pub fn topological_sort(&self) -> Result<Vec<usize>, String> {
        let n = self.nodes.len();
        let mut in_degree = vec![0; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

        for node in &self.nodes {
            for &dep in &node.dependencies {
                adj[dep].push(node.id);
                in_degree[node.id] += 1;
            }
        }

        let mut queue: std::collections::VecDeque<usize> = (0..n)
            .filter(|&i| in_degree[i] == 0)
            .collect();

        let mut order = Vec::new();
        while let Some(id) = queue.pop_front() {
            order.push(id);
            for &next in &adj[id] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push_back(next);
                }
            }
        }

        if order.len() == n {
            Ok(order)
        } else {
            Err("Cycle detected in task graph".into())
        }
    }

    /// Get ready nodes (all dependencies completed)
    pub fn ready_nodes(&self) -> Vec<&TaskNode> {
        self.nodes.iter()
            .filter(|n| n.status == TaskStatus::Pending)
            .filter(|n| n.dependencies.iter().all(|&dep| self.nodes[dep].status == TaskStatus::Completed))
            .collect()
    }

    /// Mark node as completed
    pub fn complete(&mut self, id: usize) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.status = TaskStatus::Completed;
        }
    }

    /// Mark node as failed
    pub fn fail(&mut self, id: usize, error: &str) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.status = TaskStatus::Failed(error.to_string());
        }
    }

    /// Whether all tasks are complete
    pub fn is_complete(&self) -> bool {
        self.nodes.iter().all(|n| matches!(n.status, TaskStatus::Completed | TaskStatus::Skipped))
    }

    /// Whether any task has failed
    pub fn has_failures(&self) -> bool {
        self.nodes.iter().any(|n| matches!(n.status, TaskStatus::Failed(_)))
    }
}

// ---------------------------------------------------------------------------
// LLM-driven decomposition
// ---------------------------------------------------------------------------

/// Minimal async trait for task decomposition via LLM.
///
/// Defined here (in argos-acix) to avoid a circular dependency with argos-core.
/// The caller (argos-core) implements this trait by forwarding to `LlmProvider`.
#[async_trait]
pub trait TaskDecomposer: Send + Sync {
    /// Send a prompt to the LLM and return the text response.
    async fn complete_text(&self, prompt: &str) -> anyhow::Result<String>;
}

/// LLM decomposition result (structured output)
#[derive(Serialize, Deserialize)]
struct DecompositionResult {
    nodes: Vec<DecompositionNode>,
}

#[derive(Serialize, Deserialize)]
struct DecompositionNode {
    description: String,
    action: String,
    params: serde_json::Value,
    depends_on: Vec<usize>,
}

/// Task manager: LLM-driven DAG generation
pub struct TaskManager;

impl TaskManager {
    /// Decompose a natural language goal into a task graph (simplified, no LLM)
    pub fn decompose(goal: &str) -> TaskGraph {
        let mut graph = TaskGraph::new(goal);
        for sentence in goal.split(|c: char| c == '.' || c == ',' || c == ';' || c == '\n') {
            let s = sentence.trim();
            if !s.is_empty() {
                graph.add_node(s, TaskAction::Observe, vec![]);
            }
        }
        graph
    }

    /// Alias for the simple (non-LLM) decomposition, matching the spec naming.
    pub fn decompose_simple(goal: &str) -> TaskGraph {
        Self::decompose(goal)
    }

    /// LLM-driven task decomposition.
    pub async fn decompose_with_llm(
        goal: &str,
        decomposer: &dyn TaskDecomposer,
    ) -> anyhow::Result<TaskGraph> {
        let prompt = format!(
            r#"分解以下任务为步骤。返回 JSON 格式:
{{"nodes": [{{"description": "...", "action": "click|type|screenshot|observe|hotkey|scroll|wait", "params": {{}}, "depends_on": [0]}}]}}

任务: {goal}

可用动作:
- click: {{"x": 100, "y": 200}}
- type: {{"text": "hello"}}
- screenshot: {{}}
- observe: {{}}
- hotkey: {{"keys": ["ctrl", "c"]}}
- scroll: {{"x": 100, "y": 200, "direction": "down", "amount": 3}}
- wait: {{"ms": 1000}}

只返回 JSON，不要其他文字。"#
        );

        let text = decomposer.complete_text(&prompt).await?;

        // Strip markdown code fences if present
        let cleaned = strip_code_fences(&text);

        let result: DecompositionResult = serde_json::from_str(cleaned.as_ref())
            .map_err(|e| anyhow::anyhow!("Failed to parse LLM decomposition JSON: {e}\nRaw response: {text}"))?;

        let mut graph = TaskGraph::new(goal);

        for node in &result.nodes {
            let action = match node.action.as_str() {
                "click" => {
                    let x = node.params["x"].as_i64().unwrap_or(0) as i32;
                    let y = node.params["y"].as_i64().unwrap_or(0) as i32;
                    TaskAction::Click { x, y }
                }
                "type" => {
                    let text = node.params["text"].as_str().unwrap_or("").to_string();
                    TaskAction::Type { text }
                }
                "screenshot" => TaskAction::Screenshot,
                "observe" => TaskAction::Observe,
                "hotkey" => {
                    let keys = node.params["keys"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                        .unwrap_or_default();
                    TaskAction::Hotkey { keys }
                }
                "scroll" => {
                    let x = node.params["x"].as_i64().unwrap_or(0) as i32;
                    let y = node.params["y"].as_i64().unwrap_or(0) as i32;
                    let direction = match node.params["direction"].as_str().unwrap_or("down") {
                        "up" => ScrollDirection::Up,
                        "left" => ScrollDirection::Left,
                        "right" => ScrollDirection::Right,
                        _ => ScrollDirection::Down,
                    };
                    let amount = node.params["amount"].as_i64().unwrap_or(3) as i32;
                    TaskAction::Scroll { x, y, direction, amount }
                }
                "wait" => {
                    let ms = node.params["ms"].as_u64().unwrap_or(1000);
                    TaskAction::Wait(ms)
                }
                _ => TaskAction::Observe,
            };
            graph.add_node(&node.description, action, node.depends_on.clone());
        }

        Ok(graph)
    }

    /// Replan after a failure: feed completed/failed context to LLM and get a new graph.
    pub async fn replan_with_llm(
        graph: &TaskGraph,
        failed_id: usize,
        error: &str,
        decomposer: &dyn TaskDecomposer,
    ) -> anyhow::Result<TaskGraph> {
        let completed: Vec<String> = graph.nodes.iter()
            .filter(|n| n.status == TaskStatus::Completed)
            .map(|n| n.description.clone())
            .collect();
        let failed_desc = graph.nodes.get(failed_id)
            .map(|n| n.description.clone())
            .unwrap_or_default();

        let prompt = format!(
            r#"任务执行失败，请重新规划。

原始任务: {}
已完成步骤: {:?}
失败步骤: {}
错误: {}

返回新的任务分解 JSON，格式同前。只返回 JSON，不要其他文字。"#,
            graph.goal, completed, failed_desc, error
        );

        Self::decompose_with_llm(&prompt, decomposer).await
    }
}

/// Strip markdown code fences (```json ... ```) from LLM output.
fn strip_code_fences(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.starts_with("```") {
        // Find the end of the opening fence line
        let after_open = trimmed.find('\n').map(|i| i + 1).unwrap_or(trimmed.len());
        // Find the closing fence
        let body = &trimmed[after_open..];
        if let Some(close_pos) = body.rfind("```") {
            return body[..close_pos].trim();
        }
    }
    trimmed
}

// ---------------------------------------------------------------------------
// Key parsing helper
// ---------------------------------------------------------------------------

/// Parse a string key name (e.g., "ctrl", "shift", "a") into a `Key` enum.
fn parse_key(s: &str) -> Option<Key> {
    match s.to_lowercase().as_str() {
        "ctrl" | "control" => Some(Key::Ctrl),
        "alt" => Some(Key::Alt),
        "shift" => Some(Key::Shift),
        "super" | "win" | "meta" => Some(Key::Super),
        "enter" | "return" => Some(Key::Enter),
        "space" => Some(Key::Space),
        "tab" => Some(Key::Tab),
        "escape" | "esc" => Some(Key::Escape),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" | "page_up" => Some(Key::PageUp),
        "pagedown" | "page_down" => Some(Key::PageDown),
        "f1" => Some(Key::F1),
        "f2" => Some(Key::F2),
        "f3" => Some(Key::F3),
        "f4" => Some(Key::F4),
        "f5" => Some(Key::F5),
        "f6" => Some(Key::F6),
        "f7" => Some(Key::F7),
        "f8" => Some(Key::F8),
        "f9" => Some(Key::F9),
        "f10" => Some(Key::F10),
        "f11" => Some(Key::F11),
        "f12" => Some(Key::F12),
        "a" => Some(Key::A),
        "b" => Some(Key::B),
        "c" => Some(Key::C),
        "d" => Some(Key::D),
        "e" => Some(Key::E),
        "f" => Some(Key::F),
        "g" => Some(Key::G),
        "h" => Some(Key::H),
        "i" => Some(Key::I),
        "j" => Some(Key::J),
        "k" => Some(Key::K),
        "l" => Some(Key::L),
        "m" => Some(Key::M),
        "n" => Some(Key::N),
        "o" => Some(Key::O),
        "p" => Some(Key::P),
        "q" => Some(Key::Q),
        "r" => Some(Key::R),
        "s" => Some(Key::S),
        "t" => Some(Key::T),
        "u" => Some(Key::U),
        "v" => Some(Key::V),
        "w" => Some(Key::W),
        "x" => Some(Key::X),
        "y" => Some(Key::Y),
        "z" => Some(Key::Z),
        "0" => Some(Key::Num0),
        "1" => Some(Key::Num1),
        "2" => Some(Key::Num2),
        "3" => Some(Key::Num3),
        "4" => Some(Key::Num4),
        "5" => Some(Key::Num5),
        "6" => Some(Key::Num6),
        "7" => Some(Key::Num7),
        "8" => Some(Key::Num8),
        "9" => Some(Key::Num9),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// TaskWorker
// ---------------------------------------------------------------------------

/// Task executor: runs nodes against an Aci instance.
pub struct TaskWorker {
    aci: Arc<Aci>,
}

impl TaskWorker {
    pub fn new(aci: Arc<Aci>) -> Self {
        Self { aci }
    }

    /// Execute a single task node.
    ///
    /// Returns a boxed future to support recursive Composite actions.
    pub fn execute_node<'a>(
        &'a self,
        node: &'a TaskNode,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + 'a>> {
        Box::pin(async move {
            match &node.action {
                TaskAction::Click { x, y } => {
                    self.aci.click(*x, *y)?;
                    Ok(format!("Clicked ({x}, {y})"))
                }
                TaskAction::Type { text } => {
                    self.aci.type_text(text)?;
                    Ok(format!("Typed: {text}"))
                }
                TaskAction::Screenshot => {
                    let img = self.aci.screenshot()?;
                    Ok(format!("Screenshot: {}x{}", img.width, img.height))
                }
                TaskAction::Observe => {
                    let obs = self.aci.smart_observe()?;
                    Ok(format!("Observed: {obs:?}"))
                }
                TaskAction::Hotkey { keys } => {
                    let parsed: Vec<Key> = keys.iter()
                        .filter_map(|k| parse_key(k))
                        .collect();
                    if parsed.len() != keys.len() {
                        anyhow::bail!("Failed to parse some hotkey names: {:?}", keys);
                    }
                    self.aci.hotkey(&parsed)?;
                    Ok(format!("Hotkey: {}", keys.join("+")))
                }
                TaskAction::Scroll { x, y, direction, amount } => {
                    self.aci.scroll(*x, *y, *direction, *amount)?;
                    Ok(format!("Scroll {direction:?} at ({x}, {y}) x{amount}"))
                }
                TaskAction::Composite(actions) => {
                    for action in actions {
                        let sub_node = TaskNode {
                            id: 0,
                            description: String::new(),
                            action: action.clone(),
                            dependencies: vec![],
                            status: TaskStatus::Pending,
                        };
                        self.execute_node(&sub_node).await?;
                    }
                    Ok("Composite executed".into())
                }
                TaskAction::Wait(ms) => {
                    tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
                    Ok(format!("Waited {ms}ms"))
                }
            }
        })
    }

    /// Execute the entire task graph, respecting dependencies.
    pub async fn run(&self, graph: &mut TaskGraph) -> anyhow::Result<()> {
        loop {
            let ready: Vec<usize> = graph.ready_nodes().iter().map(|n| n.id).collect();
            if ready.is_empty() {
                break;
            }

            for id in ready {
                graph.set_status(id, TaskStatus::Running);
                let node = graph.nodes[id].clone();
                match self.execute_node(&node).await {
                    Ok(_) => graph.complete(id),
                    Err(e) => {
                        graph.fail(id, &e.to_string());
                        // Continue with other ready nodes; caller checks has_failures()
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::driver::{
        a11y::MockA11yDriver,
        display::{MockDisplayDriver},
        input::MockInputDriver,
    };

    fn mock_aci() -> Aci {
        Aci::new_basic(
            Box::new(MockInputDriver::new()),
            Box::new(MockDisplayDriver::new(1920, 1080)),
            Box::new(MockA11yDriver::new()),
            None,
        )
    }

    #[test]
    fn test_topological_sort() {
        let mut graph = TaskGraph::new("test");
        graph.add_node("A", TaskAction::Screenshot, vec![]);
        graph.add_node("B", TaskAction::Click { x: 100, y: 200 }, vec![0]);
        graph.add_node("C", TaskAction::Type { text: "hello".into() }, vec![1]);

        let order = graph.topological_sort().unwrap();
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn test_ready_nodes() {
        let mut graph = TaskGraph::new("test");
        graph.add_node("A", TaskAction::Screenshot, vec![]);
        graph.add_node("B", TaskAction::Click { x: 0, y: 0 }, vec![0]);

        let ready = graph.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, 0);

        graph.complete(0);
        let ready = graph.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, 1);
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = TaskGraph::new("test");
        graph.add_node("A", TaskAction::Screenshot, vec![1]);
        graph.add_node("B", TaskAction::Screenshot, vec![0]);

        assert!(graph.topological_sort().is_err());
    }

    #[test]
    fn test_set_status() {
        let mut graph = TaskGraph::new("test");
        graph.add_node("A", TaskAction::Screenshot, vec![]);
        assert_eq!(graph.nodes[0].status, TaskStatus::Pending);

        graph.set_status(0, TaskStatus::Running);
        assert_eq!(graph.nodes[0].status, TaskStatus::Running);

        graph.set_status(999, TaskStatus::Completed); // out of bounds, no-op
    }

    #[test]
    fn test_decompose_simple() {
        let graph = TaskManager::decompose_simple("Open browser. Navigate to page.");
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].description, "Open browser");
        assert_eq!(graph.nodes[1].description, "Navigate to page");
    }

    #[test]
    fn test_parse_key() {
        assert_eq!(parse_key("ctrl"), Some(Key::Ctrl));
        assert_eq!(parse_key("CTRL"), Some(Key::Ctrl));
        assert_eq!(parse_key("a"), Some(Key::A));
        assert_eq!(parse_key("f5"), Some(Key::F5));
        assert_eq!(parse_key("enter"), Some(Key::Enter));
        assert_eq!(parse_key("nonexistent"), None);
    }

    #[test]
    fn test_strip_code_fences() {
        let input = "```json\n{\"nodes\": []}\n```";
        assert_eq!(strip_code_fences(input), "{\"nodes\": []}");

        let plain = "{\"nodes\": []}";
        assert_eq!(strip_code_fences(plain), "{\"nodes\": []}");
    }

    #[test]
    fn test_scroll_action_serde() {
        let action = TaskAction::Scroll {
            x: 100,
            y: 200,
            direction: ScrollDirection::Down,
            amount: 3,
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: TaskAction = serde_json::from_str(&json).unwrap();
        match back {
            TaskAction::Scroll { x, y, direction, amount } => {
                assert_eq!(x, 100);
                assert_eq!(y, 200);
                assert_eq!(direction, ScrollDirection::Down);
                assert_eq!(amount, 3);
            }
            _ => panic!("Expected Scroll"),
        }
    }

    // -- TaskDecomposer mock + LLM decomposition tests --

    struct MockDecomposer;

    #[async_trait]
    impl TaskDecomposer for MockDecomposer {
        async fn complete_text(&self, _prompt: &str) -> anyhow::Result<String> {
            Ok(r#"{"nodes": [
                {"description": "Click button", "action": "click", "params": {"x": 100, "y": 200}, "depends_on": []},
                {"description": "Type name", "action": "type", "params": {"text": "hello"}, "depends_on": [0]},
                {"description": "Wait", "action": "wait", "params": {"ms": 500}, "depends_on": [1]}
            ]}"#.to_string())
        }
    }

    #[tokio::test]
    async fn test_decompose_with_llm() {
        let decomposer = MockDecomposer;
        let graph = TaskManager::decompose_with_llm("Test task", &decomposer).await.unwrap();

        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.nodes[0].description, "Click button");
        assert_eq!(graph.nodes[0].action, TaskAction::Click { x: 100, y: 200 });
        assert_eq!(graph.nodes[1].description, "Type name");
        assert_eq!(graph.nodes[1].dependencies, vec![0]);
        assert_eq!(graph.nodes[2].action, TaskAction::Wait(500));
    }

    #[tokio::test]
    async fn test_decompose_with_llm_code_fenced() {
        struct FencedDecomposer;
        #[async_trait]
        impl TaskDecomposer for FencedDecomposer {
            async fn complete_text(&self, _prompt: &str) -> anyhow::Result<String> {
                Ok("```json\n{\"nodes\": [{\"description\": \"A\", \"action\": \"screenshot\", \"params\": {}, \"depends_on\": []}]}\n```".to_string())
            }
        }

        let graph = TaskManager::decompose_with_llm("test", &FencedDecomposer).await.unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].action, TaskAction::Screenshot);
    }

    // -- TaskWorker tests --

    #[tokio::test]
    async fn test_task_worker_run() {
        let aci = Arc::new(mock_aci());
        let worker = TaskWorker::new(aci);

        let mut graph = TaskGraph::new("test");
        graph.add_node("Screenshot", TaskAction::Screenshot, vec![]);
        graph.add_node("Click", TaskAction::Click { x: 100, y: 200 }, vec![0]);
        graph.add_node("Wait", TaskAction::Wait(10), vec![1]);

        worker.run(&mut graph).await.unwrap();

        assert!(graph.is_complete());
        assert!(!graph.has_failures());
    }

    #[tokio::test]
    async fn test_task_worker_execute_node_types() {
        let aci = Arc::new(mock_aci());
        let worker = TaskWorker::new(aci);

        // Test each action type
        let node = TaskNode {
            id: 0, description: "test".into(),
            action: TaskAction::Click { x: 10, y: 20 },
            dependencies: vec![], status: TaskStatus::Pending,
        };
        assert!(worker.execute_node(&node).await.unwrap().contains("Clicked"));

        let node = TaskNode {
            id: 0, description: "test".into(),
            action: TaskAction::Type { text: "hi".into() },
            dependencies: vec![], status: TaskStatus::Pending,
        };
        assert!(worker.execute_node(&node).await.unwrap().contains("Typed"));

        let node = TaskNode {
            id: 0, description: "test".into(),
            action: TaskAction::Screenshot,
            dependencies: vec![], status: TaskStatus::Pending,
        };
        assert!(worker.execute_node(&node).await.unwrap().contains("Screenshot"));

        let node = TaskNode {
            id: 0, description: "test".into(),
            action: TaskAction::Observe,
            dependencies: vec![], status: TaskStatus::Pending,
        };
        assert!(worker.execute_node(&node).await.unwrap().contains("Observed"));

        let node = TaskNode {
            id: 0, description: "test".into(),
            action: TaskAction::Hotkey { keys: vec!["ctrl".into(), "c".into()] },
            dependencies: vec![], status: TaskStatus::Pending,
        };
        assert!(worker.execute_node(&node).await.unwrap().contains("Hotkey"));

        let node = TaskNode {
            id: 0, description: "test".into(),
            action: TaskAction::Scroll { x: 0, y: 0, direction: ScrollDirection::Down, amount: 3 },
            dependencies: vec![], status: TaskStatus::Pending,
        };
        assert!(worker.execute_node(&node).await.unwrap().contains("Scroll"));

        let node = TaskNode {
            id: 0, description: "test".into(),
            action: TaskAction::Wait(10),
            dependencies: vec![], status: TaskStatus::Pending,
        };
        assert!(worker.execute_node(&node).await.unwrap().contains("Waited"));
    }

    #[tokio::test]
    async fn test_task_worker_composite() {
        let aci = Arc::new(mock_aci());
        let worker = TaskWorker::new(aci);

        let node = TaskNode {
            id: 0, description: "test".into(),
            action: TaskAction::Composite(vec![
                TaskAction::Click { x: 10, y: 20 },
                TaskAction::Type { text: "hello".into() },
            ]),
            dependencies: vec![], status: TaskStatus::Pending,
        };
        let result = worker.execute_node(&node).await.unwrap();
        assert_eq!(result, "Composite executed");
    }
}
