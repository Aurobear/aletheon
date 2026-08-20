# Agent Kernel V2：CGP-00 composition / Gateway / Interact census

Date: 2026-08-09
Baseline: `0bf690b2`
State: evidence-only census closed; `gateway-route-census.tsv` + `composition-root-census.tsv` frozen; no behavior change

## Context receipt

```text
Slice: CGP-00 composition/Gateway/Interact census
Baseline commit: 0bf690b2
Direct prerequisites: RA-00, K0, APX-00
Current authoritative writer: unchanged legacy Executive daemon/handler paths
Target owner/writer: unchanged by this census
IDs minted here: none
Production callers: enumerated per route (daemon principal auth) and per composition point
Test-only callers: excluded
Installed/config callers: cross-checked against interact-authority-census.md 53/53
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none added
Deletion owner: per-row cutover/deletion slice
Unknowns/blockers: none
Expected files: gateway-route-census.tsv, composition-root-census.tsv, this evidence record, CGP-00 gate
Out-of-scope files: all production behavior, CGP-01+
```

## 1. Gateway route surface (gateway-route-census.tsv)

16 route families covering ~70 JSON-RPC methods dispatched at `crates/aletheon/src/wiring/daemon/handler/rpc.rs:33-204`:

| family | methods | owner | notes |
|---|---|---|---|
| session-lifecycle | clear,resume,compact,new_session,load_recent,load_previous,session.create/list/new/switch | SessionService/legacy | SessionId reconstruction; compatibility |
| health-status | status,health,evaluation.* | health/status | presentation read-only |
| review | review.*, task.review/* | GovernedReviewService | |
| admin-meta | daemon.shutdown,deployment.rollback,reload_skills | admin_service | |
| skills | skills.list,skill.invoke | skill_admin/corpus | |
| approval | approval_response,diff_artifact.get,approval.* | ToolApprovalAuthority | |
| workspace-trust | workspace.trust.* | workspace_trust | effective workspace |
| turn-control | interrupt,mode_switch,turn.*,prompt.*,cancel | TurnCoordinator/TurnPipeline | TurnId mint + TaskKind::Coding local inference |
| google | google.* | google adapters | OAuth refresh |
| agents | sub_agents,agent.profile.* | AgentControlService | AgentId mint |
| extension | extension.install/enable/.../doctor | extension_install | |
| memory | memory.add/list/search/... | memory_gateway | |
| workflow | workflow.save/load/list/delete/run | WorkflowStore | optional |
| goal | goal.set/show/status/resume/create/list/pause/run/cancel | goal_service | |
| checkpoint | workspace.rewind,checkpoint.list/v1 | workspace_checkpoint | grok_hardening-gated |
| conscient | conscious.diagnostics | conscious_core_inspector | |

All routes are `compatibility` classification today: raw `serde_json::Value` request/response over JSON-RPC on a Unix socket, framed by `executive host daemon server.rs`, with no typed request DTO and no unified retry/reconnect beyond `protocol_events_after`. Target: Gateway typed protocol/client (`CGP-02/03`).

### Authority claims to migrate (plan §8.2)

- `TurnId::new` mint at `turn_coordinator.rs:610` (CGP-R-08) — Runtime canonical Turn.
- `AgentId` mint at `agent_control/mod.rs:811` (CGP-R-10) — Runtime AgentSupervisor.
- `TaskKind::Coding` local inference at `submit.rs` (CGP-R-08) — must become explicit request.
- SessionId reconstruction across routes — typed DTO decode after `RA-03`.

## 2. Composition root surface (composition-root-census.tsv)

10 composition points:

| point | composes | current owner | target |
|---|---|---|---|
| aletheon `main.rs:442` | daemon via `run_daemon` | aletheon (already the entry) | aletheon composition root |
| executive `bootstrap/services.rs:524` + `request.rs:948` | RequestHandler/SessionGateway/AgentControlService | executive | aletheon root (moves out) |
| executive `user_runtime/mod.rs:283` | CLI/message/TUI composition | executive | aletheon root |
| executive `turn_coordinator.rs:38` | SessionAppendStore/EventSpine | executive | Runtime ports |
| executive `agent_loader/mod.rs:49` | MarkdownAgentProfileLoader | executive | Runtime profile port |
| interact `host.rs:64` | ExecutiveDaemonEnsurer | interact (executive dep) | aletheon lifecycle client |
| gateway `lib.rs:25` | Telegram transport | gateway | Gateway protocol/client |
| interact `acp/gateway.rs:42` | AcpBackend | interact | GatewayClient backend |
| interact `memory_client.rs` | own UnixStream | interact | GatewayClient |
| interact `tui/model.rs:44` + `tui/controller.rs:10` | model/controller seams (renderer in `tui/render/`) | interact | TuiModel/Controller/Renderer |

Interact is NOT the primary composition root (no canonical Agent construction), but `host.rs` imports Executive (daemon auto-start) and `tui/mod.rs` is the second workspace derivation point — both must move to `aletheon`/GatewayClient per plan.

## 3. Interact 53/53 baseline

`rg --files crates/interact/src -g '*.rs' | wc -l` = **53** — matches the revised  §8 `actual=53` after caller-zero retirement of six stale/duplicate-framing shells, deletion of the caller-zero `intent.rs` facade, and registration of the CGP-07 model/controller seams. The CGP-00 gate enforces this set stays mechanically equal (no new/removed interact source files without a census update).

## 4. Validation

```text
git diff --check                                  PASS
bash -n scripts/libexec/aletheon/architecture-check.sh   PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture   PASS (23 findings, 0 deps)
bash scripts/cargo-agent.sh fmt --all -- --check  PASS
bash scripts/cargo-agent.sh check -p gateway      PASS (15.08s)
bash scripts/cargo-agent.sh check -p interact --lib  PASS (6.02s)
bash scripts/cargo-agent.sh check -p executive --lib  PASS (4.34s)
```

### Gate mutation tests (probes, reverted after each)

| probe | result |
|---|---|
| drop `CGP-R-08` (turn-control) row | REJECTED (its methods become unregistered) |
| add new RPC method `new.route` to rpc.rs | REJECTED (unregistered route) |
| clean baseline | ACCEPTED (16 route rows, 53/53 interact, all methods registered) |

The CGP-00 gate enforces: route census structural integrity, per-row evidence match, the Interact 52-file set, and complete coverage of every daemon RPC dispatch method.

## 5. Scope boundaries

- **Allowed**: gateway-route-census.tsv, composition-root-census.tsv, this evidence record, CGP-00 gate.
- **Explicitly not done**: no gateway protocol/client, no composition root move, no route behavior change, no socket cutover, no deployment.
