# Agent Kernel V2：E0 extension preservation manifest

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md` §9
State: evidence-only preservation manifest closed; `extension-preservation.tsv` frozen; no behavior change

## Context receipt

```text
Slice: E0 extension preservation manifest
Baseline commit: 0bf690b2
Plan revision: implementation-plan §9.1-9.2
Direct prerequisites: RA-00, K0, APX-00, CGP-00
Current authoritative writer: unchanged legacy Executive extension paths
Target owner/writer: unchanged by this census
IDs minted here: none
Production callers: enumerated per extension
Test-only callers: excluded
Installed/config callers: cross-checked against config/architecture/persistence-surfaces.tsv + installed config
Tables/files/wire schemas: none changed
External side effects: none (no destructive smoke run)
Compatibility seam: none added
Deletion owner: per-row cutover slice (E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d)
Unknowns/blockers: none
Expected files: config/architecture/extension-preservation.tsv, this evidence record, E0 gate
Out-of-scope files: all production behavior, extension cutovers (E-series)
```

## 1. Extension manifest (extension-preservation.tsv)

5 preserved extensions, each with production caller, config, installed state, credential ref, schema/file, worker, external effect, cancel/restart, equivalence oracle, safe smoke, target owner, cutover slice, rollback blocker:

| extension | production caller | external effect | target/cutover |
|---|---|---|---|
| Gmail/Google | google integration (`integrations.rs:316` google_enabled) + corpus google tools | external email/drive/calendar (irreversible) | E2-K6a |
| GBrain | gbrain bootstrap (`bootstrap.rs:22`) + McpManager | external MCP supplemental memory | E3 |
| Hardware | hardware simulator + deployment_gate | simulator-only; real execution fail-closed | E4-K6b |
| Robot VLA | robot bootstrap (`robot.rs:158`) + episode promoter | robot episode (fail-closed without HIL) | E5-K6c |
| Pi | `request.rs:1065` register_pi_runtime + `coding.rs:10` CodingRuntimeConfig | Pi coder worktree mutation (irreversible) | E6-K6d |

## 2. Preservation constraints honored (plan §9.2)

- **No destructive smoke.** Only installed-config no-destructive paths: Gmail no send/delete, GBrain spool open, Hardware simulator, Robot simulator, Pi probe (no spawn).
- **Hardware/Robot stay fail-closed** for real execution — `hardware/src/lib.rs:3-10` ("Real actuators are deliberately unsupported"), `deployment_gate.rs:80-86` (HIL device_id/serial/endpoint required).
- **Pi credential/safety**: executable SHA-256 + `json_protocol_version` pinning (`coding.rs`), `require_namespace_isolation`, no credential in argv.
- **Each cutover PR is the sole executor/writer owner** per the joint E-series/K-slice numbering (E2-K6a, E4-K6b, E5-K6c, E6-K6d) — Kernel seam does not own extension writers.

## 3. Equivalence oracles per extension

| extension | oracle |
|---|---|
| Gmail | `google-event-store` schema (persistence-surfaces.tsv v1 read/v2 write) + `google_tool_flow.rs` |
| GBrain | spool policy config equivalence (`SpoolPolicy` path/max_items/max_bytes) |
| Hardware | `hardware_simulation.rs` + simulator state replay |
| Robot | `robot_audit_chain.rs` + `robot_session_e2e.rs` |
| Pi | `pi_real_contract.rs` + `pi_runtime.rs` (protocol version contract) |

## 4. Validation

```text
git diff --check                                  PASS
bash -n scripts/libexec/aletheon/architecture-check.sh   PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture   PASS
bash scripts/cargo-agent.sh fmt --all -- --check  PASS
bash scripts/cargo-agent.sh check -p hardware      PASS
bash scripts/cargo-agent.sh check -p executive --lib  PASS
```

## 5. Scope boundaries

- **Allowed**: extension-preservation.tsv, this evidence record, E0 gate.
- **Explicitly not done**: no extension behavior change, no credential exposure, no real-actuator/robot/Pi spawn, no destructive smoke, no cutover.
