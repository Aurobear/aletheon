# Aletheon M4 Pi Worktree and Verification Detailed Plan

> **For agentic workers:** Execute one task at a time; do not combine worktree, runtime, and verification changes into one commit.

**Goal:** Run Pi as a supervised coding runtime in an isolated temporary git worktree, capture a structured report, and require deterministic verification evidence before a Goal may request approval.

**Architecture:** `WorktreeManager` owns git lifecycle; `PiRuntime` owns one sandboxed coding attempt; `VerificationService` owns bounded commands and policy. AttemptCoordinator persists reports, while PostTurn hooks only publish already-computed results.

**Tech Stack:** Rust, Tokio process execution, git worktree, existing SandboxBackend/Bubblewrap, Goal attempts from M3, serde, SQLite.

---

## 1. Anchors and boundaries

- Pi process/output/timeout/worktree/diff requirements: `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:94-105`.
- Verification gates and required evidence: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:239-269`, `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:107-116`.
- Existing sandbox contract: `crates/fabric/src/types/sandbox.rs:11-88`.
- Namespace-required selection: `crates/fabric/src/types/sandbox.rs:134-185`.
- Existing backends: `crates/corpus/src/security/sandbox/executor.rs:11-39`.
- Current PostTurnPipeline has no coding context: `crates/executive/src/service/post_turn.rs:5-14`.

M4 produces an approved-ready diff but does not apply it to the main worktree. M5 implements durable approval and controlled apply.

## 2. Task 1 — Define coding-job contracts

**Files:**

- Create: `crates/fabric/src/types/coding_job.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

- [ ] Write serde/path tests first, then define `CodingJobId`, `CodingJobSpec`, `WorkspaceBoundary`, `ChangedFile`, `CodingJobStatus`, `CodingJobReport`, `VerificationCheck`, `VerificationSeverity`, and `VerificationReport`.
- [ ] `CodingJobSpec` includes Goal/attempt IDs, repository root, immutable base commit, allowed/forbidden relative paths, command/args, timeout, output cap, and network policy.
- [ ] `ChangedFile` includes relative path, add/modify/delete kind, byte counts, and content hash; never store an unchecked absolute path.
- [ ] Test rejection of absolute paths, `..`, symlink escape, repository-root escape, empty allowed scope, and forbidden path precedence.
- [ ] Run `cargo test -p fabric -- types::coding_job`; expect PASS.
- [ ] Commit `feat(fabric): define isolated coding job contracts`.

## 3. Task 2 — Add cancellable command primitive

**Files:**

- Create: `crates/corpus/src/tools/subagent/mod.rs`
- Create: `crates/corpus/src/tools/subagent/command.rs`
- Modify: `crates/corpus/src/tools/mod.rs`

- [ ] Define a command runner using `tokio::process::Command`, piped stdout/stderr, `kill_on_drop(true)`, injected timeout, cancellation token, environment allow-list, working directory, and per-stream byte cap.
- [ ] On Unix, start a process group and terminate the group on cancel/timeout; return exit code, elapsed time, truncation flags, stdout, and stderr.
- [ ] Test success, nonzero exit, timeout, cancellation, child-process cleanup, output cap, missing executable, and environment stripping.
- [ ] Do not use shell interpolation for git/cargo arguments; pass argv separately.
- [ ] Run `cargo test -p corpus -- tools::subagent::command`; expect PASS.
- [ ] Commit `feat(corpus): add bounded cancellable command runner`.

## 4. Task 3 — Implement WorktreeManager

**Files:**

- Create: `crates/corpus/src/tools/subagent/worktree.rs`
- Modify: `crates/corpus/src/tools/subagent/mod.rs`

- [ ] Define `WorktreeLease { job_id, path, base_commit, created_at }` and manager configuration `{ base_dir, failed_ttl, failed_cap, disk_budget_bytes }`.
- [ ] Create via `git -C <repo> worktree add --detach <path> <base_commit>` after verifying base commit with `git rev-parse --verify <commit>^{commit}`.
- [ ] Canonicalize repository/base paths, require generated worktree path beneath configured base, and reject pre-existing nonempty paths.
- [ ] Collect status with `git status --porcelain=v2 -z`, diff with `git diff --binary --no-ext-diff <base>`, and changed-file hashes from the worktree.
- [ ] Cleanup with `git worktree remove --force <path>` only after verifying path ownership; prune metadata separately.
- [ ] Tests must prove main worktree immutability, base pinning, diff correctness, symlink/path escape rejection, success cleanup, failed retention, TTL prune, oldest-first cap, and disk-budget refusal.
- [ ] Run `cargo test -p corpus -- tools::subagent::worktree`; expect PASS.
- [ ] Commit `feat(corpus): manage isolated coding worktrees`.

## 5. Task 4 — Add Pi configuration and runtime registration

**Files:**

- Modify: `crates/cognit/src/config/mod.rs`
- Modify: `crates/executive/src/core/runtime_core.rs`
- Create: `crates/executive/src/impl/runtime/pi.rs`
- Modify: `crates/executive/src/impl/runtime/mod.rs`
- Modify: `docs/design/executive/daemon.md`

- [ ] Configure executable path, fixed argv prefix, worktree base, timeout, max output, allowed paths, forbidden paths, namespace isolation requirement, and network disabled default.
- [ ] Validate executable is absolute or resolved from an explicit trusted directory; do not accept arbitrary Goal-provided executables.
- [ ] Register RuntimeId `pi-coder` only when configuration is complete and required sandbox is available.
- [ ] Test disabled config, missing executable, invalid path policy, Noop/process-only sandbox rejection, Bubblewrap acceptance, and secret-free debug output.
- [ ] Run scoped config/runtime tests and `cargo check -p executive`; expect PASS.
- [ ] Commit `feat(executive): configure isolated Pi runtime`.

## 6. Task 5 — Implement one Pi coding attempt

**Files:**

- Modify: `crates/executive/src/impl/runtime/pi.rs`
- Test: `crates/executive/tests/pi_runtime.rs`

- [ ] Inject `WorktreeManager`, a namespace-capable `SandboxBackend`, command runner, and Clock.
- [ ] One `run_attempt()` performs:

```text
validate CodingJobSpec and runtime policy
create fresh detached worktree at base commit
construct fixed Pi argv with task input via stdin/file, never shell concatenation
execute with network isolation and worktree-only writable mount
capture bounded stdout/stderr/exit/elapsed
collect diff and ChangedFile list
validate paths again after execution
return CodingJobReport as RuntimeResult evidence
retain failed worktree or cleanup successful empty worktree according to policy
```

- [ ] Test success, Pi nonzero exit, timeout, cancellation, child process, forbidden file, main-worktree mutation attempt, symlink escape, output truncation, and unavailable sandbox.
- [ ] Any isolation uncertainty fails before executing Pi.
- [ ] Run `cargo test -p executive --test pi_runtime`; expect PASS.
- [ ] Commit `feat(executive): execute Pi in isolated worktree`.

## 7. Task 6 — Define VerificationService and policy

**Files:**

- Create: `crates/executive/src/service/verification/mod.rs`
- Create: `crates/executive/src/service/verification/policy.rs`
- Modify: `crates/executive/src/service/mod.rs`

- [ ] Define `VerificationContext` containing job/Goal/attempt IDs, worktree/base commit, changed files, tool/capability audit summary, and configured check selection.
- [ ] Define required checks: DiffScope, Format, Compile, RelevantTests, CapabilityPolicy.
- [ ] Define advisory checks: Clippy, ArchitectureReview for initial release.
- [ ] Policy returns `passed=false` when any required check fails/times out/is cancelled; advisory failure adds risk but does not pass-block.
- [ ] Test deterministic ordering, duplicate check names, required/advisory behavior, empty report rejection, and serialization.
- [ ] Run `cargo test -p executive -- service::verification::policy`; expect PASS.
- [ ] Commit `feat(executive): define coding verification policy`.

## 8. Task 7 — Implement verification commands

**Files:**

- Create: `crates/executive/src/service/verification/command.rs`
- Create: `crates/executive/src/service/verification/checks.rs`
- Modify: `crates/executive/src/service/verification/mod.rs`
- Test: `crates/executive/tests/verification_service.rs`

- [ ] Reuse the bounded Tokio command runner from Task 2.
- [ ] Format: `cargo fmt --all -- --check`.
- [ ] Compile: configurable default `cargo check --workspace`.
- [ ] RelevantTests: explicit argv derived from trusted configuration/task scope; never free-form model command.
- [ ] Clippy: `cargo clippy --workspace --all-targets -- -D warnings` as advisory.
- [ ] DiffScope: pure path-policy check over ChangedFile plus fresh git status.
- [ ] CapabilityPolicy: compare attempt audit records with allowed capabilities; missing audit is a required failure.
- [ ] ArchitectureReview: pure rules for forbidden crate dependency direction/import/path changes plus advisory evidence.
- [ ] Test pass/fail/timeout/cancel/output-limit for every command check with a temporary fixture repo.
- [ ] Run `cargo test -p executive --test verification_service`; expect PASS.
- [ ] Commit `feat(executive): run deterministic coding verification`.

## 9. Task 8 — Persist coding and verification reports

**Files:**

- Modify: `crates/executive/src/impl/goal/migrations.rs`
- Create: `crates/executive/src/impl/goal/verification.rs`
- Modify: `crates/executive/src/impl/goal/mod.rs`

- [ ] Add `goal_coding_jobs` and `goal_verification_reports` tables keyed by immutable job/attempt IDs, with base commit, worktree reference, report JSON, diff artifact reference/hash, status, and timestamps.
- [ ] Store large diffs as bounded artifact files beneath daemon data dir and persist SHA-256 plus relative artifact reference; never put unbounded binary diff into SQLite.
- [ ] Test atomic report/event insertion, duplicate job ID, tampered artifact hash, restart loading, and missing artifact.
- [ ] Run `cargo test -p executive -- impl::goal::verification`; expect PASS.
- [ ] Commit `feat(executive): persist coding verification evidence`.

## 10. Task 9 — Integrate coding attempt and verification lifecycle

**Files:**

- Modify: `crates/executive/src/impl/goal/attempt_coordinator.rs`
- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Test: `crates/executive/tests/coding_goal_flow.rs`

- [ ] Coding task selects RuntimeId `pi-coder`; successful Pi output does not complete the Goal.
- [ ] Persist CodingJobReport, run VerificationService once, persist VerificationReport, then transition:

```text
required verification pass -> Blocked with wait reason "approval required" (M5 upgrades state/request)
required verification fail -> Running/backoff with evidence according to M3 retry policy
cancel -> Suspended/Cancelled according to user action
service error -> Blocked, never approved-ready
```

- [ ] Test compile failure evidence reaches next attempt, advisory-only warning reaches approval-ready state, restart after Pi before verification, restart after report persistence, and duplicate coordinator call.
- [ ] PostTurn hook may publish summary only; it must not launch cargo/git commands.
- [ ] Run `cargo test -p executive --test coding_goal_flow`; expect PASS.
- [ ] Commit `feat(executive): gate coding goals on verification`.

## 11. Task 10 — Worktree recovery and cleanup service

**Files:**

- Create: `crates/executive/src/impl/runtime/worktree_recovery.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Test: `crates/executive/tests/worktree_recovery.rs`

- [ ] On startup reconcile retained directories with persisted coding jobs.
- [ ] Never delete an unknown directory; quarantine/log it for manual review.
- [ ] Prune expired known failed jobs, enforce count/disk cap, and block new Pi work if safe cleanup cannot restore budget.
- [ ] Test orphan metadata, unknown directory, expired job, active job, disk overflow, and interrupted cleanup.
- [ ] Run `cargo test -p executive --test worktree_recovery`; expect PASS.
- [ ] Commit `feat(executive): recover and bound coding worktrees`.

## 12. Task 11 — M4 release audit

- [ ] Run:

```bash
cargo fmt --all -- --check
cargo test -p fabric -- types::coding_job
cargo test -p corpus -- tools::subagent
cargo test -p executive --test pi_runtime
cargo test -p executive --test verification_service
cargo test -p executive -- impl::goal::verification
cargo test -p executive --test coding_goal_flow
cargo test -p executive --test worktree_recovery
cargo test --workspace
cargo build --workspace
```

- [ ] Prove main worktree hash/status is unchanged after success and failure fixtures.
- [ ] Prove Pi cannot run without namespace/network isolation.
- [ ] Prove required verification evidence exists before approval-ready state.
- [ ] Prove cancellation kills descendants and restart does not duplicate Pi/verification work.
- [ ] Prove no async path uses blocking `std::process::Command`.

## 13. DeepSeek batches

1. Tasks 1–3: contracts/command/worktree.
2. Tasks 4–5: Pi config/runtime.
3. Tasks 6–7: verification service.
4. Tasks 8–10: persistence/integration/recovery.
5. Task 11: independent audit.

Guardrails:

```text
Never edit the main worktree through Pi.
Never fall back to Noop or process-only isolation.
Never accept model-generated shell command strings for verification.
Never run verification inside context-free PostTurnPipeline.
Never delete directories outside the configured managed worktree base.
Stop after each batch with exact test evidence.
```
