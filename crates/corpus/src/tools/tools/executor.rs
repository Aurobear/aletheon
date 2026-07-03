//! Parallel tool call executor.
//!
//! Groups tool calls by [`ConcurrencyClass`] and executes them with appropriate
//! concurrency: read-only calls run in parallel, write calls are serialized only
//! when paths conflict, and side-effect calls are always serialized.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

use super::{Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Concurrency classification
// ---------------------------------------------------------------------------

pub use base::tool::ConcurrencyClass;

// ---------------------------------------------------------------------------
// Cancel mode
// ---------------------------------------------------------------------------

/// How cancellation is handled for a running tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelMode {
    /// Drop the future immediately (e.g. file reads).
    Immediate,
    /// Wait up to the given duration for graceful shutdown (e.g. bash, services).
    Graceful(Duration),
}

impl Default for CancelMode {
    fn default() -> Self {
        CancelMode::Graceful(Duration::from_secs(3))
    }
}

// ---------------------------------------------------------------------------
// Path conflict detector
// ---------------------------------------------------------------------------

/// Detects and serializes write calls that touch overlapping paths.
///
/// Each canonical path gets its own `Semaphore(1)`. Two write calls targeting
/// the same path will contend on the same semaphore and thus run sequentially.
/// Calls targeting disjoint paths acquire different semaphores and run in
/// parallel.
#[derive(Debug, Default)]
pub struct PathConflictDetector {
    semaphores: DashMap<PathBuf, Arc<Semaphore>>,
}

impl PathConflictDetector {
    pub fn new() -> Self {
        Self {
            semaphores: DashMap::new(),
        }
    }

    /// Acquire exclusive locks for all `paths`. Blocks until all are held.
    ///
    /// Returns a vector of `OwnedSemaphorePermit`s that release on drop.
    async fn acquire(&self, paths: &[PathBuf]) -> Vec<tokio::sync::OwnedSemaphorePermit> {
        let mut permits = Vec::with_capacity(paths.len());
        for path in paths {
            let sem = self
                .semaphores
                .entry(path.clone())
                .or_insert_with(|| Arc::new(Semaphore::new(1)))
                .clone();
            // SAFETY: semaphore is created with permits=1 so this never fails
            // with a closed semaphore.
            permits.push(sem.acquire_owned().await.unwrap());
        }
        permits
    }
}

// ---------------------------------------------------------------------------
// A single pending tool call (input to the executor)
// ---------------------------------------------------------------------------

/// A tool call to be executed, as produced by the LLM.
#[derive(Debug, Clone)]
pub struct PendingToolCall {
    /// Index in the original LLM response (used for result ordering).
    pub index: usize,
    /// Tool name (must match a registered tool).
    pub tool_name: String,
    /// Raw JSON input to pass to the tool.
    pub input: serde_json::Value,
    /// Concurrency class for scheduling.
    pub concurrency: ConcurrencyClass,
    /// How to handle cancellation.
    pub cancel_mode: CancelMode,
}

// ---------------------------------------------------------------------------
// Executor result (one per call, ordered by original index)
// ---------------------------------------------------------------------------

/// Result of executing a single tool call.
#[derive(Debug, Clone)]
pub struct ExecutorResult {
    /// Original index from the LLM response.
    pub index: usize,
    /// Tool execution result.
    pub result: ToolResult,
}

// ---------------------------------------------------------------------------
// ToolCallExecutor
// ---------------------------------------------------------------------------

/// Parallel tool call executor.
///
/// Groups incoming [`PendingToolCall`]s by [`ConcurrencyClass`] and runs them
/// with appropriate concurrency. Results are returned in the same order as the
/// input calls (by `index`).
pub struct ToolCallExecutor {
    max_concurrency: usize,
    cancel: CancellationToken,
    conflict_detector: PathConflictDetector,
}

impl ToolCallExecutor {
    /// Create a new executor with the given max concurrency.
    pub fn new(max_concurrency: usize, cancel: CancellationToken) -> Self {
        Self {
            max_concurrency,
            cancel,
            conflict_detector: PathConflictDetector::new(),
        }
    }

    /// Create an executor with default settings (max 8 concurrent tasks).
    pub fn with_defaults(cancel: CancellationToken) -> Self {
        Self::new(8, cancel)
    }

    /// Execute a batch of tool calls against the provided tool registry.
    ///
    /// `tools` maps tool names to their implementations. The `ctx` is shared
    /// across all calls.
    ///
    /// Returns results ordered by the original `PendingToolCall::index`.
    #[allow(clippy::redundant_locals)]
    pub async fn execute_batch(
        &self,
        calls: Vec<PendingToolCall>,
        tools: &HashMap<String, Arc<dyn Tool>>,
        ctx: &ToolContext,
    ) -> Vec<ExecutorResult> {
        if calls.is_empty() {
            return Vec::new();
        }

        // Partition into concurrency groups.
        let mut read_only: Vec<&PendingToolCall> = Vec::new();
        let mut writes: Vec<&PendingToolCall> = Vec::new();
        let mut side_effects: Vec<&PendingToolCall> = Vec::new();

        for call in &calls {
            match &call.concurrency {
                ConcurrencyClass::ReadOnly => read_only.push(call),
                ConcurrencyClass::Write { .. } => writes.push(call),
                ConcurrencyClass::SideEffect => side_effects.push(call),
            }
        }

        // Global concurrency limiter.
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));

        // Collect results. We use a Vec indexed by call index for O(1) insertion.
        let mut results: Vec<Option<ExecutorResult>> = vec![None; calls.len()];

        // --- Phase 1: ReadOnly (all in parallel) ---
        let read_futs: Vec<_> = read_only
            .into_iter()
            .map(|call| {
                let sem = semaphore.clone();
                let tools = tools;
                let cancel = self.cancel.clone();
                let ctx = ctx;
                async move {
                    if cancel.is_cancelled() {
                        return (call.index, cancelled_result());
                    }
                    let _permit = sem.acquire().await.unwrap();
                    if cancel.is_cancelled() {
                        return (call.index, cancelled_result());
                    }
                    let result =
                        dispatch_tool(tools, &call.tool_name, call.input.clone(), ctx).await;
                    (call.index, result)
                }
            })
            .collect();

        let read_results = futures::future::join_all(read_futs).await;
        for (idx, result) in read_results {
            results[idx] = Some(ExecutorResult { index: idx, result });
        }

        // --- Phase 2: Write (parallel non-conflicting, serialized conflicting) ---
        // Group writes by their path set. Calls that share any path are serialized
        // through the PathConflictDetector.
        let write_futs: Vec<_> = writes
            .into_iter()
            .map(|call| {
                let paths = match &call.concurrency {
                    ConcurrencyClass::Write { paths } => paths.clone(),
                    _ => vec![],
                };
                let detector = &self.conflict_detector;
                let sem = semaphore.clone();
                let tools = tools;
                let cancel = self.cancel.clone();
                let ctx = ctx;
                async move {
                    if cancel.is_cancelled() {
                        return (call.index, cancelled_result());
                    }
                    // Acquire path-specific locks (blocks on conflict).
                    let _permits = if paths.is_empty() {
                        vec![]
                    } else {
                        detector.acquire(&paths).await
                    };
                    let _global_permit = sem.acquire().await.unwrap();
                    if cancel.is_cancelled() {
                        return (call.index, cancelled_result());
                    }
                    let result =
                        dispatch_tool(tools, &call.tool_name, call.input.clone(), ctx).await;
                    (call.index, result)
                }
            })
            .collect();

        let write_results = futures::future::join_all(write_futs).await;
        for (idx, result) in write_results {
            results[idx] = Some(ExecutorResult { index: idx, result });
        }

        // --- Phase 3: SideEffect (serialized, exclusive lock) ---
        // Use a global RwLock write guard to serialize all side effects.
        let side_effect_lock = RwLock::new(());
        for call in side_effects {
            if self.cancel.is_cancelled() {
                results[call.index] = Some(ExecutorResult {
                    index: call.index,
                    result: cancelled_result(),
                });
                continue;
            }
            let _write_guard = side_effect_lock.write().await;
            let sem = semaphore.clone();
            let _permit = sem.acquire().await.unwrap();
            if self.cancel.is_cancelled() {
                results[call.index] = Some(ExecutorResult {
                    index: call.index,
                    result: cancelled_result(),
                });
                continue;
            }
            let result = dispatch_tool(tools, &call.tool_name, call.input.clone(), ctx).await;
            results[call.index] = Some(ExecutorResult {
                index: call.index,
                result,
            });
        }

        // Unwrap: every index should have been filled.
        results.into_iter().map(|r| r.unwrap()).collect()
    }
}

// ---------------------------------------------------------------------------
// Dispatch helper
// ---------------------------------------------------------------------------

/// Look up a tool by name and execute it.
async fn dispatch_tool(
    tools: &HashMap<String, Arc<dyn Tool>>,
    name: &str,
    input: serde_json::Value,
    ctx: &ToolContext,
) -> ToolResult {
    match tools.get(name) {
        Some(tool) => tool.execute(input, ctx).await,
        None => ToolResult {
            content: format!("Unknown tool: {}", name),
            is_error: true,
            metadata: Default::default(),
        },
    }
}

/// Create a `ToolResult` indicating the call was cancelled.
fn cancelled_result() -> ToolResult {
    ToolResult {
        content: "Tool execution cancelled".to_string(),
        is_error: true,
        metadata: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // -- Mock tool that records invocation order ---------------------------

    struct MockTool {
        tool_name: String,
        delay_ms: u64,
        invocation_counter: Arc<AtomicUsize>,
        invocation_log: Arc<std::sync::Mutex<Vec<(String, usize)>>>,
        #[allow(dead_code)]
        concurrency: ConcurrencyClass,
    }

    impl MockTool {
        fn new(
            name: &str,
            delay_ms: u64,
            counter: Arc<AtomicUsize>,
            log: Arc<std::sync::Mutex<Vec<(String, usize)>>>,
            concurrency: ConcurrencyClass,
        ) -> Self {
            Self {
                tool_name: name.to_string(),
                delay_ms,
                invocation_counter: counter,
                invocation_log: log,
                concurrency,
            }
        }
    }

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.tool_name
        }
        fn description(&self) -> &str {
            "mock"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }
        fn permission_level(&self) -> super::super::PermissionLevel {
            super::super::PermissionLevel::L0
        }
        fn boxed_clone(&self) -> Box<dyn Tool> {
            // Not needed in tests.
            unimplemented!()
        }
        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            let seq = self.invocation_counter.fetch_add(1, Ordering::SeqCst);
            self.invocation_log
                .lock()
                .unwrap()
                .push((self.tool_name.clone(), seq));
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            ToolResult {
                content: format!("{}:ok:seq{}", self.tool_name, seq),
                is_error: false,
                metadata: Default::default(),
            }
        }
    }

    fn mock_ctx() -> ToolContext {
        ToolContext {
            working_dir: PathBuf::from("/tmp"),
            session_id: "test".to_string(),
        }
    }

    fn make_tool_map(
        names: &[&str],
        delay_ms: u64,
        counter: Arc<AtomicUsize>,
        log: Arc<std::sync::Mutex<Vec<(String, usize)>>>,
        concurrency: ConcurrencyClass,
    ) -> HashMap<String, Arc<dyn Tool>> {
        names
            .iter()
            .map(|n| {
                let tool = Arc::new(MockTool::new(
                    n,
                    delay_ms,
                    counter.clone(),
                    log.clone(),
                    concurrency.clone(),
                )) as Arc<dyn Tool>;
                (n.to_string(), tool)
            })
            .collect()
    }

    // -- Tests -------------------------------------------------------------

    #[tokio::test]
    async fn empty_batch_returns_empty() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let ctx = mock_ctx();
        let results = executor.execute_batch(vec![], &tools, &ctx).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn single_readonly_call_executes() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(&["read_a"], 0, counter, log, ConcurrencyClass::ReadOnly);
        let ctx = mock_ctx();

        let calls = vec![PendingToolCall {
            index: 0,
            tool_name: "read_a".to_string(),
            input: json!({}),
            concurrency: ConcurrencyClass::ReadOnly,
            cancel_mode: CancelMode::Immediate,
        }];

        let results = executor.execute_batch(calls, &tools, &ctx).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 0);
        assert!(!results[0].result.is_error);
        assert!(results[0].result.content.contains("read_a:ok"));
    }

    #[tokio::test]
    async fn readonly_calls_run_in_parallel() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(
            &["r0", "r1", "r2"],
            100,
            counter,
            log,
            ConcurrencyClass::ReadOnly,
        );
        let ctx = mock_ctx();

        let calls: Vec<PendingToolCall> = ["r0", "r1", "r2"]
            .iter()
            .enumerate()
            .map(|(i, name)| PendingToolCall {
                index: i,
                tool_name: name.to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::ReadOnly,
                cancel_mode: CancelMode::Immediate,
            })
            .collect();

        let start = std::time::Instant::now();
        let results = executor.execute_batch(calls, &tools, &ctx).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 3);
        // 3 calls each sleeping 100ms should complete in ~100ms if parallel,
        // not ~300ms if serial. Use 200ms as generous bound.
        assert!(
            elapsed < Duration::from_millis(200),
            "Expected parallel execution, took {:?}",
            elapsed
        );
        for r in &results {
            assert!(!r.result.is_error);
        }
    }

    #[tokio::test]
    async fn result_ordering_matches_input_index() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(
            &["a", "b", "c"],
            0,
            counter,
            log,
            ConcurrencyClass::ReadOnly,
        );
        let ctx = mock_ctx();

        // Deliberately provide calls in non-alphabetical order.
        let calls = vec![
            PendingToolCall {
                index: 2,
                tool_name: "c".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::ReadOnly,
                cancel_mode: CancelMode::Immediate,
            },
            PendingToolCall {
                index: 0,
                tool_name: "a".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::ReadOnly,
                cancel_mode: CancelMode::Immediate,
            },
            PendingToolCall {
                index: 1,
                tool_name: "b".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::ReadOnly,
                cancel_mode: CancelMode::Immediate,
            },
        ];

        let results = executor.execute_batch(calls, &tools, &ctx).await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].index, 0);
        assert!(results[0].result.content.contains("a:ok"));
        assert_eq!(results[1].index, 1);
        assert!(results[1].result.content.contains("b:ok"));
        assert_eq!(results[2].index, 2);
        assert!(results[2].result.content.contains("c:ok"));
    }

    #[tokio::test]
    async fn unknown_tool_returns_error() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let ctx = mock_ctx();

        let calls = vec![PendingToolCall {
            index: 0,
            tool_name: "nonexistent".to_string(),
            input: json!({}),
            concurrency: ConcurrencyClass::ReadOnly,
            cancel_mode: CancelMode::Immediate,
        }];

        let results = executor.execute_batch(calls, &tools, &ctx).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].result.is_error);
        assert!(results[0].result.content.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn write_same_path_serialized() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(&["w0", "w1"], 100, counter, log, ConcurrencyClass::ReadOnly);
        let ctx = mock_ctx();

        let path = PathBuf::from("/tmp/same_file.txt");
        let calls = vec![
            PendingToolCall {
                index: 0,
                tool_name: "w0".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::Write {
                    paths: vec![path.clone()],
                },
                cancel_mode: CancelMode::Graceful(Duration::from_secs(3)),
            },
            PendingToolCall {
                index: 1,
                tool_name: "w1".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::Write {
                    paths: vec![path.clone()],
                },
                cancel_mode: CancelMode::Graceful(Duration::from_secs(3)),
            },
        ];

        let start = std::time::Instant::now();
        let results = executor.execute_batch(calls, &tools, &ctx).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 2);
        // Same-path writes must be serialized => ~200ms not ~100ms.
        assert!(
            elapsed >= Duration::from_millis(180),
            "Expected serialized execution for same-path writes, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn write_different_paths_parallel() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(
            &["w0", "w1", "w2"],
            100,
            counter,
            log,
            ConcurrencyClass::ReadOnly,
        );
        let ctx = mock_ctx();

        let calls = vec![
            PendingToolCall {
                index: 0,
                tool_name: "w0".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::Write {
                    paths: vec![PathBuf::from("/tmp/a.txt")],
                },
                cancel_mode: CancelMode::Graceful(Duration::from_secs(3)),
            },
            PendingToolCall {
                index: 1,
                tool_name: "w1".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::Write {
                    paths: vec![PathBuf::from("/tmp/b.txt")],
                },
                cancel_mode: CancelMode::Graceful(Duration::from_secs(3)),
            },
            PendingToolCall {
                index: 2,
                tool_name: "w2".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::Write {
                    paths: vec![PathBuf::from("/tmp/c.txt")],
                },
                cancel_mode: CancelMode::Graceful(Duration::from_secs(3)),
            },
        ];

        let start = std::time::Instant::now();
        let results = executor.execute_batch(calls, &tools, &ctx).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 3);
        // Disjoint-path writes should run in parallel.
        assert!(
            elapsed < Duration::from_millis(200),
            "Expected parallel execution for disjoint writes, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn side_effect_calls_serialized() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(
            &["s0", "s1", "s2"],
            100,
            counter,
            log,
            ConcurrencyClass::ReadOnly,
        );
        let ctx = mock_ctx();

        let calls: Vec<PendingToolCall> = ["s0", "s1", "s2"]
            .iter()
            .enumerate()
            .map(|(i, name)| PendingToolCall {
                index: i,
                tool_name: name.to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::SideEffect,
                cancel_mode: CancelMode::Graceful(Duration::from_secs(5)),
            })
            .collect();

        let start = std::time::Instant::now();
        let results = executor.execute_batch(calls, &tools, &ctx).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 3);
        // Serialized: 3 x 100ms = ~300ms.
        assert!(
            elapsed >= Duration::from_millis(280),
            "Expected serialized execution for side effects, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn cancelled_before_execution() {
        let cancel = CancellationToken::new();
        cancel.cancel(); // Pre-cancel.
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(&["t0"], 0, counter, log, ConcurrencyClass::ReadOnly);
        let ctx = mock_ctx();

        let calls = vec![PendingToolCall {
            index: 0,
            tool_name: "t0".to_string(),
            input: json!({}),
            concurrency: ConcurrencyClass::ReadOnly,
            cancel_mode: CancelMode::Immediate,
        }];

        let results = executor.execute_batch(calls, &tools, &ctx).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].result.is_error);
        assert!(results[0].result.content.contains("cancelled"));
    }

    #[tokio::test]
    async fn cancelled_side_effect_returns_error() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(&["se"], 0, counter, log, ConcurrencyClass::ReadOnly);
        let ctx = mock_ctx();

        let calls = vec![PendingToolCall {
            index: 0,
            tool_name: "se".to_string(),
            input: json!({}),
            concurrency: ConcurrencyClass::SideEffect,
            cancel_mode: CancelMode::Graceful(Duration::from_secs(5)),
        }];

        let results = executor.execute_batch(calls, &tools, &ctx).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].result.is_error);
        assert!(results[0].result.content.contains("cancelled"));
    }

    #[tokio::test]
    async fn mixed_concurrency_classes() {
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::with_defaults(cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(
            &["ro", "wr", "se"],
            50,
            counter,
            log,
            ConcurrencyClass::ReadOnly,
        );
        let ctx = mock_ctx();

        let calls = vec![
            PendingToolCall {
                index: 0,
                tool_name: "ro".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::ReadOnly,
                cancel_mode: CancelMode::Immediate,
            },
            PendingToolCall {
                index: 1,
                tool_name: "wr".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::Write {
                    paths: vec![PathBuf::from("/tmp/x.txt")],
                },
                cancel_mode: CancelMode::Graceful(Duration::from_secs(3)),
            },
            PendingToolCall {
                index: 2,
                tool_name: "se".to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::SideEffect,
                cancel_mode: CancelMode::Graceful(Duration::from_secs(5)),
            },
        ];

        let results = executor.execute_batch(calls, &tools, &ctx).await;
        assert_eq!(results.len(), 3);
        // All should succeed, ordered by index.
        assert_eq!(results[0].index, 0);
        assert!(results[0].result.content.contains("ro:ok"));
        assert_eq!(results[1].index, 1);
        assert!(results[1].result.content.contains("wr:ok"));
        assert_eq!(results[2].index, 2);
        assert!(results[2].result.content.contains("se:ok"));
    }

    #[tokio::test]
    async fn path_conflict_detector_isolation() {
        // Two separate paths should not contend.
        let detector = PathConflictDetector::new();
        let p1 = PathBuf::from("/tmp/a.txt");
        let p2 = PathBuf::from("/tmp/b.txt");

        let paths1 = vec![p1];
        let paths2 = vec![p2];
        let (r1, r2) = tokio::join!(detector.acquire(&paths1), detector.acquire(&paths2));
        assert_eq!(r1.len(), 1);
        assert_eq!(r2.len(), 1);
        // Both acquired successfully (no deadlock).
    }

    #[tokio::test]
    async fn concurrency_class_serde_round_trip() {
        let classes = vec![
            ConcurrencyClass::ReadOnly,
            ConcurrencyClass::Write {
                paths: vec![PathBuf::from("/tmp/test.txt")],
            },
            ConcurrencyClass::SideEffect,
        ];
        for class in &classes {
            let json = serde_json::to_string(class).unwrap();
            let back: ConcurrencyClass = serde_json::from_str(&json).unwrap();
            assert_eq!(*class, back);
        }
    }

    #[tokio::test]
    async fn max_concurrency_limits_tasks() {
        // With max_concurrency=2 and 4 read-only calls (each 100ms),
        // total time should be ~200ms (2 batches), not ~100ms.
        let cancel = CancellationToken::new();
        let executor = ToolCallExecutor::new(2, cancel);
        let counter = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = make_tool_map(
            &["t0", "t1", "t2", "t3"],
            100,
            counter,
            log,
            ConcurrencyClass::ReadOnly,
        );
        let ctx = mock_ctx();

        let calls: Vec<PendingToolCall> = ["t0", "t1", "t2", "t3"]
            .iter()
            .enumerate()
            .map(|(i, name)| PendingToolCall {
                index: i,
                tool_name: name.to_string(),
                input: json!({}),
                concurrency: ConcurrencyClass::ReadOnly,
                cancel_mode: CancelMode::Immediate,
            })
            .collect();

        let start = std::time::Instant::now();
        let results = executor.execute_batch(calls, &tools, &ctx).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 4);
        // With concurrency=2 and 4 tasks of 100ms each, should take ~200ms.
        assert!(
            elapsed >= Duration::from_millis(180),
            "Expected concurrency limit to enforce batching, took {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_millis(350),
            "But should not be fully serial, took {:?}",
            elapsed
        );
    }
}
