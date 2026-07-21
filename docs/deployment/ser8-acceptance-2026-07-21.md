# SER8 deployment acceptance — 2026-07-21

## Scope and requirement anchors

This record closes the SER8 deployment goal: a resident core and user daemon,
real TUI inference, bounded Pi execution, durable local/GBrain memory, and an
auditable systemd user timer
(`docs/archive/plans/2026-07-21/2026-07-21-aletheon-ser8-deployment.md:6-7,177-187`). The original
operator deployment note remains unchanged as the historical requirement
source. Current operators must use `docs/deployment/README.md`. Its `pi-rpc`
worktree wording conflicts with the runtime manifest; the approved dual-runtime
adjudication and three-column comparison are recorded in
`docs/archive/plans/2026-07-21/2026-07-21-ser8-pi-memory-closure-design.md:19-42`.

## Accepted topology

```text
TUI / timer
    -> user aletheon daemon
       -> machine core socket -> Leju inference
       -> pi-rpc   -> resident, shared but narrowed workspace authority
       -> pi-coder -> bwrap + disposable Git worktree -> bounded diff
       -> local GoalOutcome -> durable spool -> local GBrain MCP
```

The first-turn fallback is implemented at
`crates/executive/src/service/context_assembler.rs:103-120`. Pi RPC canonicalizes
its configured roots and narrows inherited authority at
`crates/executive/src/impl/runtime/pi_rpc.rs:149-173,459-482`; the generic
fail-closed authority operation is
`crates/fabric/src/types/local_authority.rs:116-152`. Terminal Agent events
become durable `GoalOutcome` records at
`crates/executive/src/service/agent_control/memory.rs:156-224`.

## Runtime acceptance evidence

| Gate | Result | Reproducible evidence |
|---|---|---|
| Core and user daemon | PASS | `aletheon-core.service` and user `aletheon.service` active; both sockets listening. The deployed daemon process includes group `984` (`aletheon`). |
| Deployed binaries | PASS | `/usr/bin/aletheon` SHA-256 `17e489d1ed29726594aaaac99e279dc0606344ff9f207e12434930f6ee519253`; `/usr/bin/pi` SHA-256 `af302f231437eaf6f37691bce4b34234fcb626bcb5eb3910d4fc3f6519bf78ca`; Pi package is pinned to `0.80.10`. |
| Real TUI | PASS ×3 | `/tmp/aletheon-goal-evidence/tui-repeat-1784570715-1`, `...0718-2`, and `...0723-3`. Each `result.json` proves `turn_done`, stable frame, returned prompt, expected answer and cwd, and absence of forbidden infrastructure errors. Event files are private mode `0600`. |
| First-turn context | PASS | The three fresh TUI runs above completed without `conscious workspace has not observed a turn`; targeted `context_assembler` integration tests passed 5/5. |
| Pi RPC resident path | PASS | `/tmp/aletheon-goal-evidence/pi-rpc-live-PI_RPC_OK_1784604633`: new Agent `1c25832a-6be2-45ff-99a0-e09dee0b7b57` settled `Succeeded`, read the fixture through real Pi/Leju, and left the source hash unchanged. |
| Pi coding isolation/diff | PASS | Real Agent `fb605204-a815-4030-83b3-ffe159c3bef7`, job `dcf5013c-2586-40b1-8388-2858ff4fdaa8`, attempt `5e498a83-9af6-4b83-ba21-dacc23516481`; retained worktree diff SHA-256 `0c8d93dfa4d83264b3535901874fd249d3edc30b8bfd38041b4952b8a322e012`. `/tmp/aletheon-pi-fixture-1784570120` remains clean at `ee6f6f2640e930349c08a039f1a18ec00715cdcd`. |
| Reviewed apply contract | PASS | `bash scripts/cargo-agent.sh test -p executive --test approved_apply_flow approved_apply_is_consumed_once_and_completes_goal` passed. It verifies hash-bound apply, one-time consumption, receipt, and goal completion against a disposable fixture; no live Aletheon checkout was applied. |
| Mnemosyne/Agent memory | PASS | `~/.local/state/aletheon/agents/agent_memory.db` contains four `goal_outcome` records, including the live pi-coder, scheduled, outage, and pi-rpc Agents. |
| GBrain projection | PASS | Authenticated MCP search returned `aletheon/goal_outcome/21ba8f0b22285562beeb01d4b05a2749`; structural response proof is `/tmp/aletheon-goal-evidence/gbrain-search-live.body`. No credential value is stored in this record. |
| GBrain outage recovery | PASS | `/tmp/aletheon-goal-evidence/gbrain-outage-1784603673`: while GBrain was stopped, local settlement created one page and one pending queue record `agent-outcome:639ff37b-f17b-4da8-b9e4-c7e5ac98e4f6`; restart delivered attempt 8, drained queue/pages to zero, left dead letters at zero, and created exactly one new receipt. Authenticated search found slug `aletheon/goal_outcome/57ea5fbdb1ccd5f2ce548fbdb3858467`. |
| Configured GBrain endpoint | PASS | The point-in-time acceptance used a same-host GBrain endpoint. The current SER8 deployment selects its reachable endpoint explicitly and supports loopback or Tailscale without a repository hard-code; see `docs/deployment/README.md`. |

## Scheduled closure acceptance

The tracked service applies a six-minute ceiling, private umask, and user-service
hardening (`deploy/systemd/user/aletheon-pi-closure.service:6-14`). The timer is
persistent, runs daily at 03:15 with up to 30 minutes randomized delay, and
activates only that oneshot service
(`deploy/systemd/user/aletheon-pi-closure.timer:4-12`). The wrapper validates
socket, executables and dedicated Git fixture, takes a nonblocking lock, creates
private evidence, generates unique job/attempt IDs, and enforces a hard timeout
(`scripts/aletheon-pi-scheduled-task.sh:4-42,51-101`).

| Timer gate | Result | Evidence |
|---|---|---|
| Installed artifacts | PASS | Tracked service, timer, and wrapper are byte-identical to their installed copies under `~/.config/systemd/user/` and `~/.local/bin/`. |
| Manual real invocation | PASS | User service completed `Result=success`, status 0; receipt `~/.local/state/aletheon/scheduled-evidence/20260720T191210Z.jsonl`, Agent `da2db428-ad97-474a-a0d8-11cee51dcb01`, Pi job `122b64ec-ddec-486e-9cf7-a6178f4b0f2e`, attempt `e7640d1c-58b5-4194-9929-dd53fcc17ff7`. |
| Outage invocation | PASS | The second real service invocation completed status 0 while GBrain was unavailable and produced the recovery evidence above. |
| Overlap | PASS | `/tmp/aletheon-goal-evidence/timer-overlap-1784604749`: first invocation 0, overlapping invocation 75, one mode-`0600` receipt. |
| Timeout | PASS | `/tmp/aletheon-goal-evidence/timer-timeout-1784604714`: a sleeping fake client was terminated with status 124 and the private receipt recorded 124. |
| Enabled schedule | PASS | `aletheon-pi-closure.timer` is enabled and active; the current randomized next trigger is `2026-07-22 03:44:42 CST`, within the configured 03:15–03:45 window. `systemd-analyze --user verify` passed. |

## Targeted validation

All repository Cargo commands used the required bounded wrapper.

```text
bash scripts/cargo-agent.sh test -p executive --test context_assembler       # 5 passed
bash scripts/cargo-agent.sh test -p executive --test pi_runtime --test pi_rpc_runtime
bash scripts/cargo-agent.sh test -p executive --test gbrain_bootstrap         # 7 passed
bash scripts/cargo-agent.sh test -p executive --test approved_apply_flow \
  approved_apply_is_consumed_once_and_completes_goal                          # 1 passed
bash scripts/cargo-agent.sh test -p fabric --test local_authority_contract     # 4 passed
bash scripts/cargo-agent.sh fmt --all -- --check                              # passed
systemd-analyze --user verify ~/.config/systemd/user/aletheon-pi-closure.{service,timer}
```

## Operational notes

- Do not place provider or GBrain tokens in repository files, argv, prompts, or
  evidence. They remain in mode-restricted external environment files.
- The scheduled task targets only
  `~/.local/state/aletheon/scheduled-fixture`; it never targets this checkout.
- Review retained Pi diffs before any apply. The timer deliberately does not
  auto-apply them.
- Inspect the next run with
  `systemctl --user status aletheon-pi-closure.service` and
  `journalctl --user -u aletheon-pi-closure.service`.
