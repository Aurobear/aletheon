# Aletheon Session Authority 收敛映射

状态：X5a reviewed mapping（doc-only）

基线：`8dc8a756`

后续实现节点：X5b、X5c

## 1. 需求边界

- X5a 只产出 reviewed `keep/extend/absorb/delete` 映射，不修改运行时代码；X5b 才实现 daemon-owned Session/Task/Activity projection，X5c 才迁移 Interact reducer（`docs/plans/Aletheon_Unified_Execution_Plan_2026-08-04.md:110-112`）。
- `SessionAuthority` 是架构角色，不新增同名 trait/store；它必须由现有 `EventSpine`、`SessionAppendStore`、`SessionProjection` 收敛而成（`docs/plans/Aletheon_Unified_Execution_Plan_2026-08-04.md:292`）。
- journal 是唯一事实链，snapshot/projection 可由其重建；SessionStore 不能成为旁路 writer（`docs/plans/Aletheon_Architecture_Stabilization_and_Convergence_Plan_2026-08-04.md:739-750`）。

## 2. 当前代码事实

```text
Executive application
    |
    | SessionAppendStore port
    v
EventSourcedSessionStore
    |-- append --> EventSpine / events.db       (事实链)
    |-- project -> EventProjectionSink          (通用投影)
    `-- materialize -> CanonicalSessionStore    (兼容读模型)
                         ^
                         `-- SessionProjection reducer
```

- `SessionAppendStore` 同时声明 create/append/fork 和 session/item reads（`crates/fabric/src/types/session.rs:238-254`）。
- `EventSpine` 的公共 port 目前只有 append，read/replay 仍依赖具体 SQLite adapter（`crates/fabric/src/events/spine.rs:154-166`；`crates/executive/src/adapters/session/event_sourced_store.rs:36-63`）。
- 生产 composition 将 application 的 `SessionAppendStore` 绑定到 `EventSourcedSessionStore`，后者先写 spine，再推进 projections 和兼容读模型（`crates/executive/src/composition/turn_coordinator.rs:34-46`；`crates/executive/src/adapters/session/event_sourced_store.rs:69-94`）。
- `SessionProjection` 已能从 session-created/forked/turn events 确定性构造公共 Session state，并能 materialize 兼容 store（`crates/executive/src/adapters/events/session_projection.rs:26-79`、`:222-249`）。
- `CanonicalSessionStore` 仍直接实现完整 `SessionAppendStore`，因此类型层仍允许绕过 spine 写入（`crates/executive/src/adapters/session/canonical_store.rs:215-285`）。
- daemon 在 event spine、projection store 或 session DB 打开失败时会退化为进程内存实例，可能产生无法恢复的分叉事实链（`crates/executive/src/host/daemon/bootstrap/services.rs:66-84`、`:292-310`）。

## 3. Reviewed 收敛裁决

| 当前符号/路径 | 裁决 | X5b/X5c 目标 | 禁止事项 |
|---|---|---|---|
| `fabric::SessionAppendStore` | **extend** | 保留为唯一 application Session authority port；将 journal-backed snapshot/event reads 加入同一契约，所有 mutation 仍要求 expected sequence/idempotent item ID | 不新增 `SessionAuthority`、`SessionStoreV2` 或第二个 mutation port |
| `fabric::EventSpine` | **extend** | 保留为唯一不可变 journal primitive；增加 transport-neutral、按 tree/sequence 有界读取能力供 replay | 不让 presentation 直接 append；不把 projection DB 当 journal |
| `EventSourcedSessionStore` | **keep** | 作为 `SessionAppendStore` 的唯一生产实现和 append transaction coordinator；append 成功以 spine durable commit 为准 | 不让 caller 同时持有 read-model writer |
| `EventProjection` / `EventProjectionSink` | **keep** | 保留通用 reducer/checkpoint/poison/lag 机制，Session/Task/Activity 使用相同 ordered input | 不为 UI 另建投影状态机或 checkpoint authority |
| `SessionProjection` / `PublicSessionState` | **extend** | 扩展为 schema-versioned Session/Task/Activity snapshot；纯 reducer、可从零 replay、重复 event 幂等 | 不接受 presentation 生成的状态作为恢复输入 |
| `CanonicalSessionStore` | **absorb** | 收缩为 `SessionProjection` 私有 materialized read store；拆除生产侧完整 `SessionAppendStore` writer 能力 | 不再以 “Canonical” 名义与 event spine 并列拥有事实 |
| daemon `HashMap<String, SessionManager>` | **absorb** | 仅保留有界 connection/live-turn cache，内容从 authority snapshot/event cursor 补齐 | 不作为历史、resume 或 crash recovery 权威 |
| legacy `sessions`/`resume` 与 `session.*` 双 wire | **delete** | X5b 提供一个 versioned snapshot/event read protocol；X5c 迁移所有 caller 后删除旧 route | 不长期双写、双读或按入口选择不同 session service |
| process-local persistence fallbacks | **delete** | installed/production daemon 对 journal/read-model/projection DB fail closed；仅显式 test composition 可用 `:memory:` | 不把降级后的成功报告为 durable Session 成功 |

## 4. 唯一权威和事务边界

```text
Command/Turn mutation
  -> SessionAppendStore (application port)
  -> EventSourcedSessionStore
  -> EventSpine durable append                AUTHORITY COMMIT
  -> Session/Task/Activity reducers
  -> projection checkpoint + read-model materialization
  -> versioned snapshot/event read protocol
  -> Interact reducer                         READ ONLY
```

裁决：

1. authoritative commit 是已持久化、已排序的 spine event；projection 成功不是第二次业务提交。
2. public Session projection 失败时 mutation 必须失败并留下可重放 journal；重启 reconciliation 从 committed watermark 恢复，不能重放 capability side effect。
3. snapshot 只代表 `through_sequence` 之前的确定性折叠；客户端随后只消费 `sequence > through_sequence` 的事件。
4. duplicate event/item ID 必须返回已有结果或 typed conflict；不得形成第二个 Task/Activity，也不得重复 patch、child 或 memory write。
5. UI draft、selection、scroll、overlay 可以本地存在；Conversation、Task、Activity、Changes 和 terminal state 必须来自 daemon projection。

## 5. X5b 实现清单

- 扩展现有 Fabric Session contract，而不是创建同义 authority：snapshot 包含 `schema_version`、`session_id`、`through_sequence`、Task/Activity/terminal projection；event page 包含严格递增 sequence 和 next cursor。
- 为 `EventSpine` 增加 port-level bounded reads，消除 application 对 `SqliteEventSpine` concrete read API 的依赖。
- 将 `CanonicalSessionStore` 分成只读查询/materializer 最小能力；生产 composition 只向 application 暴露 journal-backed `SessionAppendStore`。
- installed daemon 的三个 durable stores 打开失败时 fail closed；`:memory:` 仅允许显式 test constructor。
- 用从零 replay、snapshot+tail replay、重复事件和 crash-between-append/materialize 测试证明 A-SESSION-001/002。

## 6. X5c 迁移与删除门槛

- Interact 首次连接读取 snapshot，再订阅 `after=through_sequence`；duplicate/out-of-order 输入由 reducer 明确忽略或拒绝。
- `resume` picker 读取同一 daemon projection，不读取本地历史或第二数据库。
- caller inventory 中 legacy `sessions`、`resume`、`SessionManager` 历史读取为零后才删除兼容 route。
- 删除后，生产 Session writer 计数必须为 1：只有 `EventSourcedSessionStore` 实现并被 composition 注入。

## 7. 验收证据

X5a 的退出证据是本文映射与 `config/architecture/wire-surfaces.tsv` 的对应登记；本文不声称 A-SESSION-001/002/003 已通过。那些动态证据分别属于 X5b/X5c。
