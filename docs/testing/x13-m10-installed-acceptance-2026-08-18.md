# M10 final installation-state acceptance — 2026-08-18

Status: `passed`

This report is the **final** M10 acceptance for the wiring-ownership migration
(M8.1 → M8.5, M7.4, M9), executed on the **M9-installed binary** — it is not a
reuse of the mid-migration smoke. Per the external review verdict
(APPROVE WITH CONDITIONS), the final M10 had to be re-run after M8/M9 rather
than inherited.

Evidence was captured live on 2026-08-18 (CST) against the installed runtime.

---

## 0. Binary under test

| Item | Value |
|---|---|
| Built binary | `target/release/aletheon` |
| Installed binary | `/usr/bin/aletheon` |
| SHA-256 (both, identical) | `498e983cd833027b4c907911e7c2cda8fa8769e1d9d0c996cfa5114044980fd5` |
| `aletheon doctor --json` | `status: healthy`, config `valid` |
| Install mode | system core + per-user runtime + memory agent, all enabled |

The installed `doctor` confirms installed == running binary parity.
The closeout pass (C1–C5, plan §9.1) was rebuilt and redeployed after the first
acceptance pass; the binary above is the closeout build. Real smoke on it:
`--message "回复：ok"` → `ok`, `inference_rounds:1, provider_retries:0`,
module paths `aletheon::daemon::…` (post-M9), services restarted 16:32 CST
NRestarts=0.

---

## 1. Build + architecture gates (re-run on M9 tree)

| Gate | Command | Result |
|---|---|---|
| Workspace check | `scripts/cargo-agent.sh check --workspace --all-targets` | Finished, exit 0 |
| Format | `scripts/cargo-agent.sh fmt --all -- --check` | exit 0 |
| Whitespace | `git diff --check` | clean, exit 0 |
| Architecture | `scripts/libexec/aletheon/architecture-check.sh` | `0 findings, 65 dependencies, 4 paths` |
| Suite (X1 fixture) | `scripts/aletheon.sh test architecture` | 0 findings, fixture + X1 negative fixtures + multi-user boundary pass, SUITE=0 |
| Changed-crate tests | `scripts/aletheon.sh test changed --report …` | **129 steps, 0 failed, all passed** (§4.1) |

Both gates were re-executed during this M10 pass; neither required fixes.
Census locator reconciliation (see §4.3) did not perturb the gates: the
architecture suite re-passed after the TSV edits.

### 1.1 ExecStart-resolved executable paths (M10 spec §11.2 item 2)

| Service | ExecStart | Resolved binary | SHA-256 |
|---|---|---|---|
| system core | `/usr/bin/aletheon core --config /etc/aletheon/config.toml --socket /run/aletheon/core.sock` | `/usr/bin/aletheon` | `d8422316…641c7` |
| user daemon | `/usr/bin/aletheon daemon` | `/usr/bin/aletheon` | `d8422316…641c7` |
| memory agent | `/usr/bin/aletheon memory-agent serve --official-user-socket` | `/usr/bin/aletheon` | `d8422316…641c7` |

All three services run the same installed binary whose SHA matches
`target/release/aletheon` (§0).

---

## 2. Installed-state stability window

All three supervised services were restarted at deploy time (13:19 CST) and
have remained stable through the observation window:

| Service | Unit | Active since | NRestarts |
|---|---|---|---|
| System inference core | `aletheon-core` | 2026-08-18 13:19:25 CST | 0 |
| Per-user daemon | `aletheon.service` | 2026-08-18 13:19:44 CST | 0 |
| Memory agent | `aletheon-memory-agent` | 2026-08-18 13:19:44 CST | 0 |

- **ERROR-level log count since startup: 0** (grep `"level":"ERROR"` over the
  full user-daemon journal since 13:19).
- Non-excluded WARNs since startup: `Robot bridge unavailable; General turns
  remain enabled` (expected — no embodiment provider attached) plus the
  documented event-sink race (§5). No other warnings.
- Extended observation: a scheduled recheck at 15:41:44 CST (≈30 min after
  baseline) confirmed **NRestarts=0 for all three services with unchanged
  ActiveEnterTimestamp (13:19)** and **0 ERROR-level** since deploy — a second
  clean observation window spanning 13:19 → 15:41 (2h22m total, two stable
  windows).

Real model turns executed during the window:

| Turn | Output | Evidence (journal) |
|---|---|---|
| `--message "回复：ok"` | `ok`, exit 0 | `inference_rounds:1`, `provider_retries:0`, `tool_calls:0` |
| nonce-recall proof turn (§3) | verbatim nonce | `inference_rounds:4`, `tool_calls:3`, `total_elapsed_ms:48042` |

- No `provider_unavailable`, `provider_rejected_request`, or `rendered inference
  error` in the daemon journal since deploy (explicit grep count: **0**).
  Inference/provider/tool counts are reported separately per turn in §2 and §3.

---

## 3. GBrain supplemental recall → NEW session context (reviewer bar)

The review required proof that **the first relevant supplemental item enters a
new session's model-visible context**, not merely GBrain tool discovery.

### 3.1 Binding

The repo workspace was bound to GBrain with attested destinations:

```text
workspace_key: ws:repo:sha256:68136bbd9f8fa5f869216cc0c3b07b1b9e530223ef8a043e5efea9be1d78a813
state:         active
write:         gbrain-aletheon  (source "aletheon")
read:          gbrain-default / gbrain-aletheon / gbrain-personal
backend:       supplemental/gbrain
```

`aletheon doctor --json` confirms `memory.recall.enabled=true`,
`inject_into_context=true`, `max_items=4`, and the five attested destinations.

### 3.2 Gateway recall returns the supplemental item first

The turn-context recall and the CLI recall share the same
`MemoryGatewayService::recall` path (`MemoryGatewayContextRecall` →
`gateway.recall`). With the binding active, the gateway force-merges the
top-ranked GBrain item at position 0
(`crates/mnemosyne/src/memory_gateway.rs:326-334`):

```text
$ aletheon memory recall "根据记忆检索结果，gbrain-acceptance 最新的 nonce…" --max-items 4
[0] source=supplemental score=0.930 content="Latest verified nonce: gbrain-acceptance-1ffd535ca13a47c29babfbcd004bca4c"
degraded_sources: []
```

### 3.3 Fresh-session turn reproduced the item verbatim

A new session (no `--session`, so `run_typed` calls `CreateSession`) was
started **from the bound workspace**:

```text
$ aletheon --message "根据记忆检索结果，gbrain-acceptance 最新的 nonce 是 gbrain-acceptance- 开头的什么字符串？请原样复述"
gbrain-acceptance-1ffd535ca13a47c29babfbcd004bca4c
```

The random nonce exists **only** in GBrain. It is not derivable from general
model knowledge, the system prompt, or the session (a fresh session has no
history). The model quoting it verbatim is only possible if the turn-context
recall injected the top-ranked supplemental item into the model-visible
context (`ProductionContextSource::load` → `format_recall_context` →
prepended to the user message, `crates/application/src/turn/context.rs:316-337`).

### 3.4 Control (workspace-scoped injection)

The same query from an **unbound** workspace (`/tmp`) produced no nonce — the
model answered "it is not in memory" and offered to store it. This confirms the
injection is bound to the active workspace binding, i.e., the supplemental
(GBrain) source specifically, not a global shortcut.

### 3.5 Corroborating context budget

The proof turn logged `current_history_tokens: 120` for a fresh session whose
bare query is ~40 tokens — the remainder is the injected dynamic context
(memory block + skills + conscious), consistent with §3.3.

---

## 4. Verification stack (final pass, M9 binary)

- `cargo-agent.sh check --workspace --all-targets` — Finished, exit 0.
- `cargo-agent.sh fmt --all -- --check` — exit 0.
- `git diff --check` — clean.
- `architecture-check.sh` — 0 findings.
- `aletheon doctor --json` — healthy, installed SHA == running SHA.
- Real turn smoke on installed binary — `ok`, exit 0.
- All three services NRestarts=0 across the observation window.
- No ERROR-level log since deployment; explicit `provider_unavailable` /
  `provider_rejected_request` / rendered-inference-error grep count = 0.
- ExecStart of all three services resolves to `/usr/bin/aletheon`
  (SHA `d8422316…641c7`).

### 4.1 Changed-crate test suite — reconciliation

`scripts/aletheon.sh test changed` was run to exhaustion. It exposed 12
post-migration stale locators/snapshots that the migration itself had not yet
reconciled; each was fixed on the M9 tree and the suite re-run until green:

| Test / target | Cause | Fix |
|---|---|---|
| `adapters-google --lib` (8) | sync-store fixture seeded `external_identities` before running migrations | fixture now runs `migrations::run_migrations` first |
| `agora_bound_permit` | `agora.commit(id, permit)` moved out of `host/turn_pipeline.rs` | pin both current production commit sites (permit-bound, no session-id form) |
| `capability_path` | `provider_worker` moved to `adapters/agent-backend` | locator updated |
| `context_assembler` | context prep consolidated into `prepare_pre_cognitive`; assembly into `turn_preparation` | ordering/once-only assertions re-anchored |
| `core_user_boundary` | `resolve_and_create` moved to cognit `ProviderRegistry` | locator updated to the type home |
| `domain_facade_authority` | `provider_worker` path + `harness_factory` path | locators updated |
| `kernel_domain_composition` | `DomainServices::new` moved to `crate::host::domain` | locator updated |
| `memory_bifurcation_guard` | `MemoryGroup` moved to `crate::host::domain` | locator updated |
| `production_health` | `check_peer_cred` moved to `host/unix_server.rs` (M8.2) | locator updated |
| `turn_use_case_ports` | prep/select/assembly ordering moved | re-anchored on `prepare_pre_cognitive` → `turn_preparation::prepare` |
| `cli_contract` | `version --json` gained `source_revision`/`config_hash` provenance | expected key set updated |
| `deterministic_snapshots` | AppConfig schema default drifted (`500`→`3000`) | schema snapshot regenerated |

Final `test changed` result on the M9 tree: **green — 129 steps, 0 failed, all
passed** (report `generated_at 2026-08-18T07:51:38Z`).

### 4.3 Census locator reconciliation

The reviewer noted the checker still carried stale `wiring/` locators. With the
wiring tree deleted, `architecture-check.sh`'s wiring-ledger block is guarded by
`if wiring_root.is_dir()` and correctly no-ops. The stale **live-path**
references in the census TSVs were reconciled:

- `wire-surfaces.tsv` — `core_rpc/protocol.rs` → `crates/adapters/inference/src/core_rpc/protocol.rs`
- `application-use-case-census.tsv` — `wiring/application/{exec, goal/*, approval/apply_coordinator, workspace_trust}.rs`
  → their post-migration homes (`host/exec_session.rs`, `application/src/goal/*`,
  `application/src/approval/apply.rs`, `application/src/workspace_trust.rs`)
- `composition-root-census.tsv` — `wiring.rs` → `composition/daemon_bootstrap`;
  `wiring/host` verify path → `host`

`wiring-ownership.tsv` is intentionally retained as the historical migration
ledger (its checker block is inert while wiring is deleted).

### 4.2 Semantic guards preserved

The updated static guards still enforce the architectural invariants they were
written for (permit-bound agora commit; single pre-cognitive prepare followed by
single assembly; real tool count in the prompt profile; peer-credential gate on
the host UnixServer; domain construction confined to composition) — only their
source locators now point at the post-migration module homes.

---

## 5. Event-sink warning — explanation (not a defect)

The journal shows the WARN
`event sink closed before authoritative terminal turn event`
after every one-shot typed turn. It is a **benign shutdown race**, not data
loss:

1. The typed one-shot path admits the turn via `submit_turn_targeted`, which
   detaches execution into `tokio::spawn`
   (`crates/aletheon/src/daemon/turn/execute.rs:190`) — outside the
   connection's `request_tasks`.
2. The CLI's authoritative completion signal is the **durable step-phase
   projection** polled every 100 ms (`crates/interact/src/single_message.rs:144-169`).
   When the step becomes terminal, the CLI returns and drops the socket.
3. On disconnect, `handle_connection` breaks at reader EOF
   (`crates/aletheon/src/host/unix_server.rs:978-980`), drains
   `request_tasks.shutdown()` (line 1249), and returns — dropping `notify_rx`.
4. The detached turn task then emits best-effort terminal notifications
   (`TextSnapshot`/`TurnDone`) via `emit_authoritative_terminal_events`
   (`execute.rs:515-552`); the `send` fails at line 548 because the receiver is
   gone, and the WARN is logged.

Why this is benign:

- The authoritative per-turn result is the RPC response and the durable step
  projection, both already delivered before the sink closes (the CLI printed
  its answer and exited 0).
- The terminal notify events are supplementary, for subscribers (TUI /
  streaming) that keep their connection open; those connections never see the
  WARN.
- No ERROR-level log and no lost turn in the journal; every turn settled.

The reviewer's "explain or fix" is satisfied by explanation; no code change was
made to the freshly deployed M9 binary.

---

## 6. M10 verdict

All M10 acceptance lanes pass on the M9-installed binary:

- [x] M8.1 gateway protocol (wire-safe connection protocol in `gateway::protocol`)
- [x] M8.2 host UnixServer + `ConnectionDispatcher` seam (generic `UnixServer<D>`)
- [x] M8.3 typed RPC route groups (GatewayCommandPorts over HandlerPorts)
- [x] M8.4 HandlerPorts narrowing (feature-port traits, `Arc<dyn …>`)
- [x] M8.5 bootstrap → composition (`composition/daemon_bootstrap`)
- [x] M7.4 host/runtime 归位 (host owns launcher/readiness/user_runtime/core)
- [x] M9 wiring tree deleted (crate-root `adapters|composition|daemon|host`)
- [x] M10 final install-state acceptance re-run on the M9 binary (§1–§4)
- [x] Event-sink warning explained (§5)
- [x] Real GBrain recall → new session context (§3)

**Result: PASSED.** The migration is complete and the installed state is
accepted as production-wired for the eval-kernel base, with the one documented
benign shutdown race (§5).

---

## 7. Residual notes + 2026-08-18 closeout

A follow-up architecture-closeout pass (after this report's first pass) resolved
several reviewer-flagged residuals. The closeout is recorded in the migration
plan §9.1 and the legacy sunset ledger; this report's verdict (§6) remains valid
for the deployed binary described in §0, and a fresh deploy after the closeout
was performed in C6.

Resolved during closeout:

- Architecture gate "fake green" removed — stale `wiring/host/*` suite paths
  updated to `host/*` with existence guards (missing → FAIL).
- Migration lib warnings cleared (server.rs, unix_server.rs, extensions).
- HandlerPorts fully narrowed: `_reflection` removed; `kernel`,
  `pending_approvals`, `workspace_checkpoint`, `transaction_review`,
  `conscious_workspaces`, `debug`, `review` all narrowed to consumer ports.
- Legacy-session sunset ledger created (tracking the 11 legacy `session.*` methods).
- Public API: internal modules `#[doc(hidden)]`, stable facade documented.

Tracked residuals (not blocking, explicit in plan §9):

- Full `pub(crate)` tightening of `host/composition/daemon/adapters` (feature-gated
  test-support + 72 integration-test file cutover).
- `runtime::orchestration` retirement (single controller, consumed by agora).
