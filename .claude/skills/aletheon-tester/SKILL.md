---
name: aletheon-tester
description: Layered development, debug, and installed-runtime validation for Aletheon. Selects the smallest applicable deterministic checks before escalating to real-TUI acceptance or soak testing.
version: 2.0.0
author: aurb
triggers: ["test aletheon", "aletheon test", "aletheon 测试", "test the daemon", "压测 aletheon", "调试 aletheon", "aletheon debug", "aletheon 调试", "aletheon testing", "daemon test"]
domain: general
tags: ["aletheon", "testing", "daemon", "debug-loop", "mcp"]
---

# Aletheon Tester

Use the cheapest evidence that can answer the current question. Source
regression, installed-runtime smoke, release acceptance, and model-behavior
soak are separate modes. Do not silently escalate from one mode to another.

## Mode selection

| Mode | Use when | Required work |
|---|---|---|
| `regression` | Iterating on source or reproducing a deterministic defect | Diff-derived checks and exact failed-test reruns; no deploy |
| `smoke` | A runtime-facing candidate is ready | One deploy if provenance is stale, then one bounded real client/TUI scenario |
| `acceptance` | The user requests final/pre-merge installed acceptance | Typed deployment gate plus all applicable deterministic and real-runtime assertions |
| `soak` | Routing, model arguments, multi-turn state, cancellation, or performance is under test | Repeated fresh runs or sustained-session scenarios after acceptance is stable |

Default to `regression` while files are changing. For runtime-facing changes,
run `smoke` once after the candidate is stable. Three repetitions belong only
to `soak` for model-controlled behavior; deterministic tests never require
three repetitions.

State the selected mode, comparison base, scenario, assertions, and time budget
before execution. If the user asks for a broader mode, obey the request.

## Role boundary

The Tester is verify-only. It may inspect source, run admitted validation, and
return evidence. It must not edit production files. When a failure requires a
repair:

1. return the exact failed command, relevant output artifact, and classification;
2. let the coding/debug role apply one scoped fix;
3. rerun only the failed step against the new workspace version;
4. run the remaining selected steps after the failure passes.

Do not combine diagnosis, several speculative fixes, deployment, and acceptance
inside one opaque loop. Stop after two unchanged failure signatures and report
the blocker instead of consuming a fixed five-iteration budget.

## Regression mode

Use the repository-owned selector:

```bash
report=${TMPDIR:-/tmp}/aletheon-changed-validation.json
bash scripts/aletheon.sh test changed --plan
bash scripts/aletheon.sh test changed --report "$report"
```

The selector derives affected packages and direct workspace dependents from
Cargo metadata. A changed integration-test entry selects only that target;
shared integration support selects the package integration suite. It fails
fast by default. Repository-owned `changed_validation.rust_test_groups` may
map production sources to narrower library filters and integration targets.
Mapped packages are compiled together once, every filter must match at least
one real test, and any unmapped source or manifest change falls back to the
package's complete library or binary tests.

After a repair, do not rerun successful steps:

```bash
bash scripts/aletheon.sh test changed --rerun-failed "$report"
bash scripts/aletheon.sh test changed --resume "$report"
```

Use `--keep-going` only when a complete failure inventory is more useful than
fast feedback. Use full workspace tests only for merge/release gates or when a
workspace manifest change makes narrower evidence insufficient.

The first command proves the repaired failure in isolation. The second keeps
passed receipts and continues failed or not-yet-run steps. Preserve the report;
it records changed paths, selected commands, selection
reasons, status, exit code, and duration. Never execute a failed command from an
old report unless it remains in the current diff-derived plan.

## Installed runtime preflight

`smoke`, `acceptance`, and `soak` must identify the source and installed build:

```bash
pwd -P
git rev-parse --show-toplevel
git rev-parse HEAD
git status --short --branch
sha256sum "$(command -v aletheon)"
aletheon version
systemctl show aletheon -p ActiveEnterTimestamp -p ExecStart -p FragmentPath
```

Inspect available monitor capabilities for `aletheon_check_install`,
`aletheon_health`, and `aletheon_diagnose`. Prefer them when all are present;
otherwise use `tools/aletheon-monitor`, then the installed CLI, tmux, session
JSONL, and journal as a final fallback. State the track used.

If checkout, staged release candidate, installed binary, and running daemon
provenance cannot be reconciled, do not judge behavior. In `smoke` or above,
deploy once through the canonical boundary:

```bash
sudo bash scripts/aletheon.sh deploy
```

Do not rebuild and redeploy after every deterministic failure. Stabilize the
source candidate in `regression` first.

## Real execution path

`aletheon_ask` is introspection-only. It does not create a real ReAct turn, run
tools, advance the turn count, or exercise TUI rendering. Never use it as
agentic or user-facing acceptance evidence.

Use one of these paths:

```text
aletheon exec --prompt "<task>" --output json
aletheon_diagnose(task="<task>")
aletheon_tui_start / aletheon_tui_send / aletheon_tui_capture
```

User-facing defects require the installed TUI and official user socket. Do not
substitute a direct provider call, temporary daemon, source binary, or alternate
socket for installed acceptance.

## Assertions

Define scenario-specific assertions before the run. Always require:

- the expected tool/runtime actions reached authoritative terminal state;
- forbidden infrastructure/provider errors are absent;
- a substantive task-complete answer is visible through the exercised client;
- the prompt returned and the rendered frame settled;
- the launch cwd and active session match the requested workspace;
- audit, session, rendered frame, and daemon journal agree.

Do not use response length, a minimum tool-call count, or a fixed number of
reflection events as a pass condition. They are diagnostic measurements only.
Always reject rendered or logged occurrences of:

```text
provider_unavailable
provider_rejected_request
provider_timeout
inference provider failed
google_unauthorized_account
Can't mount proc
Permission denied
Aletheon authorization failed
```

For asynchronous Agent tools, require `agent_wait` or a durable terminal event.
A spawn handle and plausible prose are not completion evidence. Runtime facts
such as provider, model, context capacity, session, selected child runtime, and
budgets must come from effective host state, never model self-identification.

## Scenario profiles

### Smoke

Run one short deterministic task that exercises the changed surface. A TUI
change must use the real TUI; an IPC-only change may use `aletheon exec` when UI
rendering is not part of the assertion. Capture latency and the final frame or
JSON terminal result.

### Repository analysis

Require a bounded batch read of known entry files before scoped discovery.
Reject extension-by-extension inventory, unsupported architecture claims, or
continued searching after evidence coverage is sufficient. Record inference
rounds, provider retries, tool calls per round, and batched arguments
separately.

### Model/runtime routing

First prove the selected runtime from typed host events. In `soak`, run the same
task three consecutive times and require the terminal runtime receipt each
time. A single success is adequate for `smoke`, not for final routing
acceptance.

### Multi-turn and cancellation

For sustained-session changes, keep one fresh TUI session for at least three
turns: overview, grounded follow-up, and short unrelated prompt. For
cancellation, require a typed cancelled terminal state, prompt recovery, and no
empty outcome, schema mismatch, memory observation, or abnormal cleanup error.

## Completion detection and evidence

Never use a fixed sleep as completion evidence. Poll normalized frames until:

1. the frame changed after submission;
2. the frame hash stayed stable for the settle window;
3. the input prompt returned.

Preserve only the evidence applicable to the selected mode:

- regression report and exact failure output;
- source commit and workspace version;
- installed/staged/running binary hashes for installed modes;
- session ID, final rendered frame, terminal tool receipts, and audit excerpt;
- journal errors since daemon start;
- inference rounds, retries, tool counts, latency, and context occupancy for soak.

Any aggregate PASS that disagrees with the rendered frame, persisted session,
audit, or daemon logs is a monitor defect and a failed run.

## Output

Report:

```markdown
## Aletheon Test Report

- Mode / comparison base / time budget
- Source and deployed provenance when applicable
- Selected validation steps with reasons and durations
- Passed, failed, skipped, and intentionally omitted assertions
- Exact failed command and output artifact
- Runtime/TUI evidence when applicable
- Remaining blocker or next escalation mode
```

Never claim broader acceptance than the selected mode established.
