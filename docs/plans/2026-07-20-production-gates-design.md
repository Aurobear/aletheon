# Production Gates Design

> **Status:** Approved design
>
> **Date:** 2026-07-20

## 1. Objective

Close three verification gaps without adding crates or weakening architecture
boundaries:

1. prove the current architecture fitness gate passes remotely;
2. make deterministic coding checks mandatory while keeping paid inference
   explicit and reproducible;
3. add adversarial Linux filesystem tests for the operation-scoped `openat2`
   boundary.

## 2. Current-state evidence

- PR #106 failed only the `Reject architecture drift` step because it added
  dependency-baseline entries while the historical base-ref rule allowed those
  files only to lose entries. The current committed baseline and generated
  dependency graph now agree, and `bash scripts/architecture-check.sh` passes.
- The ordinary CI currently does not call `tests/coding/static_test.sh` or
  `tests/coding/replay_test.py`: `.github/workflows/ci.yml:1-106`.
- The real harness invokes the built `aletheon exec` entry point and requires a
  running inference core: `tests/coding/harness/run.py:37-96`.
- Default inference is `leju` using `deepseek/deepseek-v4-pro`:
  `config/default.toml:1-33`.
- Linux filesystem operations use pinned descriptors and
  `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)` and fail closed on
  unsupported kernels: `crates/platform/src/backend/linux/filesystem_host.rs:430-507`.

## 3. Chosen approach

Use two CI tiers:

```text
pull request / push
  -> architecture fitness
  -> coding static + replay (no provider, deterministic)
  -> Linux Platform adversarial contract tests

workflow_dispatch
  -> build aletheon
  -> create ephemeral Leju config from GitHub Secret
  -> start inference core
  -> run 3 real coding fixtures sequentially
  -> classify result and upload receipts/logs
```

The real evaluation is manual rather than per-PR or scheduled. This keeps model
spend explicit and prevents transient provider failures from blocking unrelated
pull requests.

## 4. Architecture fitness closure

The historical failure is not fixed by weakening `scripts/architecture-check.sh`.
PR #106 intentionally replaced a renamed dependency topology, so its comparison
against the pre-consolidation base correctly rejected added baseline lines. The
new change does not alter architecture baselines. A green architecture job on
the new PR is the remote proof that the current `dev` baseline is internally
consistent.

The workflow must use `scripts/cargo-agent.sh` for Cargo-backed architecture
metadata, matching repository policy. Existing fixture and path-inventory gates
remain mandatory.

## 5. Deterministic coding gate

Add one ordinary CI job that runs:

- `python3 tests/coding/replay_test.py`;
- `bash tests/coding/static_test.sh`.

This job requires no API key and validates receipt integrity, operation
correlation, false-success rejection, process-group timeout cleanup wiring and
the fixed task schema. Checked-in golden receipts remain replay inputs; the PR
job never overwrites them.

## 6. Manual Leju coding evaluation

Create a separate `.github/workflows/coding-e2e.yml` with only
`workflow_dispatch`. It has these invariants:

- provider is exactly `leju`;
- model is exactly `deepseek/deepseek-v4-pro`;
- credential comes only from the `LEJU_API_KEY` GitHub Secret;
- generated config and HOME live under the runner temporary directory;
- the core process is started once and cleaned up with a shell trap;
- fixtures run sequentially to avoid concurrent Executive builds and provider
  bursts;
- each fixture writes a distinct receipt in the workflow artifact directory;
- core logs and receipts upload even on failure;
- provider/core transport failures are reported as infrastructure failures;
  completed runs with failed acceptance remain Agent verification failures.

No secret value is printed or persisted in an uploaded artifact. Absence of the
secret fails before starting the core with a clear diagnostic.

## 7. Linux filesystem adversarial coverage

Extend the existing Platform contract suite; do not create a new test crate.
Tests cover:

1. **Root replacement:** construct a scoped host, rename the admitted directory,
   replace its original pathname with a symlink to an outside directory, then
   attempt read/write/remove. The outside directory must remain unchanged; the
   backend may continue through its pinned root fd or fail closed.
2. **Parent swap pressure:** repeatedly swap a workspace parent entry between an
   admitted directory and an outside symlink while atomic writes execute. No
   outside sentinel may change and no temporary file may escape.
3. **Unavailable syscall:** inject a narrow test-only `openat2` syscall seam and
   return `ENOSYS`; every scoped operation must return a typed fail-closed error
   without falling back to canonicalize-and-reopen.

The syscall seam remains private to the Linux backend and selects the real libc
syscall in production. It does not become a Platform contract or permission
authority.

## 8. Failure classification

| Failure | Classification | Required behavior |
|---|---|---|
| Static/replay mismatch | Repository defect | Fail ordinary CI |
| Architecture job failure | Architecture defect | Fail ordinary CI |
| Outside filesystem mutation | Security defect | Fail ordinary CI |
| Missing `LEJU_API_KEY` | Configuration error | Fail before core start |
| Provider 429/5xx or core readiness failure | Infrastructure failure | Fail manual run with logs |
| Agent exits normally but acceptance fails | Agent verification failure | Preserve failed receipt |

## 9. Verification

- `bash scripts/cargo-agent.sh test -p platform --test contract_suite`
- `python3 tests/coding/replay_test.py`
- `bash tests/coding/static_test.sh`
- `bash scripts/architecture-check.sh`
- `bash scripts/cargo-agent.sh fmt --all -- --check`
- `git diff --check`
- new pull request architecture, coding-gate and Platform jobs are green
- one manually dispatched Leju run produces three independent receipts

The last item requires the repository secret and GitHub runner; local tests do
not substitute for it.
