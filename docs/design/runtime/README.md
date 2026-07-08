# Runtime Crate — Core Runtime Infrastructure

> Code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

**Crate:** `runtime`
**Purpose:** The core runtime that drives agent reasoning, manages sessions, orchestrates multi-agent collaboration, and hosts plugins and automations.

---

## Internal Structure

```
runtime/src/
  impl/
    mod.rs
    coordinator.rs              # Runtime coordinator
    engine/                     # Cognitive engine (ReAct loop)
      mod.rs
      cognitive_loop.rs         # Core ReAct loop implementation
      config.rs                 # Engine configuration
      memory_integration.rs     # Memory integration
      streaming.rs              # Streaming support
      tool_dispatch.rs          # Tool dispatch logic
    memory/                     # Runtime-level memory (L1/L2/L3)
      mod.rs
      core_memory.rs            # CoreMemory (L1) — block-based in-context
      recall_memory.rs          # RecallMemory (L2) — SQLite-backed
      archival_memory.rs        # ArchivalMemory (L3) — vector-backed
      tools.rs                  # Memory self-edit tools
      budget.rs                 # ContextBudget (token tracking)
      scope.rs                  # MemoryScope (Global/Session/Agent)
      compaction.rs             # Legacy compaction (kept for reference)
      vector_store.rs           # VectorStore trait + implementations
      compressor/               # Advanced compression
        mod.rs                  # AdvancedCompressor
        tail.rs                 # TailProtectionConfig
        template.rs             # SummaryTemplate
      pipeline/                 # Two-phase consolidation
        mod.rs                  # MemoryPipeline
        phase1.rs               # Phase1Extractor
        phase2.rs               # Phase2Consolidator
        state_db.rs             # StateDatabase
    session/                    # Session persistence and lifecycle
      mod.rs
      store.rs                  # SessionStore (CRUD + metadata)
      journal.rs                # EventJournal (JSONL + SQLite index)
      observability/            # Observability stack
        mod.rs
        fragment.rs             # FragmentAccumulator
        metrics.rs              # MetricsExporter (Prometheus)
        publisher.rs            # EventPublisher
        reasoning_logger.rs     # ReasoningLogger
        tool_tracker.rs         # ToolTracker
    orchestration/              # Multi-agent orchestration
      mod.rs
      agent.rs                  # Agent trait
      registry.rs               # AgentRegistry
      delegate.rs               # DelegateTool
      selector.rs               # SelectorStrategy
      handoff.rs                # HandoffStrategy
      termination.rs            # TerminationCondition
      budget.rs                 # IterationBudget
      config_agent.rs           # ConfigAgent
      builtin/                  # Built-in agents
        mod.rs
        fs_agent.rs
        net_agent.rs
        code_agent.rs
      digraph/                  # DAG orchestration
        mod.rs
        edge.rs
        node.rs
        state.rs
        graph.rs
    plugin/                     # Plugin subsystem
      mod.rs
      manifest.rs               # PluginManifest
      loader.rs                 # PluginLoader
      manager.rs                # PluginManager
      runtime.rs                # PluginRuntime
    automation/                 # Automation / routines
      mod.rs
      cron.rs                   # CronParser
      delivery.rs               # DeliveryManager
      script.rs                 # ScriptRunner
      webhook.rs                # WebhookEvent
    agent/                      # Agent abstractions
      mod.rs
    daemon/                     # Daemon (aletheon daemon)
      mod.rs
      handler.rs                # Request handler
      server.rs                 # Server
```

## Subsystem Responsibilities

| Subsystem | Doc | Purpose |
|-----------|-----|---------|
| Engine | [react-loop.md](react-loop.md) | ReAct reasoning loop, ContentBlock protocol |
| Memory | [../memory/memory-system.md](../memory/memory-system.md) | L1/L2/L3 memory, compression, scoping |
| Session | [session.md](session.md) | Persistence, EventJournal, crash recovery |
| Orchestration | [orchestration.md](orchestration.md) | Multi-agent strategies, delegation |
| Observability | [observability.md](observability.md) | Metrics, reasoning logs, debug CLI |
| Plugin | [plugin.md](plugin.md) | External tool/hook plugins |
| Automation | [automation.md](automation.md) | Cron/webhook/API-triggered routines |

## Related Crates

- `base` — Trait definitions and shared types
- `memory` — Backend memory storage (episodic, semantic, procedural)
- `base` — IPC and inter-process communication
- `corpus` — Tools, sandbox, security, platform adapters
- `cognit` — LLM inference
- `dasein` — Perception, hooks
- `metacog` — Self-modification engine (MetaRuntime, morphogenesis)
