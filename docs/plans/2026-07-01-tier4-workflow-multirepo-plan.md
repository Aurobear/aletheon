# Tier 4 — Workflow Sedimentation + Multi-repo Extraction — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Make workflows persistent and reusable — serialize the in-memory `DiGraph` DAG to disk, add a workflow definition store (`save`/`load`/`list`/`run`), and expose a `list`/`run` CLI surface — then document the kernel dependency-boundary and prove one crate (`base`) extracts cleanly, so the doc-2 org split becomes possible without a big-bang.

**Architecture:** Two independent slices. **4a** is additive product code: the DAG engine (`crates/runtime/src/impl/orchestration/digraph/graph.rs:23`) executes in-memory only; this plan adds a serde mirror of the graph plus a filesystem JSON store, reusing the `~/.aletheon/` path convention from `base::paths`. **4b** is architectural readiness: it depends on Tier 2 (2b `RuntimeHost`, 2c the `cognit → corpus/interact` inversion fix) landing first, and delivers a documented dependency boundary + one proof-of-extraction, NOT the whole org split. Every claim below was re-verified against the repo on 2026-07-01 (anchors inline).

**Tech Stack:** Rust (Cargo workspace), `serde`/`serde_json`, filesystem, `clap` (interact CLI), `base::paths`.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "Tier 4 — Workflow Sedimentation + Multi-repo Extraction"

**Branch:** `auro/feat/20260701-aletheon-workflow-multirepo` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Claim | Evidence |
|---|---|
| DAG engine executes in-memory only; no save/load/reuse | `crates/runtime/src/impl/orchestration/digraph/graph.rs:23` (`pub struct DiGraph`), `graph.rs:106` (`pub async fn execute(...) -> Result<GraphState, String>`) |
| `DiGraph` fields | `graph.rs:23-29`: `id: String`, `nodes: HashMap<String, Node>`, `edges: Vec<Edge>`, `entry_node: String`, `join_strategy: JoinStrategy` — all `pub` |
| `DiGraph` is NOT serde-serializable today | no `#[derive(Serialize/Deserialize)]` on `DiGraph` (`graph.rs:22-23`); `JoinStrategy` has a `Duration` variant and no derives (`graph.rs:10-20`) |
| `Node`/`NodeKind`/`RetryPolicy`/`OnExhausted` already derive serde | `node.rs:5,18,29,39,47` all `#[derive(... Serialize, Deserialize)]`; `Node.timeout: Option<Duration>` (`node.rs:35`) |
| `Edge`/`ConditionExpr` already derive serde | `edge.rs:5-6,16-17` |
| `GraphState`/`LogEntry` already derive serde | `state.rs:5-6,14-15` |
| Graph mutators to reconstruct from a def | `graph.rs:32` `new(id, entry_node)`, `graph.rs:42` `add_node`, `graph.rs:46` `add_edge`; `join_strategy` is a `pub` field |
| `Branch`/`HumanApproval` nodes execute WITHOUT an agent | `graph.rs:221-240` (`NodeKind::Branch`/`HumanApproval` need no registry lookup) — usable in tests with an empty registry |
| `AgentRegistry::new()` exists | `crates/runtime/src/impl/orchestration/registry.rs:23` |
| Orchestration module list (where `store` mounts) | `crates/runtime/src/impl/orchestration/mod.rs:1-16` (`pub mod digraph;` etc., no `store`) |
| Canonical user dir for the store | `base::paths::config_dir()` → `~/.aletheon/` (`crates/base/src/types/paths.rs:6-8`), re-exported `base::paths` (`crates/base/src/lib.rs:53`) |
| `runtime` is a god crate depending on all five siblings | `crates/runtime/Cargo.toml:17-22`: `base, cognit, corpus, memory, dasein, metacog` |
| `base` is a true leaf (extractable today) | `crates/base/Cargo.toml` has NO `{ path = "../" }` workspace deps |
| `memory`/`corpus`/`metacog` depend only on `base` | `crates/{memory,corpus,metacog}/Cargo.toml:9` `base = { path = "../base" }` (no other path deps) |
| `cognit` has the inversion (Brain → Body + UI) | `crates/cognit/Cargo.toml:9-11`: `base`, `corpus`, `interact` |
| `interact` depends on `base` + `corpus` | `crates/interact/Cargo.toml:13-14` |
| `dasein` depends on `base, corpus, cognit, memory` | `crates/dasein/Cargo.toml:9-12` |
| interact CLI cannot depend on `runtime` (would cycle) | `runtime → cognit → interact` (`runtime/Cargo.toml:18` + `cognit/Cargo.toml:11`); so the CLI surface reaches the store over the daemon socket, not by linking `runtime` |
| interact CLI subcommand enum (where `Workflow` mounts) | `crates/interact/src/tui/cli.rs:70` `pub enum Command { ... }` |
| 4b is gated behind Tier 2 | `docs/plans/2026-07-01-modules-roadmap-design.md:40,54,286` ("needs Tier 2"; "Gate behind Tier 2"); Tier 2b `RuntimeHost` = spec:141, Tier 2c inversion = spec:165 |
| Cargo `[package]` names (for `-p`) | `base, cognit, corpus, dasein, interact, memory, metacog, runtime` (root `Cargo.toml:4-11`) |

---

## File map

| File | Change |
|---|---|
| `crates/runtime/src/impl/orchestration/store.rs` | **new** — `WorkflowDef`/`JoinStrategyDef` serde mirror + `WorkflowStore` (`save`/`load`/`list`/`run`) |
| `crates/runtime/src/impl/orchestration/mod.rs` | add `pub mod store;` + re-export `WorkflowStore`, `WorkflowDef` |
| `crates/interact/src/tui/cli.rs` | add `Command::Workflow { action: WorkflowAction }` + `enum WorkflowAction { List, Run { name } }` (arg-parse surface only; daemon wiring noted) |

4a = the three files above (Phases 1–4). 4b = documentation + build/`cargo tree` verification only (Phase 5) — **no product code**. Default checks: `cargo build -p runtime` and `cargo test -p runtime`.

---

## Phase 1 — Make the graph round-trippable (serde mirror)

`DiGraph` itself cannot derive serde (`JoinStrategy` holds a `Duration` and neither type derives it — `graph.rs:10-23`). Rather than mutate the live execution types, add a serializable mirror `WorkflowDef` whose sub-types (`Node`, `Edge`) already derive serde, plus a serde-friendly `JoinStrategyDef`. Lowest blast radius; the executor is untouched.

### Task 1: `WorkflowDef` + `JoinStrategyDef` with lossless `from_graph`/`to_graph`

**Files:** Create `crates/runtime/src/impl/orchestration/store.rs`; edit `crates/runtime/src/impl/orchestration/mod.rs`.

- [ ] **Step 1: Write the failing test**

Create `store.rs` with only the test module first (or add the test alongside the types in Step 3 and run it — either way this test must exist and fail before the impl compiles):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::orchestration::digraph::edge::ConditionExpr;
    use crate::r#impl::orchestration::digraph::graph::{DiGraph, JoinStrategy};
    use crate::r#impl::orchestration::digraph::node::{Node, NodeKind, RetryPolicy};
    use crate::r#impl::orchestration::digraph::{Edge};

    fn node(id: &str, cond: &str) -> Node {
        Node {
            id: id.to_string(),
            name: id.to_string(),
            kind: NodeKind::Branch { condition: cond.to_string() },
            retry_policy: RetryPolicy::default(),
            timeout: None,
        }
    }

    fn sample_graph() -> DiGraph {
        let mut g = DiGraph::new("wf-1", "a");
        g.join_strategy = JoinStrategy::FirstN(2);
        g.add_node(node("a", "x"));
        g.add_node(node("b", "y"));
        g.add_edge(Edge { from: "a".into(), to: "b".into(), condition: ConditionExpr::Always });
        g
    }

    #[test]
    fn workflow_def_round_trips_through_json() {
        let g = sample_graph();
        let def = WorkflowDef::from_graph(&g);
        let json = serde_json::to_string_pretty(&def).unwrap();
        let back: WorkflowDef = serde_json::from_str(&json).unwrap();
        let g2 = back.to_graph();

        assert_eq!(g2.id, "wf-1");
        assert_eq!(g2.entry_node, "a");
        assert_eq!(g2.nodes.len(), 2);
        assert_eq!(g2.edges.len(), 1);
        assert_eq!(g2.edges[0].from, "a");
        assert!(matches!(g2.join_strategy, JoinStrategy::FirstN(2)));
        // topological execution order is preserved
        assert_eq!(g2.topological_sort().unwrap(), vec!["a", "b"]);
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`WorkflowDef` undefined).

Run: `cargo test -p runtime orchestration::store::tests::workflow_def_round_trips_through_json`
Expected: `error[E0433]` / `cannot find ... WorkflowDef` — compile failure (the type does not exist yet).

- [ ] **Step 3: Implement the mirror types**

Prepend to `store.rs` (above the test module):

```rust
//! Workflow definition store: serialize a `DiGraph` DAG to disk and reload/run it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::digraph::graph::{DiGraph, JoinStrategy};
use super::digraph::state::GraphState;
use super::digraph::{Edge, Node};
use super::registry::AgentRegistry;

/// Serde-friendly mirror of [`JoinStrategy`] (which holds a `Duration` and has no derives).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JoinStrategyDef {
    All,
    Any,
    FirstN(usize),
    TimeoutAll { millis: u64 },
}

impl From<&JoinStrategy> for JoinStrategyDef {
    fn from(j: &JoinStrategy) -> Self {
        match j {
            JoinStrategy::All => JoinStrategyDef::All,
            JoinStrategy::Any => JoinStrategyDef::Any,
            JoinStrategy::FirstN(n) => JoinStrategyDef::FirstN(*n),
            JoinStrategy::TimeoutAll(d) => JoinStrategyDef::TimeoutAll { millis: d.as_millis() as u64 },
        }
    }
}

impl From<&JoinStrategyDef> for JoinStrategy {
    fn from(j: &JoinStrategyDef) -> Self {
        match j {
            JoinStrategyDef::All => JoinStrategy::All,
            JoinStrategyDef::Any => JoinStrategy::Any,
            JoinStrategyDef::FirstN(n) => JoinStrategy::FirstN(*n),
            JoinStrategyDef::TimeoutAll { millis } => JoinStrategy::TimeoutAll(Duration::from_millis(*millis)),
        }
    }
}

/// A serializable, on-disk representation of a [`DiGraph`] workflow.
///
/// Nodes are stored as a sorted `Vec` (not the runtime `HashMap`) so the JSON is
/// deterministic. `Node`/`Edge` already derive serde (`node.rs`, `edge.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub id: String,
    pub entry_node: String,
    pub join_strategy: JoinStrategyDef,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl WorkflowDef {
    /// Capture a live graph into a serializable definition.
    pub fn from_graph(g: &DiGraph) -> Self {
        let mut nodes: Vec<Node> = g.nodes.values().cloned().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            id: g.id.clone(),
            entry_node: g.entry_node.clone(),
            join_strategy: JoinStrategyDef::from(&g.join_strategy),
            nodes,
            edges: g.edges.clone(),
        }
    }

    /// Reconstruct an executable graph from this definition.
    pub fn to_graph(&self) -> DiGraph {
        let mut g = DiGraph::new(&self.id, &self.entry_node);
        g.join_strategy = JoinStrategy::from(&self.join_strategy);
        for n in &self.nodes {
            g.add_node(n.clone());
        }
        for e in &self.edges {
            g.add_edge(e.clone());
        }
        g
    }
}
```

Mount the module — edit `crates/runtime/src/impl/orchestration/mod.rs`:

```rust
pub mod store;
```
```rust
pub use store::{WorkflowDef, WorkflowStore};
```

> `WorkflowStore` is added in Phase 2; add its re-export now only if Phase 2 lands in the same change, otherwise re-export just `WorkflowDef` here and add `WorkflowStore` in Phase 2.

- [ ] **Step 4: Run — expected PASS.**

Run: `cargo test -p runtime orchestration::store::tests::workflow_def_round_trips_through_json`
Expected: `test result: ok. 1 passed`. Also `cargo build -p runtime` compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/orchestration/store.rs crates/runtime/src/impl/orchestration/mod.rs
git commit -m "feat(orchestration): add serde-serializable WorkflowDef mirror of DiGraph"
```

---

## Phase 2 — Filesystem workflow store (save / load / list)

### Task 2: `WorkflowStore` persists definitions as JSON under a directory

**Files:** Modify `crates/runtime/src/impl/orchestration/store.rs`.

- [ ] **Step 1: Write the failing test**

Add to the `store.rs` test module:

```rust
    #[test]
    fn store_saves_lists_and_reloads_losslessly() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();

        assert!(store.list().unwrap().is_empty());

        store.save("greet", &sample_graph()).unwrap();
        store.save("deploy", &sample_graph()).unwrap();

        // list() is sorted and extension-stripped
        assert_eq!(store.list().unwrap(), vec!["deploy".to_string(), "greet".to_string()]);

        // load() reproduces an executable graph
        let g = store.load("greet").unwrap();
        assert_eq!(g.id, "wf-1");
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.topological_sort().unwrap(), vec!["a", "b"]);

        // the on-disk artifact is JSON named after the workflow
        assert!(dir.path().join("greet.json").exists());
    }
```

- [ ] **Step 2: Run — expected FAIL** (`WorkflowStore` undefined).

Run: `cargo test -p runtime orchestration::store::tests::store_saves_lists_and_reloads_losslessly`
Expected: compile failure — `cannot find type WorkflowStore`.

- [ ] **Step 3: Implement `WorkflowStore`**

Append to `store.rs` (after the `WorkflowDef` impl):

```rust
/// A filesystem-backed store of named workflow definitions (one JSON file each).
///
/// Mirrors the `~/.aletheon/` convention from `base::paths`; the default store
/// dir is `~/.aletheon/workflows`.
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
        base::paths::config_dir().join("workflows")
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    /// Persist `graph` under `name` (overwrites an existing definition).
    pub fn save(&self, name: &str, graph: &DiGraph) -> anyhow::Result<()> {
        let def = WorkflowDef::from_graph(graph);
        let json = serde_json::to_string_pretty(&def)?;
        std::fs::write(self.path_for(name), json)?;
        Ok(())
    }

    /// Load and reconstruct the executable graph stored under `name`.
    pub fn load(&self, name: &str) -> anyhow::Result<DiGraph> {
        let text = std::fs::read_to_string(self.path_for(name))
            .map_err(|e| anyhow::anyhow!("workflow '{name}' not found: {e}"))?;
        let def: WorkflowDef = serde_json::from_str(&text)?;
        Ok(def.to_graph())
    }

    /// List saved workflow names (sorted, `.json` extension stripped).
    pub fn list(&self) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
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
}
```

> `base::paths::config_dir()` is available because `runtime` depends on `base`
> (`runtime/Cargo.toml:17`) and `base` re-exports `paths` (`base/src/lib.rs:53`).
> `tempfile` is already a `runtime` dev-dependency (`runtime/Cargo.toml:48`).

- [ ] **Step 4: Run — expected PASS.** Also `cargo test -p runtime orchestration::store`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/orchestration/store.rs crates/runtime/src/impl/orchestration/mod.rs
git commit -m "feat(orchestration): add filesystem WorkflowStore (save/load/list)"
```

---

## Phase 3 — Run a saved workflow (reuse == identical execution)

### Task 3: `WorkflowStore::run` loads by name and executes; result matches a direct run

**Files:** Modify `crates/runtime/src/impl/orchestration/store.rs`.

- [ ] **Step 1: Write the failing test**

The spec's acceptance criterion is *"save → reload → run reproduces the same execution."* Use `Branch` nodes so no agents/registry entries are needed (`graph.rs:221-230` executes them without a lookup); compare the `(node_id, status)` sequence from the execution log (timestamps differ, so compare only those pairs).

```rust
    #[tokio::test]
    async fn run_saved_workflow_reproduces_direct_execution() {
        use crate::r#impl::orchestration::digraph::state::GraphState;

        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path()).unwrap();
        let registry = AgentRegistry::new();

        // Direct execution of the in-memory graph.
        let direct = sample_graph()
            .execute(&registry, GraphState::new())
            .await
            .unwrap();
        let direct_trace: Vec<(String, String)> =
            direct.log.iter().map(|e| (e.node_id.clone(), e.status.clone())).collect();

        // Save, then run by name.
        store.save("wf", &sample_graph()).unwrap();
        let replayed = store.run("wf", &registry, GraphState::new()).await.unwrap();
        let replayed_trace: Vec<(String, String)> =
            replayed.log.iter().map(|e| (e.node_id.clone(), e.status.clone())).collect();

        assert_eq!(direct_trace, replayed_trace, "reloaded run must reproduce the direct run");
        assert!(!replayed_trace.is_empty());
    }
```

- [ ] **Step 2: Run — expected FAIL** (`WorkflowStore::run` undefined).

Run: `cargo test -p runtime orchestration::store::tests::run_saved_workflow_reproduces_direct_execution`
Expected: compile failure — `no method named run`.

- [ ] **Step 3: Implement `run`**

Add to `impl WorkflowStore`:

```rust
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
```

> `DiGraph::execute` returns `Result<GraphState, String>` (`graph.rs:106-110`); we
> map the `String` error into `anyhow::Error` to match the store's `Result` type.

- [ ] **Step 4: Run — expected PASS.** Full module: `cargo test -p runtime orchestration::store`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/orchestration/store.rs
git commit -m "feat(orchestration): WorkflowStore::run loads and executes saved workflows"
```

---

## Phase 4 — CLI surface: list / run saved workflows

`interact` cannot link `runtime` (that would form a cycle `runtime → cognit → interact`; see Ground truth). So this phase adds only the **argument-parsing surface** to the `aletheon` CLI. The actual list/run is served by the daemon (which owns the `WorkflowStore`) over the existing Unix-socket JSON-RPC path; wiring that request/response is a documented follow-on, not part of this phase's testable deliverable.

### Task 4: Add `Command::Workflow` + `WorkflowAction` to the CLI

**Files:** Modify `crates/interact/src/tui/cli.rs`.

- [ ] **Step 1: Write the failing test**

Add to a `#[cfg(test)] mod` in `cli.rs` (clap can be exercised without a daemon):

```rust
#[cfg(test)]
mod workflow_cli_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_workflow_list() {
        let args = Args::try_parse_from(["aletheon", "workflow", "list"]).unwrap();
        assert!(matches!(args.command, Some(Command::Workflow { action: WorkflowAction::List })));
    }

    #[test]
    fn parses_workflow_run_with_name() {
        let args = Args::try_parse_from(["aletheon", "workflow", "run", "deploy"]).unwrap();
        match args.command {
            Some(Command::Workflow { action: WorkflowAction::Run { name } }) => assert_eq!(name, "deploy"),
            other => panic!("unexpected parse: {other:?}"),
        }
    }
}
```

> If `Args`/`Command` do not already derive `Debug`, the `{other:?}` panic arm
> needs it; either add `#[derive(Debug)]` to `Command` or drop the `{other:?}`
> and `panic!("unexpected parse")`. Verify before writing (`cli.rs:70`).

- [ ] **Step 2: Run — expected FAIL** (`Command::Workflow` / `WorkflowAction` undefined).

Run: `cargo test -p interact workflow_cli_tests`
Expected: compile failure — `no variant Workflow`.

- [ ] **Step 3: Add the subcommand**

Add a variant to `pub enum Command` (`cli.rs:70`):

```rust
    /// Saved workflow management (list / run)
    #[command(alias = "wf")]
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
```

Add the action enum next to `Command`:

```rust
/// Actions for the `workflow` subcommand.
#[derive(clap::Subcommand)]
pub enum WorkflowAction {
    /// List saved workflows
    List,
    /// Run a saved workflow by name
    Run {
        /// Workflow name (as shown by `workflow list`)
        name: String,
    },
}
```

> This mirrors the existing nested-subcommand pattern already used by
> `Command::Daemon { action: DaemonAction }` and `Command::Debug { action }`
> (`cli.rs:71-103`). Dispatch (send a `workflow.list`/`workflow.run` request over
> the socket, print the reply) is added where the other `Command` arms are
> handled — verify the match site before wiring; keep this task to the parse
> surface so it is unit-testable without a running daemon.

- [ ] **Step 4: Run — expected PASS.** Also `cargo build -p interact`.

Run: `cargo test -p interact workflow_cli_tests`

- [ ] **Step 5: Commit**

```bash
git add crates/interact/src/tui/cli.rs
git commit -m "feat(interact): add `workflow list|run` CLI surface"
```

---

## Phase 5 — Multi-repo extraction readiness (4b) — GATED BEHIND TIER 2

> **Hard gate.** Do NOT start Phase 5 until **Tier 2** has landed: **2b** (`RuntimeHost` trait — `docs/plans/2026-07-01-modules-roadmap-design.md:141`) and **2c** (break the `cognit → corpus/interact` inversion — spec:165). Until 2c lands, `cognit` still depends on `corpus`+`interact` (`crates/cognit/Cargo.toml:9-11`) and `runtime` is still a god crate (`runtime/Cargo.toml:17-22`) — the kernel boundary this phase documents is not yet real. This phase is **documentation + build verification only**; it writes NO product code, so its "tests" are `cargo build`/`cargo tree` assertions rather than Rust unit tests.

### Task 5: Document the dependency boundary + prove `base` extracts cleanly

**Files:** none in product code. This task produces a boundary write-up appended to this plan's follow-up notes (or a short `docs/architecture/` note if the implementer prefers) plus the verification runs below. **Do not edit any `.rs`/`.toml`.**

- [ ] **Step 1: Capture the current dependency graph as the baseline**

Run:
```bash
cd /home/rj001/Bear-ws/work/aletheon
cargo tree -p base -e no-dev --prefix depth | grep -v '^0' || true   # expect: no intra-workspace deps
cargo tree -p cognit -e no-dev | grep -E 'corpus|interact'            # expect: shows the inversion (until 2c lands)
cargo tree -p runtime -e no-dev | grep -E 'base|cognit|corpus|memory|dasein|metacog'
```
Expected today: `base` lists only external crates (no `corpus`/`cognit`/etc.); `cognit` still shows `corpus` and `interact`; `runtime` shows all five siblings. Record this as the "before Tier 2" baseline.

- [ ] **Step 2: Classify each crate's extraction readiness (the boundary doc)**

Produce this table (verified from the per-crate `Cargo.toml` path-deps in Ground truth):

| Crate | Intra-workspace deps | Extractable today? | Blocker |
|---|---|---|---|
| `base` | none | ✅ yes (leaf ABI) | — |
| `memory` | `base` | ✅ yes | — |
| `corpus` | `base` | ✅ yes | — |
| `metacog` | `base` | ✅ yes | — |
| `interact` | `base`, `corpus` | ⚠️ yes but heavy (UI) | — |
| `cognit` | `base`, `corpus`, `interact` | ❌ no | **2c** inversion (Brain → Body + UI) |
| `dasein` | `base`, `corpus`, `cognit`, `memory` | ❌ no | transitively blocked by `cognit` (2c) |
| `runtime` | all five siblings | ❌ no | god crate; needs **2b** `RuntimeHost` kernel split + **2c** |

Kernel boundary target (post-Tier-2): `RuntimeCore` depends only on `base` traits; `cognit`/`corpus`/`memory`/`metacog` are wired in behind Provider/Memory/Plugin SDK traits (in `base` or dedicated `*-sdk` crates), not compile-time hard deps of the core (spec:272-285).

- [ ] **Step 3: Proof-of-extraction — `base` builds standalone**

The concrete, incremental deliverable (spec: *"first prove `base` + one capability crate extract cleanly; do not big-bang the split"*). Copy `base` to a throwaway location outside the workspace and build it in isolation:

```bash
cd /home/rj001/Bear-ws/work/aletheon
TMP=$(mktemp -d)
cp -R crates/base "$TMP/base"
cd "$TMP/base"
# base uses workspace inheritance (version.workspace = true); pin locally so it
# builds standalone. Do NOT commit this edit — it lives only in the temp copy.
sed -i.bak 's/^version.workspace = true/version = "0.1.0"/; s/^edition.workspace = true/edition = "2021"/; s/^license.workspace = true/license = "MIT"/' Cargo.toml
# nix/libc/dashmap are workspace deps in base/Cargo.toml; pin them to match root:
#   nix = { version = "0.29", features = ["user", "ioctl"] }
#   libc = "0.2"  |  dashmap = "6"  (see root Cargo.toml [workspace.dependencies])
cargo build 2>&1 | tail -20
```
Expected: `base` compiles in isolation once workspace-inherited fields are pinned, confirming it has no hidden path coupling to sibling crates. Record the exact edits needed as the "extraction checklist" for `base`. Clean up: `rm -rf "$TMP"`.

> Verify against `crates/base/Cargo.toml` before running: any dep marked
> `{ workspace = true }` (e.g. `nix`, `libc`, `dashmap`) must be pinned to the
> root `[workspace.dependencies]` value in the standalone copy. This proves the
> extraction procedure without touching the in-tree crate.

- [ ] **Step 4: Whole-workspace still green (no regression)**

Run:
```bash
cd /home/rj001/Bear-ws/work/aletheon
cargo build --workspace
cargo test -p runtime orchestration::store
```
Expected: workspace builds; the Phase 1–3 store tests still pass. (Phase 5 changed no product code, so this is a guard that nothing drifted.)

- [ ] **Step 5: Commit (docs only)**

```bash
git add docs/
git commit -m "docs(tier4): document crate extraction-readiness boundary + base standalone proof"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** serde-on-graph (Task 1) ↔ 4a "serialize a `DiGraph` … serde on graph"; store save/load/list (Task 2) + run (Task 3) ↔ 4a API `save/load/list/run` and "save → reload → run reproduces the same execution; round-trip serde lossless"; CLI (Task 4) ↔ 4a "CLI surface in interact to list/run saved workflows"; boundary doc + `base` proof (Task 5) ↔ 4b "documented dependency-boundary + one proof-of-extraction", gated behind Tier 2.
- **Placeholder scan:** none — real serde types compiled against the actual `Node`/`Edge`/`JoinStrategy`/`GraphState` definitions; exact `cargo`/`cargo tree` commands; real `clap` subcommand mirroring the existing `Daemon`/`Debug` pattern.
- **Type consistency:** `WorkflowDef` sub-fields (`Vec<Node>`, `Vec<Edge>`) already derive serde (`node.rs:29`, `edge.rs:5`); `JoinStrategyDef` covers all four `JoinStrategy` variants (`graph.rs:11-20`) including the `Duration` one; `WorkflowStore::run` maps `DiGraph::execute`'s `Result<_, String>` (`graph.rs:106`) into `anyhow`; `base::paths::config_dir()` reachable via `runtime → base` (`runtime/Cargo.toml:17`, `base/src/lib.rs:53`).
- **Package-name check:** `-p runtime` / `-p interact` / `-p base` / `-p cognit` match root `Cargo.toml:4-11` `[package]` names.

## Risks / notes for the implementer

- **Do not mutate the live executor types.** The plan deliberately mirrors `DiGraph`/`JoinStrategy` into serde types rather than deriving serde on the execution structs — adding derives to `DiGraph` would also require serde on `JoinStrategy`'s `Duration` and risks the executor. If a future need makes direct serde on `DiGraph` preferable, that is a separate decision.
- **`NodeKind::SubGraph` is not executed today** (`graph.rs:241-248` warns "not implemented"). A round-tripped workflow containing a `SubGraph` node will persist/load fine but still no-op at run time — persistence does not change execution semantics. Note this if you save nested workflows.
- **Journal/text-only tail caveat is unrelated here** — this store persists *workflow definitions*, not conversation history; keep it distinct from the session journal (M-A).
- **interact ⟂ runtime boundary is load-bearing.** Never add `runtime = { path = ... }` to `interact/Cargo.toml` to shortcut the CLI — it forms a cycle (`runtime → cognit → interact`). Route `workflow list/run` over the daemon socket like the other subcommands.
- **Phase 5 is gated, not optional-but-early.** Running Phase 5's extraction proof before Tier 2c lands will still "pass" for `base` (it is already a leaf), but the boundary table's `cognit`/`runtime` rows will not have changed — do not interpret a green `base` build as the org split being ready. The org split (creating repos, moving crates) remains explicitly out of scope (spec:280-283).
- **`sed`-based standalone edits in Task 5 are on a throwaway copy only.** They must never touch the in-tree `crates/base/Cargo.toml`; the whole point is to prove extraction without modifying the workspace.
