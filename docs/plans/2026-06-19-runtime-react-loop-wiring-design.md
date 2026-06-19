# Runtime ReAct Loop Wiring + Approval Gate — Design (Phase 1)

**Date:** 2026-06-19
**Status:** Draft — Pending User Review
**Branch:** auro/feat/20260619-runtime-react-loop-wiring
**Relationship to the product blueprint:** This is **Phase 1** of the broader CLI-agent
product vision captured in the mirror repo's `2026-06-19-cli-agent-design.md` (TUI,
multi-provider streaming, context compaction, MCP, session persistence, skills). That
blueprint is retained as Phases 2–4+. This document **redefines Phase 1**: the mirror's
original Phase 1 targeted "can chat" (a standalone `run` that returns text via
`llm.complete`), which — verified by running the system — does **not** make the agent
usable for real development (it can't act). Phase 1 here instead targets "can act,
safely": wire the interleaved ReAct tool loop into the runtime + add a human approval
gate, so `create hello.txt` actually creates the file.

**Two reconciliations with the blueprint:**
1. **No 4th path.** The blueprint's standalone `run` would add a 4th cognitive loop on
   top of the 3 that already don't connect. Instead we *fill* the existing
   `AletheonRuntime`/`ReActLoop` shell and wire it to the entry point — removing the
   "built-but-unwired" root cause rather than adding to it.
2. **Reuse existing LLM types.** The blueprint's Phase 1 Task 5 adds
   `ChatRequest`/`LlmEvent` to `aletheon-abi`. brain already has
   `LlmResponse`/`StreamChunk`/`ToolDefinition`; Phase 1 reuses those to avoid a
   duplicate type layer and conversion glue. New abi types are deferred to the TUI phase
   if still needed.

---

## 1. Problem Statement (verified by running the system)

Aletheon2 builds cleanly and passes 1234 tests, and the README marks Phases 1–4
"Done" — yet the agent runtime **cannot complete a single agentic task**. This was
confirmed by running it on 2026-06-19, not inferred:

- Asked the live daemon (via `aletheon -m "create hello.txt containing 'hi'"`) to
  create a file. It connected, the LLM (`mimo` provider) responded fluently — but
  emitted `echo 'hi' > hello.txt` **as text inside an `<antArtifact>` block**. The
  file was never created (`ls: cannot access 'hello.txt'`).

### Root cause: the refactor is frozen mid-flight

There are **three cognitive-loop implementations, and none is reachable from a user
message**:

| Implementation | Loops + calls tools? | Reachable from user input? |
|---|---|---|
| `Engine::run_turn` (`impl/engine/cognitive_loop.rs`) | ✅ real loop, tools, security guard | ❌ only caller is `AgentProcess::run`; daemon never spawns it for chat |
| `AletheonRuntime::process` + `ReActLoop` (`core/`) | ❌ `process()` has zero non-test callers; `ReActLoop` is a bare iteration counter (no LLM, no tools) | ❌ never called in production |
| daemon `chat` handler (`handler.rs:384`) | ❌ single `llm.complete(&messages, &[])` — empty tools, no loop | ✅ **this is the only path a user reaches** |

The handler's own comment admits it: *"A full migration of the daemon to the new
intent/plan/execute pipeline is tracked separately."* Someone began extracting
`Engine::run_turn` into the layered `AletheonRuntime`/`ReActLoop`, disconnected the
old `Engine` from the daemon, stubbed the new runtime, and shipped the daemon `chat`
path as a placeholder raw LLM call. **The execution spine is severed.**

### Secondary verified issues

- **`aletheon-exec` can't authenticate.** It passes tools (`main.rs:171`) but never
  loads `~/.aletheon/.env` (the daemon does, via `load_dotenv` at `daemon/mod.rs:89`),
  so it 401s before running any tool. Net: daemon = auth+LLM but no tools; exec =
  tools but no auth. Neither alone completes a task.
- **No interactive approval flow anywhere.** `RequireApproval` is silently converted
  to `Deny` (`security/runner.rs:94`). The one safety primitive that makes
  Codex/Claude Code trustworthy — show the risky action, wait for y/n — does not exist.
- **`ToolRunnerWithGuard` is dead code in the daemon** — constructed then dropped as
  `_tool_runner` (`handler.rs:131`).
- daemon default socket `/run/aletheond/aletheond.sock` (needs root) vs CLI default
  `/tmp/aletheon/aletheon.sock` — they disagree.
- bundled skill `~/.aletheon/skills/hello` lacks `---` frontmatter → warns every boot.

---

## 2. Goal

Deliver one **usable AND trustworthy** end-to-end loop, framed as a runtime concern
(this is an agent runtime, not a CLI tool): a user instruction flows through the
layered `AletheonRuntime`, the agent actually uses tools to act, and every risky
action is shown to the user for approval before it happens.

Decision (user-approved): **finish the new `AletheonRuntime` architecture** rather
than revive the old `Engine` god-object. The old `Engine` becomes deletable after the
new path is verified (deletion is a separate later PR).

---

## 3. Core Architecture Decision: plan-then-execute → interleaved ReAct

The current `AletheonRuntime::process()` uses a **plan-then-execute** model:
`think_fn` returns a whole `Plan` (a fixed list of `PlanStep`s decided up front),
then the runtime executes those steps in order.

That is **not how tool-calling LLM agents work.** Real agents (Codex, Claude Code, and
the existing `Engine::run_turn`) use an **interleaved ReAct loop**: the LLM decides the
*next* action only after *seeing* each tool result. A pre-baked `Plan` cannot do this.
This mismatch is almost certainly why the refactor froze.

**Resolution (user-approved):** change `AletheonRuntime`'s execution model to
interleaved ReAct.

- `ReActLoop` is upgraded from a counter to a **real loop**: it holds `messages` and
  `tools`, and each iteration calls the LLM, parses `ContentBlock::ToolUse`, executes
  each tool (with security guard + approval), pushes `ToolResult` back into `messages`,
  and repeats until the LLM emits no tool calls or `max_iterations` is reached.
- The `think_fn`/`execute_fn` closure interface is **kept but re-scoped**: `execute_fn(&Action, &Context) -> ActionResult`
  executes a single tool (existing signature — reused); the LLM call moves into
  `ReActLoop`. `Plan`/`PlanStep` types are retained for an explicit "Plan mode" but the
  default agentic path is interleaved.
- Tool-execution logic is **lifted from the verified `Engine::run_turn`** (the
  `ToolRunnerWithGuard` path, loop detector, hook firing) — not rewritten.
- `SelfField.review` still runs **once outside** the loop (intent gate, preserving the
  layered vision); per-tool approval runs **inside** the loop before each risky
  execution. The Reflex path keeps its current direct-execution behavior.

---

## 4. Components & Changes

`★` = core change.

### `crates/aletheon-runtime/src/core/react_loop.rs` ★ (counter → real loop)
- Add fields: `messages: Vec<Message>`; accept an LLM provider reference and a
  tool-executor (passed into `run`, not stored, to keep the struct simple).
- Add `async fn run(&mut self, llm, tool_defs, execute_fn) -> Result<String>`:
  interleaved loop — call LLM with `tool_defs` → parse tool_use → for each, call
  `execute_fn` (which applies guard + approval) → push `ToolResult` into `messages` →
  repeat until no tool_use or `max_iterations`.
- Keep existing counter methods (`should_continue`/`advance`/`iteration`) for internal
  bookkeeping.

### `crates/aletheon-runtime/src/core/orchestrator.rs` ★ (process re-scoped)
- `process()` becomes: `SelfField.review` (one intent gate) → select behavior path →
  Cognitive/Volitional → `react_loop.run(...)` → return result. Reflex path unchanged
  (direct execution, no loop).
- `execute_fn(&Action, &Context) -> ActionResult` reused as the single-tool executor;
  LLM call relocated into `ReActLoop`.

### `crates/aletheon-runtime/src/impl/daemon/handler.rs` ★ (wire the entry point)
- `chat` branch: replace `self.llm.complete(&messages, &[])` (line 500) with a call to
  `state.runtime.process(...)`, injecting the real `execute_fn` (backed by the
  now-actually-used `tool_runner`, line 131) and the tool definitions.
- Preserve existing SelfField review / PreTurn / PostTurn hooks / memory injection /
  cache-stable prefix logic — they are correct; only the final step called the wrong thing.

### `crates/aletheon-body/src/impl/security/runner.rs` ★ (approval gate)
- Introduce `ApprovalGate` trait (see §5). `RequireApproval` no longer auto-converts to
  `Deny` (line 94); instead it calls the gate for a decision. Triggered for **L2+**
  tools before execution.
- Fail-safe: gate timeout / closed channel → default `Deny`.

### `crates/binaries/aletheon-exec/src/main.rs` ★ (fix auth + reuse engine)
- Add `load_dotenv(~/.aletheon/.env)` (reuse the daemon's `load_dotenv`, promoted to a
  shared location).
- Replace its hand-rolled loop with a call to `AletheonRuntime`, injecting a
  `TerminalApprovalGate`.

### Cleanup (low risk)
- Unify daemon default socket with the CLI default (`/tmp/aletheon/aletheon.sock`, or
  an XDG runtime dir for both).
- Fix the bundled `~/.aletheon/skills/hello` frontmatter, or downgrade the parse log.
- Old `Engine`: **not deleted this PR**; removed in a separate PR once the new path is
  verified stable.

---

## 5. Approval Gate Design

```rust
#[async_trait]
pub trait ApprovalGate: Send + Sync {
    async fn request(&self, req: &ApprovalRequest) -> ApprovalDecision;
}

pub struct ApprovalRequest {
    pub tool: String,
    pub action_summary: String,   // e.g. "bash: echo 'hi' > hello.txt"
    pub risk_level: RiskLevel,
    pub detail: Option<String>,   // diff or full command
}

pub enum ApprovalDecision { Approve, Deny, ApproveForSession }
```

- **Permission-tiered trigger.** Approval fires only for **L2+** tools (delete files,
  modify system config, sudo). L0 (read, status) / L1 (ordinary write, install) run
  directly or sandboxed — matching Codex/Claude Code's "reads free, writes ask."
- **UI-decoupled injection.** exec path injects `TerminalApprovalGate` (terminal stdin
  y/n); daemon path injects `SocketApprovalGate` (Phase 2); tests inject
  `AutoApproveGate` / `AutoDenyGate`.
- **Approve-for-session.** `ApproveForSession` lets the same tool class skip repeated
  prompts within a session, reducing interruption.
- **Fail-safe.** Callback timeout / broken channel → `Deny`. Never default to execute.

---

## 6. End-to-End Data Flow (the verified-failing task, fixed)

```
User: aletheon -m "create hello.txt containing 'hi'"
  │
  ▼  CLI → daemon socket (unified path)
daemon handler "chat":
  1. SelfField.review(intent)              ← outer intent gate (kept, correct)
  2. PreTurn hooks                          ← kept
  3. build messages + cache-stable prefix   ← kept
  4. state.runtime.process(input, ctx, execute_fn, tool_defs)  ★ (was llm.complete(&[]))
       │
       ▼  AletheonRuntime → ReActLoop.run()  ★ new loop
       ┌─────────────────────────────────────────────┐
       │ loop (iteration < max_iterations):            │
       │  a. llm.complete(messages, TOOL_DEFS)         │ ← tools passed this time
       │  b. no tool_use? → return text, done          │
       │  c. tool_use (bash: echo 'hi' > hello.txt):   │
       │       execute_fn(action, ctx)                 │
       │         ▼  ToolRunnerWithGuard ★ actually used │
       │         1. PolicyEngine.check → tier           │
       │         2. RequireApproval (L2+)? → ApprovalGate│
       │            n → "denied" pushed back; y → proceed│
       │         3. loop detector                       │
       │         4. (L1+) sandbox / direct execute      │
       │         5. audit log                           │
       │       → ToolResult pushed into messages        │
       │  d. PostTool hook → continue loop              │
       └─────────────────────────────────────────────┘
  5. PostTurn hooks + memory persist + reflection ← kept
  6. return final text to CLI
  │
  ▼
File actually created ✓ (not echo-as-text in an artifact)
```

---

## 7. Phasing (reconciled with the product blueprint)

- **Phase 1 (this PR) — "can act, safely":** interleaved `ReActLoop`; `process()`
  re-scoped; daemon `chat` wired to `process()` with real tools; `ApprovalGate`
  abstraction + `TerminalApprovalGate`; `aletheon-exec` fixed (auth via `.env` + engine
  reuse) → end-to-end usable+trustworthy loop on the exec path. daemon path regains tool
  capability with a conservative gate (L2+ default Deny via the new abstraction — no
  safety regression). Plus low-risk cleanup (socket unify, skill frontmatter).
- **Phase 2 (blueprint TUI + permissions):** TUI work **inside `body/impl/ui/`** (NOT a
  separate `aletheon-tui` crate — see the architecture decision in §9),
  `SocketApprovalGate` cross-process approval + TUI approval dialog, the
  `PermissionMode`/`PermissionRule` system from blueprint §5.
- **Phase 3 (blueprint context + tools):** multi-layer compaction, tool-output
  truncation/streaming, new tools (`web_fetch`/`glob`/`grep`/`task_*`), sandbox profiles.
- **Phase 4 (blueprint MCP + session + skills):** MCP discovery/invoke, session
  persistence, hooks/skills enhancement.
- **Cross-cutting:** delete the orphaned `Engine` once the new path is verified (after
  Phase 1 lands and is stable).

---

## 8. Verification

- **Regression:** full `cargo test --workspace` stays green (currently 1234).
- **The defining manual test:** rebuild, run `aletheon-exec --prompt "create hello.txt
  containing 'hi'"`, confirm the file **is actually created** (the exact scenario that
  failed on 2026-06-19), and confirm a `rm`/L2 action triggers a y/n prompt and that
  `n` aborts it.
- **Unit:** `ReActLoop::run` with a mock LLM that emits one tool call then stops →
  asserts the tool executed and the result fed back. `ApprovalGate` with
  `AutoDenyGate` → asserts L2 action is blocked and reported, not executed.
- **No new clippy warnings**; `cargo fmt` clean.

---

## 9. Resolved Decisions & Open Questions

**Resolved (user-approved):**
- Engine choice: finish `AletheonRuntime` (new architecture), not revive `Engine`.
- Execution model: interleaved ReAct, not plan-then-execute.
- Phasing: Phase 1 = "can act, safely" on exec path; daemon cross-process approval +
  TUI deferred to blueprint Phase 2.
- `ApproveForSession` scope: per tool-name within the session (the blueprint's richer
  `PermissionRule` arg-pattern matching arrives with the Phase 2 permission system).
- `SandboxFirst` verdict: Phase 1 keeps current behavior (inject a note only). Forcing
  every tool in the turn into sandbox mode is deferred to the Phase 3 sandbox-profile work.

**Architecture decision (user-approved): keep CLI/TUI in `body` as its interface layer.**
`body` (BodyRuntime) is treated as the unified *interface* layer — both the
agent-to-computer interface (`acix`, `tools`, `sandbox`, `security`, `driver`, `mcp`)
and the agent-to-human interface (`cli`, `ui`/TUI) live there. We do **not** extract
TUI/CLI into separate crates; the blueprint's `aletheon-tui` crate is **superseded** —
the TUI work in later phases happens inside `body/impl/ui/`. This preserves the current
architecture and avoids a large dependency refactor. Phase 1 does not move any of these
modules; it only adds the approval gate inside `body/impl/security/runner.rs`.

**Still open (non-blocking for Phase 1):**
- Whether `aletheon-exec` keeps its own thin binary or becomes a flag on the unified CLI
  — decided during implementation based on the body→brain dependency check (blueprint
  Phase 1 Task 7's BLOCKER note applies here too: keep runtime-driving logic in a binary
  crate to avoid a body→brain cycle).
