# Executive Crate — Core Runtime Infrastructure

> Code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

**Crate:** `executive`
**Purpose:** The core runtime that drives agent reasoning, manages sessions, orchestrates multi-agent collaboration, and hosts plugins and automations.

---

## Internal Structure

```
executive/src/
  core/
    mod.rs                      # Core types
    config/                     # Configuration (agent, genome, infra, provider)
    session_gateway/            # Session gateway and turn context
    runtime_core.rs             # RuntimeCore (host-agnostic bootstrap)
    core_systems.rs             # CoreSystems (aggregated subsystem wiring)
    orchestrator.rs             # Orchestrator
    ...
  host/
    mod.rs                      # Host types
    systemd.rs                  # SystemdHost
    container.rs                # ContainerHost
  service/
    mod.rs
    turn_service.rs             # TurnService stream coordination
    daemon_turn/                # DaemonTurnOrchestrator
      mod.rs
      orchestrator.rs
      session.rs
      execute.rs
      ...
  tools/
    mod.rs                      # Bridge tool registry
  impl/
    mod.rs
    coordinator.rs              # Runtime coordinator
    engine/                     # Cognitive engine (ReAct loop)
      mod.rs
      config.rs                 # Engine configuration
      modules/                  # Pluggable engine modules
        mod.rs
        body_module.rs          # Corpus/tool integration
        memory_module.rs        # Memory subsystem integration
        perception_module.rs    # Perception feed integration
        self_field_module.rs    # Dasein self-field integration
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
| Memory | [../mnemosyne/memory-system.md](../mnemosyne/memory-system.md) | L1/L2/L3 memory, compression, scoping |
| Session | [session.md](session.md) | Persistence, EventJournal, crash recovery |
| Orchestration | [orchestration.md](orchestration.md) | Multi-agent strategies, delegation |
| Observability | [observability.md](observability.md) | Metrics, reasoning logs, debug CLI |
| Plugin | [plugin.md](plugin.md) | External tool/hook plugins |
| Automation | [automation.md](automation.md) | Cron/webhook/API-triggered routines |

## Related Crates

- `fabric` — Trait definitions, shared types, IPC (CommunicationBus)
- `mnemosyne` — Backend memory storage (episodic, semantic, procedural)
- `corpus` — Tools, sandbox, security, platform adapters
- `cognit` — LLM inference
- `dasein` — Perception, hooks
- `metacog` — Self-modification engine (MetaRuntime, morphogenesis)
