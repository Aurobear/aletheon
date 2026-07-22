# Phase 3 Coding Runtime Decoupling Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Remove Pi-specific types, names, protocol knowledge, and storage policy branches from Goal and Agent Control while retaining a private Pi adapter.

**Architecture:** Extend the runtime contract with explicit governed requirements, introduce canonical coding request/outcome/evidence ports, migrate application callers, and isolate Pi JSONL/RPC parsing under adapters.

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

Prerequisite: Phase 2.

- Modify: `crates/runtime/src/manifest.rs`, `selector.rs`, `lib.rs`
- Modify: Executive application Goal/Agent Control paths
- Move/private: Executive Pi runtime/protocol modules under adapters
- Modify: `crates/cognit/src/config/mod.rs` only to remove Pi ownership through Phase 4-compatible bridge
- Add runtime, Executive application, adapter, compatibility, and architecture tests

## Task 1: Lock current Pi behavior

- [x] Preserve argv validation, executable hash/version pinning, JSONL/RPC lifecycle validation, sandbox/network fail-closed behavior, cancellation, timeout, evidence bounds, and worktree settlement.
- [x] Add application tests using a fake runtime that do not mention Pi.
- [x] Add adapter contract tests that explicitly mention Pi and its wire format.

## Task 2: Extend RuntimeManifest with requirements

Introduce bounded requirements conceptually equivalent to:

```rust
pub struct RuntimeResourceRequirements {
    pub storage_bytes: u64,
    pub storage_items: u64,
}
```

- [x] Defaults preserve non-Pi current behavior.
- [x] Values have validated system maxima and cannot overflow admission accounting.
- [x] The manifest declaration is a request, never an authorization.
- [x] Admission/quota policy clamps or rejects and returns a reservation.
- [x] Update all manifest constructors and selector tests.

## Task 3: Canonical coding contracts

Introduce stable types:

```text
CodingAttemptRequest
CodingAttemptOutcome
RuntimeEvidence
RuntimeCapabilityAudit
VerificationPolicy
CodingRuntime / AgentRuntimeLauncher ports
```

- [x] No type or evidence kind exposed to application contains Pi.
- [x] Canonical evidence preserves package identity, protocol version, diff, tests, usage, capability audit, and terminal outcome.
- [x] Persisted request/evidence compatibility is inventoried and migrated or dual-read.

## Task 4: Migrate Goal and Agent Control

- [x] Replace imports of `PiAttemptRequest` and `PI_CODER_RUNTIME_ID` with canonical contracts/capabilities.
- [x] Replace `runtime_id.contains("pi")` with manifest resource requirements passed through admission policy.
- [x] Runtime selection uses capability and interaction requirements.
- [x] Verification gates consume canonical evidence, not Pi-specific evidence kind strings.
- [x] Application tests use two differently named fake runtimes to prove name neutrality.

## Task 5: Isolate the Pi adapter

- [x] Move Pi argv, process setup, JSONL/RPC parser, protocol event names, executable pinning, and error text into `executive/adapters/coding_runtime/pi/` or the Phase 2 canonical adapter path.
- [x] Adapter implements canonical ports and returns normalized outcomes/errors.
- [x] Concrete Pi types are crate-private and absent from Executive crate-root exports.
- [x] Deployment config compatibility may still accept old Pi fields, but canonical runtime objects do not contain legacy names outside adapter ID/diagnostics.

## Validation

```bash
bash scripts/cargo-agent.sh test -p runtime
bash scripts/cargo-agent.sh test -p executive --test pi_runtime
bash scripts/cargo-agent.sh test -p executive --test pi_rpc_runtime
bash scripts/cargo-agent.sh test -p executive --test coding_goal_flow
bash scripts/cargo-agent.sh test -p executive --test approval_goal_flow
bash scripts/cargo-agent.sh test -p executive --test agent_control_repository
bash tests/architecture_check.sh
```

Expected: adapter tests preserve Pi behavior; application paths and tests contain no Pi dependency/name branch.

## Commit stages

1. `test(runtime): lock coding runtime requirements and admission behavior`
2. `feat(runtime): add governed resource requirements`
3. `refactor(executive): introduce neutral coding runtime contracts`
4. `refactor(executive): isolate Pi runtime adapter`
5. `chore(arch): prohibit runtime-name business policy`

## Completion evidence (2026-07-23)

- `RuntimeManifest` now carries bounded, serde-defaulted resource requirements; Agent Control validates them and admission remains the reservation authority.
- Goal uses `CodingAttemptRequest` and persisted canonical coding evidence without importing a Pi identifier or adapter type. Neutral fake-runtime goal and approval tests cover restart, verification, settlement, and evidence flows.
- Pi argv, executable pinning, JSONL/RPC lifecycle, sandbox/network behavior, and protocol parsing remain isolated under `executive::adapters::runtime`; the former request name is a counted Phase 9 compatibility alias.
- The architecture checker rejects Pi types, IDs, and runtime-name policy branches in application and Goal production code. All commands in **Validation** pass.
