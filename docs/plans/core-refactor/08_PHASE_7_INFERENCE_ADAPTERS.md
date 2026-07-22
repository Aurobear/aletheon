# Phase 7 Inference Adapter Isolation Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Hide concrete Anthropic, OpenAI-compatible, and Ollama implementations behind a stable inference contract and route exclusively by validated capabilities, health, and policy.

**Architecture:** Preserve provider wire implementations in crate-private adapters, centralize constructor selection in composition, migrate callers to the stable facade, and remove URL/error-text provider inference.

**Tech Stack:** Rust 1.85+, Bash, Python 3, Cargo via `scripts/cargo-agent.sh`, repository architecture gates.

---

## Global execution constraints

- Treat `docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md` as the architecture source of truth.
- Re-read that document and every cited symbol before editing; record changed line anchors in the task report.
- Do not modify files outside the declared paths. Stop if a required change crosses the boundary and report it.
- Preserve unrelated working-tree changes. Never use `git reset --hard`, `git checkout --`, or broad cleanup commands.
- Never invoke Cargo directly. Use `bash scripts/cargo-agent.sh <cargo arguments>` and the narrowest package/test target.
- Do not run concurrent Executive or workspace builds. Only the final integration owner runs workspace-wide commands.
- Keep security-sensitive behavior fail-closed. Do not weaken credential, scope, sandbox, network, lease, or trust checks.
- Each non-trivial commit must use a conventional subject, blank line, problem/solution context, and concrete bullets.
- Before each commit run `git diff --cached --check` and inspect the complete staged diff.
- A task is incomplete if tests pass but its architecture gate, compatibility evidence, or inventory update is missing.

## Prerequisites and owned paths

Prerequisite: Phase 2; coordinate config normalization with Phase 4.

- `crates/cognit/src/impl/llm/`
- `crates/cognit/src/impl/provider_registry.rs`
- `crates/cognit/src/impl/inference/`
- `crates/cognit/src/config/`
- `crates/cognit/src/lib.rs`
- Executive inference composition/callers
- Provider timeout/scheduler/contract tests

## Task 1: Lock provider behavior

- [ ] Preserve request/response conversion, streaming, tool calls, usage, stop reasons, timeout, cancellation, context-limit handling, rate limits, authentication, and retry disposition.
- [ ] Add contract tests against fake HTTP endpoints for every wire adapter.
- [ ] Keep tests naming concrete providers only under adapter tests.

## Task 2: Stable inference contract/facade

Expose only:

```text
InferenceProvider
InferenceRequest / Response / Stream
Message / ContentBlock / ToolCall / Usage / StopReason
InferenceCapabilities
IntegrationFailureKind + stable inference-domain errors
```

- [ ] Concrete provider structs are crate-private.
- [ ] Crate root does not export concrete provider modules.
- [ ] External callers build providers only through composition/factory.

## Task 3: Composition factory and config

- [ ] Static adapter ID -> constructor match exists only in composition.
- [ ] Remove URL suffix inference and fallback guessing.
- [ ] Unknown adapter ID fails explicitly.
- [ ] Provider/model values remain deployment data.
- [ ] SecretRef resolves only during adapter construction.

## Task 4: Capability-based scheduling

- [ ] Scheduler selects by required capabilities, configured routing policy, health, cost/limits where explicitly modeled, and deterministic tie-breaking.
- [ ] Scheduler does not match provider names or parse provider error strings.
- [ ] Adapter failures normalize before scheduler retry decisions.
- [ ] Add two differently named fake adapters with identical capabilities to prove neutrality.

## Task 5: Public API contraction and gates

- [ ] Remove `pub mod anthropic`, `ollama`, and `openai_provider` from public facade.
- [ ] Migrate tests/callers to factory or explicit testing facade.
- [ ] Update compatibility counts and architecture gates.

## Validation

```bash
bash scripts/cargo-agent.sh test -p cognit --test anthropic_provider_timeout
bash scripts/cargo-agent.sh test -p cognit --test openai_provider_timeout
bash scripts/cargo-agent.sh test -p cognit provider_registry
bash scripts/cargo-agent.sh test -p cognit scheduler
bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity
bash tests/architecture_check.sh
```

## Commit stages

1. `test(inference): lock provider adapter contracts`
2. `refactor(cognit): expose stable inference facade`
3. `refactor(cognit): centralize inference adapter construction`
4. `refactor(cognit): route inference by capability and policy`
5. `chore(arch): hide concrete inference providers`
