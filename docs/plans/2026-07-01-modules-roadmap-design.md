# Aletheon → Auro Runtime — Modules Roadmap Design

**Date:** 2026-07-01
**Status:** Design (design-only; no implementation)
**Source docs:** `docs/guide/gpt-suggestion{1,2,3,4}.md`
**Companion spec:** `2026-07-01-governed-memory-mvp-design.md` (Tier 1, already specced)

This document is the design record for the remaining gaps found by auditing the
codebase against the four "Auro Runtime" suggestion docs. Tier 1 (Governed
Memory) has its own detailed spec. Each module below is designed to a level
sufficient to drive its own spec + implementation plan later; when we pick one
up, it graduates through the normal brainstorm → plan → implement cycle.

## Verdict recap

The project already **is** the architecture the docs describe (~70% built):
`dasein`=Self, `cognit`=Brain, `runtime`=Runtime, `corpus`=Body, `interact`=Interface.
The Provider trait is clean, there is a `MemoryBackend` trait, a DAG workflow
engine, a subagent spawner, and a plugin manager. Remaining work is
**completion, boundary correction, and hygiene** — not a rebuild.

## Decisions (2026-07-01)

- **Do all modules, one step at a time** (owner directive "都做吧,一步一步来").
- **Rebrand deferred** (M-G) until Tier 0–2 stabilize.
- **Recommended implementation sequence** (each step: spec if needed → `plans` →
  implement → validate, on its own `auro/feat/*` branch):
  1. **Tier 1 — Governed Memory** (specced; highest daily value) ← first
  2. **Tier 0 — Hygiene** (cheap, unblocks, needed for OSS)
  3. **M-A — Context Manager** (fixes long-conversation break)
  4. **Tier 2 — Boundary corrections** (blocks Tier 4)
  5. **Tier 3 — Provider Manager** (independent)
  6. **M-B / M-C / M-E** (small additive: plugin trait, verify seam, subagent)
  7. **Tier 4 — Workflow persist + multi-repo split** (needs Tier 2)
  8. **M-D — Self-Evolution loop** (needs 2a), **M-F — Hosts** (needs 2b)
  9. **M-G — Rebrand** (decision, deferred)

## Ordering & dependencies

```
Tier 0 (Hygiene)  ──► Tier 2 (Boundaries) ──► Tier 4 (Workflow persist + multi-repo split)
                          │
Tier 1 (Memory) ─────────┘   Tier 3 (Provider Mgr) — independent, can land anytime
```

- **Tier 0** unblocks everything and is needed before open-sourcing (credibility).
- **Tier 2** boundary corrections are prerequisite to the **Tier 4** multi-repo split.
- **Tier 3** is independent and can be scheduled opportunistically.

---

# Tier 0 — Hygiene & Truth

**Goal:** make the repo's structure match its own description before anyone (a
teammate or the open-source public) reads it.

### Problems (root causes, path:line)

1. **Orphaned `binaries/` crate.** `crates/binaries/aletheond/Cargo.toml:12`
   depends on `aletheon-runtime` and `crates/binaries/aletheon-cli/Cargo.toml:14`
   on `aletheon-body` — **neither crate exists** (workspace has `runtime` and
   `corpus`). The real binaries are declared in-workspace:
   `crates/runtime/Cargo.toml:8` (`aletheond`, `aletheon-exec`) and the
   `aletheon` binary in `interact`. The `binaries/` crate is dead and would
   fail to build if referenced.
2. **Stale README / crate names.** README refers to `aletheon-self` /
   `aletheon-brain` / `aletheon-body`; reality is `dasein` / `cognit` / `corpus`.
   Docs also reference an `aletheon-cli` binary and provider TOML fields
   (`url`/`path`) that have drifted from the code.
3. **Broken `config/default.toml`.** Contains only `[agent] default_model`; no
   `[[providers]]` and no `[agent] default_provider`, so a fresh daemon exits
   with "Default provider '' not found". The shipped default config cannot start
   the product.

### Design / approach

- **Delete** the `binaries/` crate; keep binaries where they already live
  (`runtime`, `interact`). Verify nothing in the workspace `Cargo.toml` members
  list or CI references it.
- **Rewrite README architecture section** from the actual crate list and the
  actual binary/entry names; add the real crate-name ↔ concept mapping table
  (`dasein`=Self, `cognit`=Brain, `corpus`=Body, `runtime`=Runtime,
  `interact`=Interface, `metacog`=Meta, `memory`=Memory, `base`=ABI).
- **Fix `config/default.toml`** into a minimal-but-runnable default: one
  `[[providers]]` block (documented placeholder key + base_url) and a matching
  `[agent] default_provider`, so a fresh checkout starts once a key is filled in.

### Non-goals
No behavior changes; no renaming of live crates. Pure deletion + docs + config.

### Affected files
`crates/binaries/**` (delete), root `Cargo.toml` (members), `README.md`,
`config/default.toml`.

### Risk / testing
Low risk. Validate: `cargo build --workspace` succeeds after deletion; a fresh
daemon starts against the fixed default config (with a key filled in); no dangling
references to `binaries/` in CI or scripts.

---

# Tier 2 — Boundary Corrections

**Goal:** correct three boundary violations the docs call out, so the layering is
honest and multi-repo extraction (Tier 4) becomes possible.

## 2a. Permission policy → Runtime `PermissionManager`

**Problem.** Doc 4 (§9) says *Security **Policy** in Runtime, Security **Tool** in
Body*. Today the policy decision leaks into Self: `dasein/src/core/mod.rs:391`
makes the approval/confirmation decision (`ctx.permissions.max_level() <
SystemChange` → `RequireConfirmation`) inside the `review()` pipeline. Execution
correctly lives in `corpus/src/security/`, but there is **no unified Runtime
permission manager** — policy is scattered between `dasein` and `corpus`.

**Design.** Introduce a `PermissionManager` in `runtime` that owns policy:
permission-level judgment, sandbox policy selection, tool whitelist, and the
"require user confirmation" decision. `dasein.review()` keeps *identity/care/
boundary* judgment but **delegates the permission verdict** to the Runtime
manager (via a trait it's handed, so `dasein` doesn't depend on `runtime`).
`corpus/src/security/` remains the executor of whatever policy resolves to.

**Non-goals.** No new sandbox backends; reuse existing `SandboxConfig`
(`runtime/src/core/config/infra.rs:7`) and corpus executors.

**Affected files.** `runtime/src/` (new `PermissionManager` + wiring in the
orchestrator/verdict path `runtime/src/core/verdict_handler.rs`),
`dasein/src/core/mod.rs` (delegate instead of decide), a policy trait in `base`.

**Risk.** Medium — touches the approval path; must preserve current
confirmation behavior. Test: a `SystemChange`-level action still triggers
confirmation; read-only actions still pass without prompting.

## 2b. `RuntimeHost` trait (daemon becomes one host)

**Problem.** Doc 4 (§5–§8): *Runtime Core ≠ daemon*. Today the daemon is the only
entry: `runtime/src/impl/daemon/mod.rs:77` `run()` directly builds the
`UnixServer`; `crates/binaries/aletheond/src/main.rs:33` calls it directly. No
host abstraction, so CLI-one-shot / systemd / container / robot deployment forms
have no seam.

**Design.** Define `trait RuntimeHost { fn init(&self); fn run(&self, core:
RuntimeCore); fn shutdown(&self); }` in `runtime`. Refactor the current daemon
into a `DaemonHost` implementing it. `RuntimeCore` is the host-agnostic core
(session/task/memory/workflow/provider/permission). Additional hosts
(`CliHost`, `SystemdHost`, …) are follow-ons — this spec delivers the trait +
`DaemonHost` only, proving the seam without regressing today's daemon.

**Non-goals.** Implementing systemd/container/robot hosts (later); no change to
the wire protocol.

**Affected files.** `runtime/src/impl/daemon/mod.rs` (extract core vs host),
new `runtime/src/host/` (trait + `DaemonHost`), `runtime` binary entry.

**Risk.** Medium — restructures startup. Test: daemon still starts/stops/serves
over the socket identically after the refactor.

## 2c. Break the `cognit → corpus / interact` inversion

**Problem.** The dependency graph shows **Brain depending on Body and Interface**:
`cognit → base, corpus, interact` (from workspace Cargo audit). A cognition crate
should not depend on the tool/execution crate or the UI crate. This inversion is
the main structural blocker (with the runtime god-crate) to extracting
`auro-cognition` as its own repo (doc 2).

**Design.** Identify what `cognit` actually uses from `corpus`/`interact` (likely
a handful of types or a tool-invocation interface) and **invert the dependency**:
move the shared contract into `base` (ABI) as a trait, and have `corpus`/`interact`
implement it. `cognit` then depends only on `base`. This is the same pattern used
elsewhere in the workspace (traits in `base`, impls in higher crates).

**Non-goals.** No functional change; purely dependency-direction refactor.

**Affected files.** `crates/cognit/Cargo.toml` (drop `corpus`/`interact` deps),
`crates/base/src/` (new trait(s)), `crates/corpus`, `crates/interact` (impl side),
call sites in `cognit`.

**Risk.** Medium — compile-time churn; caught by `cargo build --workspace`.
Test: workspace builds; behavior unchanged; `cognit`'s only internal dep is `base`.

---

# Tier 3 — Provider Manager Hardening

**Goal:** make multi-provider use robust for real work and long runs (doc 1
"Provider Manager": failover, retry, cost, timeout).

### Problems (root causes, path:line)

1. **No failover / retry.** `cognit/src/impl/llm/scheduler.rs:123` only falls back
   to `default_provider` when routing fails; there is no retry, backoff, or
   switch-to-next-provider on an actual request error.
2. **Health check is a stub.** `scheduler.rs:157` `health_check()` always returns
   `available: true, latency_ms: 0` — never measures anything.
3. **No per-provider cost/token accounting.** `TokenUsageBreakdown`
   (`runtime/src/impl/session/observability/metrics.rs:6`) is session-global; no
   attribution per provider and no pricing/cost.

### Design / approach

- **Retry + failover policy** in the scheduler: bounded retries with exponential
  backoff on transient errors; on hard failure, fall through an ordered provider
  list (config-driven), not just the single default. Classify errors (transient
  vs terminal vs context-overflow) to decide retry vs failover vs surface.
- **Real health checks:** lightweight probe (cheap `complete` or a HEAD/models
  call per transport) recording latency + availability; scheduler prefers healthy
  providers and can circuit-break a failing one.
- **Per-provider accounting:** attribute `TokenUsageBreakdown` by provider name;
  add an optional per-provider pricing table to compute cost. Surface via the
  existing metrics exporter.

### Non-goals
No new provider transports; no autoscaling; pricing table is optional/static.

### Affected files
`cognit/src/impl/llm/scheduler.rs` (retry/failover/health), `cognit/src/config/mod.rs`
(provider ordering + pricing config), `runtime/src/impl/session/observability/metrics.rs`
(per-provider attribution).

### Risk / testing
Medium. Test: injected transient error retries then succeeds; hard failure fails
over to next provider; unhealthy provider is skipped; token/cost attributed to
the right provider name.

---

# Tier 4 — Workflow Sedimentation + Multi-repo Extraction

**Goal:** make workflows persistent/reusable (doc 1/2), then decompose the
runtime god-crate so the org-split (doc 2) is possible.

## 4a. Workflow sedimentation

**Problem.** Doc 1/2: *Workflow > Prompt* — learned workflows should be
"sedimented" (saved) and reused, not re-derived each time. Today the DAG engine
`runtime/src/impl/orchestration/digraph/graph.rs:22` executes **in-memory only**;
there is no save/load/reuse. (Markdown `SKILL.md` files exist but are not
runnable persisted workflows.)

**Design.** Add a workflow **definition store** (serialize a `DiGraph` of steps to
disk — JSON, reusing the SQLite/filesystem patterns from memory). API:
`save(name, graph)`, `load(name)`, `list()`, `run(name, inputs)`. A completed
successful run can be offered for sedimentation (save as a named reusable
workflow). Retrieval by task/intent is deferred (ties into Governed Memory's
Workflow scope later).

**Non-goals.** No automatic workflow synthesis from traces (later); no visual
editor. Persist + reuse only.

**Affected files.** `runtime/src/impl/orchestration/digraph/` (serde on graph),
new `runtime/src/impl/orchestration/store.rs`, CLI surface in `interact` to
list/run saved workflows.

**Risk.** Medium. Test: save → reload → run reproduces the same execution;
round-trip serde is lossless.

## 4b. Multi-repo extraction readiness

**Problem.** Doc 2 wants an org split (`auro-runtime`, `auro-provider`,
`auro-cognition`, `auro-memory`, `auro-workflow`, …). Today `runtime` is a **god
crate** depending on all five siblings (`runtime/Cargo.toml:17`), and `cognit`
has the inversion (Tier 2c). `base` and `memory` could extract cleanly today;
`runtime` cannot.

**Design.** After Tier 2 lands (inversion fixed, host trait, permission manager),
define the **kernel boundary**: `RuntimeCore` depends only on `base` traits;
`cognit`/`corpus`/`memory`/`metacog` are wired in as **implementations behind
traits** (Provider SDK, Memory SDK, Plugin SDK — doc 2), not compile-time hard
deps of the core. Then the crates can move to separate repos and be consumed as
versioned deps. Deliver incrementally: first prove `base` + one capability crate
extract cleanly; do not big-bang the split.

**Non-goals.** Actually creating the GitHub org / moving repos in this spec;
`auro-robot` (no robot code exists in-tree yet — nothing to extract).

**Affected files.** Workspace `Cargo.toml`, per-crate `Cargo.toml` dep lists,
new SDK trait crates in `base` or dedicated `*-sdk` crates.

**Risk.** High — architectural. Gate behind Tier 2. Test: `cargo build
--workspace` at each step; a smoke extraction of one crate into a throwaway repo
builds against the others as path/registry deps.

---

# Additional Modules (M-A … M-G)

A second pass over the four docs surfaced doc-prescribed parts not covered by
Tiers 0–4. They are lettered to keep them distinct from the tiers; each still
graduates to its own spec + plan when picked up.

## M-A. Context Manager — unify conversation compaction  *(doc 1 "Context Manager")*

**Problem.** Doc 1: *Context belongs to Runtime; Runtime decides what to send.*
Two compaction implementations coexist and diverge:
- `runtime/src/impl/memory/compressor/mod.rs:14` — `AdvancedCompressor` is
  **tool-boundary-safe** (`find_tail_cut`, prunes tool outputs before
  summarizing) but is used **only inside the ReAct loop** (`react_loop/step.rs:42,216`).
- `runtime/src/impl/daemon/session_manager.rs:113` — `compact_if_needed()` keeps
  the **last 6 non-system messages** and summarizes the rest with **no
  tool_use/tool_result pairing protection**. This is what governs the persisted
  multi-turn history.

Consequence: a blind cut can orphan a `tool_result` (its `tool_use` was
summarized away) → malformed provider request → the long-conversation
"报错/卡住/失忆" failure mode.

**Design.** Make the multi-turn path reuse the **tool-boundary-safe** compactor:
`SessionManager::compact_if_needed` delegates to the same
`find_tail_cut`/boundary-aligned logic (or a shared `ContextManager` that both
the ReAct loop and the session manager call). Run compaction **proactively
before** the turn's first LLM call (on the seeded history), not only post-turn,
and persist the compacted history so it stops regrowing unbounded.

**Scope / non-goals.** Unify + proactive-trigger + persist. No new summarization
model; no semantic memory offload (that's Governed Memory's job).

**Affected files.** `runtime/src/impl/daemon/session_manager.rs`,
`runtime/src/impl/memory/compressor/` (extract shared entry point),
`runtime/src/impl/daemon/handler/chat.rs` (pre-turn trigger + persist).

**Risk / testing.** Medium-high (touches the hot path). Test: a synthesized
history that orphans a tool_result is repaired before send; a long multi-turn
run stays under budget without malformed requests; summaries preserve the last
user turn.

## M-B. Plugin lifecycle trait  *(doc 2 "Plugin SDK" / kernel-driver)*

**Problem.** Doc 2's seam is `trait Plugin { init/run/shutdown }`. Today plugins
have a `PluginState` machine (`plugin/manager.rs:15`) and a manifest with
`cmd:`/`native:`/`wasm:` entries (`plugin/manifest.rs:47`), but capability
plugins are surfaced only as **execute-only `Tool`s** (`PluginTool`,
`manager.rs:184`) — there is no long-lived `init/run/shutdown` lifecycle a plugin
can hook.

**Design.** Add a `Plugin` trait in `base` (`init/run/shutdown` + capability
registration) that a plugin *may* implement for long-lived behavior, while
`Tool`-only plugins keep working (the trait is additive; `PluginRuntime` calls
`init` on load, `shutdown` on unload, tracked by the existing `PluginState`).

**Scope / non-goals.** Trait + lifecycle wiring only; no WASM host work; no new
plugins shipped.

**Affected files.** `base/src/` (trait), `runtime/src/impl/plugin/manager.rs`
(call init/run/shutdown around `PluginState` transitions).

**Risk.** Low-medium; additive. Test: a sample plugin's init/shutdown fire on
load/unload; Tool-only plugins unaffected.

## M-C. Result / Verification pipeline  *(doc 1 "Result Pipeline")*

**Problem.** Doc 1: *the model's output is not the final answer* — it should pass
Runtime Verify → (execute) → Observation → Memory Update → Final Response. Today
`react_loop/step.rs:74` returns the assistant text **directly** when there are no
tool calls; there is no verification/critique seam.

**Design.** Add an **optional, pluggable verification step** between "LLM produced
final text" and "return": a `Verifier` trait (`base`) with a default no-op impl,
so behavior is unchanged unless a verifier is configured. Candidate verifiers
(later): self-critique via `cognit`, schema/goal checks, tool-result consistency.
Wire the hook at the `step.rs` return site.

**Scope / non-goals.** The seam + no-op default only. Concrete verifiers are
follow-ups. Keep latency opt-in.

**Affected files.** `base/src/` (Verifier trait), `runtime/src/core/react_loop/step.rs`,
`runtime/src/core/react_loop/mod.rs` (config).

**Risk.** Low if default is no-op. Test: no-op verifier reproduces current
behavior; a rejecting verifier forces a re-try/annotation path.

## M-D. Self-Evolution loop wiring  *(doc 1 "Runtime Goal", doc 2 "Self Evolution", doc 3 §20)*

**Problem.** The project's identity is a *self-evolving* runtime, but `metacog` is
**scaffolding not wired in**: `metacog/src/core/meta_cognition.rs:58` `decide()`
returns `EvolutionAction`s and `traits.rs:39` `DefaultMetaRuntime` has
`generate_candidate/sandbox_test/evaluate/migrate/rollback`, yet `runtime` never
calls `metacog`, and there is no closed loop.

**Design.** Define the evolution loop as *runtime state accumulation* (doc 2's
framing — NOT model self-training): after a task completes, `runtime` invokes
`metacog.decide()` on accumulated trace/metrics; `TriggerEvolution` runs the
existing `generate_candidate → sandbox_test → evaluate → migrate|rollback`
sequence against **workflow/memory/policy** artifacts (not code). Start with the
smallest safe loop: observe → evaluate → refine-workflow, gated behind a config
flag and the Runtime `PermissionManager` (Tier 2a).

**Scope / non-goals.** Wire the existing metacog steps into a bounded,
config-gated loop over workflow/memory artifacts. No genome/code self-mutation
in this spec; `migrate` limited to declarative artifacts.

**Affected files.** `runtime/src/core/orchestrator.rs` (post-task hook),
`metacog/src/core/` (expose a callable loop), config flag in `runtime`.

**Risk.** High (autonomy). Must be default-off, sandboxed, rollback-able,
permission-gated. Test: loop triggers only when enabled; every migration has a
rollback; sandbox failure aborts cleanly.

## M-E. SubAgent lifecycle  *(doc 1 "SubAgent")*

**Problem.** Doc 1 prescribes Created→Running→Waiting→Completed→Destroyed. Today
`sub_agent.rs` has `spawn/update_status/remove` with no explicit state machine or
teardown guarantees.

**Design.** Introduce an explicit `SubAgentState` enum + transitions and a
`destroy()` that guarantees resource cleanup (cancel tasks, drop handles). Small,
self-contained.

**Scope / non-goals.** State machine + teardown; no new scheduling policy.

**Affected files.** `runtime/src/core/sub_agent.rs`, status enum in `base`.

**Risk.** Low. Test: lifecycle transitions are legal-only; destroy cancels
in-flight work and frees the map slot.

## M-F. Additional Hosts (systemd / user-service / container)  *(doc 4 §5–§8)*

**Problem.** Tier 2b delivers the `RuntimeHost` trait + `DaemonHost`. Doc 4 wants
`systemctl --user` and system-level deployment plus container. These are the
follow-on host implementations.

**Design.** Implement `SystemdHost` (user + system units) and a container entry
on the Tier-2b trait; ship unit files (`aletheond.service`, `--user` variant).
Depends entirely on Tier 2b landing first.

**Scope / non-goals.** systemd + container hosts + unit files. Cloud/robot hosts
deferred.

**Affected files.** `runtime/src/host/` (new hosts), packaging (`*.service`).

**Risk.** Low-medium; deployment-only. Test: `systemctl --user start` brings the
daemon up; graceful shutdown on stop.

## M-G. Positioning / rebrand — Aletheon → "Auro Runtime"  *(doc 1 & 2 titles)*

**Problem/decision (not a code task yet).** All four docs title the project
**"Auro Runtime"** and prescribe an `auro-*` crate/org naming. The code is
`aletheon`/`aletheon-*`. Before open-sourcing, this is a naming + org-structure
**decision** the owner must make: keep `aletheon`, rename to `auro`, or brand the
product "Auro" while keeping internal crate names. Rename touches every crate,
binary, config path, and doc.

**Recommendation.** Defer the rename until after Tier 0–2 (don't rename a moving
target); decide it explicitly as its own change with a mechanical, scripted
crate-rename + a compatibility note. Flagged here so it isn't forgotten, not
designed in detail pending the owner's decision.

---

## Summary table

| Tier/ID | Module | Effort | Blocks / depends | Open-source value | Daily-use value |
|---|---|---|---|---|---|
| 0 | Hygiene & Truth | Low | — | High | Low |
| 1 | Governed Memory (own spec) | Med | — | Med | **High** |
| 2 | Boundary corrections | Med | needs 0; blocks 4 | High | Low |
| 3 | Provider Manager | Med | independent | Med | **High** |
| 4 | Workflow + multi-repo | High | needs 2 | High | Med |
| M-A | Context Manager (compaction unify) | Med | independent | Med | **High** |
| M-B | Plugin lifecycle trait | Low-Med | — | High | Low |
| M-C | Result/Verification pipeline | Low (seam) | — | Med | Med |
| M-D | Self-Evolution loop wiring | High | needs 2a | High | Med |
| M-E | SubAgent lifecycle | Low | — | Med | Med |
| M-F | Additional Hosts | Low-Med | needs 2b | Med | Med |
| M-G | Rebrand Aletheon→Auro (decision) | — | needs 0–2 | High | — |
