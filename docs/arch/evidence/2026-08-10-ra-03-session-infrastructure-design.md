# RA-03 SessionInfrastructure 修订设计

Date: 2026-08-10
Plan: `docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md` §12.2, rework brief §4 P0-1..P0-4
Status: **CODE_CANDIDATE — focused validation and installed smoke passed; full cutover matrix remains**

> §1–§2 preserve the historical evidence captured before the implementation.
> The current code status and remaining gates are recorded in §8; do not use
> the old line-number descriptions below as present-tense claims.

## 1. 修订后的 Context receipt

```text
Slice: RA-03 SessionInfrastructure + RuntimeSessionWriter cutover (design)
Baseline: 0bf690b2..d0286ca7 (rework), production user daemon PID 102238
Current authority: 双链 — legacy `SessionStore`(sessions.db, request.rs:81) 写初始 session;
  canonical `CanonicalSessionStore`(sessions-v1.db, services.rs:377) 写主链
目标: 唯一 `SessionInfrastructure` 在第一个 session_id consumer 前构造,
  Runtime mint canonical SessionId + append SessionCreated; 所有组件复用同一
  store/spine/projection/authority; legacy sessions.db 停写只读
IDs minted here: 无 (writer 在 runtime mint)
Production callers: `request.rs` constructs and invokes the injected
  `SessionInfrastructure` writer before the first session consumer; the
  `build_turn_services` writer/service path receives the same instances.
Test-only callers: Runtime writer, migration, fork, and multi-turn recovery
  tests are present; installed wrong-generation/rollback evidence remains.
Installed/config: `[bootstrap].session_writer` is typed and defaults to
  `runtime`; installed deployment and rollback evidence remains pending.
Tables: sessions.db 停写; sessions-v1.db 唯一 canonical
```

## 2. 历史构造顺序（取证）

```text
request.rs:75  mint session_id (uuid)  ← 当前唯一 mint，需移除
request.rs:81  SessionStore(sessions.db).create_session(uuid)  ← legacy 生产写，需移除
request.rs:255 sessions::compose { data_dir, session_id, ... }
              └ 我此前 gated 分支重开 sessions.db（错文件+错schema → 启用即崩 P0-4）
request.rs:276 default_id = sessions_composition.default_id
request.rs:461 ExecutiveConfig { session_id, ... }   ← 消费 session_id
request.rs:902 capability 注入 (&session_id)
request.rs:1116 agent_svc = build_agent_services(...)  ← 构造 canonical_event_spine + event_projections
request.rs:1134 canonical_event_spine = agent_svc.canonical_event_spine
request.rs:1148 turn_svc = build_turn_services(..., session_id, ...)
              └ 内部 services.rs:377 CanonicalSessionStore::open(sessions-v1.db)
              └ 内部 services.rs:394 compose_session_store(canonical_store, spine, projections)
              └ 内部 services.rs:524 SessionGateway::new(..., session_service, ...)
request.rs:1562 session_lifecycle.start(session_id)
```

**矛盾**：sessions::compose(255) 最早消费 session_id 且需持久化初始 session，但权威 spine(1116)/store(1148) 晚于它。

## 3. 目标构造顺序（唯一 SessionInfrastructure，资源拆分）

```text
request.rs:75 移除 uuid mint; 改为构造共享资源（唯一，在第一个 consumer(255) 前）:
   └ RuntimeJournalResources {                        ← 不归 SessionInfrastructure 所有
        spine:  SqliteEventSpine::open_bounded(events.db)        ← 唯一 spine
        projections: DefaultEventProjectionSet::open(event-projections.db)  ← 唯一 projection
      }
   └ SessionInfrastructure {                          ← canonical store + append store + writer + receipt
        canonical_store: CanonicalSessionStore::open(sessions-v1.db)  ← 唯一 store
        append_store: compose_session_store(canonical_store, spine, projections)
        authority: SessionAuthority (Runtime mint)
        writer: RuntimeSessionWriter { authority, append_store }
        receipt: writer.create_session(...)  → minted canonical SessionId + SessionCreated
      }
request.rs:255 sessions::compose 回退为纯内存（不再打开任何 DB）
   └ 接收 minted canonical SessionId + 已构造的 ContextWorkingSet/registry 作为输入
request.rs:276 default_id = minted canonical id
request.rs:461 ExecutiveConfig { session_id: minted id }
request.rs:1116 build_agent_services 接收注入的 spine/projections（不再自开）
request.rs:1148 build_turn_services 接收注入的 RuntimeJournalResources + SessionInfrastructure
              └ 复用同一 store/spine/projection/authority（禁止重开）
request.rs:1562 session_lifecycle.start(minted id)
```

**资源归属**：`RuntimeJournalResources`（spine + projections）是共享 journal 资源，被 agent/build_turn/executive 复用；`SessionInfrastructure` 持有 canonical store + append store + writer + receipt。两者都在第一个 consumer 前构造一次，`Arc` 注入。**projection 权威路径 = `event-projections.db`**（不是 projections.db）。

## 3b. 配置形态（调整后）

```toml
[bootstrap]
session_writer = "legacy" | "runtime"
```

- 未知值 fail closed，进入 effective config diagnostics。
- production 默认 `runtime`；`legacy` 仅用于明确 rollback/migration window。
- 登记 legacy mode 的删除 owner；禁止永久保留反向切回旧 writer 的通道。

## 4. 唯一 Store / ID / Writer 表（资源拆分后）

| 资源 | 唯一实例 | 构造位置 | 消费方 |
|---|---|---|---|
| EventSpine | `SqliteEventSpine::open_bounded(events.db)` | RuntimeJournalResources（新，首 consumer 前） | SessionAppendStore、agent_svc、progress、build_turn_services |
| Projection | `DefaultEventProjectionSet::open(event-projections.db)` | RuntimeJournalResources（新，首 consumer 前） | SessionAppendStore、event_projections |
| canonical store | `CanonicalSessionStore::open(sessions-v1.db)` | SessionInfrastructure（新） | append store |
| SessionAppendStore | `compose_session_store(canonical_store, spine, projections)` | SessionInfrastructure（新） | RuntimeSessionWriter、SessionService、build_turn_services |
| SessionId | **Runtime mint**（canonical） | SessionInfrastructure（新） | 全部（255..1562） |
| SessionCreated append | RuntimeSessionWriter | SessionInfrastructure（新） | 初始 session |
| legacy sessions.db | **只读** | 移除生产写（request.rs:81） | 迁移期只读兼容 |
| sessions-v1.db | 唯一 canonical | SessionInfrastructure（新） | 主链 |

**无第二 composition root**：`RuntimeJournalResources`（spine+projections）与 `SessionInfrastructure`（store+append+writer+receipt）各构造一次，`Arc` 注入所有消费方；`sessions::compose` 不再打开任何 DB；`build_agent_services`/`build_turn_services` 只接收同一 Arc 实例，不再自开 spine/store。

## 5. 待实现明细（P0 修复映射）

- **P0-1** `[bootstrap] session_writer = "legacy"|"runtime"` 真实配置；未知值 fail closed 进 diagnostics；默认 `runtime`；登记 legacy 删除 owner。
- **P0-2** `created_at` 用 minted id 作 key（registry/default/created_at/ContextWorkingSet/SessionGroup/progress 全用同一 canonical id）。
- **P0-3** `created_at_ms` 用真实 clock；`principal_hint`/`correlation` 按已验证 contract 持久化，或显式返回 typed `unsupported`，不静默丢弃；sequence/head/pending 走真实 append store（不依赖进程内 SessionHeadIndex）。
- **P0-4** 移除 `sessions::compose` 里的 DB 打开；复用唯一 store/spine/projection 实例。
- **legacy 审计** `LegacySessionService` 的 create/create_and_switch/fork 委托 Runtime command port；list/resume/switch 委托 Runtime query port；clear/compact 经 Runtime/ContextWorkingSet。`sessions.db` 停写只读，不删历史；`LegacySessionService` 不得成为 canonical store 的第二 writer。

## 6. 预计修改文件

| 文件 | 改动 |
|---|---|
| `crates/runtime/src/session_writer.rs` | 绑定真实 store；durable head；created_at/principal/correlation；typed unsupported |
| `crates/runtime/src/session_head.rs` | 或移除进程内 head，改为 store-backed（若契约允许） |
| `crates/executive/src/host/daemon/bootstrap/request.rs` | 构造 RuntimeJournalResources + SessionInfrastructure（新 fn）；移除 uuid mint + legacy create；各消费点改用 minted id |
| `crates/executive/src/host/daemon/bootstrap/sessions.rs` | 回退纯内存；接收 minted id + ContextWorkingSet |
| `crates/executive/src/host/daemon/bootstrap/services.rs` | 接收注入的 RuntimeJournalResources + SessionInfrastructure；不再自开 |
| `crates/executive/src/host/daemon/mod.rs` + config | `[bootstrap] session_writer` typed 配置解析 |
| `crates/aletheon-config/src/` | `session_writer` enum 解析 + diagnostics + fail-closed |
| `crates/executive/src/adapters/session/store.rs` | legacy 停写（或保留只读） |
| `crates/executive/src/compatibility/legacy_session_service.rs` | create/fork→command port；list/resume→query port；clear/compact→ContextWorkingSet；停写 sessions.db |
| 新增测试 | runtime=true、同 Session 多操作、重启恢复、旧库迁移读取、wrong schema、duplicate/wrong-generation |

## 7. 确认结论（已获人工确认，2026-08-10）

1. **store 提前构造可行**：拆分 RuntimeJournalResources（spine+projections）与 SessionInfrastructure（store+append+writer+receipt）；`build_agent_services`/`build_turn_services` 只接收同一 `Arc` 实例，不得重开。projection 权威路径 = `event-projections.db`。
2. **legacy sessions.db 只读**：create/create_and_switch/fork 委托 Runtime command port；list/resume/switch 委托 Runtime query port；clear/compact 经 Runtime/ContextWorkingSet。LegacySessionService 不得成为 canonical store 第二 writer。
3. **配置形态**：`[bootstrap] session_writer = "legacy"|"runtime"`，未知值 fail closed，默认 runtime；legacy 仅用于明确 rollback/migration window，登记 legacy 删除 owner。

## 8. 当前代码候选状态（2026-08-10）

- 已完成：typed 配置、首 consumer 前的唯一 journal/store 构造、对 agent/turn services
  的精确 `Arc` 注入、Runtime UUID mint、真实 clock、store-backed append，以及
  correlation/principal 的 typed unsupported。
- 已完成：Runtime mode 下初始 session 与 legacy facade 的 create/clear/compact/workspace
  create 统一经过 `RuntimeSessionWriter`；`sessions::compose` 不再打开 DB。
- 已完成代码侧 migration：Runtime 启动以 read-only 打开旧 `sessions.db`，将历史记录 append 到
  canonical store；legacy Runtime-mode list/resume/switch 读取 canonical projection。`RuntimeSessionWriter`
  现在提供 maintenance freeze/drain、active-write watermark、generation rollover，以及
  `Retired`/`WrongGeneration` typed fence；仍待正式安装态执行这组运维演练并核对 rollback binary。
- 已完成 RA-04 代码侧 recovery：RuntimeTurnWriter 通过独立 runtime-turn schema 写入 durable
  event spine，daemon bootstrap replay 未完成的 Runtime turn 并以 typed Interrupted fence；
  `TurnCoordinator` 仍是执行 facade，不再 mint canonical TurnId。
- RA-05 RuntimeAgentSupervisor 已完成 generic registry、Runtime run ID/generation、backend
  pinning、typed lifecycle sink 与 orphan recovery 单元约束；生产 AgentControl 的完整
  mailbox/backend adapter 切换和 installed acceptance 仍未完成，不能宣称 RA-05 完成。
- 因此本状态仍是 `CODE_CANDIDATE`：代码侧主链与 installed smoke 已通过，但在完成
  maintenance/drain、legacy caller-zero、wrong-generation、crash/restart 与 rollback
  binary 演练前，不能把 RA-03 标记为最终关闭。

## 9. Legacy facade write-cutover evidence (2026-08-11)

`LegacySessionService` now routes `create`, `create_and_switch`, `clear`, and
`compact` through the injected Runtime command port
(`crates/executive/src/compatibility/legacy_session_service.rs:203-254,487-548`).
Historical `sessions.db` access is opened with `SessionStore::open_read_only`
only (`:265-283`); canonical-first `resume`/`switch` and the legacy fallback do
not mutate that file (`:392-429`). A historical fallback can seed the in-memory
facade and project into canonical storage on first read, without introducing a
second legacy writer.

Focused evidence:
- `bash scripts/cargo-agent.sh test -p executive --test session_use_case_port`: 5 passed,
  including clear/compact through the Runtime writer and an assertion that the
  legacy database still contains only its original fixture row.
- `bash scripts/cargo-agent.sh check -p executive -p runtime`: PASS.
- `ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture`: PASS.

This closes the code-side legacy write path. Installed caller-zero, migration
replay, maintenance/drain, rollback, and final system acceptance remain open.

Bootstrap follow-up (2026-08-11): `create_initial_session` now uses the
canonical Runtime writer regardless of the compatibility read mode
(`crates/executive/src/host/daemon/bootstrap/session_infrastructure.rs:107-128`).
The `Legacy` mode no longer has a bootstrap branch that calls
`SessionStore::new(...).create_session`; `sessions.db` is therefore read-only
in both modes. The session-infrastructure unit tests and package check pass.

The facade list path now queries canonical sessions in both modes and adds only
historical, read-only legacy rows that have not yet been imported
(`crates/executive/src/compatibility/legacy_session_service.rs:307-390`). The
session use-case suite is now 6 tests and covers this restart/list invariant.

## 10. Maintenance resume fence (2026-08-11)

`SessionAuthority::resume_next_generation` now fails closed unless maintenance is
currently frozen and `active_writes == 0`
(`crates/runtime/src/session_authority.rs:79-96`). A new typed
`RuntimeError::MaintenanceNotDrained` prevents reopening a generation while an
admitted writer is still alive. The writer facade propagates the `Result` rather
than exposing an unchecked rollover (`crates/runtime/src/session_writer.rs:62-64`).

Focused evidence:
- `bash scripts/cargo-agent.sh test -p runtime --lib session_authority`: 3 passed,
  including the held-permit rollover rejection.
- `bash scripts/cargo-agent.sh test -p runtime --lib session_writer`: 3 passed,
  including old-generation fencing after a drained rollover.
- `bash scripts/cargo-agent.sh check -p runtime -p executive`: PASS.

This closes the code-side unchecked maintenance reopen window. Installed
maintenance/drain rehearsal and rollback acceptance remain open.


## 11. Binary-owned compatibility adapter cutover (2026-08-12)

The installed daemon no longer imports Executive's compatibility session or
context-compactor modules. The request-facing DTO/port and one-way adapter now
live at `crates/aletheon/src/wiring/daemon/legacy_session.rs:1-662`; daemon
bootstrap constructs that local adapter at
`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:1343-1358`. Creation and
mutation still dispatch through the injected Runtime command port
(`crates/aletheon/src/wiring/daemon/legacy_session.rs:95,203-267`), while the
historical store remains read-only (`:272-286`). This is a composition-owner
move, not a new Session authority.

Executive's copies are now rollback/test-only remnants assigned to `XRET-04`.
Production caller-zero is enforced by:

```bash
! rg -n 'executive::compatibility::(legacy_session_service|context_compactor)' crates/aletheon/src
```

Validation after the cutover:
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: PASS;
- `bash scripts/cargo-agent.sh test -p aletheon --lib wiring::daemon`: 112 passed;
- `bash scripts/cargo-agent.sh test -p executive --test session_use_case_port`: 6 passed;
- changed validation: 92/92 passed.
