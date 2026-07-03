# Aletheon Codebase Summary — GPT Reference

> Generated: 2026-07-02 | Branch: `auro/feat/20260701-aletheon-governed-memory-design`
> Purpose: A factual summary of actual code structure for AI context injection.

---

## Project Identity

- **Name:** Aletheon
- **Subtitle:** A Persistent Self-Evolving Agent Runtime
- **Language:** Rust (edition 2021, rust-version 1.85)
- **Platform:** Linux (Arch primary), Android, Embedded
- **Repo:** `github.com/Aurobear/aletheon`
- **License:** MIT

---

## Cargo Workspace — 8 Crates + 2 Examples

```
Cargo.toml (workspace root)
├── crates/base/          →  crate: base
├── crates/cognit/        →  crate: cognit
├── crates/corpus/        →  crate: corpus
├── crates/dasein/        →  crate: dasein
├── crates/interact/      →  crate: interact       (binary: aletheon)
├── crates/memory/        →  crate: memory
├── crates/metacog/       →  crate: metacog
├── crates/runtime/       →  crate: runtime        (binaries: aletheond, aletheon-exec)
├── examples/basic-agent/
└── examples/self-evolution-loop/
```

---

## Crate 1: `base` — ABI / Shared Types

**Concept:** The ABI layer. Defines all shared types, IPC primitives, and kernel abstractions.

**Module tree:**
```
crates/base/src/
├── lib.rs
├── dasein/mod.rs                          # Dasein self-model types (shared)
├── events/
│   ├── mod.rs
│   ├── event.rs                           # Core event type
│   ├── event_bridge.rs                    # Event bridge between subsystems
│   ├── event_log.rs                       # Event logging
│   ├── evolution.rs                       # Evolution event types
│   ├── routing_policy.rs                  # Event routing rules
│   ├── subscription.rs                    # Event subscription model
│   └── ui_event.rs                        # UI-facing event types
├── include/
│   ├── mod.rs
│   ├── body.rs                            # Body subsystem trait
│   ├── brain.rs                           # BrainCore subsystem trait
│   ├── event_bus.rs                       # EventBus subsystem trait
│   ├── memory.rs                          # Memory subsystem trait
│   ├── meta.rs                            # MetaRuntime subsystem trait
│   ├── plugin.rs                          # Plugin subsystem trait
│   ├── runtime.rs                         # Runtime subsystem trait
│   ├── self_field.rs                      # SelfField subsystem trait
│   └── subsystem.rs                       # Base Subsystem trait definition
├── ipc/
│   ├── mod.rs
│   ├── envelope.rs                        # IPC message envelope
│   ├── ipc_msg.rs                         # IPC message types
│   ├── ipc_types.rs                       # IPC type definitions
│   ├── protocol.rs                        # Wire protocol
│   ├── backends/
│   │   ├── mod.rs
│   │   ├── io_uring.rs                    # io_uring backend
│   │   ├── io_uring_transport.rs
│   │   ├── json_rpc.rs                    # JSON-RPC backend
│   │   ├── json_rpc_transport.rs
│   │   ├── manager.rs                     # Backend manager
│   │   ├── priority_queue.rs              # Priority queue for messages
│   │   ├── shared_mem.rs                  # Shared memory backend
│   │   ├── shared_mem_transport.rs
│   │   ├── transport_adapter.rs
│   │   └── unix_socket.rs                 # Unix socket backend
│   ├── bus/
│   │   ├── mod.rs
│   │   ├── communication_bus.rs           # Communication bus abstraction
│   │   ├── in_process.rs                  # In-process bus
│   │   ├── kernel_bus.rs                  # Kernel-level bus
│   │   ├── pubsub.rs                      # Pub/sub messaging
│   │   └── request_response.rs            # Request/response pattern
│   └── transport/
│       ├── mod.rs
│       └── unix_socket_transport.rs
├── kernel/
│   ├── mod.rs
│   ├── debug.rs                           # Kernel debug facilities
│   ├── debug_bus.rs                       # Debug bus
│   ├── error/mod.rs                       # Error types
│   ├── observable.rs                      # Observable pattern
│   └── registry.rs                        # Subsystem registry
├── policy/
│   ├── mod.rs
│   ├── execpolicy.rs                      # Execution policy types
│   ├── permission_authority.rs            # Permission authority model
│   └── verifier.rs                        # Policy verifier
└── types/
    ├── mod.rs
    ├── agent.rs                           # Agent type definitions
    ├── capability.rs                      # Capability model
    ├── context.rs                         # Context type
    ├── genome.rs                          # Genome (self-description) type
    ├── grounding.rs                       # Grounding (world-connection) types
    ├── hook.rs / hook_ext.rs              # Hook system types
    ├── llm_types.rs                       # LLM provider types
    ├── message.rs                         # Message types
    ├── objective.rs                       # Objective/goal types
    ├── paths.rs                           # Path resolution
    ├── permission.rs                      # Permission types
    ├── resource.rs                        # Resource types
    ├── sandbox.rs                         # Sandbox types
    ├── tool.rs                            # Tool interface types
    └── vision.rs                          # Vision/perception types
```

---

## Crate 2: `dasein` — Self / Identity

**Concept:** The "self" layer. Identity, boundary, care (Sorge), narrative continuity, self-model.

**Module tree:**
```
crates/dasein/src/
├── lib.rs
├── bridge/
│   ├── mod.rs
│   ├── hook.rs                           # Hook bridge
│   ├── loop_detector.rs                  # Loop detection bridge
│   ├── perception.rs                     # Perception bridge
│   └── policy.rs                         # Policy bridge
├── core/
│   ├── mod.rs
│   ├── attention.rs                      # Attention mechanism
│   ├── awareness_growth.rs               # Awareness growth tracking
│   ├── boundary.rs                       # Self/other boundary
│   ├── care.rs                           # Care (Sorge) — what matters
│   ├── conflict.rs                       # Internal conflict resolution
│   ├── continuity.rs                     # Temporal self-continuity
│   ├── evolution_validator.rs            # Evolution validation
│   ├── identity.rs                       # Identity core
│   ├── mutation.rs                       # Self-mutation
│   ├── narrative.rs                      # Self-narrative
│   └── store.rs                          # Self-state persistence
├── dasein/
│   ├── mod.rs
│   ├── bewandtnis.rs                     # "Bewandtnis" — relevance/meaning
│   ├── care_structure.rs                 # Care structure model
│   ├── context_injection.rs              # Context injection
│   ├── event_bridge.rs                   # Dasein event bridge
│   ├── negativity.rs                     # Negativity/constraint awareness
│   ├── persistence.rs                    # Self persistence
│   ├── self_model.rs                     # Self model
│   ├── sorge.rs                          # Sorge (Heideggerian care)
│   ├── temporality.rs                    # Temporal awareness
│   └── types.rs                          # Dasein-specific types
├── impl/
│   ├── mod.rs
│   ├── llm_bridge.rs                     # LLM bridge for self-reflection
│   ├── hook/
│   │   ├── mod.rs
│   │   ├── config.rs
│   │   ├── dispatcher.rs
│   │   └── types.rs
│   ├── mutation/
│   │   ├── mod.rs
│   │   └── approver.rs                   # Mutation approval
│   ├── perception/
│   │   ├── mod.rs
│   │   ├── aggregator.rs                 # Perception aggregation
│   │   ├── bridge.rs
│   │   ├── event.rs
│   │   ├── manager.rs                    # Perception manager
│   │   ├── fuse/
│   │   │   ├── mod.rs
│   │   │   ├── controls.rs               # FUSE controls
│   │   │   ├── filesystem.rs             # FUSE filesystem
│   │   │   ├── mount.rs                  # FUSE mount
│   │   │   └── provider.rs               # FUSE provider
│   │   └── sources/
│   │       ├── mod.rs
│   │       ├── bottleneck_detector.rs
│   │       ├── ebpf_source.rs            # eBPF perception source
│   │       ├── inotify_source.rs         # inotify file watch
│   │       ├── journald_source.rs        # systemd journal
│   │       └── proc_source.rs            # /proc filesystem
│   ├── resilience/
│   │   ├── mod.rs
│   │   ├── guardian.rs                   # Guardian process
│   │   ├── safe_mode.rs                  # Safe mode
│   │   └── watchdog.rs                   # Watchdog
│   └── security/
│       ├── mod.rs
│       ├── audit.rs
│       ├── circuit_breaker.rs
│       ├── loop_detector.rs
│       ├── output_guardrail.rs
│       ├── policy.rs
│       ├── rate_limiting/
│       │   ├── mod.rs
│       │   ├── backpressure.rs
│       │   ├── flood_protector.rs
│       │   ├── token_limiter.rs
│       │   └── tool_limiter.rs
│       ├── risk_classifier.rs
│       ├── rollback/
│       │   ├── mod.rs
│       │   └── types.rs
│       ├── runner.rs
│       ├── sandbox/
│       │   ├── mod.rs
│       │   └── writable_root.rs
│       └── self_protection/
│           ├── mod.rs
│           ├── emergency_killswitch.rs
│           ├── input_sanitizer.rs
│           ├── integrity_monitor.rs
│           └── resource_governor.rs
└── testing/
    ├── mod.rs
    └── mock_perception.rs
```

---

## Crate 3: `cognit` — Brain / Cognition

**Concept:** Reasoning, planning, reflection, provider routing, LLM abstraction.

**Module tree:**
```
crates/cognit/src/
├── lib.rs
├── bridge/
│   ├── mod.rs
│   ├── dual_model.rs                     # Dual-model reasoning
│   ├── inference.rs                      # Inference bridge
│   ├── learning.rs                       # Learning bridge
│   └── llm.rs                            # LLM bridge
├── config/mod.rs                         # Configuration
├── core/
│   ├── mod.rs
│   ├── awareness.rs                      # Awareness core
│   ├── awareness_signal.rs               # Awareness signals
│   ├── brain_core_ops.rs                 # BrainCore operations
│   ├── brain_core_subsystem.rs           # BrainCore subsystem impl
│   ├── critic.rs                         # Self-critic
│   ├── evolution_trigger.rs              # Evolution triggers
│   ├── experience_summarizer.rs          # Experience summarization
│   ├── learner.rs                        # Learning engine
│   ├── planner.rs                        # Planner
│   ├── reasoner.rs                       # Reasoner
│   ├── reflector.rs                      # Reflector
│   ├── skill_extractor.rs                # Skill extraction from experience
│   ├── tests.rs                          # Core tests
│   └── world_model.rs                    # World model
├── impl/
│   ├── mod.rs
│   ├── provider_registry.rs              # Provider registry
│   ├── event_handlers/
│   │   ├── mod.rs
│   │   └── tool_observer.rs              # Tool execution observer
│   ├── grounding/
│   │   ├── mod.rs
│   │   └── vision.rs                     # Vision grounding
│   ├── inference/
│   │   ├── mod.rs
│   │   ├── classifier.rs                 # Inference classifier
│   │   ├── provider_config.rs            # Provider configuration
│   │   └── router.rs                     # Model router
│   ├── learning/
│   │   ├── mod.rs
│   │   ├── outcome.rs                    # Learning outcomes
│   │   ├── pattern.rs                    # Pattern extraction
│   │   └── rule.rs                       # Rule extraction
│   └── llm/
│       ├── mod.rs
│       ├── anthropic.rs                  # Anthropic/Claude provider
│       ├── ollama.rs                     # Ollama local provider
│       ├── openai_provider.rs            # OpenAI provider
│       ├── provider.rs                   # Provider trait
│       ├── provider_factory.rs           # Provider factory
│       ├── pulse.rs                      # Provider health pulse
│       └── scheduler.rs                  # Request scheduler
└── testing/
    ├── mod.rs
    └── mock_llm.rs                       # Mock LLM for testing
```

---

## Crate 4: `corpus` — Body / Execution

**Concept:** Tools, sandbox, perception drivers, MCP integration, security sandboxing.

**Module tree:**
```
crates/corpus/src/
├── lib.rs
├── bridge/mod.rs
├── core/
│   ├── mod.rs
│   └── conversions.rs                    # Type conversions
├── drivers/
│   ├── mod.rs
│   ├── driver/
│   │   ├── mod.rs
│   │   ├── factory.rs                    # Driver factory
│   │   ├── types.rs                      # Driver types
│   │   ├── a11y/
│   │   │   ├── mod.rs
│   │   │   └── atspi.rs                  # AT-SPI accessibility
│   │   ├── display/
│   │   │   ├── mod.rs
│   │   │   ├── clipboard.rs
│   │   │   ├── clipboard_x11.rs
│   │   │   ├── drm.rs                    # DRM display
│   │   │   ├── window.rs
│   │   │   ├── window_x11.rs
│   │   │   └── x11.rs
│   │   ├── input/
│   │   │   ├── mod.rs
│   │   │   └── uinput.rs                 # uinput device
│   │   ├── io/mod.rs
│   │   ├── ocr/
│   │   │   ├── mod.rs
│   │   │   └── tesseract.rs              # OCR via Tesseract
│   │   ├── proc/mod.rs                   # Process management
│   │   └── sandbox_driver/mod.rs
│   └── platform/
│       ├── mod.rs
│       ├── adapter.rs                    # Platform adapter
│       ├── android.rs                    # Android platform
│       ├── boot.rs                       # Boot sequence
│       ├── linux.rs                      # Linux platform
│       └── awareness/
│           ├── mod.rs
│           ├── communication.rs
│           ├── conflict.rs
│           ├── discovery.rs
│           └── lifecycle.rs
├── security/
│   ├── mod.rs
│   ├── sandbox/
│   │   ├── mod.rs
│   │   ├── backend.rs
│   │   ├── bubblewrap.rs                 # Bubblewrap sandbox
│   │   ├── bwrap_builder.rs
│   │   ├── container.rs
│   │   ├── env.rs
│   │   ├── executor.rs
│   │   ├── glob_scanner.rs
│   │   ├── noop.rs
│   │   ├── policy.rs
│   │   ├── process.rs
│   │   └── profile.rs
│   └── security/
│       ├── mod.rs
│       ├── approval.rs
│       ├── audit.rs
│       ├── circuit_breaker.rs
│       ├── exec_policy.rs
│       ├── loop_detector.rs
│       ├── output_guardrail.rs
│       ├── permission_rules.rs
│       ├── policy.rs
│       ├── risk_classifier.rs
│       ├── runner.rs
│       └── socket_approval.rs
├── tools/
│   ├── mod.rs
│   ├── hooks/
│   │   ├── mod.rs
│   │   ├── registry.rs
│   │   ├── runner.rs
│   │   └── types.rs
│   ├── mcp/
│   │   ├── mod.rs
│   │   ├── auth.rs                       # MCP auth
│   │   ├── client.rs                     # MCP client
│   │   ├── config.rs                     # MCP config
│   │   ├── manager.rs                    # MCP manager
│   │   ├── transport.rs                  # MCP transport
│   │   └── wrapper.rs                    # MCP tool wrapper
│   ├── skills/
│   │   ├── mod.rs
│   │   ├── loader.rs                     # Skill loader
│   │   └── markdown_skill.rs             # Markdown-defined skills
│   └── tools/
│       ├── mod.rs
│       ├── agent_tool.rs                 # Sub-agent tool
│       ├── apply_patch.rs
│       ├── bash_exec.rs                  # Bash execution tool
│       ├── code_graph.rs
│       ├── ebpf_compile.rs
│       ├── executor.rs                   # Tool executor
│       ├── exposure.rs
│       ├── file_read.rs
│       ├── file_search.rs
│       ├── file_write.rs
│       ├── glob.rs
│       ├── grep.rs
│       ├── kernel_build.rs
│       ├── module_build.rs
│       ├── module_load.rs
│       ├── output/
│       │   ├── mod.rs
│       │   ├── capture.rs
│       │   ├── config.rs
│       │   ├── persistence.rs
│       │   ├── pruner.rs
│       │   ├── truncation.rs
│       │   └── turn_budget.rs
│       ├── process_list.rs
│       ├── registry.rs                   # Tool registry
│       ├── script_tool.rs
│       ├── search/
│       │   ├── mod.rs
│       │   ├── agent_tool.rs
│       │   └── tool_search.rs
│       ├── system_status.rs
│       ├── task_tools.rs
│       ├── toolset.rs
│       ├── web_fetch.rs
│       └── web_search.rs
└── testing/
    ├── mod.rs
    └── mock_sandbox.rs
```

---

## Crate 5: `runtime` — Runtime / Orchestration

**Concept:** Cognitive loop, orchestration, daemon, session management, agent lifecycle, memory pipeline.

**Binaries:** `aletheond` (daemon), `aletheon-exec` (executor)

**Module tree:**
```
crates/runtime/src/
├── lib.rs
├── bin/
│   ├── aletheond.rs                      # Daemon binary entry
│   └── aletheon-exec.rs                  # Executor binary entry
├── bridge/mod.rs
├── core/
│   ├── mod.rs
│   ├── behavior_paths.rs                 # Behavior path definitions
│   ├── checkpoint.rs                     # Checkpoint/snapshot
│   ├── config/
│   │   ├── mod.rs
│   │   ├── agent.rs                      # Agent config
│   │   ├── genome.rs                     # Genome config
│   │   ├── infra.rs                      # Infrastructure config
│   │   └── provider.rs                   # Provider config
│   ├── controller.rs                     # Main controller
│   ├── event_sink.rs                     # Event sink
│   ├── evolution_coordinator.rs          # Evolution coordinator
│   ├── interrupt.rs                      # Interrupt handling
│   ├── mode_router.rs                    # Mode-based routing
│   ├── orchestrator.rs                   # Task orchestrator
│   ├── permission_manager.rs             # Permission management
│   ├── react_loop/
│   │   ├── mod.rs
│   │   ├── circuit_breaker.rs
│   │   ├── goal_tracker.rs
│   │   ├── reflection.rs
│   │   ├── step.rs                       # React loop step
│   │   ├── tool_budget.rs
│   │   └── tool_exec.rs                  # Tool execution in loop
│   ├── session.rs                        # Session management
│   ├── storm_breaker.rs                  # Storm breaker (circuit breaker variant)
│   ├── sub_agent.rs                      # Sub-agent management
│   └── verdict_handler.rs                # Verdict processing
├── host/mod.rs                           # Host abstraction
├── impl/
│   ├── mod.rs
│   ├── coordinator.rs                    # Coordinator implementation
│   ├── skill_router.rs                   # Skill-based routing
│   ├── agent/
│   │   ├── mod.rs
│   │   ├── budget.rs                     # Agent budget
│   │   ├── fork.rs                       # Agent forking
│   │   ├── harness.rs                    # Agent harness
│   │   └── process.rs                    # Agent process
│   ├── agent_loader/mod.rs               # Agent loader
│   ├── agents/
│   │   ├── mod.rs
│   │   └── loader.rs                     # Multi-agent loader
│   ├── automation/
│   │   ├── mod.rs
│   │   ├── cron.rs                       # Cron scheduling
│   │   ├── delivery.rs                   # Delivery automation
│   │   ├── script.rs                     # Script automation
│   │   └── webhook.rs                    # Webhook triggers
│   ├── daemon/
│   │   ├── mod.rs
│   │   ├── cache_shape.rs                # Cache shaping
│   │   ├── debug_handler.rs              # Debug handler
│   │   ├── handler/
│   │   │   ├── mod.rs
│   │   │   ├── chat.rs                   # Chat handler
│   │   │   ├── format.rs                 # Format handler
│   │   │   └── rpc.rs                    # RPC handler
│   │   ├── mcp_embedded.rs               # Embedded MCP server
│   │   ├── model_router.rs               # Model routing in daemon
│   │   ├── prefix_builder.rs             # Prefix building
│   │   ├── server.rs                     # Daemon server
│   │   └── session_manager.rs            # Session manager
│   ├── engine/
│   │   ├── mod.rs
│   │   ├── cognitive_loop.rs             # Cognitive loop engine
│   │   ├── config.rs                     # Engine config
│   │   ├── memory_integration.rs         # Memory integration
│   │   ├── modules/
│   │   │   ├── mod.rs
│   │   │   ├── body_module.rs
│   │   │   ├── memory_module.rs
│   │   │   ├── perception_module.rs
│   │   │   └── self_field_module.rs
│   │   ├── streaming.rs                  # Streaming responses
│   │   └── tool_dispatch.rs              # Tool dispatch
│   ├── goal/
│   │   ├── mod.rs
│   │   └── store.rs                      # Goal store
│   ├── hooks/
│   │   ├── mod.rs
│   │   ├── loader.rs                     # Hook loader
│   │   ├── registry.rs                   # Hook registry
│   │   ├── builtin/
│   │   │   ├── mod.rs
│   │   │   └── audit_hook.rs             # Built-in audit hook
│   │   └── lifecycle/
│   │       ├── mod.rs
│   │       ├── recall_inject.rs          # Recall injection hook
│   │       └── session_distiller.rs      # Session distillation hook
│   ├── kernel/
│   │   ├── mod.rs
│   │   ├── global_pool.rs                # Global thread/resource pool
│   │   ├── ipc.rs                        # IPC integration
│   │   ├── kernel.rs                     # Kernel runtime
│   │   └── supervisor.rs                 # Process supervisor
│   ├── memory/
│   │   ├── mod.rs
│   │   ├── archival_memory.rs            # Archival memory
│   │   ├── auto_memory.rs                # Auto-memory (automatic capture)
│   │   ├── budget.rs                     # Memory budget
│   │   ├── compaction.rs                 # Memory compaction
│   │   ├── compressor/
│   │   │   ├── mod.rs
│   │   │   ├── tail.rs                   # Tail compression
│   │   │   └── template.rs               # Template compression
│   │   ├── core_memory.rs                # Core memory
│   │   ├── core_memory_store.rs          # Core memory persistence
│   │   ├── fact_store/
│   │   │   ├── mod.rs
│   │   │   ├── index.rs                  # Fact index
│   │   │   └── query.rs                  # Fact query
│   │   ├── memory_pipeline.rs            # Memory pipeline
│   │   ├── pipeline/
│   │   │   ├── mod.rs
│   │   │   ├── phase1.rs                 # Pipeline phase 1
│   │   │   ├── phase2.rs                 # Pipeline phase 2
│   │   │   └── state_db.rs               # Pipeline state DB
│   │   ├── recall_memory.rs              # Recall memory
│   │   ├── scope.rs                      # Memory scoping
│   │   ├── tools.rs                      # Memory tools
│   │   └── vector_store.rs               # Vector store
│   ├── orchestration/
│   │   ├── mod.rs
│   │   ├── agent.rs                      # Orchestration agent
│   │   ├── budget.rs                     # Orchestration budget
│   │   ├── builtin/
│   │   │   ├── mod.rs
│   │   │   ├── code_agent.rs             # Built-in code agent
│   │   │   ├── fs_agent.rs               # Filesystem agent
│   │   │   └── net_agent.rs              # Network agent
│   │   ├── config_agent.rs               # Config agent
│   │   ├── delegate.rs                   # Task delegation
│   │   ├── digraph/
│   │   │   ├── mod.rs
│   │   │   ├── edge.rs                   # Task graph edge
│   │   │   ├── graph.rs                  # Task graph
│   │   │   ├── node.rs                   # Task graph node
│   │   │   └── state.rs                  # Graph state
│   │   ├── handoff.rs                    # Task handoff
│   │   ├── registry.rs                   # Agent registry
│   │   ├── selector.rs                   # Agent selector
│   │   ├── store.rs                      # Orchestration store
│   │   └── termination.rs                # Termination handling
│   ├── plugin/
│   │   ├── mod.rs
│   │   ├── loader.rs                     # Plugin loader
│   │   ├── manager.rs                    # Plugin manager
│   │   ├── manifest.rs                   # Plugin manifest
│   │   └── runtime.rs                    # Plugin runtime
│   ├── session/
│   │   ├── mod.rs
│   │   ├── journal.rs                    # Session journal
│   │   ├── store.rs                      # Session store
│   │   └── observability/
│   │       ├── mod.rs
│   │       ├── fragment.rs               # Observation fragment
│   │       ├── metrics.rs                # Metrics collection
│   │       ├── publisher.rs              # Metrics publisher
│   │       ├── reasoning_logger.rs       # Reasoning trace logger
│   │       └── tool_tracker.rs           # Tool usage tracker
│   └── skills/
│       ├── mod.rs
│       ├── inject.rs                     # Skill injection
│       ├── keyword_matcher.rs            # Keyword-based matching
│       ├── loader.rs                     # Skill loader
│       ├── manifest.rs                   # Skill manifest
│       └── plugin.rs                     # Skill plugin
└── tools/
    ├── mod.rs
    └── self_observe.rs                   # Self-observation tool
```

---

## Crate 6: `interact` — Interface / CLI + TUI

**Concept:** User-facing CLI and TUI client. Binary: `aletheon`.

**Module tree:**
```
crates/interact/src/
├── lib.rs
├── bin/aletheon.rs                       # CLI binary entry
├── acix/
│   ├── mod.rs
│   ├── aci.rs                            # ACI (Agent-Computer Interface)
│   ├── experience.rs                     # Experience recording
│   ├── grounding.rs                      # Grounding interface
│   ├── task.rs                           # Task interface
│   └── tools.rs                          # Tool interface
└── tui/
    ├── mod.rs
    ├── app/
    │   ├── mod.rs
    │   ├── key_handler.rs                # Key bindings
    │   ├── lifecycle.rs                  # App lifecycle
    │   └── submit.rs                     # Message submission
    ├── approval_dialog.rs                # Approval dialog UI
    ├── awareness.rs                      # Awareness display
    ├── chat.rs                           # Chat view
    ├── cli.rs                            # CLI parsing
    ├── command.rs                        # Command handling
    ├── completion.rs                     # Tab completion
    ├── computer.rs                       # Computer use mode
    ├── debug.rs                          # Debug view
    ├── goal.rs                           # Goal display
    ├── help_overlay.rs                   # Help overlay
    ├── history_search.rs                 # History search
    ├── input.rs                          # Input handling
    ├── markdown.rs                       # Markdown rendering
    ├── pager.rs                          # Pager
    ├── plan_view.rs                      # Plan visualization
    ├── render/
    │   ├── mod.rs
    │   ├── draw.rs                       # Drawing primitives
    │   ├── header.rs                     # Header rendering
    │   └── input_line.rs                 # Input line rendering
    ├── response.rs                       # Response handling
    ├── rpc_client.rs                     # RPC client to daemon
    ├── skill.rs                          # Skill UI
    ├── state.rs                          # UI state
    ├── status.rs                         # Status bar
    ├── streaming.rs                      # Streaming display
    ├── subagent_view.rs                  # Sub-agent view
    ├── term_compat.rs                    # Terminal compatibility
    ├── test_infra.rs                     # Test infrastructure
    ├── toolcard.rs                       # Tool card rendering
    └── workflow.rs                       # Workflow visualization
```

---

## Crate 7: `memory` — Cognitive Memory Backends

**Concept:** Episodic, semantic, procedural, and self-memory backends with activation, consolidation, and decay.

**Module tree:**
```
crates/memory/src/
├── lib.rs
├── backends/
│   ├── mod.rs
│   ├── episodic/
│   │   ├── mod.rs
│   │   ├── query.rs                      # Episodic query
│   │   ├── schema.rs                     # Episodic schema
│   │   └── storage.rs                    # Episodic storage
│   ├── procedural.rs                     # Procedural memory backend
│   ├── self_memory.rs                    # Self-memory backend
│   └── semantic/
│       ├── mod.rs
│       ├── query.rs                      # Semantic query
│       ├── schema.rs                     # Semantic schema
│       └── storage.rs                    # Semantic storage
├── ops/
│   ├── mod.rs
│   ├── activation.rs                     # Memory activation
│   ├── consolidation.rs                  # Memory consolidation
│   ├── decay.rs                          # Memory decay
│   ├── router.rs                         # Memory routing
│   └── schema.rs                         # Operation schema
└── testing/
    ├── mod.rs
    └── mock_memory.rs                    # Mock memory for testing
```

---

## Crate 8: `metacog` — Meta-Cognition / Self-Evolution

**Concept:** Self-evolution scaffolding, genome loading, mutation pipeline, meta-runtime.

**Module tree:**
```
crates/metacog/src/
├── lib.rs
├── bridge/
│   ├── mod.rs
│   ├── candidate_bridge.rs               # Candidate mutation bridge
│   └── genome_bridge.rs                  # Genome bridge
├── core/
│   ├── mod.rs
│   ├── meta_cognition.rs                 # Meta-cognition core
│   ├── traits.rs                         # Meta-cognition traits
│   └── types.rs                          # Meta-cognition types
└── impl/
    ├── mod.rs
    ├── event_handlers/
    │   ├── mod.rs
    │   └── mutation_executor.rs          # Mutation execution
    ├── genome/
    │   ├── mod.rs
    │   └── loader.rs                     # Genome loader
    ├── meta_runtime/
    │   ├── mod.rs
    │   ├── evaluator.rs                  # Meta-runtime evaluator
    │   ├── lineage.rs                    # Lineage tracking
    │   ├── migration.rs                  # Migration between versions
    │   ├── rollback.rs                   # Rollback
    │   ├── runtime_builder.rs            # Runtime builder
    │   ├── sandbox_runner.rs             # Sandboxed runner
    │   ├── self_reader.rs                # Self-code reader
    │   └── spec_editor.rs                # Spec editor
    └── morphogenesis/
        ├── mod.rs
        ├── candidate.rs                  # Mutation candidate
        ├── mutation_intent.rs            # Mutation intent
        └── pipeline.rs                   # Morphogenesis pipeline
```

---

## Crate Dependency Graph

```
aletheon (bin)   →  interact  →  base, corpus
aletheond (bin)  →  runtime   →  base, cognit, corpus, dasein, memory, metacog
aletheon-exec    →  /
cognit           →  base, corpus, interact  (* dependency inversion, to be fixed)
```

---

## Key Workspace Dependencies

| Dependency | Version | Purpose |
|---|---|---|
| tokio | 1 (full) | Async runtime |
| serde / serde_json | 1 | Serialization |
| anyhow | 1 | Error handling |
| tracing / tracing-subscriber | 0.1 / 0.3 | Structured logging |
| async-trait | 0.1 | Async trait support |
| uuid | 1 (v4, serde) | Unique identifiers |
| chrono | 0.4 (serde) | Time handling |
| rusqlite | 0.31 (bundled) | SQLite for persistence |
| reqwest | 0.12 (json, stream) | HTTP client |
| dashmap | 6 | Concurrent hashmap |
| nix | 0.29 (user, ioctl) | Linux syscall wrappers |
| tree-sitter / tree-sitter-rust | 0.25 / 0.24 | Code parsing |
| walkdir | 2 | Directory traversal |
| bitflags | 2 | Bitflag types |
| bincode | 1 | Binary serialization |
| toml | 0.8 | Config parsing |
| regex | 1 | Pattern matching |

---

## Binary Entry Points

| Binary | Crate | File |
|---|---|---|
| `aletheond` | runtime | `crates/runtime/src/bin/aletheond.rs` |
| `aletheon-exec` | runtime | `crates/runtime/src/bin/aletheon-exec.rs` |
| `aletheon` | interact | `crates/interact/src/bin/aletheon.rs` |

---

## Architecture Summary (Triune: Soul / Brain / Body)

```
User / Environment
        │
        ▼
  Intent Gateway
        │
        ▼
  ┌─────────────┐
  │   EventBus   │  ← all events, state, tasks flow through
  └─────────────┘
   │       │       │
   ▼       ▼       ▼
┌──────┐ ┌──────┐ ┌──────┐
│Self  │ │Brain │ │Body  │  ← SelfField / BrainCore / BodyRuntime
│Field │ │Core  │ │Runtime│
└──────┘ └──────┘ └──────┘
   │       │       │
   └───┬───┴───┬───┘
       ▼       ▼
   ┌──────────────┐
   │    Memory     │  ← Episodic / Semantic / Procedural / Self
   └──────────────┘
           │
           ▼
   ┌──────────────┐
   │ MetaRuntime   │  ← Self-update, self-generation, morphological evolve
   └──────────────┘
```

Crate-to-layer mapping:
- **SelfField** → `dasein`
- **BrainCore** → `cognit`
- **BodyRuntime** → `corpus`
- **Memory** → `memory`
- **MetaRuntime** → `metacog`
- **Runtime/Orchestration** → `runtime`
- **Interface** → `interact`
- **ABI/Shared** → `base`
