# Agent Kernel V2：K0 kernel effect / operation / process census

Date: 2026-08-09
Baseline: `0bf690b2`
State: evidence-only census closed; `kernel-effect-census.tsv` frozen; no behavior change

## Context receipt

```text
Slice: K0 kernel effect/operation/process census
Baseline commit: 0bf690b2
Direct prerequisites: RA-00 (frozen runtime authority census)
Current authoritative writer: unchanged legacy KernelRuntime/DefaultCapabilityInvoker paths
Target owner/writer: unchanged by this census
IDs minted here: none
Production callers: enumerated per row (executor / admission / operation terminal / process spawn)
Test-only callers: excluded (production_callers=0 convention)
Installed/config callers: cross-checked against config/architecture/*.tsv
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none added
Deletion owner: per-row target K/E slice
Unknowns/blockers: none CLOSED as INVESTIGATE; see §4 for K0-OP-01..05 non-durable finding
Expected files: config/architecture/kernel-effect-census.tsv, this evidence record, gate
Out-of-scope files: all production behavior, writer cutovers, K1+
```

## 1. Current effect surface (answers to kernel plan §10 K0)

The production kernel admission path is a single admit→execute→settle chain:

```text
CapabilityRequest
  -> DefaultCapabilityInvoker::invoke_inner (kernel/src/capability/mod.rs:151)
       admit()  -> ExecutionPermit          (kernel/src/admission/production.rs:187)
       sandbox check (fail closed)          (mod.rs:212)
       execute_with_permit()                (mod.rs:243)
       settle(permit, usage)                (mod.rs:287)
  -> TurnToolExecutor (tool_executor.rs:337) delegates to Corpus Tool implementations
  -> Corpus tools spawn real OS children (git/managed_command/script/etc.)
```

### Key findings

1. **Kernel operation table is in-memory, NOT durable** — `OperationTable.records` is `Mutex<HashMap<OperationId, OperationRuntime>>` (`kernel/src/operation/table.rs:17,30`). `submit_with_id`/`start`/`succeed`/`fail` mutate an in-memory map only. There is **no durable ExecutionJournal and no restart recovery** for operations. This is the exact gap `K1` must close (durable Operation authority + journal).

2. **The single enforcement seam is `DefaultCapabilityInvoker`**, but OS-spawn effects live deep in Corpus tools (`managed_command.rs:185`, `git_tools.rs:39`, `script_tool.rs`, `kernel_build.rs`, `workspace_version.rs`, `change_transaction.rs`) below the `ToolExecutor` boundary. The executor's guarantee holds only if every tool reaches admission via the invoker — a bypass-risk noted as `K0-BP-01/02`.

3. **`execd` is a separate process boundary** spawned directly by daemon bootstrap (`execd_client.rs:100`) and running its own `ProcessManager::spawn` (`execd/src/process.rs:49,91,96,119`) with `RpcError` protocol auth — it is **not gated by a capability permit**. `K5` must introduce a `ProcessController` here.

4. **Receipt diversity** — no single terminal receipt type: kernel operations use `OperationTable` terminal + `CapabilityResult.audit_id`; Agent runs use `agent_terminal_receipts` (sqlite_repository.rs:794); tools use `fabric::ToolResult`; execd uses RPC responses. `K4` must unify admit/invoke/receipt/recovery.

5. **External irreversibility** — Gmail/Google (`K0-EX-10`), Pi worktree mutation (`K0-EX-08`), managed_command arbitrary execution (`K0-EX-01`), and script/kernel-build are externally irreversible; Hardware is simulator-only (fail-closed per `architecture-status.toml:55-60`), Robot is fail-closed/unsupported without HIL evidence.

## 2. Census artifact

- `config/architecture/kernel-effect-census.tsv` — 22 rows covering operation lifecycle (5), admission/permit (2), process-spawn executors (10), filesystem (1), network Gmail (1), hardware (1), robot (1), journal (2), bypass-risk (2). All rows `CLOSED`.

## 3. Stop-condition check (plan §6.3)

| stop condition | result |
|---|---|
| production effect bypassing Kernel without confirmed business owner | Corpus tools execute below ToolExecutor but the admitted path (`TurnToolExecutor`) is the confirmed owner; direct spawns like execd are flagged `K0-BP-02` with owner = daemon bootstrap |
| current success doesn't depend on authoritative receipt | All rows show a receipt (ToolResult/audit_id/receipt table/RPC); no silent-success row found in production paths |
| real Hardware/Robot path lacks safety veto/freshness | Hardware is simulator-only; Robot is fail-closed — no real-execution row was censed as supported |
| need to change Session/Turn writer to describe effect | not required for any K0 row |

No stop condition triggered; census closed with all rows CLOSED.

## 4. Non-durable operation journal (feeds K1)

`OperationTable` (`kernel/src/operation/table.rs`) is in-memory:
- restart loses all submitted/running operations;
- recovery relies on higher-level AgentRecoveryCoordinator, not on the operation table;
- `K1` must introduce the durable `ExecutionJournal`/`Operation authority` seam before any writer cutover.

## 5. Validation

```text
git diff --check                                  PASS
bash -n scripts/libexec/aletheon/architecture-check.sh   PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture   PASS (23 findings, 0 deps)
bash scripts/cargo-agent.sh fmt --all -- --check  PASS
bash scripts/cargo-agent.sh check -p kernel       PASS (18.09s)
bash scripts/cargo-agent.sh check -p execd        PASS (5.08s)
bash scripts/cargo-agent.sh check -p executive --lib  PASS (14.57s)
```

### Gate mutation tests (probes, reverted after each)

| probe | result |
|---|---|
| new `Command::new` in an uncovered file | REJECTED (unregistered effect site) |
| clean baseline | ACCEPTED (38 rows, 13 cols, all evidence matches) |
| census structural corruption (wrong col count) | REJECTED |

K0 gate classifies by owner (file-level registration), per kernel plan §10 K0 — it does not ban every `Command::new`/`.spawn()` occurrence.

## 6. Scope boundaries

- **Allowed**: kernel-effect-census.tsv, this evidence record, K0 gate.
- **Explicitly not done**: no Operation API, no durable journal, no executor change, no process-spawn behavior change, no deployment.
