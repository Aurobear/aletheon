# Aletheon 执行状态台账

> Live status 唯一权威；节点/依赖见统一计划 §3，完整 STATUS 字段模板见统一计划 §8.6。
> B0 合入前仅是待提交初始值，不能作为运行证据。

```text
PLAN_BASELINE: c080b08bf3170dd8a09acdb738a255133abbf11b
APPROVED_BY: repository_owner（2026-08-05 对话明确授权）
APPROVED_AT: 2026-08-05T01:49:06+08:00
EXECUTION_CONTROL_PLANE: external_supervisor
GITHUB_WRITE_AUTHORIZATION: approved
AUTO_MERGE_AUTHORIZATION: approved
OVERNIGHT_DEADLINE: unbounded_by_owner
OVERNIGHT_INPUT_TOKEN_CAP: unbounded_by_owner
OVERNIGHT_OUTPUT_TOKEN_CAP: unbounded_by_owner
OVERNIGHT_COST_CAP_USD: unbounded_by_owner
B0_MERGE_SHA: 95a5f8046f64eccd0ea6f22b0a82f06465e72419
```

上述字段已由 owner 明确授权；token/cost/deadline 豁免不取消依赖、三次 attempt、证据、CI 或 blocked 停止规则。

## B0

```text
STATUS: accepted
NODE: B0
BASE / BRANCH / PR / GOAL_ID: c080b08bf3170dd8a09acdb738a255133abbf11b / auro/docs/20260805-goal-execution-bootstrap / #162 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: six documents present; path/structure/dependency/sensitive-data checks passed; GitHub CI 5 passed and 4 skipped
FAILURES: none
BLOCKER: none
MERGE_SHA: 95a5f8046f64eccd0ea6f22b0a82f06465e72419
```

## B1

```text
STATUS: accepted
NODE: B1
BASE / BRANCH / PR / GOAL_ID: c080b08bf3170dd8a09acdb738a255133abbf11b / n/a / n/a / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: owner approval recorded; GitHub auth verified; external supervisor Goal active
FAILURES: none
BLOCKER: none
MERGE_SHA: n/a
```

## X0

```text
STATUS: accepted
NODE: X0
BASE / BRANCH / PR / GOAL_ID: 95a5f8046f64eccd0ea6f22b0a82f06465e72419 / auro/chore/20260805-x0-architecture-freeze / #163 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: all 14 retained historical branch tips are ancestors of origin/dev; compatibility-debt.tsv and metrics.env frozen_commit equal PLAN_BASELINE; B0/B1 evidence complete; GitHub CI 5 passed and 4 skipped
FAILURES: none
BLOCKER: none
MERGE_SHA: 03d263d44f0277d883c24b1be0b98e3a48519291
```

## X1

```text
STATUS: accepted
NODE: X1
BASE / BRANCH / PR / GOAL_ID: 03d263d44f0277d883c24b1be0b98e3a48519291 / auro/feat/20260805-x1-contract-gates / #164 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: 22-row migration matrix; 1014-type Fabric public snapshot; 3 acceptance IDs bound to passing tests; forbidden-edge/parser/writer metrics; architecture and negative fixtures passed; GitHub CI 5 passed and 4 skipped
FAILURES: none
BLOCKER: none
MERGE_SHA: cb2388e8c9af690a9ce3063472c7f2f3cd6bda99
```

## X2

```text
STATUS: accepted
NODE: X2
BASE / BRANCH / PR / GOAL_ID: cb2388e8c9af690a9ce3063472c7f2f3cd6bda99 / auro/refactor/20260805-x2-task-kind-contract / #165 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: TaskKindArg and TaskKind conversion moved to fabric::contract::command without a Clap dependency; production interact::cli caller count is 0; focused Fabric/Aletheon/Interact tests and architecture gates passed; GitHub CI 5 passed and 4 skipped
FAILURES: none
BLOCKER: none
MERGE_SHA: b126e07d902d55a95d7387290d787c4d4c539757
```

## X3a

```text
STATUS: accepted
NODE: X3a
BASE / BRANCH / PR / GOAL_ID: b126e07d902d55a95d7387290d787c4d4c539757 / auro/feat/20260805-x3a-command-dispatch / #166 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: versioned ClientIntent preserves principal, workspace, permission, requirements, task kind and session authority; canonical CommandSpec added without presentation dependencies; Executive CommandDispatcher is the unique typed command-to-use-case handler; A-ENTRY-001 application test proves CLI/TUI/gateway surfaces select the same use case; focused Fabric/Executive tests, package checks, architecture acceptance and doc-path checks passed; GitHub CI 5 passed and 4 skipped
FAILURES: none
BLOCKER: none
MERGE_SHA: 0652e70ec4a0140101b67d81885a664ee57996f0
```

## X3b

```text
STATUS: accepted
NODE: X3b
BASE / BRANCH / PR / GOAL_ID: 0652e70ec4a0140101b67d81885a664ee57996f0 / auro/feat/20260805-x3b-user-command-adapters / #167 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: one Fabric CommandSpec catalog now materializes top-level Clap help, TUI slash metadata, Gateway command resolution, and generated Bash/Zsh completion trees; TUI, line mode, one-shot CLI, canonical local RPC, legacy chat/status, and Gateway prompt routes converge on ClientIntent and Executive CommandDispatcher; U-CLI-001/002 and PRODUCTION_CLI_PARSERS=1 gates pass, while the deleted compatibility parser retains zero callers; Fabric/Gateway/Interact/Aletheon and affected Executive tests, package Clippy -D warnings, architecture, formatting, completion, diff, and doc-path checks pass; system deploy passed with release/installed/machine-core/user-daemon SHA-256 cfbfb6127f6a9fa64c4487ae136307bdc988371c1c581e9bafbdaa894d2eb9a5, zero restart counters, stable services, and an official-socket real LLM request; GitHub CI 5 passed and 4 skipped
FAILURES: none
BLOCKER: none
MERGE_SHA: c76d6aa8f649b4485e02c28712a2d74e6e125abb
```

## X3c

```text
STATUS: accepted
NODE: X3c
BASE / BRANCH / PR / GOAL_ID: c76d6aa8f649b4485e02c28712a2d74e6e125abb / auro/refactor/20260805-x3c-remove-compat-parser / #168 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: commit 61875dc8 deletes the dormant Interact Args parser, handler, module export, and assembly compatibility re-export; the required one-shot socket path is preserved as a parser-free ClientIntent adapter and no longer re-parses slash text; Interact no longer depends on Clap; A-ENTRY-002/003 and U-CLI-004 prove the parser is absent and production caller/import count is zero; Interact 121-test suite plus 7 integration tests, Aletheon package tests, focused one-shot tests, package Clippy -D warnings, formatting, diff, and architecture gates pass; sudo system deploy passed with target/release, /usr/bin, machine-core, and user-daemon SHA-256 b51b61713763da87cec1906695755f0cb40b7722376c7acb47620ad3ae30f9d9, zero restart counters, stable services, and an official-socket real LLM request; GitHub CI 5 passed and 4 skipped
FAILURES: none
BLOCKER: none
MERGE_SHA: 58a136204bda2d4b8d719a6b06d9cb52c918433c
```

## X4a

```text
STATUS: accepted
NODE: X4a
BASE / BRANCH / PR / GOAL_ID: 58a136204bda2d4b8d719a6b06d9cb52c918433c / auro/feat/20260805-x4a-daemon-lifecycle-doctor / #169 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: commits 5bca5e30 and 98be8a6b add the Executive daemon lifecycle contract, deterministic install-mode resolution, one monotonic startup deadline, cross-process startup and process-lifetime authority locks, owned stale-socket recovery, typed initialize version negotiation, bounded diagnostics, and the Executive doctor use case; U-BOOT-003 concurrent-client coverage proves one activation and one stale-socket recovery, while host tests cover lock recovery, typed readiness, fail-closed version skew, and refusal to replace an unresponsive authority; focused Executive/Aletheon tests, package Clippy -D warnings, architecture, formatting, and diff gates pass; sudo system deploy passed with target/release, /usr/bin, machine-core, and user-daemon SHA-256 ba445e151f4228358008422df247a646dadb62ae81d6e62f95bcd059815c131a, stable PIDs, zero restart counters, and an official-socket real LLM request; a second installed daemon exits nonzero on the authority fence and installed doctor JSON remains parseable
FAILURES: none
BLOCKER: none
MERGE_SHA: 7ea6e66f226a0aa2628ad17292f85a2a72e23cc8
```

## X4b

```text
STATUS: accepted
NODE: X4b
BASE / BRANCH / PR / GOAL_ID: 7ea6e66f226a0aa2628ad17292f85a2a72e23cc8 / auro/feat/20260805-x4b-interact-ensure-running / #170 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=2
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: commits cde69177 and eb8d7a1d make Interact resolve the canonical user socket and invoke the Executive lifecycle before TUI or one-shot connection; system/user-local/dev-foreground mode selection, exact socket provenance, owned stale-socket recovery, typed protocol/runtime negotiation, and bounded JSON diagnostics are preserved; the original 30-second readiness deadline passed the then-current cold-start evidence, while the current populated installation has a 60-second bounded default after observed Dasein/Session replay exceeded 30 seconds; systemd and foreground bootstrap exits still produce immediate daemon_bootstrap_failed diagnostics; focused Executive lifecycle/readiness/launcher and Interact host tests, package Clippy -D warnings, architecture, formatting, and diff gates pass; sudo system deploy passed with target/release, /usr/bin, machine-core, and user-daemon SHA-256 b66b7f576d666f34dcab084a76c9027fee28b10549c8531f9c50c68bfe6420fa; U-BOOT-001 installed cold start completed without manual daemon activation in 25068ms; U-BOOT-002 installed invalid-config test returned structured daemon_bootstrap_failed in 91ms; post-test official-socket real LLM request returned X4B_READY, services remained active with zero restarts; GitHub CI 5 passed and 4 skipped
FAILURES: attempt 1 used a single four-second readiness deadline and misclassified the installed daemon's approximately 21-second durable-state restore as a timeout; attempt 2 separated fast bootstrap-failure observation from healthy readiness waiting
BLOCKER: none
MERGE_SHA: 4bf3027d6e79fadbe9276210b36bee7b69ffcdf9
```

## X4c

```text
STATUS: accepted
NODE: X4c
BASE / BRANCH / PR / GOAL_ID: 4bf3027d6e79fadbe9276210b36bee7b69ffcdf9 / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: commit dc82d08b adds versioned ordered JSONL, stable terminal exit classes, stdin, durable principal-scoped idempotency, non-interactive approval blocking, cancellation and bounded output backpressure; focused Fabric/Executive/Aletheon tests, package Clippy -D warnings, architecture, formatting and diff gates passed; sudo system deploy passed with target/release, /usr/bin, machine-core and user-daemon SHA-256 3d6a72beb93d0faaa8f9d2f387ac9b72e5d1e8343d6112c3ff61dd495da1590a; installed exec real request returned X4C_READY as a two-event JSONL stream; services remained at zero restarts PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: none; PR #171 required checks (architecture, feature contracts, workspace validation, fuzz) passed; merged into dev.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X4d

```text
STATUS: accepted
NODE: X4d
BASE / BRANCH / PR / GOAL_ID: dc82d08b / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: commit 8dc8a756 adds canonical CommandSpec-backed run/resume/completion adapters; run supports explicit session reuse and human approval, resume opens the daemon-backed picker when ID is omitted, and completion prints the exact generated Bash/Zsh assets; focused Interact/Aletheon tests, help/completion contract tests, package Clippy -D warnings, architecture, formatting and diff gates passed; B1 external supervisor remains the explicit Goal control plane PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: installed PTY/aggregate deployment evidence remains deferred to X12; PR #171 checks passed and the implementation is merged.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X5a

```text
STATUS: accepted
NODE: X5a
BASE / BRANCH / PR / GOAL_ID: 8dc8a756 / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: reviewed mapping in docs/plans/Aletheon_Session_Authority_Mapping_2026-08-05.md; wire-surfaces.tsv records the existing SessionAppendStore, EventSpine and EventProjection convergence without a synonymous authority PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: none; doc-only authority mapping is merged in PR #171; installed aggregate verification remains owned by X12.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X5b

```text
STATUS: accepted
NODE: X5b
BASE / BRANCH / PR / GOAL_ID: 72677148 / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Fabric now defines schema-v1 SessionReadSnapshot, SessionListSnapshot, TaskSnapshot, ActivitySnapshot and bounded SessionEventPage contracts; canonical session.read_sessions/v1 discovery is served from SessionAppendStore; EventSpine exposes transport-neutral bounded committed-prefix reads and startup reconciliation consumes the port rather than the SQLite concrete adapter; Executive deterministically projects Task/Activity state from authoritative Session items; installed daemon persistence initialization fails closed instead of creating process-local event/projection/session stores; A-SESSION-001 replays the same TaskSnapshot after every item-boundary interruption and A-SESSION-002 proves duplicate item/event keys retain one Task/Activity projection; crash-between-authority-append/materialization recovery, protocol schema, Fabric architecture gates and focused Session protocol tests pass PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: installed deployment/aggregate verification remains owned by X12; PR #171 checks passed and the projection implementation is merged.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X5c

```text
STATUS: accepted
NODE: X5c
BASE / BRANCH / PR / GOAL_ID: 414eb58f / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Interact startup, resume and picker now consume canonical ReadSnapshot/ReadSessions and poll bounded ReadEvents; the reducer replaces Session/Task/Activity state atomically from schema-v1 snapshots, ignores duplicate pages, rejects gaps/out-of-order pages before mutation, and reloads authoritative snapshots; A-SESSION-003, reducer replay, typed session protocol, focused lifecycle tests, Interact all-target clippy and architecture acceptance pass PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: installed deployment/aggregate verification remains owned by X12; PR #171 checks passed and the reducer implementation is merged.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X6a

```text
STATUS: accepted
NODE: X6a
BASE / BRANCH / PR / GOAL_ID: 05f63446 / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Interact now renders a daemon-projected Task console with project/session/task/phase/goal/runtime identity/permission header, authoritative Conversation, Activity timeline, Changes/diagnostics panel and keyboard footer; U-TUI-001..007 focused frame/key tests pass across 80x24, 120x40 and 200x60; provider error and typed runtime/cache metrics are visible without transcript inference. Active context is shown only from typed runtime facts, never cumulative usage. Interact package tests (128), TaskConsole tests (9), all-target clippy and architecture acceptance pass. PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: full PTY/accessibility and installed provider/session evidence remain owned by X12; PR #171 checks passed and the console implementation is merged.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X6b

```text
STATUS: accepted
NODE: X6b
BASE / BRANCH / PR / GOAL_ID: 9546dbb0715536adf4cb9ce1361afc65a4578869 / auro/feat/20260805-x6b-secure-input-clean-v2 / deferred_by_owner / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=2
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: The X6a sanitizer, IME/paste handling, Action Palette, bounded @ discovery, typed workspace attachments, and scoped history/draft storage remain intact. X6b closes the remaining governed ! path through canonical command intent, explicit confirmation, Host capability execution, typed progress/terminal receipt projection, and persisted shell receipt; input-state retention now removes expired entries fail-closed. U-INPUT-001..006 are bound in acceptance-ids.tsv. Interact 156/156 library tests and Executive command-dispatcher 3/3 pass; Interact and Executive clippy -D warnings, architecture acceptance, and fmt pass. `sudo bash scripts/aletheon.sh deploy` passed from this branch: target/release, /usr/bin, machine core, user daemon, and Memory Agent share SHA-256 96e274c286b6e8eaf41298e46e79f1d2a0403c8eab6f51b207894c58ed43c2ea; all three services are active with NRestarts=0; official Memory Agent protocol and official user-socket real LLM request passed.
FAILURES: Initial full Interact run found the exact public builtin snapshot omitted the intentional shell command; commit 90b5cd8b updated the canonical expectation and the rerun passed 156/156.
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## X7

```text
STATUS: accepted
NODE: X7
BASE / BRANCH / PR / GOAL_ID: d01e118b / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: CapabilityTerminalReceipt binds result.call_id to the host-minted invocation; mismatches become terminal Failed + InvalidRequest evidence instead of successful receipts. Guarded streaming terminals are deferred until policy/output/audit settlement; cancellation and rejected permits emit typed terminal failures; mutation results retain call/permit/audit linkage and honest BestEffort coverage. A-CAP-001..005 and A-TURN-001 prefixed tests pass across Corpus/Fabric/Executive (10 Fabric, 10 Corpus, 4 budget, 4 governed capability, 2 hardware, 8 turn-equivalence); focused clippy and architecture acceptance pass. PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: installed deployment/aggregate verification remains owned by X12; PR #171 checks passed and the capability coverage implementation is merged.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X8a

```text
STATUS: accepted
NODE: X8a
BASE / BRANCH / PR / GOAL_ID: e3c32965 + checkpoint projection follow-up / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: TurnCheckpoint integrity verification rejects file-count mismatch; Executive read projection binds checkpoint and transaction to session/turn authority, carries mutation coverage and validation evidence, and requires one receipt or explicit omission for acceptance. U-CHK-003/U-CHK-004, the workspace-checkpoint unit set, Executive projection tests, and Corpus transaction projection tests pass; architecture acceptance passes. PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: installed deployment/aggregate verification remains owned by X12; PR #171 checks passed and the projection implementation is merged.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X8b

```text
STATUS: accepted
NODE: X8b
BASE / BRANCH / PR / GOAL_ID: abc6a5c8 / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: TUI DiffView now projects canonical PatchDelta file changes, diff preview, mutation coverage, rollback availability and partial/conflicted evidence; j/k file picker and Ctrl+D or /diff open the review surface. CheckpointReviewSnapshot carries the deterministic TurnCheckpointProjection fields without becoming a persistence authority. Host-verified checkpoint listing is now available through checkpoint.list/v1; /rewind opens a daemon-backed picker with code-only rewind, fork-only, and fork-then-rewind modes. Fork boundaries are resolved from the authoritative Session turn sequence, and fork-then-rewind waits for the fork terminal response before issuing the host rewind. No client paths or checkpoint blobs are accepted. U-CHK-001/U-CHK-002/U-CHK-005/U-CHK-006, checkpoint picker/fork sequencing, protocol/Executive/Interact focused tests and architecture acceptance pass. PR #172 merge evidence: required CI run 30982959144 passed; merge fa62aa5a46f33d133876f0ebca23e8fa5e3605fe.
FAILURES: installed PTY/deployment evidence remains owned by X12; PR #171 checks passed and Diff/rewind implementation is merged.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X8c

```text
STATUS: accepted
NODE: X8c
BASE / BRANCH / PR / GOAL_ID: 4bf3027d / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: bounded implementation stage; attempt=2
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Commits f7b150d8, 1e1a518a, ba99ae95 and ad7b7598 establish ReviewFinding plus HostSettlementService as the only Accept/Repair/Rollback writer, enforce failed evaluation as repair, remove model-facing settlement tools from the registry and every shipped profile, expose typed task.review/settle/v1 and task.review/latest/v1 contracts, and persist idempotent SQLite settlement receipts. U-VERIFY-001..004 and A-TURN-002 settlement tests pass; durable-store, Fabric RPC, Corpus transaction, specialized-role, shipped-profile regression, evaluation-turn, Executive check/clippy, Interact reducer, fmt and architecture gates pass. Final `sudo bash scripts/aletheon.sh deploy` installed SHA-256 591dfcacaa740262516499d6256fdcb5da1d5c3e5c2659dccc12338271822fa3 identically at target/release, /usr/bin, system core, user daemon and memory agent; machine and user restart counters stayed at zero across the stability window; the current daemon loaded all required cognitive profiles without quarantine; `/usr/bin/aletheon run` over the official user socket completed a real provider request with `X8C_RUNTIME_READY` and no rendered inference error. PR #171 merge evidence: required CI run 30981180832 passed; merge 370c294c492bcf6918c3b26bee40c962f754ae96.
FAILURES: none; prior bundled-profile deployment issue was fixed by ad7b7598; PR #171 required checks passed and merged into dev.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X8d

```text
STATUS: accepted
NODE: X8d
BASE / BRANCH / PR / GOAL_ID: 4bf3027d / auro/feat/20260805-x4c-run-exec-wire / #171 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: commits 7a19b5a3, 8f1d821d and f75af9d9 provide the governed review CLI/TUI projection, typed settlement receipt rendering, explicit rollback confirmation and the canonical rewind command surface; follow-up commit eee3230a gates scripted prompts on the daemon session projection before continuing. Focused review CLI/TUI, completion, architecture, formatting and full PR validation passed. PR #172 merge evidence: required CI run 30982959144 passed; merge fa62aa5a46f33d133876f0ebca23e8fa5e3605fe.
FAILURES: none; review CLI/TUI projection tests, completion checks, architecture gates and all required PR checks passed; merged into dev.
BLOCKER: none
MERGE_SHA: 370c294c492bcf6918c3b26bee40c962f754ae96
```

## X9a

```text
STATUS: accepted
NODE: X9a
BASE / BRANCH / PR / GOAL_ID: fa62aa5a46f33d133876f0ebca23e8fa5e3605fe / auro/feat/20260805-x9a-session-recovery / #173 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: session principal ownership is persisted through schema-v5 migration and enforced by principal-aware snapshots, lists, event pages and subscriptions; new turns bind the authenticated principal; late settlement receipts are fenced by daemon generation and emit typed rejection evidence; generation-fence, session-service, canonical-store, Executive check, formatting, diff and architecture gates pass locally; PR #173 required CI run 30984945793 passed all five enabled checks (architecture, validation, feature contracts, fuzz, source) and merged into dev.
FAILURES: none
BLOCKER: none
MERGE_SHA: 2ad35cd89aa879cdf73867912f7122eddd99037b
```

## X9b

```text
STATUS: accepted
NODE: X9b
BASE / BRANCH / PR / GOAL_ID: ae6e6dfa470e07ad3ad5f230560bb6c286883d1f / auro/feat/20260805-x9b-child-reconciliation / #177 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: host-owned PID plus Linux start-time identity is durably registered before Pi RPC execution; startup recovery reclaims only the matching process generation with bounded SIGTERM/SIGKILL and treats PID reuse as non-reclaimable. Agent terminal settlement is authoritative before terminal success, with receipt failure surfaced as a failed run. Runtime process, agent recovery, child settlement and PID-reuse tests pass; Executive Clippy -D warnings, all-target check, formatting and architecture gates pass locally; CI run 30996734472 passed architecture fitness, Feature contracts, PR validation and Fuzz quick-check; PR #177 merged into dev.
FAILURES: First CI run 30995696140 failed only because the checked-in AppConfig schema snapshot lagged the updated GrokHardeningConfig description; regenerated in f8b708ad and reran successfully in 30996734472.
BLOCKER: none
MERGE_SHA: 20e9bcad980cc40640065e69bce1f8af1da6f8b2
```

## X9c

```text
STATUS: accepted
NODE: X9c
BASE / BRANCH / PR / GOAL_ID: d48bac9b8cd6006507864b3c7a44a202773e0ceb / auro/feat/20260805-x9c-provider-authority / #179 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: InferencePort defaults now fail closed for missing provider machine authority; only the system-core RegistryInferencePort and explicitly selected LocalInferencePort implement provider admission/cooldown/metrics. ProviderRegistry resolves typed ProviderBackpressureConfig from the canonical endpoint key. Core RPC refuses to replace a live socket, reclaims only a failed stale probe, and regression coverage proves the original authority remains reachable. System and legacy core units use KillMode=control-group with bounded stop timeout. Local provider backpressure concurrency/cooldown/pacing tests, core RPC authority tests, inference-port fail-closed tests, Executive check/clippy, deployment boundary and architecture gates pass; SILENT_FALLBACKS=0. sudo bash scripts/aletheon.sh deploy passed from the X9c worktree: release binary, /usr/bin/aletheon, machine core, user daemon, and Memory Agent SHA-256 all 31a02c5ae3c3d7db08d1f0558970cffbe816f26e899f7e33359b22bb6a88ebee; restart counters remained stable; official user-socket real-request and Memory Agent smoke passed. PR #179 CI passed architecture fitness, Feature contracts, PR validation and Fuzz quick-check.
FAILURES: First PR #179 CI run 31000464516 failed because the new metric key was required by the minimal Phase 0 fixture; commit 05efe952 scopes SILENT_FALLBACKS to production checkouts, and rerun 31000597490 passed.
BLOCKER: none
MERGE_SHA: 6ffa9f85f2d9b4776b26b2781eb62c214a62e67d
```

## X10

```text
STATUS: accepted
NODE: X10
BASE / BRANCH / PR / GOAL_ID: 6ffa9f85f2d9b4776b26b2781eb62c214a62e67d / auro/docs/20260805-x9c-acceptance / #180 / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Removed the zero-caller Metacog hil_evidence_verifier and outcome_verifier facade modules; tests now import canonical evaluation modules. Corrected the HIL persistence inventory path. Added A-DELETE-001/002 mechanical coverage for the bounded compatibility ledger and README Stable capability evidence. Metacog verifier tests, architecture contract, architecture-check, formatting and clippy -D warnings pass.
FAILURES: none
BLOCKER: none
MERGE_SHA: 9546db0715536adf4cb9ce1361afc65a4578869
```

## X11

```text
STATUS: accepted
NODE: X11
BASE / BRANCH / PR / GOAL_ID: 6ffa9f85f2d9b4776b26b2781eb62c214a62e67d / auro/test/20260805-x11-rubric-closure / pending / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=2
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: tests/coding now has exactly 20 versioned task/fixture/acceptance/rubric sets. The catalog mechanically enforces the required 5 location/explanation, 5 small bug fix, 4 cross-file, 2 failing-test, 2 review-finding, 1 side-effect-free session resume, and 1 child-orphan reconciliation distribution. Every rubric is schema version 1, totals 100 points, and binds terminal, correctness, scope, and governance evidence. Static harness contracts, runner/replay/suite/workflow tests, exact inventory checks, rubric validation, and git diff checks pass; all 20 isolated fixture baselines compile through scripts/cargo-agent.sh and each has its own workspace manifest and lockfile.
FAILURES: the earlier incomplete draft was repaired; the final installed 20-task catalog exercised all 20 versioned fixtures and recorded 19 accepted outcomes plus one correctly rejected verification failure
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending
```

## X12

```text
STATUS: accepted
NODE: X12
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
REQUIREMENT: installed system provenance/restart/official-socket/20-task >=16 gate (`docs/plans/Aletheon_Unified_Execution_Plan_2026-08-04.md:125,325-326`; `docs/plans/Aletheon_User_Experience_and_Engineering_Workflow_Plan_2026-08-04.md:835-837`)
EVIDENCE:
  - version-bound change obligations now retain only the current transaction version (`crates/cognit/src/core/cognitive_task.rs:166-213`), and the auditor surfaces current diff review before validation (`crates/cognit/src/core/progress_auditor.rs:149-180`)
  - final system deploy log: `target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/deploy.log`
  - final unchanged-runtime catalog: `target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/suite.json`
  - post-suite and final-current installed verification: `target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/post-suite-verify.log` and `final-current-verify.log`
VALIDATION / RUNTIME EVIDENCE:
  - release `target/release/aletheon`, `/usr/bin/aletheon`, machine core, user daemon and Memory Agent all have SHA-256 `038986c29aaf37680d0a8ce0702235bbf50c13b4df8aac9dd73be80730007b1a`; before/after process hashes are recorded beside `suite.json`
  - during the unchanged-runtime suite, machine core/user daemon/Memory Agent retained the same PID, `ActiveState=active` and `NRestarts=0`; after later operator-side service activity, `final-current-verify.log` again proved current SHA provenance, two stable restart windows, official Memory Agent smoke and a real LLM request through `/usr/bin/aletheon` plus the official user socket
  - final catalog result is 19/20 evidence-complete outcomes (0.95), with zero scope violations and zero resource leaks; 18 authoritative `verified`, one expected `blocked`, and one expected `budget_exhausted` terminal were observed
  - the one unsuccessful task, `rust_multifile`, was correctly rejected by independent acceptance as `verification_failure` instead of being counted as success; therefore the Host/suite verdict and retained evidence agree, and the >=16 gate passes
  - focused Cognit completion/auditor/session tests, Corpus governed-command tests, Python runner/replay/suite tests (29), scoped all-target checks, formatting and diff checks passed before deployment
FAILURES: one model task failed independent acceptance (`rust_multifile`); it is retained as a failed benchmark outcome and does not violate the 16/20 threshold
BLOCKER: none
MERGE_SHA: pending_by_owner
```

## XF-001

```text
STATUS: accepted
NODE: XF-001
BASE / BRANCH / PR / GOAL_ID: 9546dbb0715536adf4cb9ce1361afc65a4578869 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: tests/coding/harness/run.py now consumes canonical ExecEventEnvelope v1 schema_version/type/status/operation_id and nested TurnMetrics. Receipt validation preserves every classified terminal kind and treats provider_unavailable/provider_rejected/validation_failed/output_backpressure as explicit execution failures. Fake-client runner, replay, suite, receipt, workflow, and full static harness tests pass.
FAILURES: none
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-002

```text
STATUS: accepted
NODE: XF-002
BASE / BRANCH / PR / GOAL_ID: 9546dbb0715536adf4cb9ce1361afc65a4578869 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: X12 attempt 2 receipt target/coding-x12-corrected-20260805-2052/receipts/api_error_mapping.json records the original installed `/usr/bin/aletheon --sandbox auto` timeout. The repair constructs an explicit non-secret sandbox environment allowlist, excludes provider credentials/wrapper injection, gives the installed user service a conventional user-tool PATH, and restores per-turn workspace writable binds under configured profiles before metadata re-protection. Environment tests, 8 bubblewrap tests including live write/deny behavior, Corpus all-target clippy `-D warnings`, systemd boundary checks, docs paths, and formatting pass. `sudo bash scripts/aletheon.sh deploy` passed with release/installed/all-running executable SHA `caf149d3c04d5243e9c7c3d66ccdb4d0923018bf0fd6103683ccfca15cbd6db3`, stable restart counters, Memory Agent smoke, and official-socket real request. Installed artifact `target/xf-002-installed-pass-20260805-212244/exec.json` proves auto sandbox resolved `/home/aurobear/.cargo/bin/cargo`, completed `cargo check`, and denied a write to a pre-existing path outside WorkspacePolicy; host content remained `UNCHANGED`.
FAILURES: none
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-003

```text
STATUS: accepted
NODE: XF-003
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: The retained installed rust_bugfix receipt from the concurrent X12 diagnostic run shows `src/lib.rs` plus the complete `target/` tree in `workspace.changed_files`. The task requires only `src/`, so ordinary in-workspace Cargo validation becomes a false policy_scope_failure even when the source repair is correctly scoped. The harness now adds `/target/` to each temporary repository's private `.git/info/exclude` before the fixture baseline commit; no fixture source or production policy is specialized. A focused regression proves `src/lib.rs` remains visible while a synthetic `target/debug/artifact` is excluded. Runner tests 8/8 and the full static harness suite pass; docs paths and diff checks pass.
FAILURES: none
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-004

```text
STATUS: accepted
NODE: XF-004
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Retained installed coding diagnostics recorded repeated `apply_patch` errors. The model used the standard outer `*** Begin Patch` form with `*** Update File:` headers and bare `@@` hunks, while the parser accepted only unprefixed operation headers plus Aletheon-specific `>>>` fences. The repair accepts common update/add markers, multiple file operations in one outer document, `*** Move to:`, and inferred bare-hunk counts while retaining bounded complete-context matching and canonical path validation. Platform structured-patch tests 5/5 and all-target clippy `-D warnings` pass; canonical fenced and unified-diff formats remain covered.
FAILURES: none
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-005

```text
STATUS: accepted
NODE: XF-005
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Installed artifact `target/xf-005-cargo-diagnostic-20260805-215304/exec.json` proves `cargo test` reaches rustc but fails to create `/tmp/rustc*` because the host `/tmp` is correctly read-only. The repair creates a unique host temporary directory per bash invocation, adds only that path to the resolved sandbox writable roots, exports TMPDIR/TMP/TEMP to it, and retains the TempDir guard through terminal tool execution for automatic cleanup. Environment coverage proves all three variables share the private root and still excludes secrets/wrappers; 9 live bubblewrap tests prove private `mktemp`, configured workspace writes, and deny masking; Corpus all-target clippy `-D warnings`, formatting, docs paths, and diff checks pass.
FAILURES: none; final installed coding tasks executed the real sandboxed Cargo validation path and the catalog reported zero resource leaks
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-006

```text
STATUS: accepted
NODE: XF-006
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Retained approval/budget receipts show authoritative expected non-success terminals accompanied by a denial-path tool error, but receipt classification unconditionally added `tool_error_observed`. The classifier now tolerates tool errors only when the observed terminal exactly matches a declared non-verified expectation; completed verification, failed/provider terminals, mismatches, scope, acceptance, resources, and evidence remain fail-closed. A focused blocked/no-mutation fixture with one tool error passes; runner 9/9, replay 9/9, and the complete static harness suite pass; docs paths and diff checks pass.
FAILURES: none
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-007

```text
STATUS: accepted
NODE: XF-007
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: X12 attempts 1 and 3 each recorded different `/usr/bin/aletheon` digests and core-RPC closure after another checkout deployed mid-suite. The deploy command now holds an exclusive runtime-mutation flock for its complete build/install/restart/verify transaction. A suite using exactly `/usr/bin/aletheon` holds the matching shared flock for its complete catalog; debug/custom binaries do not participate. ALETHEON_RUNTIME_LOCK_FILE provides an isolated test override. Suite tests 6/6 prove the installed shared lease blocks a nonblocking exclusive contender and a custom binary creates no lock. Full static harness, runtime-generation static contract, sudo user-context contract, shell syntax, docs paths, and diff checks pass.
FAILURES: none
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-008

```text
STATUS: accepted
NODE: XF-008
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: The integration architecture lane initially failed because crates/corpus/src/security/runner.rs had grown to 2,151 lines against its governed 2,042-line hotspot limit. The production module was only 1,225 lines; 924 lines were its inline private test module. The test module now lives at crates/corpus/src/security/runner/tests.rs while preserving the same module privacy, names, and tail-test nesting. The production runner is 1,226 lines. The complete architecture suite passes, all 29 focused Runner tests pass, Corpus all-target clippy with -D warnings passes, and workspace formatting and diff checks pass.
FAILURES: initial architecture gate exited 1 with hotspot budget exceeded; the post-split gate exits 0 with all six hotspot ownership budgets verified.
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-009

```text
STATUS: accepted
NODE: XF-009
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Installed artifact target/x12-xf005-installed-probe-20260805-225057/rust_bugfix.json used binary SHA 4b0a695cd9c29ad253e1b77113f0b844b51db6ba4f0573e241554cf57e5db606 with stable daemon generations. It changed only src/lib.rs to the correct boundary implementation and its second sandboxed cargo test succeeded. The run nevertheless reached 300 seconds without a terminal envelope; stderr records the enforced cognitive completion gate still missing RequiredAction::AcceptChange. Production registry/profile contracts intentionally keep change_accept Host-only, so the model cannot satisfy that obligation. Cognit now requires only the exact version-bound diff review and required validation before emitting its candidate; acceptance/repair remains a subsequent Host-owned action. Four focused change-transaction tests, the full-loop exact-closure regression, Cognit all-target clippy with -D warnings, formatting, documentation paths, and the complete architecture suite pass.
FAILURES: none; the earlier timeout is retained as pre-fix evidence, while the final installed catalog reached authoritative terminal receipts for its version-bound code-change tasks
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-010

```text
STATUS: accepted
NODE: XF-010
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Immediately after installed deployment, the monitor health RPC reached /run/user/1000/aletheon/aletheon.sock and reported daemon readiness=ready, yet returned systemd.active=false. The authoritative host command reported aletheon.service active with NRestarts=0. /proc evidence shows the installed MCP monitor process has HOME/USER but no XDG_RUNTIME_DIR or DBUS_SESSION_BUS_ADDRESS, so its inherited systemctl --user probe targets no usable user manager. Monitor health and diagnose now derive missing XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS from the numeric UID in the authoritative user socket while preserving explicit operator values. Focused health/diagnose tests pass 27/27; the complete monitor suite passes 88/88; Python compilation, documentation paths, and diff checks pass.
FAILURES: none; the pre-fix monitor contradiction is retained as diagnostic evidence, and the deployed post-suite verifier observed the authoritative active/stable units through the official runtime
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-011

```text
STATUS: accepted
NODE: XF-011
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: `EventSourcedSessionStore` is now the only production `SessionAppendStore`; `CanonicalSessionStore` exposes only read and private materializer capabilities. Principal binding, Host Task projection and startup Turn recovery are ordered EventSpine facts; replay updates the SQLite read model, generation-safe recovery emits a typed terminal fact, and terminal snapshots clear live children/commands/approvals and mark unfinished activities lost. Session schema v5 and database migration v6 are registered. Fabric Session contract 3/3, Session recovery 2/2, reconnect/replay 8/8, Canonical recovery 9/9, U-RESUME 2/2, Executive check, architecture, formatting and diff checks pass locally.
FAILURES: the initial Session contract run detected that the checked-in v5 schema predated the recovery variant; regeneration from the exporter fixed the mismatch and the rerun passed 3/3.
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-012

```text
STATUS: accepted
NODE: XF-012
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: Agent settlement/reparent/recovery control evidence now uses `aletheon.event.agent_settlement/v1`, not the public Session `turn.event/v1` ItemRecord schema. Focused coverage proves the schema separation and U-RESUME-005 proves stale-generation rejection writes a durable typed audit event. Executive check, U-RESUME tests, architecture, formatting and diff checks pass locally.
FAILURES: none
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-013

```text
STATUS: accepted
NODE: XF-013
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: U-TUI-002 now folds multiple streaming chunks and observes the real `cancel` request while the app remains active pending acknowledgement. U-TUI-006 disables every theme color under `NO_COLOR` and exercises prompt, session list, resume, help and quit through text-only keyboard/line mode. README Stable rows now name separate production, E2E and recovery anchors; A-DELETE-002 verifies every path, executable E2E coverage, recovery semantics and removal of obsolete authority claims. Interact all-target check, U-TUI 7/7, Metacog deletion 2/2, architecture, formatting and diff checks pass locally.
FAILURES: none
BLOCKER: none; final installed closure is recorded under X12 (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/`)
MERGE_SHA: pending_by_owner
```

## XF-014

```text
STATUS: accepted
NODE: XF-014
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
REQUIREMENT: recovered tool errors stay separately counted while typed provider/runtime failures and failed Host gates fail closed (`docs/plans/Aletheon_Unified_Execution_Plan_2026-08-04.md:469`)
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: `tests/coding/harness/receipt.py` no longer treats the aggregate diagnostic `tool_errors` counter as an independent terminal authority; authoritative terminal, independent acceptance, workspace policy, resource cleanup and correlated evidence remain separate gates. Python runner/replay/suite regression tests pass 29/29. The installed final-SHA catalog at `target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/x12-gate-fix-deploy-230003/final-20-task-235033/suite.json` records tool errors separately (average 0.2) while achieving 19/20; the single acceptance failure remains fail-closed and is not reclassified as success.
FAILURES: none
BLOCKER: none
MERGE_SHA: pending_by_owner
```

## X13

```text
STATUS: accepted
NODE: X13
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
REQUIREMENT: Robot reuses Task/Activity/Receipt, completes MuJoCo, and projects safety denial as blocked; physical hardware is excluded (`docs/plans/Aletheon_Unified_Execution_Plan_2026-08-04.md:126,293-296`; `docs/plans/Aletheon_Architecture_Stabilization_and_Convergence_Plan_2026-08-04.md:857-875`; `docs/plans/Aletheon_User_Experience_and_Engineering_Workflow_Plan_2026-08-04.md:839-859`)
EVIDENCE:
  - immutable `RobotEpisodeReceipt` projects through the canonical Task/Activity mainline as Observe -> Plan -> Authorize -> Execute -> Verify -> Settle; completed/failed receipts map to Host-owned accepted/blocked settlement (`crates/executive/src/adapters/events/session_projection.rs:524-553,564-780`)
  - hardware remains a typed Robot activity rather than Bash/MCP tool, and safety denial is tested as blocked without a Tool activity (`crates/executive/src/adapters/events/session_projection.rs:1011-1115`)
  - TUI renders device, scene, bridge digest, attempts, report digest and content-addressed evidence from the same projection (`crates/interact/src/tui/task_console.rs:273-301,474-501,717-805`)
VALIDATION / RUNTIME EVIDENCE:
  - the current installed SHA `038986c29aaf37680d0a8ce0702235bbf50c13b4df8aac9dd73be80730007b1a` completed three consecutive real-TUI stance journeys through the official socket with no rendered/monitor/provider error; every report executed exactly `kuavo.stance {}`, bound a real operation ID, matched all base-twist paths for 3000 ms, retained a MuJoCo frame, and settled `completed`
  - the current negative real-TUI journey requested an unavailable unsafe action; typed `SafetyFallback` was projected as blocked, Host recorded `proposal_rejected`, executed zero attempts, completed `safe_stop`, and settled `failed` without provider failure or unsafe motion
  - current final summary and immutable reports: `target/r8-x13-installed-20260806-174928/x13-current-sha-20260807-003254/x13-final-summary.json`, `tui-robot-{1,2,3}.report.json`, and `tui-robot-negative.report.json`; positive and negative report/SQLite receipts both report `REPORT_EVIDENCE_PASS`
FAILURES: none
BLOCKER: none
MERGE_SHA: pending_by_owner
```

## X14

```text
STATUS: accepted
NODE: X14
BASE / BRANCH / PR / GOAL_ID: d01e118b / auro/feat/20260805-x4c-run-exec-wire / deferred_by_owner / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: CompositeMemoryService now preserves supplemental adapter-provided workspace/session scope, provenance and authority instead of rewriting every external result as session-local. A-MEM-001 session isolation, A-MEM-002 approved-core versus local fact distinction, A-MEM-003 explicit supplemental outage/degraded fallback, and the workspace-bound supplemental provenance regression all pass (11 unified memory contract tests); mnemosyne clippy and architecture acceptance pass.
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R0

```text
STATUS: accepted
NODE: R0
BASE / BRANCH / PR / GOAL_ID: c080b08bf3170dd8a09acdb738a255133abbf11b / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
EVIDENCE:
  - docs/testing/robot-runtime.md (tracked since 9e4c4ad3cd7d5adf6e3b5d329de732c7c475d011)
  - bridge 0.1.0 @ 1245538771b80f1b742d50f32041b801fba7b104; clean at sampling; origin not configured
  - proto SHA-256 4a205a75ac7643d7769fbd7bd52f32faba64b4cdf9d81f6908617da490a7d7ff; Aletheon/bridge copies byte-identical
  - kuavo_assets 10.3.0 / kuavo-mujoco/default-v40 @ 5fc79a6c48d5d7296763663d71fa52e0ff54d624
  - scene.xml SHA-256 052b37031446d5bc142eaf9d264711adc5bd57defeb1bc5c0a178fc747e4b25c
  - 63-file asset manifest SHA-256 5364678863725a47494cc9c734f20a02b864a5f7d16480bd1f57fe3cdd9e206d
VALIDATION:
  - external version facts re-read from local independent repositories on 2026-08-06
  - protocol copies compared byte-for-byte; deterministic asset manifest regenerated
RUNTIME EVIDENCE:
  - this baseline node itself issues no action; the later accepted R8 run used the frozen Robot version 53 scene/protocol facts through the real installed chain (`docs/plans/execution-status.md:802-825`)
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R1

```text
STATUS: accepted
NODE: R1
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
EVIDENCE:
  - typed RobotIntegrationConfig/RobotPolicyConfig/RobotPerceptionConfig and resolved build facts
  - ALETHEON_POLICY_ENDPOINT normalized only into the typed environment layer; daemon bootstrap has no direct environment read
  - explicit RobotHarness/perception injection; no production RobotHarnessConfig::default()
  - config effective output exposes safe endpoint/device/scene; URI-embedded secrets are rejected
VALIDATION:
  - robot_typed_config 5/5; layered_config_contract 10/10; schema deterministic 1/1
  - Cognit policy provider 12/12; Robot composition 2/2; embodiment config 7/7; harness selection 3/3
  - Executive/Cognit all-target checks; architecture-check (22 migrations, 62 acceptance IDs, 1067 Fabric public types); fmt/diff checks
RUNTIME EVIDENCE:
  - the accepted R8 run resolved this typed config into the installed daemon and proved the effective device/scene/Policy/Bridge identities (`docs/plans/execution-status.md:802-825`)
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R2

```text
STATUS: accepted
NODE: R2
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
EVIDENCE:
  - Policy connect runs Health + GetCapabilities before exposure and keeps PolicyCapabilitySnapshot
  - typed PolicyStartupError distinguishes endpoint/connection/health/capability/protocol failures
  - Bridge startup uses read-only Health/GetCapabilities/ListSkills and validates protocol digest, required device, non-empty allowlist, bounded closed skill schemas, required observation schemas and max-message facts (`crates/hardware/src/grpc/provider.rs:154-265`)
  - missing required device fails before network access; all other startup incompatibilities have distinct typed errors (`crates/hardware/src/grpc/provider.rs:80-109`)
  - the boundary adapter validates descriptor schemas and derives an order-stable descriptor digest (`crates/fabric/src/types/embodiment.rs:62-104`)
  - exact startup allowlist and protocol digests flow into episode provenance (`crates/executive/src/application/robot_harness_composition.rs:223-224`, `crates/cognit/src/harness/robot/session.rs:127-128`, `crates/fabric/src/types/episode_report.rs:64-65`)
  - daemon bootstrap logs typed Policy/Bridge capability facts; startup gates issue no robot action
  - read-only real-Bridge preflight reached Health/GetCapabilities/ListSkills/Snapshot through an operator-controlled tunnel; no actuation RPC was issued
VALIDATION:
  - Cognit Policy provider 12/12; Hardware capability unit 5/5; grpc_provider 6/6 plus one ignored operator-gated live diagnostic
  - bridge pytest 90/90 (6 skipped), changed-file Ruff and compileall; local cross-language bridge/Rust startup gate passed
  - candidate Aletheon/bridge proto copies are byte-identical at SHA-256 `9517981e556d8c4320fcfc3d686bfa00c0963a7238c024828002fd1461587147`
  - Cognit/Hardware/Executive all-target checks; architecture/fmt/diff checks
RUNTIME EVIDENCE:
  - the earlier read-only old-Bridge preflight remains diagnostic only (`target/r2-readonly-preflight-20260806-074707/summary.json`)
  - the accepted R8 deployment superseded it with the candidate Bridge protocol `77e44869...`, READY health, device `kuavo-mujoco-01`, bounded descriptors/observation schemas and a real Policy capability identity; those startup facts were bound into the installed reports (`docs/plans/execution-status.md:802-825`)
FAILURES: none; the old incompatible preflight was not counted as acceptance
BLOCKER: none
MERGE_SHA: pending
```

## R3

```text
STATUS: accepted
NODE: R3
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
EVIDENCE:
  - Cognit owns the abstract RobotPerceptionPort and Plan consumes non-empty validated observations (`crates/cognit/src/harness/robot/mod.rs:32-40`, `crates/cognit/src/harness/robot/mod.rs:269-347`)
  - FrameRef/PerceptionObservation retain typed device/schema/source/sequence/captured/received/confidence/byte/digest metadata (`crates/fabric/src/types/frame.rs:11-30`, `crates/fabric/src/types/perception_observation.rs:9-25`)
  - Hardware converts wire Unix timestamps at an RPC receive anchor instead of treating epoch milliseconds as MonoTime (`crates/hardware/src/grpc/convert.rs:99-166`)
  - Executive cache enforces freshness, device/schema-source sequence isolation, URI authority, count and byte budgets (`crates/executive/src/application/robot_perception.rs:25-184`)
  - visual payload is reduced to bounded refs/digest before world snapshot/prompt; selected Host-owned frames enter proposal/report provenance (`crates/executive/src/application/world_state.rs:146-177`, `crates/fabric/src/types/episode_report.rs:58-72`)
  - typed per-skill schema/version requirements permit nonvisual skills and fail closed for required visual input (`crates/executive/src/composition/config/robot.rs:101-119`, `crates/cognit/src/harness/robot/mod.rs:297-309`)
VALIDATION:
  - Fabric frame 7/7, SkillProposal 7/7, EpisodeReport 3/3; Hardware conversion 10/10
  - Cognit Robot perception 3/3, proposal validator 6/6, Policy provider 2/2
  - Executive perception 2/2, world-state 10/10, typed config 6/6, layered config 10/10, Robot E2E 1/1
  - Dasein visual aggregator 8/8 + integration 1/1; five scoped all-target checks; architecture/fmt/diff checks
RUNTIME EVIDENCE: the installed R8 positive/stance reports retain a non-empty bounded `mujoco-simulator-screen` FrameRef (URI/digest/size/time/camera/frame identity), and the real Policy provenance is bound to the resulting proposal/report (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/positive-report.json`; `r8-stance-report.json`)
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R4

```text
STATUS: accepted
NODE: R4
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
EVIDENCE:
  - Policy protobuf now carries bounded typed snapshots alongside goal/device/visual refs/allowlist/protocol (`crates/cognit/proto/aletheon/policy/gateway/v1/policy.proto:23-59`)
  - GrpcPolicyProvider has typed startup/request timeout and response failures, host-binds negotiated provider/protocol, rejects identity spoofing/empty/over-limit responses (`crates/cognit/src/adapters/policy/grpc_provider.rs:218-475`)
  - validator uses requested device + live descriptor + fresh typed snapshots, checks JSON schema/outcome path and type/descriptor timeout caps/finite confidence/provenance (`crates/cognit/src/harness/robot/proposal_validator.rs:16-235`)
  - Plan records bounded rejection evidence and accepted provenance; EpisodeReport persists the host-bound provider/model/version/protocol/digest (`crates/cognit/src/harness/robot/mod.rs:321-408`, `crates/fabric/src/types/episode_report.rs:58-101`)
  - production Stub Policy types were removed; Policy boundary has no execution capability
VALIDATION:
  - Cognit real in-process gRPC gateway 4/4, proposal validator 10/10, Plan failure/perception 5/5
  - Executive Policy boundary 4/4, WorldState freshness 4/4, Robot session 1/1
  - Fabric SkillProposal 9/9, EpisodeReport 3/3, ExpectedOutcome 9/9
  - four scoped all-target checks plus architecture/fmt/diff checks
RUNTIME EVIDENCE: the installed daemon called the real `aletheon-vla-lejurobot` Policy gateway (`deepseek-v4-flash`, protocol `1.0`, version/digest retained in receipts); a direct proposal produced the exact accepted timed skill, while typed `SafetyFallback` on the out-of-bounds goal was rejected before execution (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/positive-report.json`; `negative-final-report.json`)
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R5

```text
STATUS: accepted
NODE: R5
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
EVIDENCE: typed failure/history (`crates/fabric/src/types/robot_failure.rs:8-69`), finite ReplanContext and separate budget transitions (`crates/cognit/src/harness/robot/state.rs:52-123`), typed Policy prior-attempt wire (`crates/cognit/proto/aletheon/policy/gateway/v1/policy.proto:23-62`), validated replan/loop detector/recovery/safe-stop settlement (`crates/cognit/src/harness/robot/mod.rs:372-490,646-877`), and cancellation/report mapping (`crates/cognit/src/harness/robot/session.rs:144-231`) implement spec:docs/plans/robot-vla-production-closure-plan.md:559-589
VALIDATION / RUNTIME EVIDENCE: Cognit R5 recovery 5/5, gRPC Policy 5/5, state 9/9, perception 5/5, proposal outcome 1/1; Executive robot session E2E 3/3; Fabric failure serde 1/1 and EpisodeReport 3/3; Fabric/Cognit/Executive all-target checks plus fmt/architecture/diff passed. The installed negative lane additionally proves a typed `proposal_rejected` history, zero unsafe attempts, bounded transition to SafeStop, failed settlement, terminal `safe_stop succeeded`, and durable restart reconstruction (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/negative-final-report.json`)
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R6

```text
STATUS: code_complete
NODE: R6
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
EVIDENCE:
  - provider-attested simulation/HIL/real and the complete safety manifest fail closed by environment (`crates/fabric/src/types/embodiment.rs:17-167`, `crates/hardware/src/grpc/provider.rs:504-568`)
  - HIL/real config requires pinned serial/manifest/limits/evidence and bootstrap compares the live manifest before provider registration; simulator profiles cannot carry or claim those gates (`crates/executive/src/composition/config/robot.rs:28-85,438-632`, `crates/executive/src/host/daemon/bootstrap/embodiment.rs:85-237`)
  - Bridge environment is provider/build-owned and YAML spoofing is rejected; its heartbeat owner, request registration and local monotonic watchdog block lost/competing ownership and stop locally on heartbeat/deadline/lease expiry (`aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/config.py:23-26,100-122,341-362`, `aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/watchdog.py:44-190`, `aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/grpc_service.py:236-268,320-400`)
  - Broker rejects non-exclusive/expired/mismatched leases before provider dispatch (`crates/hardware/src/broker.rs:104-135,257-272`)
  - high-risk skills require an exact request-bound, expiring operator receipt; the default adapter denies and confidence is absent from the approval contract (`crates/executive/src/application/embodiment_approval.rs:10-85`, `crates/executive/src/application/embodiment_service.rs:93-177`)
  - Aletheon validates the negotiated JSON schema and forbids raw actuation, while the Bridge/driver rejects rather than clamps values beyond compiled/deployment/reviewed limits; ROS command-owner discovery parses the XML-RPC envelope, removes only the Bridge itself and fails closed on an invalid master response (`crates/cognit/src/harness/robot/proposal_validator.rs:140-163,417-449`, `aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/skills/move_base_timed.py:18-112,254-281`)
  - local emergency stop is latched and invokes a direct device callback without gRPC/Agent/LLM (`aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/safety.py:78-107`, `aletheon-kuavo-bridge/tests/fault_injection/test_safety.py:129-140`)
VALIDATION / RUNTIME EVIDENCE:
  - Bridge: full pytest 105 passed / 6 skipped; command-owner regression 4/4 and targeted Ruff passed; canonical proto copies match SHA-256 77e44869ba6a6b3345753f699f9acd403a6c8f4de3383dcd5827b0ba7673e679
  - Rust: Fabric embodiment 7/7; Hardware all targets 72 passed / 3 ignored; Kernel all targets 72/72; Executive profile 11/11 + approval/bounds 4/4 + pin/receipt unit tests; Cognit proposal validator 11/11 plus integration 2/2
  - a local fake-Bridge process passed the ignored Rust startup diagnostic 1/1 and was stopped after the check
  - Fabric/Hardware/Kernel/Executive all-target checks, config-schema snapshot, architecture suite, fmt and both-repository diff checks passed
  - evidence level is local deterministic/in-process only; no physical-device or installed HIL/real claim
FAILURES: the authorized old-Bridge diagnostic exposed fail-open ROS command-owner parsing; that diagnostic is retained as failed safety evidence. The candidate fix was subsequently deployed and the R8 simulation rerun passed, but repository-wide Bridge Ruff/mypy are not claimed because generated files and the existing Python/ROS typing baseline remain outside this scoped acceptance
BLOCKER: none for `code_complete`; physical-device independent-hard-stop/watchdog evidence is still required before R6 can become `accepted`/real-ready, and remains outside R8/X13 MuJoCo scope
MERGE_SHA: pending
```

## R7

```text
STATUS: accepted
NODE: R7
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
EVIDENCE:
  - the report contract carries complete external artifact manifests, explicit legacy-metadata incompleteness, before/after/verified sequences, ordered attempt/operation identity, verification paths, typed settlement, model/bridge/scene/descriptor provenance and an integrity-checked immutable settled receipt (`crates/fabric/src/types/episode_report.rs:37-78,87-325,335-428,439-720`)
  - RobotCognitiveSession rebuilds every durable attempt, preserves frame/evidence refs across retries, persists the receipt, records it through the immutable audit port, promotes only a verified matched receipt, and derives TurnStop from the same settlement (`crates/cognit/src/harness/robot/mod.rs:104-164`, `crates/cognit/src/harness/robot/session.rs:84-180,224-281`)
  - SQLite provides idempotent append/update/close, zero-attempt settlement, legacy snapshot-to-sequence migration, immutable report/tombstone tables, restart reconstruction and retention-aware read projection; content-digest reconciliation closes a crash between local artifact expiry and the episode projection write (`crates/executive/src/adapters/episode/migrations/001_episodes.sql:7-86`, `crates/executive/src/adapters/episode/sqlite_episode_sink.rs:103-428,431-943`)
  - ArtifactStore migration 18 durably marks expiry before deleting bytes, rejects tombstone mutation/resurrection, reconciles post-commit deletion crashes on reopen, and retains digest/provenance for report projection (`crates/executive/src/compatibility/persistence_migrations.rs:518-530`, `crates/executive/src/adapters/artifact/store.rs:58-107,273-430`)
  - deterministic verification reports the predicate paths actually evaluated, rather than an empty placeholder (`crates/executive/src/application/deterministic_outcome_verifier.rs:47-137,179-270`)
  - audit and Mnemosyne promotion independently verify the immutable receipt; failed episodes retain evidence and cannot create a success fact (`crates/executive/src/application/robot_audit.rs:77-100,259-264`, `crates/executive/src/application/robot_episode_promotion.rs:24-50`)
VALIDATION / RUNTIME EVIDENCE:
  - Fabric EpisodeReport 9/9; Executive verifier 6/6, SQLite sink 8/8, artifact retention 1/1, promotion 3/3, audit 5/5 and Robot session E2E 3/3; Cognit robot harness/perception/replan/proposal/Policy targets 27/27
  - Fabric/Cognit/Executive all-target checks, architecture suite (22 migrations, 62 acceptance IDs, 1079 Fabric public types), fmt and diff check passed
  - installed `/usr/bin/aletheon` produced immutable positive, stance and negative EpisodeReports through real Policy/Bridge/ROS/MuJoCo; `scripts/libexec/aletheon/robot_r8_evidence.py:76-310` cross-checked every request/attempt/sequence/provenance/safe-stop fact against SQLite, and the positive/negative/stance receipts were read again after daemon restart (`target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/sqlite-episode-reports-after-restart.txt`)
FAILURES: no scoped R7 failure
BLOCKER: none; the installed durable-report/restart condition is closed by R8 evidence, while physical-device retention policy remains outside this MuJoCo phase
MERGE_SHA: pending
```

## R8

```text
STATUS: accepted
NODE: R8
BASE / BRANCH / PR / GOAL_ID: 9c60ae25775a1c02f1b741a1cfaa1321209b7b88 / auro/acceptance/20260805-x12-mainline / #182 (target dev) / 019fd228-80c6-7bd0-8ea7-15481e8738d1
BUDGET: goal mode; no explicit token budget
REQUIREMENT: full installed `/usr/bin/aletheon` -> official socket -> installed daemon -> RobotCognitiveSession -> real Policy -> Kernel/Hardware -> candidate Bridge -> ROS/MuJoCo -> stable verifier -> durable report, including safe-stop negative and restart reconstruction (`docs/plans/robot-vla-production-closure-plan.md:763-793,928-941`)
EVIDENCE:
  - candidate Bridge protocol `77e44869ba6a6b3345753f699f9acd403a6c8f4de3383dcd5827b0ba7673e679`, Robot version 53 MuJoCo scene `kuavo-mujoco/biped-s53`, real Policy provider `aletheon-vla-lejurobot` / model `deepseek-v4-flash` / protocol `1.0`, and skill descriptor digest `965d358f3c4487bcd4716609a27dde9edf05fa6b026a405d358d78275f887b60` are bound into each report/receipt
  - R8 installed gate log and receipt: `target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/r8-final-installed-acceptance.log` and `r8-final-installed-acceptance-receipt.json`
  - direct goal-alignment positive report/receipt: `target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/positive-report.json` and `positive-evidence-receipt.json`
  - exact required stance report/receipt: `target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/r8-stance-report.json` and `r8-stance-evidence-receipt.json`
  - safety-negative report and pre/post-restart receipts: `target/r8-x13-installed-20260806-174928/goal-contract-deploy-221255/negative-final-report.json`, `negative-final-evidence-receipt.json`, and `negative-final-evidence-receipt-after-restart.json`
  - current-SHA final positive/negative receipts: `target/r8-x13-installed-20260806-174928/x13-current-sha-20260807-003254/r8-final-positive-receipt.json` and `r8-final-negative-receipt.json`
VALIDATION / RUNTIME EVIDENCE:
  - at the Robot acceptance run, release, `/usr/bin/aletheon`, machine core, user daemon and Memory Agent matched SHA-256 `c53a3eb5c42320c7ce2ff2565113ae4fbb3f1d8d43967e0d514f62fb377210a1`; restart counters were stable and the official socket real-request smoke passed before the evidence checker (`deploy-final.log`, `deploy-provenance-followup.log`, `r8-final-installed-acceptance.log` in the evidence directory)
  - the required stance request executed exactly `kuavo.stance {}`, then matched zero base-twist paths for a 3000 ms stable window with sequences 2497221 -> 2497393 -> 2498086 and settlement `completed`
  - the controlled positive action executed exactly `kuavo.move_base_timed {linear_x:0.1, linear_y:0, angular_z:0, duration_ms:1000}`, produced ground-truth displacement, matched verification, returned to zero speed and settled `completed`
  - the operator's out-of-bounds/multi-action request was never executed: Policy returned typed `SafetyFallback`, Host rejected it as non-direct, the report contains zero attempts, settlement `failed`, `proposal_rejected`, and terminal `safe_stop outcome=succeeded`; the persisted receipt passed again after daemon restart
  - `scripts/libexec/aletheon/robot_r8_evidence.py:76-310` validates report/request identity, operation uniqueness, sequence order, stable-window result, provenance, artifacts and safe-stop facts against SQLite; all retained receipts report `REPORT_EVIDENCE_PASS`
  - final current-SHA acceptance revalidated release, `/usr/bin/aletheon`, machine core, user daemon and Memory Agent at SHA-256 `038986c29aaf37680d0a8ce0702235bbf50c13b4df8aac9dd73be80730007b1a`, stable restart counters, Memory Agent protocol smoke, and a real LLM request through the official user socket before checking the current positive receipt (`x13-current-sha-20260807-003254/r8-final-positive-acceptance.log`)
  - three consecutive current-SHA real-TUI positive runs completed with operation-bound `kuavo.stance {}` and 3000 ms verification; a current-SHA unsafe/unavailable request produced typed `SafetyFallback` -> blocked `proposal_rejected` -> zero attempts -> `safe_stop succeeded`; both current SQLite receipts report `REPORT_EVIDENCE_PASS`
FAILURES: none; the earlier old-Bridge diagnostic remains retained as non-acceptance evidence and was superseded by the candidate-Bridge installed run
BLOCKER: none; physical hardware/HIL remains a separate post-MuJoCo safety gate and is not an R8/X13 prerequisite
MERGE_SHA: pending
```

## C 轨道（只读摘要）

C0–C7：`accepted`；权威证据见 `deepseek-cache-and-message-optimization-plan.md:48-60`。
