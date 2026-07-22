# H9 Pi Governance Evidence — 2026-07-22

## Requirement anchors

- Pi 已有 exit/elapsed/token/cost/diff hash；H9 只修 capability 空占位与 `diff_artifact` 稳定引用：
  `docs/plans/2026-07-21-production-readiness-hardening.md:260-264`。
- capability 必须来自可观测信号或明确 unavailable；artifact/hash 必须一致且有界；fixture 与
  env-gated real Pi 均需发现协议漂移：
  `docs/plans/2026-07-21-production-readiness-hardening.md:265-269`。

## Receipt contract

```text
SandboxBackend::capabilities()
              |
              v
coding_capability_audit
  observed / allowed / unavailable

bounded worktree diff --sha256--> CodingJobReport
              |                  diff_artifact = coding-diffs/<job>.diff
              v
coding_diff_base64 evidence --verified/persisted--> artifact store
```

- `CapabilityAuditSummary` now has a backward-compatible `unavailable_capabilities` field and
  normalizes all three sets (`crates/executive/src/service/verification/mod.rs:368-387`).
- Pi records the sandbox backend's actual boolean capability report: enabled signals go to
  `observed_capabilities`, disabled signals to `unavailable_capabilities`, and the runtime-owned
  allow-list is explicit. Empty vectors no longer masquerade as a present audit
  (`crates/executive/src/impl/runtime/pi.rs:285-315,685-693`). The existing verifier still fails
  missing audits and observed capabilities outside the allow-list
  (`crates/executive/src/service/verification/checks.rs:59-85`).
- Pi report assigns the stable relative reference `coding-diffs/<job-id>.diff` and retains the
  worktree snapshot SHA-256 (`crates/executive/src/impl/runtime/pi.rs:641-658`). The same job-derived
  reference is used by the durable artifact store; a conflicting incoming reference fails closed
  before artifact write (`crates/executive/src/impl/goal/verification.rs:82-120`).
- Diff evidence remains bounded by the worktree collector's 8 MiB output cap
  (`crates/corpus/src/tools/subagent/worktree.rs:14,32-39`) and by the durable artifact store's
  16 MiB hard limit (`crates/executive/src/impl/goal/verification.rs:14,82-114`). Existing path scope,
  forbidden-path, hash and verification checks remain the sensitive-content boundary.

## Deterministic validation

```text
bash scripts/cargo-agent.sh test -p executive --test pi_runtime
# 7 passed; fixture asserts diff evidence hash/reference and observed/unavailable capability signals

bash scripts/cargo-agent.sh test -p executive --test verification_service
# 7 passed

bash scripts/cargo-agent.sh test -p executive impl::goal::verification::tests --lib
# 6 passed

ALETHEON_REAL_PI=pi ALETHEON_REAL_PI_VERSION=0.80.10 \
  bash scripts/cargo-agent.sh test -p executive --test pi_real_contract -- \
  --ignored --exact pinned_pi_rpc_get_state_obeys_reviewed_jsonl_contract
# 1 passed against /usr/bin/pi 0.80.10
```

The real gate checks reviewed build identity, LF-framed JSONL, request correlation, command identity,
success status and `get_state.data.isStreaming`; the fixture gate checks Aletheon receipt semantics.
