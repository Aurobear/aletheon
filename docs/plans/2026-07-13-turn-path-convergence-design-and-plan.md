# Turn-Path Convergence (daemon ↔ exec) — Design + Staged Plan

> **For agentic workers (DeepSeek/Claude):** This is the largest, highest-risk
> item (design `Final(2).md` P0 #1). Execute in the three stages below, each its
> own PR, each independently green. Do NOT attempt all at once. Stop at any stage
> boundary if behavior can't be preserved.

**Goal:** Collapse the two production turn-orchestration paths into one, so daemon,
TUI, and `exec` share identical security, memory, Agora, and event semantics, and
there is exactly one place that constructs the cognitive loop.

---

## 1. Verified current state (the problem)

Two orchestration paths wrap the SAME cognitive engine (`ReActLoop` /
`LinearCognitiveSession`) with DIFFERENT surrounding logic:

| | Daemon path | Exec path |
|---|---|---|
| Entry | `DaemonTurnOrchestrator::execute_turn` (`crates/executive/src/service/daemon_turn/execute.rs`) | `bin/src/main.rs::run_exec` → `ExecSessionBuilder` |
| Orchestration | `submit_streaming_daemon_turn` (`daemon_turn/daemon_react.rs:52,67`) → `ReActLoop` **directly** | `TurnService` → `CognitiveSession` (`crates/executive/src/service/exec_session.rs`) |
| Event sink | streaming `ChannelEventSink` | `NoopTurnEventSink` |
| Agora | full `AgoraOps` (propose/commit) | **empty `AgoraView`** (`exec_session.rs:179`) — degraded |
| Approval | kernel `AdmissionController` + admit/settle | `TerminalApprovalGate` |

So the doc claim "both converge on `TurnService`" is FALSE — only exec uses
`TurnService`; daemon bypasses it. The shared piece is `ReActLoop`, not the
orchestration. Fixing a security/memory/agora rule in one path does not fix the
other.

**Target (design §6.1):** one pipeline —
`Input → Session → submit Operation → Admission → CognitiveSession(Harness) → Capability → Result → Agora/Mnemosyne/Events → structured exit` — where daemon vs exec differ ONLY by the injected `TurnEventSink` and which optional services are present.

---

## 2. Key types already present (reuse, don't reinvent)

- `CognitiveSession` trait — `crates/cognit/src/harness/session.rs:35`. `LinearCognitiveSession` wraps `ReActLoop`.
- `TurnServices` trait (fabric) — capability/recall/dasein/agora views passed to the session.
- `TurnEventSink` — `ChannelEventSink` (streaming) and `NoopTurnEventSink` (silent).
- `TurnService` + `ExecSessionBuilder` — `crates/executive/src/service/` (the exec path already routes through these).
- Daemon Pre/Cognit/Post phases — `daemon_turn/{injection.rs, self_field.rs, execute.rs, post_phases.rs}` (the rich logic that must become shared, not daemon-only).

---

## 3. Staged plan

### Stage 1 — Close the exec semantic gap (LOWER risk, do first)

Make the exec path stop being a degraded twin, so convergence later is a
merge of equals rather than an upgrade.

- [ ] Wire a real Agora into the exec session: `exec_session.rs:179` currently
  returns an empty `AgoraView`. Construct/inject an `AgoraRegistry` (same as the
  daemon does) so exec `agora_view` reflects real state. If exec should stay
  single-user with no shared workspace, make that an explicit documented policy
  rather than an empty stub.
- [ ] Align approval: decide whether exec keeps `TerminalApprovalGate` (interactive)
  but STILL routes side-effects through the kernel `AdmissionController` (as the
  daemon does at `execute.rs:394` admit / `403` settle). Exec already calls
  `admit`/`settle` (`exec_session.rs:199,228,256`) — verify parity of the
  `AdmissionRequest` construction with the daemon's.
- [ ] Verify: an `exec` run and a daemon turn on the same input produce the same
  admission decisions and (if agora enabled) the same Agora commits. Add an
  integration test comparing the two on a scripted input.
- [ ] Commit: `fix(executive): close exec-path agora/admission gap vs daemon`.

### Stage 2 — Extract a shared TurnPipeline (MEDIUM risk)

Move the daemon's Pre/Cognit/Post orchestration out of `DaemonTurnOrchestrator`
into a reusable pipeline that both paths call.

- [ ] Define `TurnPipeline` (new module under `crates/executive/src/service/`)
  with a single entry, e.g.:
  ```rust
  pub struct TurnPipeline { /* services: admission, agora(opt), memory, hooks, self_field, harness_factory */ }
  impl TurnPipeline {
      pub async fn run(&self, req: TurnRequest, events: &dyn TurnEventSink) -> TurnResult;
  }
  ```
  It performs: pre-turn injection (skills/memory/self_field/hooks) → build
  `CognitiveSession` via a `HarnessFactory` → run the session with a
  `TurnServices` impl → post-turn (agora commit, mnemosyne record, reflection,
  hooks). This is the daemon's current `execute.rs` body, generalized.
- [ ] Reduce `DaemonTurnOrchestrator::execute_turn` to: parse request → resolve
  session/process (`ensure_main_agent`) → submit operation → `pipeline.run(req, &ChannelEventSink)` → format JSON-RPC response. (Matches design §6.3: the
  handler only parses/routes/forwards.)
- [ ] Keep the space-lifecycle behavior from Phase 2a/2b (reuse `process.space`,
  upsert bindings) inside the pipeline's pre/post steps.
- [ ] Verify: daemon behavior byte-for-byte unchanged (same events, same
  responses) — snapshot a few turns before/after. All `executive` tests green.
- [ ] Commit: `refactor(executive): extract shared TurnPipeline from daemon orchestrator`.

### Stage 3 — Route exec through the same TurnPipeline (MEDIUM risk)

- [ ] Make `ExecSessionBuilder`/`run_exec` construct the SAME `TurnPipeline` with
  a `NoopTurnEventSink` (or a simple stdout sink) and the exec-appropriate
  service set (from Stage 1). Delete the exec-only `TurnService` bespoke loop if
  it now duplicates the pipeline; if `TurnService` becomes the pipeline's thin
  wrapper, keep one.
- [ ] Ensure `ReActLoop` is constructed in exactly ONE place — inside the
  `HarnessFactory` used by the pipeline. Grep `ReActLoop::` / `LinearCognitiveSession::new`
  after: only the factory should build it.
- [ ] Delete the dead `AletheonExecutive::process`/`process_react` duplicate
  cognitive path if still present (design §20 Phase 0 item 4).
- [ ] Verify (acceptance, design §20 Phase 0): daemon + TUI + exec on the same
  input use the same security, tool, memory, and event semantics; exactly one
  production cognitive loop; the chat handler creates no `ReActLoop`.
- [ ] Commit: `refactor: route exec through shared TurnPipeline — single turn path`.

---

## 4. Risks & stop rules

- **Highest-risk area in the codebase** — daemon turn behavior is user-facing.
  Each stage must preserve daemon output exactly; use before/after transcript
  snapshots as the gate.
- `AletheonExecutive` is ALIVE (hosts the sub-agent spawner via `subsystems.runtime`,
  `init.rs:362,538`) — do not delete it wholesale; only remove its duplicate
  cognitive loop if one exists, and only after confirming the sub-agent path
  (now shared-table per Phase 2c) still works.
- If Stage 2's extraction can't preserve daemon behavior within a bounded diff,
  STOP after Stage 1 (which already removes the worst divergence) and report.
- Keep each stage independently shippable; do not merge a half-converged state
  that leaves two loops both live but subtly different.

---

## 5. Acceptance (whole item)

- One production construction site for `ReActLoop`/`LinearCognitiveSession`.
- daemon/exec identical on: admission decisions, memory record/recall, agora
  commits (when enabled), and event semantics (modulo sink).
- `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` §4 P0 #1 updated to
  ✅ with evidence.
