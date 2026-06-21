> New document — code paths reflect aletheon-* crate structure

# ACIX (Agent-Computer Interface)

> High-level perception, action grounding, experience memory, and task management for computer-use agents.

**Crate:** `corpus`
**Module:** `crates/corpus/src/impl/acix/`
**Last updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Aci (main interface) | ✅ Implemented | `acix/aci.rs` | High-level observe-act loop |
| GroundingProvider | ✅ Implemented | `acix/grounding.rs` | Visual grounding (locate elements on screen) |
| ExperienceMemory | ✅ Implemented | `acix/experience.rs` | Experience storage and retrieval |
| TaskManager | ✅ Implemented | `acix/task.rs` | Task decomposition and execution graph |

---

## 1. Architecture

ACIX provides a high-level abstraction over the raw driver subsystem, implementing the Agent-Computer Interface pattern:

```
User Goal
    │
    ▼
TaskManager
    ├── TaskDecomposer — break goal into steps
    ├── TaskGraph — dependency graph of actions
    └── TaskWorker — execute steps sequentially
         │
         ▼
    Aci (observe-act loop)
         ├── observe() → GroundingProvider → locate UI elements
         ├── act() → perform action (click, type, scroll)
         └── record() → ExperienceMemory → store for future reference
```

## 2. ACI Protocol

**Aci** — the main Agent-Computer Interface, implementing an observe-act loop:

```rust
struct Aci {
    grounding: Box<dyn GroundingProvider>,
    experience: ExperienceMemory,
    driver: DriverFactory,
}

impl Aci {
    /// Observe the current screen state
    async fn observe(&self) -> Observation { ... }

    /// Perform an action (click, type, scroll, key press)
    async fn act(&self, action: Action) -> Result<()> { ... }

    /// Full observe-act cycle with experience recording
    async fn step(&mut self, goal: &str) -> Result<StepResult> { ... }
}
```

## 3. Grounding

**GroundingProvider** — locates UI elements on screen by natural language description:

```rust
trait GroundingProvider {
    /// Find element matching description
    fn ground(&self, description: &str, observation: &Observation) -> GroundingResult;
}

struct GroundingResult {
    element: Option<Element>,
    confidence: f32,
    bounds: Option<Bounds>,
}
```

**Grounding strategies:**
- AT-SPI accessibility tree search (role + name matching)
- OCR text search (find text on screen)
- Coordinate-based (direct pixel coordinates)

**MockGroundingProvider** — test implementation with pre-set responses.

## 4. Experience Memory

**ExperienceMemory** — stores action records for learning and retrieval:

```rust
struct ExperienceMemory {
    records: Vec<ActionRecord>,
    embedder: Box<dyn Embedder>,
}

struct ActionRecord {
    observation: Observation,
    action: Action,
    result: ActionResult,
    timestamp: Instant,
    success: bool,
}

enum ExperienceLevel {
    /// Just observed, not yet acted
    Observed,
    /// Acted once, uncertain outcome
    Attempted,
    /// Successfully completed multiple times
    Proven,
}
```

**Embedder** — trait for computing embeddings of experience records for similarity search.

**MockEmbedder** — test implementation.

## 5. Task Management

**TaskManager** — decomposes goals into executable task graphs:

```rust
struct TaskManager {
    decomposer: Box<dyn TaskDecomposer>,
    worker: TaskWorker,
}

trait TaskDecomposer {
    fn decompose(&self, goal: &str) -> TaskGraph;
}

struct TaskGraph {
    nodes: Vec<TaskNode>,
    edges: Vec<(usize, usize)>,  // dependency edges
}

struct TaskNode {
    id: usize,
    action: TaskAction,
    status: TaskStatus,
}

enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
    Skipped,
}
```

**TaskWorker** — executes task graph nodes in dependency order, respecting edges.

## 6. Integration Points

| Component | Integration |
|-----------|------------|
| Driver subsystem | ACIX uses `DriverFactory` for raw input/output |
| Perception layer | Observations feed into `PerceptionEvent` stream |
| Tool system | ACIX actions exposed as tools (computer_click, computer_type, etc.) |
| Experience memory | Persists across sessions for learning |

## 7. Implementation Notes

**Code location:** `crates/corpus/src/impl/acix/` (5 files: `mod.rs`, `aci.rs`, `experience.rs`, `grounding.rs`, `task.rs`)

**Key design decisions:**
- Trait-based grounding allows multiple strategies (AT-SPI, OCR, coordinate)
- Experience memory with embedding-based retrieval for similar past actions
- Task graph with dependency edges for complex multi-step operations
- Mock implementations for testing (`MockGroundingProvider`, `MockEmbedder`)
