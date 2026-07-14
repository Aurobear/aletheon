# Agent-Google Integration — Implementation Plan

> **For agentic workers:** Use `/workflow feature` to implement this plan phase-by-phase. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the 8-phase Aletheon agent-Google integration: Telegram channel, Goal Runtime, DeepSeek/Pi workers, verification pipeline, Google OAuth+sync, Gmail channel+approval, and GBrain backend.

**Architecture:** Integrate into existing crates (executive, corpus, mnemosyne, fabric). Extend existing `SubAgentRuntime`/`SubAgentSpawner`/`SupervisorTree` rather than building parallel worker/supervisor/store abstractions. Channel abstraction routes external messages through existing `DaemonTurnOrchestrator`. Google OAuth uses existing `McpServerConfig` infrastructure. Verification runs as `PostTurnHook`. Approval extends existing `SessionGateway::approval_flow`.

**Tech Stack:** Rust (tokio async), rusqlite (SQLite), reqwest (HTTP), teloxide (Telegram), Docker Compose (GBrain+PostgreSQL).

**Spec:** `docs/plans/2026-07-14-agent-google-design.md`

---

## 1. Phase Dependency Graph

```
A ──→ B ──→ C ──→ D ──→ E
              │
              └──→ F ──→ G
                        │
              └──→ H ──┘
```

| Phase | Name | Depends On | Crates |
|-------|------|-----------|--------|
| A | Channel Core + Telegram | none | fabric, executive |
| B | Goal Runtime v1 | A | fabric, executive |
| C | DeepSeek Runtime | B | executive |
| D | Pi Coding Subagent | B | executive |
| E | Verification Pipeline | D | executive (service/) |
| F | Google OAuth + Sync | B | corpus |
| G | Gmail Channel + Approval | A, F | executive |
| H | GBrain Backend | B | mnemosyne |

---

## 2. Relationship to Existing Infrastructure

Every new concept maps to an existing component. No parallel shadow systems.

| New Concept | Existing Component | File |
|---|---|---|
| Channel trait | New; routes into `DaemonTurnOrchestrator` | `crates/executive/src/service/daemon_turn/orchestrator.rs` |
| ChannelRegistry | New; registered during daemon init | `crates/executive/src/impl/daemon/handler/init.rs` |
| GoalSpec (type only) | New type in `fabric/src/types/goal.rs` (no GoalStore/GoalSupervisor) | — |
| AgentState extensions | Extend `ProcessState`/`AgentProfile` in fabric | `crates/fabric/src/lib.rs` |
| DeepSeekRuntime | Implements `SubAgentRuntime` trait | `crates/executive/src/core/sub_agent.rs:47` |
| PiRuntime | Implements `SubAgentRuntime` trait | `crates/executive/src/core/sub_agent.rs:47` |
| Goal retry | `SupervisorTree` restart policies | `aletheon_kernel::supervision::RestartPolicy` |
| Goal escalation | Supervisor policy with different runtime dispatch | `aletheon_kernel::supervision::SupervisorTree` |
| Verification pipeline | `PostTurnHook` registered in hook system | `crates/executive/src/service/post_turn.rs` |
| Google OAuth | `McpServerConfig` (add OAuth fields) | `crates/corpus/src/tools/mcp/config.rs` |
| Gmail/Calendar MCP tools | MCP tool servers configured via `McpServerConfig` | — |
| GoogleSyncManager | New; polls Gmail+Calendar, converts to `InboundMessage` | — |
| ApprovalRequest types | Extend `SessionGateway::approval_flow` | `crates/executive/src/core/session_gateway/approval_flow.rs` |
| GBrain backend | New memory backend in mnemosyne | `crates/mnemosyne/src/impl/backends/` |

---

## 3. Phase A: Channel Core + Telegram

**Estimated:** 1-2 weeks | **Crates:** fabric, executive

### Task A.1: Add channel types to fabric

Create `crates/fabric/src/types/channel.rs` with these types:

- `ChannelId(String)`, `MessageId(String)`, `ConversationId(String)` — newtype wrappers for channel-scoped identifiers.
- `MessageContent` — enum with variants: `Text { text }`, `Command { command, args }`, `File { name, mime_type, data_base64 }`, `Voice { transcription, audio_base64 }`. Serde tag `"type"` with `rename_all = "snake_case"`.
- `UserAction` — struct: `action_id`, `action_type` (ActionType enum: Approve, Reject, ViewDiff, RequestRevision, Custom), `label`, `payload`.
- `InboundMessage` — struct with fields: `channel_id: ChannelId`, `message_id: MessageId`, `conversation_id: Option<ConversationId>`, `sender_id: String`, `content: MessageContent`, `timestamp: String`, `reply_to_action: Option<String>`. **These field names are canonical across all phases — Phase G must use these exact names, not `id`, `channel`, `principal`, `conversation`, or `received_at`.**
- `OutboundMessage` — struct: `channel_id: ChannelId`, `conversation_id: Option<ConversationId>`, `content: MessageContent`, `actions: Vec<UserAction>`.
- Include `#[cfg(test)]` module with serde round-trip tests for each `MessageContent` variant and an `OutboundMessage` with actions serialization test.

Wire into fabric: add `pub mod channel;` to `crates/fabric/src/types/mod.rs`, add `pub use types::channel;` to `crates/fabric/src/lib.rs`.

- [ ] Create `crates/fabric/src/types/channel.rs`
- [ ] Modify `crates/fabric/src/types/mod.rs` — add `pub mod channel;`
- [ ] Modify `crates/fabric/src/lib.rs` — add `pub use types::channel;`
- [ ] Commit

### Task A.2: Define Channel trait and ChannelRegistry

Create `crates/executive/src/impl/channel/mod.rs`:

Define `Channel` trait with async methods:
```rust
#[async_trait]
pub trait Channel: Send + Sync {
    fn channel_id(&self) -> &ChannelId;
    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()>;
    async fn send(&self, msg: OutboundMessage) -> Result<()>;
    async fn shutdown(&self) -> Result<()>;
}
```

Define `ChannelRegistry` — owns `HashMap<ChannelId, Arc<dyn Channel>>` and an `mpsc::Receiver<InboundMessage>`. Methods: `register()`, `send_to()`, `take_receiver()`, `shutdown_all()`. The registry is held by the daemon; incoming messages flow through the receiver channel.

Include tests: two mock channels registered, message sent to one, message from the other received through the stream.

Add `pub mod channel;` to `crates/executive/src/impl/mod.rs`.

- [ ] Create `crates/executive/src/impl/channel/mod.rs`
- [ ] Modify `crates/executive/src/impl/mod.rs` — add `pub mod channel;`
- [ ] Commit

### Task A.3: Implement TelegramChannel

Create directory `crates/executive/src/impl/channel/telegram/` with these files:

- `mod.rs` — `TelegramChannel` struct (holds `teloxide::Bot`, `chat_ids: Mutex<Vec<i64>>`), implements `Channel` trait. On `start()`, spawns polling task. On `send()`, formats and dispatches `OutboundMessage` to each registered chat.
- `polling.rs` — long-polling loop using `teloxide` `GetUpdates`; parses Telegram `Update` into `InboundMessage` (maps text to `MessageContent::Text`, `/command` to `MessageContent::Command`, attachments to `MessageContent::File`). Sends through the `mpsc::Sender` provided at `start()`.
- `binding.rs` — `TelegramBinding` struct and `BindingState` enum (Active, Paused) for managing one Telegram bot instance per daemon. Handles bot token from environment/config.
- `formatting.rs` — `format_outbound()` converts `OutboundMessage` to Telegram `SendMessage` with inline keyboard markup for `actions`. Includes tests for action keyboard generation and markdown escaping.

Add `teloxide = "0.13"` to `crates/executive/Cargo.toml`.

- [ ] Create `crates/executive/src/impl/channel/telegram/` directory with mod.rs, polling.rs, binding.rs, formatting.rs
- [ ] Modify `crates/executive/Cargo.toml` — add teloxide dependency
- [ ] Commit

### Task A.4: Integrate ChannelRouter into daemon initialization

Modify `crates/executive/src/impl/daemon/handler/init.rs`:

- Add `channel_registry: Arc<ChannelRegistry>` field to the handler context struct.
- During daemon startup, construct `ChannelRegistry`, register `TelegramChannel`, call `registry.take_receiver()`, and spawn a background task that reads from the receiver and dispatches each `InboundMessage` into the turn orchestrator.
- Implement `process_channel_message(msg: InboundMessage)` on the handler — converts channel message into an agent turn, preserving `channel_id`, `sender_id`, `conversation_id`, and `reply_to_action` for context routing.

Modify `crates/executive/src/impl/daemon/server.rs` — add `channel_registry` to the `RequestHandler` struct initialization.

- [ ] Modify `crates/executive/src/impl/daemon/handler/init.rs` — add channel registry + dispatch loop
- [ ] Modify `crates/executive/src/impl/daemon/server.rs` — wire channel_registry into RequestHandler
- [ ] Commit

### Task A.5: Compilation and tests

- [ ] `cargo test -p fabric -- types::channel` — channel type serde round-trips
- [ ] `cargo test -p executive -- impl::channel` — ChannelRegistry register/send/receive
- [ ] `cargo test -p executive -- impl::channel::telegram` — formatting + binding tests
- [ ] `cargo build --workspace` — no regressions
- [ ] Commit

**Phase A acceptance:** Channel types compile and serialize; Channel trait + registry pass tests; Telegram polling loop parses messages; daemon init wires registry and dispatches to turn orchestrator.

---

## 4. Phase B: Goal Runtime v1

**Estimated:** 2-3 weeks | **Depends on:** Phase A | **Crates:** fabric, executive

**Architecture:** Goals extend the existing AgentProcess model. There is no standalone `GoalSupervisor`, `GoalStore`, 10-state FSM, `objectives_v2` table, or `ALTER TABLE` migrations. Goal lifecycle is managed by the existing `ProcessState` / `SupervisorTree` machinery.

### Task B.1: Define GoalSpec type in fabric

Create `crates/fabric/src/types/goal.rs`:

Define `GoalSpec` — the canonical goal definition type:
```rust
pub struct GoalSpec {
    pub goal_id: String,
    pub description: String,
    pub criteria: Vec<String>,
    pub priority: GoalPriority,
    pub max_attempts: u32,
    pub max_cost_usd: Option<f64>,
    pub timeout_secs: u64,
    pub auto_approve: bool,
    pub tags: Vec<String>,
    pub parent_goal_id: Option<String>,
}
```

Define `GoalPriority` enum: Critical, High, Normal, Low. Define `GoalPhase` enum: Planning, Executing, Verifying, AwaitingApproval, AwaitingHuman, Completed, Failed, Cancelled.

Define `MemoryProjection` struct — result of recalling past experiences for a goal:
```rust
pub struct MemoryProjection {
    pub relevant_facts: Vec<String>,
    pub past_experiences: Vec<String>,
    pub summary: String,
    pub memory_type: String,
    pub provenance_goal: String,
    pub retrieved_at: String,
    pub freshness: f64,
}
```
**Phase H references these fields** — `summary`, `memory_type`, and `provenance_goal` must be present.

Extend `ProcessState` in fabric to include two new variants: `AwaitingApproval` and `AwaitingHuman`. These are used when a goal pauses for channel-based approval or user input.

Wire into fabric: add `pub mod goal;` to `crates/fabric/src/types/mod.rs`, add `pub use types::goal;` to `crates/fabric/src/lib.rs`.

- [ ] Create `crates/fabric/src/types/goal.rs` with GoalSpec, GoalPriority, GoalPhase, MemoryProjection
- [ ] Extend `ProcessState` with `AwaitingApproval` and `AwaitingHuman` variants (in fabric lib.rs or types)
- [ ] Modify `crates/fabric/src/types/mod.rs` — add `pub mod goal;`
- [ ] Modify `crates/fabric/src/lib.rs` — add `pub use types::goal;`
- [ ] Commit

### Task B.2: Goal prompt assembly in SubAgentRuntime

Create `crates/executive/src/impl/goal/mod.rs` with:
- `goal_prompt.rs` — assembles agent prompts from `GoalSpec` + channel context + `MemoryProjection`. Produces system prompt text and initial task description. Does NOT persist goals — that is handled by Mnemosyne recall.
- `goal_routing.rs` — determines which `SubAgentRuntime` to dispatch based on goal type (coding goals route to PiRuntime; research/reasoning to DeepSeekRuntime). Reads from `GoalSpec.tags`.

Goals are ephemeral — they live as `AgentProcess` entries in the kernel ProcessTable. Persistence is via Mnemosyne journal (last N session entries), not a dedicated store.

- [ ] Create `crates/executive/src/impl/goal/mod.rs` with goal_prompt and goal_routing submodules
- [ ] Modify `crates/executive/src/impl/mod.rs` — add `pub mod goal;`
- [ ] Commit

### Task B.3: Wire goal spawning into daemon handler

Modify `crates/executive/src/impl/daemon/handler/init.rs`:

- When `MessageContent::Command { command: "goal", args }` is received, parse args into a `GoalSpec` (description from args), create an `AgentProcess` via `SubAgentSpawner`, and set the process state to `Planning`.
- When a goal reaches a gate requiring approval, transition to `AwaitingApproval` and send an `OutboundMessage` with `ActionType::Approve`/`ActionType::Reject` actions back through the originating channel.
- On `ActionType::Approve` reply (tracked via `reply_to_action`), transition state and resume execution.

Modify `crates/executive/src/impl/daemon/server.rs` — ensure goal-related state transitions are visible through existing debug/session handlers.

- [ ] Modify `crates/executive/src/impl/daemon/handler/init.rs` — handle "goal" command, spawn via SubAgentSpawner
- [ ] Modify `crates/executive/src/impl/daemon/server.rs` — expose goal state in handlers
- [ ] Commit

### Task B.4: Compilation and tests

- [ ] `cargo test -p fabric -- types::goal` — GoalSpec serde, MemoryProjection field presence
- [ ] `cargo test -p executive -- impl::goal` — prompt assembly, routing dispatch
- [ ] `cargo build --workspace` — no regressions
- [ ] Commit

**Phase B acceptance:** GoalSpec type compiles; MemoryProjection includes all 7 fields; ProcessState has AwaitingApproval + AwaitingHuman; `/goal` command via Telegram spawns an AgentProcess; approval actions flow through the channel.

---

## 5. Phase C: DeepSeek Runtime

**Estimated:** 1-2 weeks | **Depends on:** Phase B | **Crates:** executive

**Architecture:** DeepSeekRuntime implements `SubAgentRuntime` trait at `crates/executive/src/core/sub_agent.rs:47`. No `GoalWorker` trait, no `WorkerRegistry`, no standalone `RetryPolicy` module, no standalone `Escalation` module. Retry is handled by `SupervisorTree` restart policies. Escalation is a supervisor policy that spawns a replacement sub-agent with a different runtime (e.g., Claude) when DeepSeek fails repeatedly.

### Task C.1: Implement DeepSeekRuntime

Create `crates/executive/src/impl/runtime/deepseek.rs`:

`DeepSeekRuntime` struct holds:

- An `Arc<dyn LlmProvider>` — the DeepSeek API provider (setup via existing provider config).
- Optional goal context: `Option<GoalSpec>` for goal-aware prompt construction.
- `cancel: CancellationToken` for cooperative cancellation.
- `attempt_count: AtomicU32` for tracking within a single runtime session.

Implements `SubAgentRuntime::run(&self, task: &str, cancel: CancellationToken) -> Result<String, String>`:

1. Constructs messages from goal context + task description.
2. Calls `provider.complete(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse>` **(not `chat()` — see `crates/fabric/src/types/llm_types.rs:61`)**.
3. Handles tool calls by dispatching to existing tool registry.
4. Loops until stop reason or exhaustion.
5. Returns final response text on success, error string on failure.

- [ ] Create `crates/executive/src/impl/runtime/deepseek.rs`
- [ ] Create `crates/executive/src/impl/runtime/mod.rs` — `pub mod deepseek;`
- [ ] Modify `crates/executive/src/impl/mod.rs` — add `pub mod runtime;`
- [ ] Commit

### Task C.2: Failure classification and retry via SupervisorTree

When `DeepSeekRuntime::run()` fails, the error propagates through `SubAgentSpawner`, which transitions the sub-agent's `ProcessState` to `Failed`. The `SupervisorTree` at `crates/executive/src/core/sub_agent.rs:25` consults its `RestartPolicy`:

- Classify failures into: transient (network timeout, rate limit — retry), permanent (auth error, invalid model — escalate), tool error (tool call failed — retry with reduced tool set), token budget exceeded (truncate context and retry).
- Transient: restart with backoff (exponential, max 3 retries).
- Permanent: escalate — spawn a replacement sub-agent with Claude runtime via `RuntimeTask` dispatch.
- Tool error: restart once with a reduced tool set (omit the failing tool).
- Token budget: restart once with truncated conversation history.

Implement `classify_failure(error: &str) -> FailureClass` in `deepseek.rs` as a private function. The `SupervisorTree` already supports `RestartDecision` — configure the policy accordingly.

- [ ] Implement `classify_failure()` in `crates/executive/src/impl/runtime/deepseek.rs`
- [ ] Configure `SupervisorTree` restart policies for DeepSeek sub-agents in handler init
- [ ] Commit

### Task C.3: Compilation and tests

- [ ] `cargo test -p executive -- impl::runtime::deepseek` — runtime construction, failure classification
- [ ] `cargo build --workspace` — no regressions
- [ ] Commit

**Phase C acceptance:** DeepSeekRuntime implements SubAgentRuntime; uses `provider.complete()` (not `chat()`); failures classified correctly; SupervisorTree restarts transient errors and escalates permanent errors.

---

## 6. Phase D: Pi Coding Subagent

**Estimated:** 1-2 weeks | **Depends on:** Phase B | **Crates:** executive

**Architecture:** PiRuntime implements `SubAgentRuntime` trait. No `PiWorker as GoalWorker`, no new `impl/agent/pi/` directory. PiRuntime is spawned by `SubAgentSpawner::with_runtime()` like any other runtime.

### Task D.1: Define PiSubagentTask and PiSubagentReport

Create `crates/executive/src/impl/runtime/pi_types.rs`:

- `PiSubagentTask` — struct: `task_id`, `description`, `changed_files: Vec<String>`, `diff: String`, `tests_run: usize`, `tests_passed: usize`, `linter_output: String`, `build_status: BuildStatus`.
- `PiSubagentReport` — struct: `task`, `verification_report: Option<VerificationReport>`, `commit_hash: Option<String>`, `duration_secs: f64`, `retries: u32`.
- `FileChange` — struct: `path: String`, `change_type: ChangeType` (Added, Modified, Deleted), `lines_added: u32`, `lines_removed: u32`.
- `BuildStatus` — enum: Success, Failed(String), NotAttempted.

These types are used by the verification pipeline (Phase E) to inspect Pi output.

- [ ] Create `crates/executive/src/impl/runtime/pi_types.rs`
- [ ] Modify `crates/executive/src/impl/runtime/mod.rs` — add `pub mod pi_types;`
- [ ] Commit

### Task D.2: Implement PiRuntime

Create `crates/executive/src/impl/runtime/pi.rs`:

`PiRuntime` struct holds:

- An `Arc<dyn LlmProvider>` — the coding LLM.
- `worktree_base: PathBuf` — root for git worktrees.
- `cancel: CancellationToken`.

Implements `SubAgentRuntime::run(&self, task: &str, cancel: CancellationToken) -> Result<String, String>`:

1. Creates a git worktree at `worktree_base/goal-{id}/` (using `git worktree add`).
2. Runs LLM coding loop via `provider.complete()` with file-editing tools.
3. After each tool call, collects `FileChange` entries.
4. On completion, produces a `PiSubagentReport` with file changes, diff, test results.
5. Cleans up worktree on success (keeps on failure for debugging).
6. Calls `provider.complete()` (not `chat()` — see B.1).

Include `WorktreeManager` helper — creates, lists, and prunes git worktrees. Uses `std::process::Command` to invoke `git worktree`.

- [ ] Create `crates/executive/src/impl/runtime/pi.rs`
- [ ] Modify `crates/executive/src/impl/runtime/mod.rs` — add `pub mod pi;`
- [ ] Commit

### Task D.3: Compilation and tests

- [ ] `cargo test -p executive -- impl::runtime::pi` — PiRuntime construction, worktree lifecycle
- [ ] `cargo test -p executive -- impl::runtime::pi_types` — task/report serde
- [ ] `cargo build --workspace` — no regressions
- [ ] Commit

**Phase D acceptance:** PiRuntime implements SubAgentRuntime; creates git worktrees; produces PiSubagentReport with FileChange list; cleans up on success; uses `provider.complete()`.

---

## 7. Phase E: Verification Pipeline

**Estimated:** 1 week | **Depends on:** Phase D | **Crates:** executive

**Architecture:** Verification runs as a `PostTurnHook` registered in the existing hook system at `crates/executive/src/service/post_turn.rs`. When PiRuntime completes a coding turn, the post-turn pipeline runs 7 verification gates on the output. No separate `impl/goal/verify/` directory — verification lives in `crates/executive/src/service/`.

### Task E.1: Define VerificationGate trait

Create `crates/executive/src/service/verify.rs`:

Define `VerificationContext` struct: `goal_id`, `attempt_id`, `worktree_path`, `changed_files: Vec<FileChange>`, `diff: String`, `worker_output: String`.

Define traits and types:
```rust
#[async_trait]
pub trait VerificationGate: Send + Sync {
    fn name(&self) -> &'static str;
    fn priority(&self) -> GatePriority;
    async fn check(&self, ctx: &VerificationContext) -> Result<GateResult>;
}
```

`GatePriority` enum: MustPass, Advisory. `GateResult` struct: `passed: bool`, `name: String`, `output: String`, `blocking: bool`. `VerificationReport` struct: `passed: bool`, `gates: Vec<GateResult>`, `summary: String`, `risks: Vec<String>`, `recommendation: VerificationAction`. `VerificationAction` enum: Accept, Revise, Reject.

- [ ] Create `crates/executive/src/service/verify.rs`
- [ ] Modify `crates/executive/src/service/mod.rs` — add `pub mod verify;`
- [ ] Commit

### Task E.2: Implement 7 standard gates

Add `Gates` submodule in `crates/executive/src/service/verify.rs`:

1. **FormatGate** (MustPass) — runs `cargo fmt --check` in worktree. Fails if any file is unformatted.
2. **CompileGate** (MustPass) — runs `cargo build` in worktree. Fails on compile errors.
3. **TestGate** (MustPass) — runs `cargo test` in worktree. Fails if any test fails.
4. **ClippyGate** (Advisory) — runs `cargo clippy -- -D warnings`. Warns on lint violations.
5. **DiffScopeGate** (MustPass) — checks that changed files are within allowed paths (no changes to `Cargo.lock` alone, no `src/` outside expected modules). Compares against `ALLOWED_WRITE_PATHS`.
6. **ArchitectureGate** (Advisory) — checks that no new file violates layer boundaries (no fabric importing executive, etc.).
7. **CapabilityPolicyGate** (MustPass) — checks that tool calls made during the coding task were within the sub-agent's capability policy.

Each gate uses `std::process::Command` to invoke cargo/git tools. All gates are async-compatible (`async fn check`).

- [ ] Implement all 7 gates with tests in `crates/executive/src/service/verify.rs`
- [ ] Commit

### Task E.3: Register as PostTurnHook

Create `crates/executive/src/service/verify/hook.rs`:

Define a `PostTurnHook` implementation that:
1. Checks if the turn was a PiRuntime coding turn (inspects `TurnResult` for `SubAgentHandle` with PiRuntime type).
2. Constructs `VerificationContext` from the turn result.
3. Runs `VerificationPipeline::standard()` with all 7 gates.
4. If MustPass gates fail, sets `TurnStop::Blocked` with the verification report.
5. If Advisory gates warn, attaches warnings to the turn result without blocking.

Modify `crates/executive/src/service/post_turn.rs` — register the verification hook in `PostTurnPipeline::run()`.

- [ ] Create `crates/executive/src/service/verify/hook.rs`
- [ ] Modify `crates/executive/src/service/post_turn.rs` — register verification hook
- [ ] Modify `crates/executive/src/service/mod.rs` if needed
- [ ] Commit

### Task E.4: Tests

- [ ] `cargo test -p executive -- service::verify` — all 7 gates, pipeline run, MustPass blocks, Advisory warns
- [ ] `cargo test -p executive -- service::post_turn` — hook integration
- [ ] `cargo build --workspace` — no regressions
- [ ] Commit

**Phase E acceptance:** 7 gates defined; MustPass gates block turn completion; Advisory gates produce warnings; verification hooks run automatically after Pi coding turns.

---

## 8. Phase F: Google OAuth + Sync

**Estimated:** 1-2 weeks | **Depends on:** Phase B | **Crates:** corpus

**Architecture:** Google OAuth is configured via `McpServerConfig` (extend with OAuth fields). Gmail and Calendar are MCP tool servers — no custom REST clients needed for read operations. No `CredentialVault`, no `drivers/google/vault.rs`, no AES-256-GCM deps. A `GoogleSyncManager` polls Gmail+Calendar and converts events into `InboundMessage` for the channel system.

### Task F.1: Extend McpServerConfig for OAuth

Modify `crates/corpus/src/tools/mcp/config.rs`:

Add an `McpOAuthConfig` struct to `McpServerConfig`:
```rust
pub struct McpOAuthConfig {
    pub provider: OAuthProvider,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
    pub token_url: String,
    pub auth_url: String,
    pub redirect_uri: String,
}
pub enum OAuthProvider { Google, GitHub, Custom(String) }
```

Add `oauth: Option<McpOAuthConfig>` field to `McpServerConfig`.

The existing MCP auth module at `crates/corpus/src/tools/mcp/auth.rs` handles token exchange and refresh. Extend it to support the Google OAuth flow.

- [ ] Modify `crates/corpus/src/tools/mcp/config.rs` — add McpOAuthConfig
- [ ] Modify `crates/corpus/src/tools/mcp/auth.rs` — add Google OAuth token exchange
- [ ] Commit

### Task F.2: Define Google sync types in fabric

Create `crates/fabric/src/types/google.rs`:

Define shared types: `GmailMessageSummary` (id, thread_id, subject, from, snippet, received_at, is_unread), `GmailQuery` (query string, max_results), `GoogleCalendarEvent` (id, summary, description, start, end, attendees, location), `TimeRange` (start, end), `GoogleEvent` enum (MailReceived, CalendarEventStarting, CalendarEventUpdated, CalendarEventCancelled).

Wire into fabric: add `pub mod google;` to `crates/fabric/src/types/mod.rs`, add `pub use types::google;` to `crates/fabric/src/lib.rs`.

- [ ] Create `crates/fabric/src/types/google.rs`
- [ ] Modify `crates/fabric/src/types/mod.rs` — add `pub mod google;`
- [ ] Modify `crates/fabric/src/lib.rs` — add `pub use types::google;`
- [ ] Commit

### Task F.3: Implement GoogleSyncManager

Create `crates/corpus/src/drivers/google/sync.rs`:

`GoogleSyncManager` struct holds:
- `reqwest::Client` for API calls.
- MCP tool client handles for Gmail and Calendar (discovered via MCP server).
- Polling interval configuration.

Methods:
- `start()` — spawns a background task that polls Gmail (list unread messages) and Calendar (upcoming events) on a configurable interval (default 60s).
- Converts new Gmail messages into `GoogleEvent::MailReceived`.
- Converts new/updated/cancelled calendar events into `GoogleEvent::CalendarEvent*` variants.
- `set_callback(f: impl Fn(GoogleEvent))` — registers a callback that the daemon wires to convert `GoogleEvent` into `InboundMessage` for the Gmail channel (Phase G).

This is the **only** new file in `drivers/google/`. No vault, no credential store — OAuth tokens come from `McpServerConfig` auth module.

- [ ] Create `crates/corpus/src/drivers/google/sync.rs`
- [ ] Create `crates/corpus/src/drivers/google/mod.rs` — `pub mod sync;`
- [ ] Modify `crates/corpus/src/drivers/mod.rs` — add `pub mod google;`
- [ ] Commit

### Task F.4: Compilation and tests

- [ ] `cargo test -p fabric -- types::google` — type serde
- [ ] `cargo test -p corpus -- drivers::google::sync` — sync manager construction, event conversion
- [ ] `cargo build --workspace` — no regressions
- [ ] Commit

**Phase F acceptance:** McpServerConfig supports Google OAuth; MCP auth handles token exchange; GoogleSyncManager polls Gmail+Calendar; no CredentialVault, no AES-256-GCM deps.

---

## 9. Phase G: Gmail Channel + Approval

**Estimated:** 1-2 weeks | **Depends on:** Phase A, Phase F | **Crates:** executive

**Architecture:** Approval extends the existing `SessionGateway::approval_flow` module at `crates/executive/src/core/session_gateway/approval_flow.rs`. No standalone `ApprovalManager` module. The GmailChannel implements the `Channel` trait and receives inbound messages from `GoogleSyncManager`.

### Task G.1: Extend SessionGateway approval_flow

Modify `crates/executive/src/core/session_gateway/approval_flow.rs`:

Add `ApprovalRequest` type:
```rust
pub struct ApprovalRequest {
    pub id: String,
    pub goal_id: String,
    pub request_type: ApprovalType,
    pub description: String,
    pub details: ApprovalDetails,
    pub timeout_secs: u64,
    pub created_at: String,
    pub channel_id: String,
    pub conversation_id: Option<String>,
}
```

Add `ApprovalType` enum: ApplyCodeDiff, SendEmail, DeleteFile, ModifyCalendar, DangerousCommand, CapabilityExpansion, BudgetIncrease.

Add `ApprovalDetails` enum with variants: CodeDiff { changed_files, diff_summary }, EmailDraft { to, subject, body_preview }, FileDeletion { paths }, CalendarModification { summary, start }, Generic { description }.

Add `ApprovalResult` enum: Approved, Rejected { reason }, TimedOut.

Add methods to `SessionGateway`:
- `request_approval(req: ApprovalRequest)` — stores in pending map, sends `OutboundMessage` with `UserAction::Approve`/`UserAction::Reject` through the originating channel.
- `resolve_approval(id: &str, result: ApprovalResult)` — resolves pending request, returns the original `ApprovalRequest`.

The pending store is a `HashMap<String, ApprovalRequest>` protected by the existing `Mutex<SessionManager>`.

- [ ] Modify `crates/executive/src/core/session_gateway/approval_flow.rs` — add ApprovalRequest types and methods
- [ ] Commit

### Task G.2: Implement GmailChannel

Create `crates/executive/src/impl/channel/gmail/mod.rs`:

`GmailChannel` struct holds:
- `reqwest::Client` for Gmail API access.
- OAuth token (from `McpServerConfig`).
- `polling_interval: Duration`.
- `last_check: Mutex<String>` (ISO 8601 timestamp).

Implements `Channel` trait:
- `start()` — spawns a polling task that calls `GoogleSyncManager` to fetch unread messages, converts each `GmailMessageSummary` into an `InboundMessage` with:
  - `channel_id: ChannelId("gmail")`
  - `message_id: MessageId(email.message_id)` **(not `id`)**
  - `conversation_id: Option<ConversationId>` — set from `email.thread_id` **(not `conversation`)**
  - `sender_id: email.from` **(not `principal`)**
  - `content: MessageContent::Text { text: email.snippet }`
  - `timestamp: email.received_at` **(not `received_at` on InboundMessage — it is `timestamp`; see Phase A field spec)**
  - `reply_to_action: None`
- `send()` — converts `OutboundMessage` to Gmail draft/send via Gmail API (write operations: `POST /gmail/v1/users/me/messages/send`). Maps `OutboundMessage.actions` to inline reply options.

**Critical B.2 fix:** Every `InboundMessage` constructed in Phase G must use the field names defined in Phase A: `channel_id`, `message_id`, `conversation_id`, `sender_id`, `content`, `timestamp`, `reply_to_action`. Fields like `id`, `channel`, `principal`, `conversation`, `reply_to`, `received_at` are **not valid** on `InboundMessage`.

On test: spawn the channel, simulate an incoming email, verify field names and routing correctness.

- [ ] Create `crates/executive/src/impl/channel/gmail/mod.rs`
- [ ] Modify `crates/executive/src/impl/channel/mod.rs` — register GmailChannel
- [ ] Commit

### Task G.3: Compilation and tests

- [ ] `cargo test -p executive -- core::session_gateway::approval_flow` — request, resolve, timeout
- [ ] `cargo test -p executive -- impl::channel::gmail` — InboundMessage field names, send draft, polling
- [ ] `cargo build --workspace` — no regressions
- [ ] Commit

**Phase G acceptance:** ApprovalRequest extends SessionGateway; GmailChannel implements Channel trait; InboundMessage field names match Phase A spec exactly; write operations send via Gmail API.

---

## 10. Phase H: GBrain Memory Backend

**Estimated:** 1-2 weeks | **Depends on:** Phase B | **Crates:** mnemosyne

**Architecture:** GBrain is a new memory backend in `crates/mnemosyne/src/impl/backends/gbrain/`. It implements the existing Mnemosyne backend trait (or defines its own REST-based backend). Docker Compose manages the GBrain service + PostgreSQL.

### Task H.1: Define GBrain API types and REST client

Create `crates/mnemosyne/src/impl/backends/gbrain/types.rs`:

DTOs matching the GBrain REST API contract from `docs/plans/2026-07-14-agent-google-design.md Phase H`:
- `GBrainHealth` — status, version.
- `StoreRequest` — memory_type, content, payload, goal_id, attempt_id, provenance (ProvenanceDTO), tags.
- `ProvenanceDTO` — source, goal_id, attempt_id, recorded_by.
- `StoreResponse` — id, created_at.
- `BatchStoreRequest` / `BatchStoreResponse`.
- `RecallRequest` — query, goal_id, memory_types, max_results, freshness_weight, min_score.
- `RecallResponse` / `ScoredMemoryDTO`.
- `ForgetRequest` / `ForgetResponse`.

Create `crates/mnemosyne/src/impl/backends/gbrain/client.rs`:

`GBrainClient` struct holds `reqwest::Client` + `base_url: String`. Methods:
- `health_check() -> Result<GBrainHealth>`
- `store(req: StoreRequest) -> Result<StoreResponse>`
- `store_batch(reqs: Vec<StoreRequest>) -> Result<BatchStoreResponse>`
- `recall(req: RecallRequest) -> Result<RecallResponse>`
- `forget(req: ForgetRequest) -> Result<ForgetResponse>`

All methods are async, JSON-encoded, with error handling for connection/timeout/4xx/5xx.

Add `reqwest = { version = "0.12", features = ["json"] }` to `crates/mnemosyne/Cargo.toml`.

- [ ] Create `crates/mnemosyne/src/impl/backends/gbrain/types.rs`
- [ ] Create `crates/mnemosyne/src/impl/backends/gbrain/client.rs`
- [ ] Modify `crates/mnemosyne/Cargo.toml` — add reqwest
- [ ] Commit

### Task H.2: Write IngestionPipeline, MemoryExtraction, and Recall

Create `crates/mnemosyne/src/impl/backends/gbrain/ingestion.rs`:

`IngestionPipeline` — async batch ingestion with buffered flush:
- `spawn(client, batch_size, flush_interval, max_buffer)` — spawns a background task that buffers `StoreRequest` entries and flushes them in batches to the GBrain API.
- `buffer(entry: StoreRequest)` — non-blocking enqueue; drops entries if buffer is full (with warning log).

Create `crates/mnemosyne/src/impl/backends/gbrain/extraction.rs`:

`MemoryExtractor` — converts session events into `StoreRequest` entries:
- Extracts facts, decisions, errors, and context from session transcripts.
- Tags entries by goal_id, attempt_id, event type.
- Produces `ProvenanceDTO` with recorded_by = "aletheon-agent".

Create `crates/mnemosyne/src/impl/backends/gbrain/recall.rs`:

`MemoryRecall` — queries GBrain for relevant memories:
- Builds `RecallRequest` from a query + optional goal_id.
- **B.3 fix:** Maps `RecallResponse.results` into `MemoryProjection` structs with all required fields: `relevant_facts`, `past_experiences`, `summary`, `memory_type`, `provenance_goal`, `retrieved_at`, `freshness`.
- `memory_type` is set from the result's type tag.
- `provenance_goal` is extracted from `ScoredMemoryDTO.provenance.goal_id`.
- `summary` is the first 200 chars of the stored content.
- Returns `Vec<MemoryProjection>`.

Create `crates/mnemosyne/src/impl/backends/gbrain/health.rs`:

GBrain health check — pings the health endpoint on startup and periodically.

- [ ] Create ingestion.rs, extraction.rs, recall.rs, health.rs
- [ ] Implement MemoryProjection mapping with all 7 fields in recall.rs
- [ ] Commit

### Task H.3: Wire into mnemosyne backend registry

Create `crates/mnemosyne/src/impl/backends/gbrain/mod.rs`:

Public module re-exports: `GBrainClient`, `IngestionPipeline`, `MemoryExtractor`, `MemoryRecall`, types module.

Modify `crates/mnemosyne/src/impl/backends/mod.rs` — add `pub mod gbrain;`.

- [ ] Create `crates/mnemosyne/src/impl/backends/gbrain/mod.rs`
- [ ] Modify `crates/mnemosyne/src/impl/backends/mod.rs` — add gbrain module
- [ ] Commit

### Task H.4: Add Docker Compose and configuration

Add `docker-compose.gbrain.yml` at repo root:

Services: `gbrain` (GBrain API, port 8080), `postgres` (PostgreSQL 16, port 5432, volume for data). GBrain depends on postgres. Environment variables for database connection, API keys, log level.

Add configuration section to Aletheon's config TOML:
```toml
[gbrain]
base_url = "http://localhost:8080"
batch_size = 50
flush_interval_sec = 30
max_buffer = 1000
ingestion_enabled = true
health_check_interval_sec = 30
```

- [ ] Create `docker-compose.gbrain.yml`
- [ ] Add gbrain config section to existing agent config
- [ ] Commit

### Task H.5: Compilation and tests

- [ ] `cargo test -p mnemosyne -- impl::backends::gbrain` — client CRUD, ingestion pipeline, recall projection
- [ ] `cargo test -p mnemosyne -- impl::backends::gbrain::recall::memory_projection_fields` — verify all 7 MemoryProjection fields populated
- [ ] `cargo build --workspace` — no regressions
- [ ] Commit

**Phase H acceptance:** GBrainClient connects to REST API; IngestionPipeline buffers and flushes; MemoryRecall produces MemoryProjection with all 7 fields; Docker Compose starts GBrain + PostgreSQL.

---

## 11. File Manifest

### Files Created

| Phase | File | Purpose |
|-------|------|---------|
| A | `crates/fabric/src/types/channel.rs` | Channel type definitions |
| A | `crates/executive/src/impl/channel/mod.rs` | Channel trait + ChannelRegistry |
| A | `crates/executive/src/impl/channel/telegram/mod.rs` | TelegramChannel implementation |
| A | `crates/executive/src/impl/channel/telegram/polling.rs` | Long-polling loop |
| A | `crates/executive/src/impl/channel/telegram/binding.rs` | Bot binding config |
| A | `crates/executive/src/impl/channel/telegram/formatting.rs` | Message formatting |
| B | `crates/fabric/src/types/goal.rs` | GoalSpec, GoalPriority, GoalPhase, MemoryProjection |
| B | `crates/executive/src/impl/goal/mod.rs` | Goal prompt assembly + routing |
| C | `crates/executive/src/impl/runtime/mod.rs` | Runtime module |
| C | `crates/executive/src/impl/runtime/deepseek.rs` | DeepSeekRuntime (SubAgentRuntime impl) |
| D | `crates/executive/src/impl/runtime/pi_types.rs` | PiSubagentTask, PiSubagentReport, FileChange |
| D | `crates/executive/src/impl/runtime/pi.rs` | PiRuntime (SubAgentRuntime impl) |
| E | `crates/executive/src/service/verify.rs` | VerificationGate trait + 7 gates + pipeline |
| E | `crates/executive/src/service/verify/hook.rs` | PostTurnHook registration |
| F | `crates/fabric/src/types/google.rs` | Google shared types |
| F | `crates/corpus/src/drivers/google/mod.rs` | Google driver module |
| F | `crates/corpus/src/drivers/google/sync.rs` | GoogleSyncManager |
| G | `crates/executive/src/impl/channel/gmail/mod.rs` | GmailChannel implementation |
| H | `crates/mnemosyne/src/impl/backends/gbrain/mod.rs` | GBrain backend module |
| H | `crates/mnemosyne/src/impl/backends/gbrain/types.rs` | GBrain API DTOs |
| H | `crates/mnemosyne/src/impl/backends/gbrain/client.rs` | GBrain REST client |
| H | `crates/mnemosyne/src/impl/backends/gbrain/ingestion.rs` | IngestionPipeline |
| H | `crates/mnemosyne/src/impl/backends/gbrain/extraction.rs` | MemoryExtractor |
| H | `crates/mnemosyne/src/impl/backends/gbrain/recall.rs` | MemoryRecall + MemoryProjection mapping |
| H | `crates/mnemosyne/src/impl/backends/gbrain/health.rs` | Health check |
| H | `docker-compose.gbrain.yml` | GBrain + PostgreSQL services |

### Files Modified

| Phase | File | Change |
|-------|------|--------|
| A | `crates/fabric/src/types/mod.rs` | Add `pub mod channel;` |
| A | `crates/fabric/src/lib.rs` | Add `pub use types::channel;` |
| A | `crates/executive/src/impl/mod.rs` | Add `pub mod channel;` |
| A | `crates/executive/Cargo.toml` | Add teloxide dep |
| A | `crates/executive/src/impl/daemon/handler/init.rs` | Add channel_registry, dispatch loop |
| A | `crates/executive/src/impl/daemon/server.rs` | Wire channel_registry |
| B | `crates/fabric/src/types/mod.rs` | Add `pub mod goal;` |
| B | `crates/fabric/src/lib.rs` | Add `pub use types::goal;`, extend ProcessState |
| B | `crates/executive/src/impl/mod.rs` | Add `pub mod goal;` |
| B | `crates/executive/src/impl/daemon/handler/init.rs` | Handle /goal command |
| B | `crates/executive/src/impl/daemon/server.rs` | Expose goal state |
| C | `crates/executive/src/impl/mod.rs` | Add `pub mod runtime;` |
| D | `crates/executive/src/impl/runtime/mod.rs` | Add pi_types, pi modules |
| E | `crates/executive/src/service/mod.rs` | Add verify module |
| E | `crates/executive/src/service/post_turn.rs` | Register verification hook |
| F | `crates/fabric/src/types/mod.rs` | Add `pub mod google;` |
| F | `crates/fabric/src/lib.rs` | Add `pub use types::google;` |
| F | `crates/corpus/src/tools/mcp/config.rs` | Add McpOAuthConfig |
| F | `crates/corpus/src/tools/mcp/auth.rs` | Add Google OAuth token exchange |
| F | `crates/corpus/src/drivers/mod.rs` | Add google module |
| G | `crates/executive/src/impl/channel/mod.rs` | Register GmailChannel |
| G | `crates/executive/src/core/session_gateway/approval_flow.rs` | Add ApprovalRequest types + methods |
| H | `crates/mnemosyne/src/impl/backends/mod.rs` | Add gbrain module |
| H | `crates/mnemosyne/Cargo.toml` | Add reqwest dep |

---

## 12. Execution Strategy

**Sequential within phases, parallel across independent phases:**

1. **Phase A** — first; all other phases depend on channel types.
2. **Phase B** — after A; defines goal types used by C, D, E, F, H.
3. **Phases C, D, F, H** — in parallel after B (all four are independent):
   - C (DeepSeekRuntime) and D (PiRuntime) both implement SubAgentRuntime independently.
   - F (Google OAuth) and H (GBrain) are separate crates with no cross-dependencies.
4. **Phase E** — after D (needs PiSubagentReport + FileChange types from D).
5. **Phase G** — after A + F (needs Channel trait from A, GoogleSyncManager from F).

**Estimated total:** 8-12 weeks (2 developers).

---

## 13. Acceptance Criteria

### Phase A
- [ ] Channel types (`InboundMessage`, `OutboundMessage`, `MessageContent`) compile and serialize correctly
- [ ] `Channel` trait defines `start()`, `send()`, `shutdown()` contract
- [ ] `ChannelRegistry` registers channels and routes messages
- [ ] `TelegramChannel` polls Telegram API, parses text/commands/files into `InboundMessage`
- [ ] Daemon init spawns channel dispatch loop; incoming messages enter turn orchestrator
- [ ] All unit tests pass; workspace builds without regressions

### Phase B
- [ ] `GoalSpec` type compiles with all fields; serde round-trips
- [ ] `MemoryProjection` includes all 7 fields: `relevant_facts`, `past_experiences`, `summary`, `memory_type`, `provenance_goal`, `retrieved_at`, `freshness`
- [ ] `ProcessState` has `AwaitingApproval` and `AwaitingHuman` variants
- [ ] `/goal` command parsed from Telegram, spawns AgentProcess via SubAgentSpawner
- [ ] Approval actions (`Approve`/`Reject`) flow through the channel and transition process state
- [ ] No `GoalSupervisor`, `GoalStore`, `objectives_v2` table, or `ALTER TABLE` migrations exist
- [ ] All unit tests pass; workspace builds without regressions

### Phase C
- [ ] `DeepSeekRuntime` implements `SubAgentRuntime` trait
- [ ] Uses `provider.complete()`, **not** `provider.chat()`
- [ ] `classify_failure()` correctly categorizes transient/permanent/tool/token errors
- [ ] SupervisorTree retries transient errors with backoff (max 3)
- [ ] Permanent errors escalate to Claude runtime
- [ ] No `GoalWorker` trait, `WorkerRegistry`, or standalone `RetryPolicy`/`Escalation` modules exist
- [ ] All unit tests pass; workspace builds without regressions

### Phase D
- [ ] `PiRuntime` implements `SubAgentRuntime` trait
- [ ] `PiSubagentTask` and `PiSubagentReport` types compile and serialize
- [ ] PiRuntime creates git worktrees, runs coding loop, produces report with FileChange list
- [ ] Worktrees cleaned up on success, preserved on failure
- [ ] Uses `provider.complete()`, **not** `provider.chat()`
- [ ] No `PiWorker as GoalWorker`, no `impl/agent/pi/` directory
- [ ] All unit tests pass; workspace builds without regressions

### Phase E
- [ ] `VerificationGate` trait defined with `check()` method
- [ ] All 7 gates implemented: Format, Compile, Test, Clippy, DiffScope, Architecture, CapabilityPolicy
- [ ] MustPass gates block turn completion; Advisory gates produce warnings
- [ ] Verification runs as `PostTurnHook` in `crates/executive/src/service/`, **not** `impl/goal/verify/`
- [ ] Hook registered in `PostTurnPipeline::run()`
- [ ] All unit tests pass; workspace builds without regressions

### Phase F
- [ ] `McpServerConfig` extended with `McpOAuthConfig` (Google OAuth provider)
- [ ] MCP auth module handles Google token exchange
- [ ] `GoogleSyncManager` polls Gmail and Calendar APIs, converts events to `GoogleEvent`
- [ ] No `CredentialVault`, `vault.rs`, `drivers/google/` directory (beyond sync.rs), or AES-256-GCM deps exist
- [ ] All unit tests pass; workspace builds without regressions

### Phase G
- [ ] `ApprovalRequest` types extend `SessionGateway::approval_flow`
- [ ] `GmailChannel` implements `Channel` trait
- [ ] All `InboundMessage` field names match Phase A spec: `channel_id`, `message_id`, `conversation_id`, `sender_id`, `content`, `timestamp`, `reply_to_action`. No `id`, `channel`, `principal`, `conversation`, `reply_to`, `received_at`.
- [ ] No standalone `ApprovalManager` module exists
- [ ] All unit tests pass; workspace builds without regressions

### Phase H
- [ ] `GBrainClient` connects to REST API; health check passes
- [ ] `IngestionPipeline` buffers and flushes batches to GBrain
- [ ] `MemoryRecall` returns `MemoryProjection` with all 7 fields populated (including `summary`, `memory_type`, `provenance_goal`)
- [ ] `docker-compose.gbrain.yml` starts GBrain + PostgreSQL successfully
- [ ] All unit tests pass; workspace builds without regressions

---

## 14. What Was Removed vs. What Was Added

| Removed (parallel shadow system) | Replaced By (existing infrastructure) |
|---|---|
| GoalSupervisor trait + 10-state FSM | AgentProcess lifecycle via ProcessState/SupervisorTree |
| GoalStore + objectives_v2 table + ALTER TABLE | Ephemeral goals in ProcessTable; persistence via Mnemosyne journal |
| GoalWorker trait + WorkerRegistry | SubAgentRuntime trait + SubAgentSpawner::with_runtime() |
| Standalone RetryPolicy module | SupervisorTree restart policies |
| Standalone Escalation module | Supervisor policy dispatching different runtime |
| PiWorker as GoalWorker | PiRuntime implements SubAgentRuntime |
| impl/agent/pi/ directory | impl/runtime/pi.rs |
| impl/goal/verify/ directory | service/verify.rs + PostTurnHook |
| CredentialVault + vault.rs + AES-256-GCM | McpServerConfig OAuth + MCP auth module |
| drivers/google/ directory (gmail.rs, calendar.rs) | MCP tool servers + GoogleSyncManager (sync.rs only) |
| ApprovalManager standalone module | SessionGateway::approval_flow extension |
| ~5400 lines of inline Rust code | 3-5 line task descriptions |
