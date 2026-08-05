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
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: one Fabric CommandSpec catalog now materializes top-level Clap help, TUI slash metadata, Gateway command resolution, and generated Bash/Zsh completion trees; TUI, line mode, one-shot CLI, canonical local RPC, legacy chat/status, and Gateway prompt routes converge on ClientIntent and Executive CommandDispatcher; U-CLI-001/002 and PRODUCTION_CLI_PARSERS=0 gates pass; Fabric/Gateway/Interact/Aletheon and affected Executive tests, package Clippy -D warnings, architecture, formatting, completion, diff, and doc-path checks pass; system deploy passed with release/installed/machine-core/user-daemon SHA-256 cfbfb6127f6a9fa64c4487ae136307bdc988371c1c581e9bafbdaa894d2eb9a5, zero restart counters, stable services, and an official-socket real LLM request; GitHub CI 5 passed and 4 skipped
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
STATUS: code_complete
NODE: X6b
BASE / BRANCH / PR / GOAL_ID: 9546dbb0715536adf4cb9ce1361afc65a4578869 / auro/feat/20260805-x6b-secure-input-clean-v2 / deferred_by_owner / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=2
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: The X6a sanitizer, IME/paste handling, Action Palette, bounded @ discovery, typed workspace attachments, and scoped history/draft storage remain intact. X6b closes the remaining governed ! path through canonical command intent, explicit confirmation, Host capability execution, typed progress/terminal receipt projection, and persisted shell receipt; input-state retention now removes expired entries fail-closed. U-INPUT-001..006 are bound in acceptance-ids.tsv. Interact 156/156 library tests and Executive command-dispatcher 3/3 pass; Interact and Executive clippy -D warnings, architecture acceptance, and fmt pass. `sudo bash scripts/aletheon.sh deploy` passed from this branch: target/release, /usr/bin, machine core, user daemon, and Memory Agent share SHA-256 96e274c286b6e8eaf41298e46e79f1d2a0403c8eab6f51b207894c58ed43c2ea; all three services are active with NRestarts=0; official Memory Agent protocol and official user-socket real LLM request passed.
FAILURES: Initial full Interact run found the exact public builtin snapshot omitted the intentional shell command; commit 90b5cd8b updated the canonical expectation and the rerun passed 156/156.
BLOCKER: publication/PR/merge intentionally deferred by owner; implementation and installed evidence are complete.
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
STATUS: code_complete
NODE: X11
BASE / BRANCH / PR / GOAL_ID: 6ffa9f85f2d9b4776b26b2781eb62c214a62e67d / auro/test/20260805-x11-rubric-closure / pending / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=2
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: tests/coding now has exactly 20 versioned task/fixture/acceptance/rubric sets. The catalog mechanically enforces the required 5 location/explanation, 5 small bug fix, 4 cross-file, 2 failing-test, 2 review-finding, 1 side-effect-free session resume, and 1 child-orphan reconciliation distribution. Every rubric is schema version 1, totals 100 points, and binds terminal, correctness, scope, and governance evidence. Static harness contracts, runner/replay/suite/workflow tests, exact inventory checks, rubric validation, and git diff checks pass; all 20 isolated fixture baselines compile through scripts/cargo-agent.sh and each has its own workspace manifest and lockfile.
FAILURES: PR #180 X11 draft had 20 tasks but only 19 acceptance overlays, zero scoring rubrics, no scenario-distribution gate, and no session-resume or orphan-reconciliation task. This repair replaces that incomplete corpus; full 20-task real-model execution remains deferred to X12 installed acceptance.
BLOCKER: corrective branch is local by owner instruction; dev merge pending.
MERGE_SHA: pending
```

## X12

```text
STATUS: in_progress
NODE: X12
BASE / BRANCH / PR / GOAL_ID: 9546dbb0715536adf4cb9ce1361afc65a4578869 / auro/acceptance/20260805-x12-mainline / deferred_by_owner / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=2
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: U-INST-001/002 system deployment and official-socket smoke were reached on the integrated X branch. Attempt 1 is invalid because the runtime changed and XF-001 found a canonical terminal parser mismatch. Attempt 2 used the corrected 20-task corpus and parser, but was stopped after its first task produced authoritative evidence for XF-002 rather than spending the remaining suite budget against a broken sandbox environment.
FAILURES: Attempt 1 exposed canonical terminal parsing and mid-suite restart invalidators. Attempt 2 `api_error_mapping` timed out after the installed auto sandbox cleared PATH/toolchain identity; the model repeatedly searched for cargo/rustc and could not perform its normal build loop. No result from either attempt is counted toward the 16/20 gate.
BLOCKER: XF-002 must complete installed acceptance, then the full suite must rerun from a new artifact root without changing binary or daemon generation.
MERGE_SHA: pending
```

## XF-001

```text
STATUS: code_complete
NODE: XF-001
BASE / BRANCH / PR / GOAL_ID: 9546dbb0715536adf4cb9ce1361afc65a4578869 / auro/acceptance/20260805-x12-mainline / deferred_by_owner / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: tests/coding/harness/run.py now consumes canonical ExecEventEnvelope v1 schema_version/type/status/operation_id and nested TurnMetrics. Receipt validation preserves every classified terminal kind and treats provider_unavailable/provider_rejected/validation_failed/output_backpressure as explicit execution failures. Fake-client runner, replay, suite, receipt, workflow, and full static harness tests pass.
FAILURES: none
BLOCKER: publication/PR/merge intentionally deferred by owner; X12 rerun remains pending.
MERGE_SHA: pending_by_owner
```

## XF-002

```text
STATUS: code_complete
NODE: XF-002
BASE / BRANCH / PR / GOAL_ID: 9546dbb0715536adf4cb9ce1361afc65a4578869 / auro/acceptance/20260805-x12-mainline / deferred_by_owner / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: X12 attempt 2 receipt target/coding-x12-corrected-20260805-2052/receipts/api_error_mapping.json records the original installed `/usr/bin/aletheon --sandbox auto` timeout. The repair constructs an explicit non-secret sandbox environment allowlist, excludes provider credentials/wrapper injection, gives the installed user service a conventional user-tool PATH, and restores per-turn workspace writable binds under configured profiles before metadata re-protection. Environment tests, 8 bubblewrap tests including live write/deny behavior, Corpus all-target clippy `-D warnings`, systemd boundary checks, docs paths, and formatting pass. `sudo bash scripts/aletheon.sh deploy` passed with release/installed/all-running executable SHA `caf149d3c04d5243e9c7c3d66ccdb4d0923018bf0fd6103683ccfca15cbd6db3`, stable restart counters, Memory Agent smoke, and official-socket real request. Installed artifact `target/xf-002-installed-pass-20260805-212244/exec.json` proves auto sandbox resolved `/home/aurobear/.cargo/bin/cargo`, completed `cargo check`, and denied a write to a pre-existing path outside WorkspacePolicy; host content remained `UNCHANGED`.
FAILURES: none
BLOCKER: publication/PR/merge intentionally deferred by owner; X12 full-suite rerun remains pending.
MERGE_SHA: pending_by_owner
```

## XF-003

```text
STATUS: code_complete
NODE: XF-003
BASE / BRANCH / PR / GOAL_ID: adf35191b1a5391b147d00f3cef42dc7c9134b37 / auro/acceptance/20260805-x12-mainline / deferred_by_owner / external supervisor active
BUDGET: token/cost/deadline unbounded_by_owner; max_attempts=3; attempt=1
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: The retained installed rust_bugfix receipt from the concurrent X12 diagnostic run shows `src/lib.rs` plus the complete `target/` tree in `workspace.changed_files`. The task requires only `src/`, so ordinary in-workspace Cargo validation becomes a false policy_scope_failure even when the source repair is correctly scoped. The harness now adds `/target/` to each temporary repository's private `.git/info/exclude` before the fixture baseline commit; no fixture source or production policy is specialized. A focused regression proves `src/lib.rs` remains visible while a synthetic `target/debug/artifact` is excluded. Runner tests 8/8 and the full static harness suite pass; docs paths and diff checks pass.
FAILURES: none
BLOCKER: publication/PR/merge intentionally deferred by owner; X12 rerun remains pending.
MERGE_SHA: pending_by_owner
```

## X13

```text
STATUS: not_started
NODE: X13
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
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
STATUS: in_progress
NODE: R0
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: bridge version/proto digest/scene version not recorded
MERGE_SHA: pending
```

## R1

```text
STATUS: not_started
NODE: R1
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R2

```text
STATUS: not_started
NODE: R2
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R3

```text
STATUS: not_started
NODE: R3
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R4

```text
STATUS: not_started
NODE: R4
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R5

```text
STATUS: not_started
NODE: R5
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R6

```text
STATUS: not_started
NODE: R6
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R7

```text
STATUS: not_started
NODE: R7
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## R8

```text
STATUS: not_started
NODE: R8
BASE / BRANCH / PR / GOAL_ID: pending
BUDGET: pending
EVIDENCE / VALIDATION / RUNTIME EVIDENCE: pending
FAILURES: none
BLOCKER: none
MERGE_SHA: pending
```

## C 轨道（只读摘要）

C0–C7：`accepted`；权威证据见 `deepseek-cache-and-message-optimization-plan.md:48-60`。
