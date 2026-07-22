# H10 Real Coding E2E Evidence — 2026-07-22

## Scope and requirement

H10 requires a disposable-repository chain of goal → Pi/fixed executor contract → production
verifier → approval → hash-bound production apply → settled evidence. It also requires fail-closed
hash mismatch, verification failure and duplicate consumption, repository-wrapper test execution,
plus real CI/timer configuration (`docs/plans/2026-07-21-production-readiness-hardening.md:271-282`).

## Implemented closure

```text
disposable Git repository
  -> GoalCoordinator + fixed Pi contract executor
  -> production VerificationService + real git
  -> durable approval resolution
  -> production ApplyCoordinator / git apply
  -> completed goal + durable apply receipt
  -> duplicate callback recovers receipt without reapplying
```

- The fixed executor creates a real detached worktree, emits a real binary diff and the same
  report/evidence contract as Pi (`crates/executive/tests/coding_production_e2e.rs:36-107`).
- The fixture binds the repository root, allowed path, base commit, timeout and disabled network
  policy into `PiAttemptRequest`; `VerificationService` remains the production implementation and
  invokes real Git (`crates/executive/tests/coding_production_e2e.rs:192-287`).
- The success case resolves approval through the allowed local RPC channel, invokes production
  `ApplyCoordinator`, checks the exact source content and completed goal, then proves a duplicate
  callback returns `Recovered` (`crates/executive/tests/coding_production_e2e.rs:290-364`).
- Negative cases prove a tampered diff hash errors before approval and a real verifier command
  failure produces a failed attempt without approval or source mutation
  (`crates/executive/tests/coding_production_e2e.rs:366-389`).

## Continuous and scheduled gates

- The deterministic closure test runs in the current CI through the required bounded Cargo wrapper
  (`.github/workflows/ci.yml:29-45`).
- The reviewed real Pi version is pinned to 0.80.10 and its env-gated JSONL contract also runs
  through the wrapper (`.github/workflows/ci.yml:100-117`).
- The deployed user timer remains a persistent, randomized daily gate
  (`deploy/systemd/user/aletheon-pi-closure.timer:1-12`). Its oneshot script checks socket,
  executables and disposable Git workspace, takes a nonblocking lock, applies a hard timeout and
  writes a private receipt (`scripts/aletheon-pi-scheduled-task.sh:6-42,51-102`).

## Validation

All Rust commands used the repository wrapper.

| Command | Result |
|---|---|
| `bash scripts/cargo-agent.sh fmt --all -- --check` | PASS |
| `bash scripts/cargo-agent.sh test -p executive --test coding_production_e2e` | PASS, 2 tests |
| `bash scripts/cargo-agent.sh test -p executive --test coding_goal_flow` | PASS, 6 tests |
| `bash scripts/cargo-agent.sh test -p executive --test approved_apply_flow` | PASS, 5 tests |
| `ALETHEON_REAL_PI=pi ALETHEON_REAL_PI_VERSION=0.80.10 bash scripts/cargo-agent.sh test -p executive --test pi_real_contract -- --ignored --exact pinned_pi_rpc_get_state_obeys_reviewed_jsonl_contract` | PASS, 1 real Pi test |
| `bash tests/operations_cli_test.sh` | PASS |
| `bash tests/operations_cli_static_test.sh` | PASS |
| `bash tests/systemd_runtime_boundary.sh` | PASS |
| `git diff --check` | PASS |

H10 therefore satisfies every acceptance item at
`docs/plans/2026-07-21-production-readiness-hardening.md:277-282`. H11 remains a separate
behavior-preserving composition stage.
