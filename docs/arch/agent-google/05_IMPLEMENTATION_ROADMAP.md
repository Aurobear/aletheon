# Implementation Roadmap

> **Status:** Proposed  
> **Priority:** Build one complete vertical slice before expanding the platform.

## 1. Target Vertical Slice

```text
Telegram
    ↓
/goal
    ↓
Native Cognit
    ↓
Goal Supervisor
    ↓
DeepSeek or Pi
    ↓
Verification
    ↓
Telegram approval or progress
    ↓
Mnemosyne/GBrain record
```

## 2. Recommended Crates

```text
crates/
├── aletheon-channel-core/
├── aletheon-channel-telegram/
├── aletheon-channel-gmail/
├── aletheon-goal-core/
├── aletheon-goal-runtime/
├── aletheon-google-core/
├── aletheon-google-gmail/
├── aletheon-google-calendar/
├── aletheon-google-drive/
├── aletheon-subagent-core/
├── aletheon-subagent-pi/
├── aletheon-mnemosyne-core/
├── aletheon-mnemosyne-gbrain/
├── aletheon-cognit-core/
├── aletheon-cognit-native/
├── aletheon-agora/
├── aletheon-dasein/
└── aletheon-executive/
```

Provider implementations depend on core protocols, never the reverse.

## 3. Phase 0 — Preserve Native Agent

- identify the current Native Cognit entrypoint;
- preserve existing conversation, tools, sessions and DeepSeek path;
- wrap before rewriting;
- define the current implementation as `NativeCognit v0`.

Acceptance: existing CLI/TUI conversations still work.

## 4. Phase 1 — Telegram

- owner binding;
- long polling;
- `InboundMessage` and `OutboundMessage` mapping;
- text, file and button support;
- offset persistence;
- unknown-user rejection.

Acceptance: phone chat works and restart does not duplicate messages.

## 5. Phase 2 — Single Goal Runtime

- GoalSpecification;
- Goal state machine;
- Goal Supervisor;
- persistent one-active-Goal model;
- status, pause, resume and cancel;
- Telegram progress.

Acceptance: Goal survives restart and preserves original intent.

## 6. Phase 3 — DeepSeek Worker

- Goal Frame construction;
- Attempt records;
- failure classification;
- retry policy;
- evidence collection;
- cost and token accounting.

Acceptance: compilation failures retry within limits and repeated failures escalate.

## 7. Phase 4 — Pi Coding Subagent

- `SubagentTask`;
- `SubagentReport`;
- shell/RPC process launch;
- stdout, stderr and exit status;
- timeout and cancellation;
- temporary worktree;
- diff collection;
- Native review.

Acceptance: Pi never modifies the main worktree directly.

## 8. Phase 5 — Verification

- formatting;
- compilation;
- unit and integration tests;
- diff scope;
- capability policy;
- architecture review.

Acceptance: tasks cannot complete without required evidence.

## 9. Phase 6 — GBrain

- implement `MnemosyneBackend`;
- store architecture decisions and Goal outcomes;
- recall with provenance and freshness;
- project into Agora;
- prohibit direct Dasein mutation.

Acceptance: current and obsolete architecture decisions can be distinguished.

## 10. Phase 7 — Google Read-Only

- OAuth;
- encrypted credentials;
- Gmail read-only;
- Calendar read-only;
- account binding;
- manual refresh;
- Telegram queries.

Acceptance: user can ask for today's events and important unread mail.

## 11. Phase 8 — Google Sync

- Gmail cursor;
- Calendar sync token;
- Drive change cursor;
- normalized events;
- deduplication;
- retry and recovery;
- Goal wake-up;
- Telegram notifications.

Acceptance: synchronization resumes after restart without duplicating events.

## 12. Phase 9 — Gmail Channel

- sender allowlist;
- subject classification;
- Goal Draft creation;
- attachment ingestion;
- Telegram approval;
- report delivery.

Acceptance: `[GOAL]` email creates a draft, never unrestricted execution.

## 13. Phase 10 — Web Dashboard

Build only after the messaging loop works.

Screens:

- Goals;
- task DAG;
- attempts;
- diffs;
- approvals;
- memory search;
- system health.

## 14. Phase 11 — Deployment Hardening

- Ubuntu Server;
- systemd;
- Docker Compose;
- backups;
- Tailscale;
- secret management;
- health checks;
- log rotation;
- disk quotas.

## 15. First Milestone

The first milestone is complete when:

```text
1. User sends `/goal fix current cargo check errors` in Telegram.
2. Aletheon creates and persists a Goal.
3. Native Cognit creates a bounded plan.
4. DeepSeek analyzes the failure.
5. Pi edits an isolated worktree.
6. Verification runs cargo check and tests.
7. Telegram requests approval.
8. User approves or rejects.
9. Goal state updates.
10. Mnemosyne records the outcome.
11. A server restart does not lose the Goal.
```

## 16. Non-Goals

Do not prioritize:

- native Android app;
- multi-user SaaS;
- unrestricted autonomy;
- all Google products at once;
- Rust rewrite of GBrain;
- replacing Native Cognit with Pi;
- distributed scheduling;
- local large-model inference;
- public internet exposure;
- automatic Dasein mutation.
