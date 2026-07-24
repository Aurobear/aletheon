---
name: aletheon-tester
description: End-to-end daemon testing loop for aletheon using MCP monitor tools. Sends tasks, watches execution, diagnoses issues, applies fixes, rebuilds, and retests until the agent completes complex tasks successfully. Triggers on "test aletheon", "aletheon test", "aletheon 测试", "test the daemon", "aletheon debug", "aletheon 调试".
version: 1.1.0
author: aurb
triggers: ["test aletheon", "aletheon test", "aletheon 测试", "test the daemon", "压测 aletheon", "调试 aletheon", "aletheon debug", "aletheon 调试", "aletheon testing", "daemon test"]
domain: general
tags: ["aletheon", "testing", "daemon", "debug-loop", "mcp"]
---

# Aletheon Tester — MCP-Powered Daemon Test Loop

## Mandatory execution contract

Never report that Aletheon was tested from source inspection or an RPC-only
response. A successful run must identify the deployed build and exercise the
same client path the user reported.

### Capability selection

1. Inspect the tools available in the current session for
   `aletheon_check_install`, `aletheon_health`, and `aletheon_diagnose`.
2. If all are present, use the monitor MCP track.
3. Otherwise, if `tools/aletheon-monitor` exists in the active source tree,
   run its pytest suite and use its tmux helpers directly.
4. Otherwise fall back to `aletheon -m`, tmux, session JSONL, and journalctl.
5. State which track was used. Missing MCP registration is not permission to
   skip the real TUI test.

### Source and deployment preflight

Do not assume `/home/aurobear/Bear-ws/aletheon` is the source being deployed.
Record all of the following before changing or testing anything:

```bash
pwd -P
git rev-parse --show-toplevel
git rev-parse --show-prefix
git rev-parse HEAD
git status --short --branch
sha256sum "$(command -v aletheon)"
aletheon version
systemctl show aletheon -p ActiveEnterTimestamp -p ExecStart -p FragmentPath
```

If the active checkout and deployed binary provenance cannot be reconciled,
stop and fix deployment before evaluating behavior. Never edit a different
checkout merely because it is the historical default path.

### Task assertions

Define assertions before each run. Response length and tool count are
diagnostic metrics, not success criteria. At minimum assert:

- required tool calls completed successfully;
- forbidden infrastructure errors are absent;
- a substantive final answer is visible in the real TUI;
- the input prompt returned after the answer;
- the TUI launch cwd matches the expected canonical cwd;
- the model did not scan unrelated host roots to guess a project.

Always forbid these strings unless the test explicitly targets them:

```text
google_unauthorized_account
Can't mount proc
Permission denied
Aletheon authorization failed
```

For model-controlled tool arguments or routing, run the same real-TUI task
three consecutive times. A single success is insufficient.

### Completion and evidence

Do not use a fixed `sleep` as the completion condition. Poll normalized frames
until all of these are true:

1. the frame changed after input submission;
2. the frame hash remained stable for the settle window;
3. the input prompt (`❯`) returned.

Preserve the final frame, session ID, session JSONL path, tool inputs/results,
journal errors since the daemon start timestamp, source commit, binary hash,
and unit properties in the report.

A closed-loop testing skill for aletheon that uses MCP monitor tools to send complex tasks to the running daemon, observe execution in real time, diagnose failures, apply fixes, rebuild, and retest — all without leaving Claude Code.

## When to Use

- User wants to verify aletheon can handle complex tasks (code analysis, multi-file edits, debugging)
- After deploying aletheon changes — run a quick smoke test to check nothing broke
- User says "test aletheon", "aletheon 测试", "test the daemon", "压测 aletheon"
- Debugging aletheon bugs where the agent returns empty/shorter-than-expected responses
- Performance regression testing: does the agent still complete tasks within tool budget?

## Core Loop

```
┌──────────────────────────────────────────────────────────────────┐
│              Aletheon Tester — MCP-Powered Loop                   │
├──────────────────────────────────────────────────────────────────┤
│  1. SETUP   — Health check + install check + define test goal    │
│  2. TEST    — Send task via aletheon_ask, wait for response       │
│  3. MONITOR — Watch real-time events: tool calls, errors, perf    │
│  4. ANALYZE — Composite diagnostic: anomalies, journal, logs      │
│  5. FIX     — Apply fixes to aletheon source, rebuild, restart    │
│  6. VERIFY  — Re-test with same task, compare results             │
│  LOOP       — Repeat until goal achieved or max 5 iterations      │
└──────────────────────────────────────────────────────────────────┘
```

## Phase 1: Setup

### 1.1 Define the Test Goal

Ask the user or infer from context:

- **Task complexity** — Simple (file listing), Medium (code search + read), Hard (multi-file analysis/refactor)?
- **Success criteria**:
  - Agent produces a substantive response (>200 chars, not "Reflection recommended stopping")
  - No unrecoverable errors in tool calls (sandbox Permission denied, provider auth failures)
  - Agent uses at least N tool calls before responding (N≥3 for simple, N≥10 for hard)
- **Max iterations** — default: 5, ask user if the environment looks unstable
- **Aletheon source path** — discover with `git rev-parse --show-toplevel`; never hard-code a checkout
- **Example test tasks**:
  - Simple: "List all Rust source files in crates/ and count them by crate"
  - Medium: "Read crates/runtime/src/core/config/agent.rs and summarize the AgentLoopConfig struct"
  - Hard: "Analyze the aletheon project: identify the 3 most complex modules and explain why"

### 1.2 Pre-Flight Checks

Run these MCP tools before starting the test loop:

1. **Install check**: `aletheon_check_install` — verify setup.sh was run, socket exists, env file found.
2. **Health check**: `aletheon_health` — daemon reachable, systemd active, no provider errors.

If either check fails:
- Socket missing → Run `sudo systemctl restart aletheon`
- Daemon unreachable after restart → **ESCALATE** to user (low-level system issue)
- Provider unhealthy → Check `config/default.toml` for provider config, escalate if API key issue

### 1.3 Record Baseline

Before testing, snapshot current state:

1. `aletheon_snapshot(include_memory=false)` — record: version, session state, turn count, config
2. `aletheon_sessions(action="list")` — note active session IDs for reference

## Phase 2: Test

### 2.1 Send the Test Task

Use `aletheon_ask` to send the task to the running agent:

```
aletheon_ask(question="<task description>")
```

The response includes:
- `question` — the task that was sent
- `response` — the agent's answer text

### 2.2 Evaluate Response Quality

Check the response against these criteria:

| Criterion | Check | Action if Failed |
|-----------|-------|-----------------|
| **Non-empty** | `len(response) > 10` | Phase 4 — investigate why agent stops early |
| **Substantive** | `len(response) > 200` for medium/hard tasks | May be reflection limit — check Phase 4 |
| **Not truncated** | Doesn't end with "stopping" or "BudgetExhausted" | Likely reflection tool_call_limit — raise it |
| **On-topic** | Addresses the actual question | May be context pollution or wrong model routing |

### 2.3 Quick Smoke Test

For fast iteration, use a simple test:

```
aletheon_ask(question="List the top-level directories in /home/aurobear/Bear-ws/aletheon/")
```

Expected: returns a list of directories. If this fails, the daemon has fundamental issues.

### 2.4 TUI Track — test what the USER actually sees

`aletheon_ask` uses `session.ask` RPC and **bypasses the TUI entirely**, so it
cannot see render bugs (duplicate drawing, unrendered markdown, `Reflection:
Reflection:` double-prefix, `未知技能: /path` slash mis-parse). For anything
user-facing, drive the real TUI instead:

```
aletheon_diagnose(task="<the task>")
```

This launches the real `aletheon` TUI in tmux, sends the task, waits for the
frame to settle, and returns:
- `rendered_frame` — what the user actually sees
- `tui_checks` — render assertions that fired (dup_render / raw_markdown /
  double_reflection / unknown_skill_path / permission_denied)
- `daemon.analyze` + `daemon.logs` + `audit_tail` + `timeline`
- `verdict` — pass/fail

Lower-level control if you need it: `aletheon_tui_start` / `aletheon_tui_send`
/ `aletheon_tui_capture` / `aletheon_tui_stop`.

## Phase 3: Monitor

### 3.1 Real-Time Event Watch

Use `aletheon_watch` to subscribe to daemon events while the task runs:

```
aletheon_watch(topic="all", duration_seconds=60)
```

This captures:
- **Tool calls** — which tools the agent invoked, in what order
- **Errors** — tool failures, sandbox issues, provider timeouts
- **Perf events** — LLM call latency, token usage
- **Session events** — compaction, reflection triggers

### 3.2 Key Metrics to Watch

| Metric | Healthy Range | Red Flag |
|--------|--------------|----------|
| Tool calls per turn | 5–40 | 0 (agent didn't use tools) or >80 (infinite loop) |
| Tool error rate | <10% | >25% per-tool error rate |
| Reflection triggers | 1–3 per turn | 0 (no self-check) or >5 (constantly stopping) |
| LLM latency | <30s per call | >60s (provider timeout risk) |
| Context compaction events | 0–1 per turn | >3 (bloated context, Fix 3 regression) |

### 3.3 Watch Command Variations

```bash
# Just tool call events (for debugging Storm Breaker / sandbox issues)
aletheon_watch(topic="tool", duration_seconds=30)

# Performance-only (for profiling)
aletheon_watch(topic="perf", duration_seconds=45)

# All events for comprehensive capture
aletheon_watch(topic="all", duration_seconds=60)
```

## Phase 4: Analyze

### 4.1 Composite Diagnostic

Run `aletheon_analyze` which combines snapshot + performance + journal + anomaly scan in one call:

```
aletheon_analyze()
```

Returns:
- `healthy` — boolean, True if no CRITICAL anomalies
- `anomalies` — list of detected issues with severity (CRITICAL/WARN)
- `snapshot` — current runtime state
- `perf` — LLM and tool performance stats
- `recent_journal` — last 20 session events

### 4.2 Deep Dive — Journal

If the response was truncated or short, check the journal:

```
aletheon_journal(last_n=50)
```

Look for:
- `storm_breaker` events → excessive success/failure warnings (Fix 2 regression?)
- `compaction` events → context bloat (Fix 3 regression?)
- `reflection` events → tool call limit hit? what was the verdict?
- `error` events → provider failures, sandbox errors, JSON parse failures

### 4.3 Deep Dive — Logs

If the daemon itself might be crashing:

```
aletheon_logs(last_n=100, level="ERROR")
```

Check for:
- Panic messages in Rust code
- Socket binding failures
- Provider connection errors
- Bubblewrap/sandbox setup failures

### 4.4 Root Cause Classification

Map symptoms to likely root causes:

| Symptom | Likely Root Cause | Fix Direction |
|---------|------------------|---------------|
| Agent returns 0 tool calls | Provider auth failure, wrong model routing | Check provider config, check cognit logs |
| Agent returns empty after 10 calls | reflection_tool_call_limit too low | Raise in config/default.toml |
| "Permission denied" on /dev/null | Bubblewrap arg order (Fix 1 regression) | Check bubblewrap.rs |
| Storm breaker spam | sb.reset() not called (Fix 2 regression) | Check chat.rs |
| Context blows up in 2 turns | History seeding too much (Fix 3 regression) | Check chat.rs seed logic |
| max_tokens: 4096 in API call | Provider not using config value (Fix 5 regression) | Check provider_registry.rs |
| Agent stuck in infinite tool loop | ToolBudget/circuit breaker not working | Check react_loop/mod.rs |
| Response truncated mid-sentence | LLM stopped by max_tokens or stop reason | Check provider max_tokens, check StopReason |
| TUI shows duplicated blocks | Double draw: stream append + full-message | `crates/interact/src/tui/response.rs` + `chat.rs` |
| `Reflection: Reflection:` double prefix | Prefix added twice | `crates/interact/src/tui/response.rs:212` |
| `未知技能: /path` on an absolute path | Slash-command parser eats file paths | `crates/interact/src/tui/app/submit.rs:25` |
| Markdown tables printed raw | No table rendering | `crates/interact/src/tui/markdown.rs` |

## Phase 5: Fix

### 5.1 Apply Fixes to Aletheon Source

Based on the analysis, edit the relevant source files. Fixes are in the aletheon repo at `/home/aurobear/Bear-ws/aletheon/`.

Common fix locations:

| Component | File Path |
|-----------|-----------|
| Sandbox (bubblewrap) | `crates/corpus/src/security/sandbox/bubblewrap.rs` |
| Sandbox builder | `crates/corpus/src/security/sandbox/bwrap_builder.rs` |
| ReAct loop | `crates/runtime/src/core/react_loop/mod.rs` |
| Reflection engine | `crates/runtime/src/core/react_loop/reflection.rs` |
| Agent loop config | `crates/runtime/src/core/config/agent.rs` |
| Daemon chat handler | `crates/runtime/src/impl/daemon/handler/chat.rs` |
| Provider registry | `crates/cognit/src/impl/provider_registry.rs` |
| Anthropic provider | `crates/cognit/src/impl/llm/anthropic.rs` |
| OpenAI provider | `crates/cognit/src/impl/llm/openai_provider.rs` |
| Storm breaker | `crates/runtime/src/core/storm_breaker.rs` |

### 5.2 Rebuild

```bash
repo=$(git rev-parse --show-toplevel)
cd "$repo"
cargo build --release -p aletheon-bin
```

If build fails, fix compilation errors before proceeding. Common issues:
- Missing imports after adding fields to structs
- Type mismatches (usize vs u32)
- Unused variable warnings that became errors

### 5.3 Restart Daemon

```bash
sudo systemctl restart aletheon
sleep 2
aletheon_health  # verify it came back up
```

If daemon fails to start:
```bash
journalctl -u aletheon --no-pager -n 30  # check startup errors
```

## Phase 6: Verify

### 6.1 Re-run Same Test Task

Send the exact same task from Phase 2 and compare results:

```
aletheon_ask(question="<same task as Phase 2>")
```

### 6.2 Acceptance Criteria

For the fix to be considered successful:

- [ ] Agent produces a substantive response (>200 chars for medium/hard tasks)
- [ ] No CRITICAL anomalies in `aletheon_analyze`
- [ ] Tool error rate < 10%
- [ ] No "Permission denied" errors in tool calls
- [ ] No storm breaker spam in recent journal
- [ ] Context stays bounded across turns
- [ ] `aletheon_diagnose` returns `verdict: pass`
- [ ] `tui_checks` is empty (no dup_render / raw_markdown / double_reflection / unknown_skill_path / permission_denied)
- [ ] TUI frame is `stable: true` (no runaway re-render)
- [ ] TUI reports the canonical launch cwd and does not scan unrelated host roots
- [ ] Source commit, deployed binary hash, service timestamp, session JSONL, and final frame are recorded
- [ ] Model-controlled paths pass three consecutive real-TUI runs

### 6.3 Regression Check

If the task passed before, check that it still passes:

- Compare `response` length and content to the previous baseline
- Compare `tool_calls` count — significant drop may indicate new issues
- Run the smoke test from Phase 2.3 to confirm basic functionality

## Loop Control

### Decision Matrix

After each iteration:

| Condition | Action |
|-----------|--------|
| All acceptance criteria met | **DONE** — Report success |
| Max iterations (5) reached | **ESCALATE** — Report remaining issues to user |
| Same issue 2 iterations in a row | **ESCALATE** — Fix direction is wrong, ask user |
| Daemon fails to start after fix | **ESCALATE** — Fix broke daemon, revert and ask |
| New issues found, previous fixed | **Continue** — Go to Phase 5 with new issues |
| Minor regression only | **Continue** — Tune config values |

### Iteration Tracking

Maintain a running log:

```markdown
## Test Iteration Log

**Task**: "<task description>"
**Goal**: Agent produces substantive response with ≥10 tool calls

| Iter | Response Len | Tool Calls | Healthy | Issues Found |
|------|-------------|------------|---------|-------------|
| 1    | 32          | 20         | ❌      | Reflection stops at 20, response empty |
| 2    | 1,200       | 18         | ✅      | —                            |
```

## Special Scenarios

### Provider Testing

When testing provider configurations:

1. `aletheon_snapshot(include_memory=false)` — check `providers` section for health status
2. If provider is `unhealthy` — verify API key env var is set, network is reachable
3. Switch provider via `config/default.toml` if needed

### Sandbox / Bubblewrap Testing

When testing sandbox permissions:

1. Send task that requires `git`, `cargo`, `rustc`, or `/dev/null` access
2. `aletheon_watch(topic="tool", duration_seconds=30)` — watch for Permission denied
3. If sandbox blocks legitimate tools — check bubblewrap arg order (Fix 1)

### Context / Memory Testing

When testing context management:

1. `aletheon_memory(query="<topic>")` — verify memory recall works
2. Send multi-turn conversation and check `aletheon_analyze` for compaction events
3. If >3 compaction events per turn — context is bloated, check Fix 3

### Performance Regression Testing

When checking for performance regressions:

1. `aletheon_watch(topic="perf", duration_seconds=60)` — capture baseline
2. Send identical task before and after code changes
3. Compare: tool call count, total wall time, LLM calls, error rate
4. >20% degradation in any metric → investigate

## Guardrails

- **Write scope**: Only modify files under `/home/aurobear/Bear-ws/aletheon/`. Do not touch aurb, system config, or unrelated projects.
- **Config safety**: Do not modify `setup.sh`, `config/default.toml`, or provider config unless the analyze phase explicitly identifies a config-level issue. Changing config can break production deployments.
- **Restart safety**: `systemctl restart aletheon` is allowed without user approval (it's a test daemon). Any other destructive system operations require user confirmation.
- **Fix discipline**: One fix at a time. Don't batch unrelated changes — you can't isolate which fix worked if you apply 3 at once.
- **Revert path**: Before applying a fix, note the original state. If the fix makes things worse, revert immediately and escalate.
- **Memory hygiene**: After each iteration, record key findings to memory so context is preserved across sessions.

## Output Format

At the end of the test session, produce:

```markdown
## Aletheon Test Report

**Task**: <task description>
**Iterations**: <number>
**Status**: PASS / PARTIAL / ESCALATED

### Issues Found and Fixed
1. ✅ <issue> — Fixed in <file>:<line>
2. ✅ <issue> — Fixed in <file>:<line>
3. ❌ <issue> — Not fixed, <reason>

### Files Changed
- <file> — <what changed and why>

### Daemon Health
- Version: <version>
- Socket: OK / MISSING
- Systemd: active / inactive
- Provider: healthy / unhealthy
- Anomalies: <count> (CRITICAL: <n>, WARN: <n>)

### Performance
- Tool calls per turn: <avg>
- Tool error rate: <percentage>
- LLM calls: <count>
- Context compaction events: <count>

### Remaining Issues
- <issue if any>

### Recommendations
- <suggestion for future improvements>
```

## Tips

1. **Start with a smoke test** — If `aletheon_ask(question="List top-level directories")` fails, don't bother with complex tasks.
2. **Watch, don't poll** — Use `aletheon_watch` for real-time events rather than polling `aletheon_health` repeatedly.
3. **Analyze before fixing** — Always run `aletheon_analyze` before jumping to code changes. The anomaly rules catch common patterns.
4. **Compare baselines** — Record snapshot before and after each fix. A fix that improves one thing can break another.
5. **One iteration per turn** — Don't try to do 3 fix-verify cycles in one turn. The daemon state changes between turns; re-check health each time.
6. **Escalate early** — If the daemon won't start or the same issue persists, tell the user rather than burning all 5 iterations on a dead end.
