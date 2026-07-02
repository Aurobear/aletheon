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

## Summary table

| Tier | Module | Effort | Blocks / depends | Open-source value | Daily-use value |
|---|---|---|---|---|---|
| 0 | Hygiene & Truth | Low | — | High | Low |
| 1 | Governed Memory (own spec) | Med | — | Med | **High** |
| 2 | Boundary corrections | Med | needs 0; blocks 4 | High | Low |
| 3 | Provider Manager | Med | independent | Med | **High** |
| 4 | Workflow + multi-repo | High | needs 2 | High | Med |
