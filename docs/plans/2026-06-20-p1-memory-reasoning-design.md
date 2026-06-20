# P1: Memory → Reasoning Integration

**Date**: 2026-06-20
**Status**: 🟡 In Progress
**Goal**: Make the 4-type memory system actually influence reasoning in the ReActLoop path.

## Problem

Two disconnected memory systems:
1. `aletheon-memory` (4-type MemoryRouter with SQLite) — rich, structured, but **never queried during reasoning**
2. Runtime memory (CoreMemory blocks + FactStore + RecallInjector) — simpler, wired into legacy Engine only

The newer ReActLoop/AletheonRuntime path completely skips memory. The reasoner never queries past reflections, stored knowledge, or learned skills before acting.

## Design

### Key Insight

We don't need to unify both memory stacks. We just need to make the **ReActLoop path** query `MemoryRouter` before each LLM call and inject the results into context.

### Architecture

```
User prompt
  → MemoryRecaller::recall(prompt)         # NEW
    → episodic.recall(recent reflections)   # last N reflections
    → semantic.recall(keyword search)       # FTS5 on prompt keywords
    → procedural.recall(matching skills)    # skills relevant to intent
  → MemoryContext { reflections, knowledge, skills }
  → injected into system prompt or user message
  → LLM call with memory-enriched context
```

### Tasks

| # | Task | Files | Status |
|---|---|---|---|
| 1 | **MemoryContext struct** — lightweight recall result bundle | aletheon-memory/src/core/manager.rs | 🟡 |
| 2 | **MemoryRecaller** — queries MemoryRouter, produces MemoryContext | aletheon-memory/src/core/manager.rs | 🟡 |
| 3 | **Wire into orchestrator** — call MemoryRecaller before LLM | aletheon-runtime/src/core/orchestrator.rs | 🟡 |
| 4 | **Format for injection** — render MemoryContext into prompt section | aletheon-runtime/src/core/orchestrator.rs | 🟡 |
| 5 | **Tests** — unit + integration | both crates | 🟡 |
| 6 | **Validation** — cargo test, commit, PR | — | 🟡 |

### MemoryContext Struct

```rust
/// Lightweight bundle of memories recalled for a specific prompt.
pub struct MemoryContext {
    /// Recent reflections (from episodic memory)
    pub recent_reflections: Vec<ReflectionSummary>,
    /// Relevant knowledge (from semantic memory, keyword search)
    pub relevant_knowledge: Vec<String>,
    /// Matching skills/procedures (from procedural memory)
    pub matching_skills: Vec<SkillSummary>,
}

pub struct ReflectionSummary {
    pub content: String,
    pub occurred_at: i64,
    pub tool_used: Option<String>,
}

pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub success_rate: f64,
}
```

### Recall Logic

```rust
impl MemoryRouter {
    pub async fn recall_for_prompt(&self, prompt: &str, max_items: usize) -> MemoryContext {
        // 1. Episodic: recent reflections (last 5)
        let reflections = self.episodic.recall_recent_reflections(max_items).await;

        // 2. Semantic: keyword search on prompt
        let knowledge = self.semantic.search_keywords(prompt, max_items).await;

        // 3. Procedural: skills matching prompt intent
        let skills = self.procedural.search_skills(prompt, max_items).await;

        MemoryContext { recent_reflections: reflections, relevant_knowledge: knowledge, matching_skills: skills }
    }
}
```

### Injection Point

In `AletheonRuntime::process_react()`, after building the system prompt with care weights, append a memory section:

```
## Relevant Memory
### Recent Reflections
- [reflection 1]
- [reflection 2]
### Relevant Knowledge
- [knowledge 1]
### Matching Skills
- [skill 1] (success rate: 85%)
```

This gets appended to the system prompt so the LLM sees it before reasoning.

### Design Decisions

1. **Prompt-based recall, not embedding-based** — FTS5 keyword search is good enough for v1. Embeddings can come later.
2. **System prompt injection** — append to system prompt (same pattern as care weights), not user message. This keeps the user message clean.
3. **Async recall** — MemoryRouter operations are async, so the orchestrator needs to handle this. Use `tokio::spawn` or `.await`.
4. **Max items cap** — limit recall to 3-5 items per category to avoid prompt bloat.
5. **No new crate** — MemoryRecaller lives inside aletheon-memory, exposed via a new method on MemoryRouter.

### Risks

1. **MemoryRouter requires paths** — constructor needs `(episodic_path, semantic_path, procedural_path, self_path)`. Orchestrator needs to receive these or an already-constructed MemoryRouter.
2. **Legacy vs new path** — the Engine path already has RecallInjector. We're adding memory to the ReActLoop path only. This is intentional — the two paths converge later.
3. **Async complexity** — MemoryRouter is async, ReActLoop is async. Should be straightforward with tokio.

### Dependencies

- `aletheon-memory` crate must be added to `aletheon-runtime/Cargo.toml` if not already present
- `aletheon-abi` types (MemoryEntry, MemoryQuery, etc.) already shared

## Philosophy Connection

- **Spinoza's idea ideae**: Memory recall = the mind knowing what it knows before acting
- **Heidegger's Geworfenheit**: Past reflections shape current interpretation (thrown-into-situation)
- **Growth principle**: With memory influencing reasoning, the agent can learn from past mistakes in real-time
