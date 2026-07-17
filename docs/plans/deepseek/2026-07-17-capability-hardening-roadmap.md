# Aletheon Capability Hardening — Index and Roadmap

> **Status:** Proposed（部分 quick-win 已完成，2026-07-17 复核）
>
> **Date:** 2026-07-17
>
> **⚠ 复核校正 (2026-07-17):** §6 的两个 "quick win" 已经完成，勿重复做——`max_iterations` 默认已是 **50**（`crates/executive/src/core/config/agent.rs:42`），Clock 注入已存在（`kernel/runtime.rs:107 with_clock`）。§2 的 "Agent profiles under-activated" 行仍成立，但注意授权源是 `.toml` 而非 `.md`（详见 activation plan 的前提校正）。
>
> **Baseline:** Aletheon current working tree (branch `auro/docs/executable-architecture-plans`)
>
> **Context:** Comprehensive analysis of Aletheon's actual execution capability against Codex reference, identifying gaps between architecture design and runtime capability.

## 1. Background

Aletheon has 5 existing architecture plans covering internal structure optimization:

- `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md` — crate boundary enforcement, duplicate mechanism removal
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md` — conscious core loop
- `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md` — unified memory system
- `docs/plans/2026-07-15-subagent-unified-harness-plan.md` — multi-agent orchestration

These plans focus on **internal architecture elegance** — converging duplicate paths, enforcing ownership, cleaning up coupling.

The following new plans focus on **external capability hardening** — what the agent can actually **do** in the real world, and how safely/reliably it can do it.

## 2. Capability Assessment Summary

### What works today (production-quality)

| Capability | Status | Evidence |
|------------|--------|----------|
| Daemon turn execution (JSON-RPC → LLM → tool → response) | ✅ Complete | `crates/executive/src/service/turn_pipeline.rs` wired end-to-end |
| 3 LLM providers (Anthropic, OpenAI, Ollama) | ✅ Complete | `crates/cognit/src/impl/llm/` — streaming, tool use, prompt caching |
| 20 built-in tool implementations | ✅ Complete | `crates/corpus/src/tools/tools/` — no stubs, no todo!() |
| Session persistence (3-layer event-sourced SQLite) | ✅ Complete | `crates/executive/src/impl/session/` — legacy + canonical + event-sourced |
| SelfField intent review (8-layer policy) | ✅ Complete | `crates/dasein/src/` |
| Sandbox for bash_exec (bubblewrap + process + noop) | ⚠️ Partial | Only bash_exec goes through sandbox; other tools bypass it |
| Agent profiles (code-agent, fs-agent, net-agent) | ⚠️ Under-activated | 20 tools implemented, only 3 granted to default code-agent |

### Critical gaps

| Gap | Severity | Plan |
|-----|----------|------|
| Only 3 of 20 tools activated in default profile | **High** | Capability Activation Plan |
| Non-bash_exec tools bypass sandbox isolation | **High** | Tool Execution Hardening Plan |
| Zero integration tests on critical paths (TurnCoordinator, Pipeline, Session) | **High** | Testing Infrastructure Plan |
| No MCP integration (zero community tool access) | **High** | MCP Integration Plan |
| No structured code editing (apply_patch is basic) | **Medium** | Structured Code Editing Plan |
| No separate exec-server process (all execution in-process) | **Medium** | Tool Execution Hardening Plan |
| No streaming output deltas for tool execution | **Low** | Tool Execution Hardening Plan |
| No cross-platform support (Linux only) | **Low** | Future |

## 3. New Plans

| # | Plan | File | Focus | Estimated Effort |
|---|------|------|-------|------------------|
| 1 | Testing Infrastructure Hardening | `docs/plans/2026-07-17-testing-infrastructure-hardening-plan.md` | TestAletheonBuilder, mock LLM, integration tests, snapshot tests, fuzzing, chaos tests, benchmarks | 2-3 weeks |
| 2 | MCP Integration | `docs/plans/2026-07-17-mcp-integration-plan.md` | MCP client, server management, tool aggregation, resource access, OAuth | 2-3 weeks |
| 3 | Tool Execution Hardening | `docs/plans/2026-07-17-tool-execution-hardening-plan.md` | Universal sandbox wrapping, exec-server process isolation, file system protocol, shell escape detection, network policy | 3-4 weeks |
| 4 | Structured Code Editing | `docs/plans/2026-07-17-structured-code-editing-plan.md` | Structured patch format, robust application, streaming progress, delta tracking, model awareness | 2-3 weeks |
| 5 | Capability Activation & Agent Profiles | `docs/plans/2026-07-17-capability-activation-and-agent-profiles-plan.md` | Tool audit, tiered agent profiles, immediate activation of safe tools, config-driven grants | 1 week |

## 4. Priority and Dependencies

```
Phase 0 (Week 1):    Capability Activation — audit tools, activate safe ones
                         ↓
Phase 1 (Weeks 1-3): Testing Infrastructure — TestAletheonBuilder, mock LLM, integration tests
                         ↓
Phase 2 (Weeks 3-6): ──┬── Tool Execution Hardening — universal sandbox, exec-server
                        │
                        ├── MCP Integration — MCP client, server management
                        │
                        └── Structured Code Editing — structured patch, delta tracking
```

**Rationale:**
- **Capability Activation first** — zero-risk config changes that immediately multiply agent utility
- **Testing Infrastructure second** — all subsequent hardening work needs regression protection
- **Three capability tracks in parallel** — they are independent (tool execution, MCP, code editing touch different crates)

## 5. Relationship to Existing Architecture Plans

The existing architecture plans (coupling optimization, conscious core, memory, subagent) define **internal structure**. These new plans define **external capability**.

They are complementary and non-conflicting:

| Architecture Plan | Capability Plan Relationship |
|-------------------|------------------------------|
| Coupling Optimization | Testing infrastructure protects refactoring safety |
| SubAgent Unified Harness | MCP integration provides tools for sub-agents |
| Mnemosyne Unified Memory | Tool execution hardening secures memory-relevant operations |
| Dasein-Agora Conscious Core | Structured code editing gives the agent real-world effectors |
| All architecture plans | Capability activation ensures architecture work yields user-visible results |

## 6. Quick Wins (Config-Only Changes, No Code)

These changes require zero new code and can be done immediately:

1. **Activate 8 read-only tools in code-agent profile**: glob, grep, file_search, web_search, web_fetch, code_graph, system_status, process_list — all safe, all implemented
2. **Activate task tools**: task_create, task_update, task_list, task_get — structured task management
3. **Activate apply_patch**: already implemented at permission_level L1 with sandbox compatibility
4. **Increase max_iterations from 20 to 50**: the default is very conservative

Total estimated effort: **30 minutes** of TOML editing. Result: agent tool surface increases from 3 to 16+ tools.

## 7. Definition of Completion

The capability hardening program is complete when:

1. Default code-agent profile grants 16+ tools with appropriate permission levels
2. Every file/tool execution path goes through sandbox isolation, not just bash_exec
3. Critical integration paths (TurnCoordinator, Session, Pipeline) have deterministic integration tests
4. MCP tools can be registered via config stanzas and called by agents
5. Structured code editing (multi-file, multi-operation patches) works with error recovery and delta tracking
6. TestAletheonBuilder exists and can create a fully wired test agent instance in <100ms
7. All new capability tests pass in CI alongside existing architecture tests
