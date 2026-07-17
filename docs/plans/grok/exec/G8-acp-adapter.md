# G8 可执行 Spec：ACP 适配器

> 对应研究文档 `../08-acp-and-runtime-adapters.md`。优先级 P2（依赖 G2、G3、approval mapping）。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：在 Interact/Executive 边缘新增 ACP（Agent Client Protocol）适配器，把编辑器/IDE 客户端的 initialize/new session/prompt/cancel/notification 映射到 Aletheon **已有权威路径**；ACP 只做协议翻译与恢复，不成为新领域运行时。

**非目标**（分阶段）：
- 第一版只做 initialize/new session/prompt/cancel/session notifications。
- load session/reconnect、permission round-trip、client-provided FS/terminal、mode/model 为后续阶段。
- 不用 ACP permission mode 替换 Aletheon approval authority。
- 不把 ACP 协议类型渗透进 Cognit/Dasein/Agora。
- 不移植 Grok leader/version/update 机制。

## 2. 当前代码锚点（已验证 @ commit bec15695）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| 传输 | `crates/interact/src/tui/cli.rs:22-25`；`rpc_client.rs:10-31` | Unix socket + JSON-RPC 行分隔；`send_rpc` 单请求→响应 |
| `ConnectionId` | `crates/fabric/src/types/local_authority.rs:13-26` | `ConnectionId(Uuid)` |
| 协议协商 | `crates/fabric/src/protocol/client.rs:161-173` | `negotiate_protocol_version(offered: &[u16]) -> Result<u16>`；当前 v1 |
| `LocalOsPrincipal` | `crates/fabric/src/types/local_authority.rs:40-43` | `{ uid, gid }` |
| `check_peer_cred` | `crates/executive/src/impl/daemon/server.rs:570-604` | SO_PEERCRED → `LocalOsPrincipal` |
| `PrincipalId::local_uid` | `crates/fabric/src/types/admission.rs:32-39` | `local-uid:{uid}` |
| `PrincipalContext::new` | `crates/fabric/src/types/local_authority.rs:279-311` | principal/os/connection/thread/turn/workspace/permission_profile/approval_policy |
| session resume/create | `crates/executive/src/service/session_service.rs:47-82` | `resume(session_id)->ResumeResult`；create |
| turn 入口 | `crates/executive/src/service/daemon_turn/execute.rs:19-26` | `execute_turn(id, message, context)` |
| `submit_with` | `crates/executive/src/service/turn_coordinator.rs:140-149` | `submit_with(request, policy, runner)` |
| `ClientEvent` | `crates/fabric/src/protocol/client.rs:131-143` | InitializeResponse/Snapshot/Item/Approval/Agent/Reconnected/Failed |
| `ItemEvent` | 同上 `:103-114` | cursor/item_id/phase(Started/Streaming/Completed/Failed)/delta/item/error |
| `ApprovalEvent` | 同上 `:117-121` | cursor/approval |
| `ApprovalUseCases` | `crates/executive/src/service/approval_service.rs:46-59` | list_pending/get_approval/resolve_approval |
| `CapabilityExecutionContext` | `crates/executive/src/service/governed_capability.rs:22-37` | 完整可信字段 |
| **ACP 依赖** | — | **无**（grep 零结果，Cargo.toml 无 agent-client-protocol） |

**核心事实**：Aletheon 已有完整结构化事件流（`ClientEvent`/`ItemEvent`/`ApprovalEvent` + `EventCursor` 重连游标）+ principal 建立 + approval 解析。ACP 适配器主要是**语义映射**，非重造事件源。

## 3. 权威归属决策（doc10 §6 八问）

1. **owner**：Interact 拥有 ACP 适配器（边缘）；Executive use-case ports 仍是权威入口；Cognit/AgentControl/Kernel 不感知 ACP。
2. **scope**：每个 ACP connection 绑定 authenticated principal；客户端给的 session id 不能直接作为 authority。
3. **crash 恢复**：reconnect 先同步权威 session snapshot（复用 `EventCursor`/`Reconnected`），再补增量；适配器崩溃不改 Executive 权威 run 状态。
4. **fail 模式**：客户端 session 不可见（跨 principal）→ 拒绝 load；permission 请求必关联 turn/call id。
5. **上限**：复用现有事件流上限；协议 correlation id 表有界。
6. **兼容**：ACP 是新增边缘入口，不改现有 JSON-RPC 客户端路径。
7. **进 event spine**：无新增领域事件；ACP 把已有 `ClientEvent` 翻译为 ACP session update。
8. **许可证**：`agent_client_protocol` 是外部标准 crate（若引入需审查其许可证）；适配器代码为 Aletheon 原创。

## 4. 类型定义

### 4.1 Interact ACP 适配器 — `crates/interact/src/acp/mod.rs`（新文件）

```rust
//! ACP 边缘适配器。把 ACP 请求映射到 Aletheon Executive use-case ports。
//! 不持有领域状态；不感知 Cognit/Dasein/Agora。

/// ACP 协议方法（第一版子集）。
pub enum AcpRequest {
    Initialize { client_capabilities: serde_json::Value, protocol_versions: Vec<u16> },
    NewSession { cwd: std::path::PathBuf },
    Prompt { session_id: String, text: String },
    Cancel { session_id: String },
}

/// ACP → Aletheon 映射结果。
pub enum AcpResponse {
    Initialized { protocol_version: u16, agent_capabilities: serde_json::Value },
    SessionCreated { session_id: String },
    /// prompt 的 turn 结果经 session update 流异步返回。
    Accepted,
    Cancelled,
    Error { message: String },
}

/// 协议 correlation：ACP session id ↔ Aletheon (ConnectionId, ThreadId)。
/// 领域层继续用 TurnId/OperationId/AgentId；适配器只维护映射。
pub struct AcpCorrelation {
    acp_session_to_thread: std::collections::HashMap<String, (fabric::ConnectionId, fabric::ThreadId)>,
}

/// 适配器主体。持有 Executive use-case ports（非领域内部类型）。
pub struct AcpAdapter {
    // turn 提交端口、approval 端口、session 端口（经 Executive 边界）
    correlation: AcpCorrelation,
}
```

### 4.2 事件映射（Aletheon → ACP session update）

```rust
/// 把 Aletheon ClientEvent 翻译为 ACP session update。纯映射，可单测。
pub fn map_client_event_to_acp(ev: &fabric::protocol::client::ClientEvent) -> Option<serde_json::Value> {
    use fabric::protocol::client::ClientEvent::*;
    match ev {
        // ItemEvent(delta/phase) → ACP session/update text chunk
        Item(item) => Some(map_item_event(item)),
        // ApprovalEvent → ACP request permission（关联 turn/call id）
        Approval(a) => Some(map_approval_to_permission(a)),
        // Reconnected(cursor) → ACP 先发权威 snapshot 再补增量
        Reconnected(cursor) => Some(map_reconnect(cursor)),
        Failed { message, .. } => Some(map_error(message)),
        _ => None,
    }
}
```

### 4.3 映射表（对齐研究文档 ../08 §3）

| ACP 概念 | Aletheon 映射 | 锚点 |
|---|---|---|
| initialize/client capabilities | connection negotiation | `client.rs:161-173` |
| authenticate | principal establishment | `check_peer_cred` server.rs:570-604 |
| new/load session | session use cases | `session_service.rs:47-82` |
| prompt | turn 提交（后续接 G3 队列） | `execute_turn` execute.rs:19-26 |
| cancel | turn cancellation token | `submit_with` runner 的 CancellationToken |
| request permission | scoped approval use case | `approval_service.rs:46-59` |
| session notification | `ClientEvent`/`ItemEvent` 投影 | `client.rs:103-143` |

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 新增 | `crates/interact/src/acp/mod.rs` | ACP 适配器 + 请求/响应类型 |
| 新增 | `crates/interact/src/acp/event_map.rs` | ClientEvent → ACP session update 纯映射 |
| 新增 | `crates/interact/src/acp/transport.rs` | ACP stdio/socket 传输（stdio 为 IDE 常用） |
| 修改 | `crates/interact/src/tui/cli.rs:27-83` | 新增 `--acp` 入口模式 |
| 修改 | (可能) `Cargo.toml` | 引入 `agent-client-protocol` crate（许可证审查后） |
| 修改 | feature flag | `grok_hardening.acp_adapter` 默认关 |

## 6. 任务分解（TDD）

**阶段 A：纯映射（无 I/O，最高价值先做）**
- T1. `event_map.rs`：`map_client_event_to_acp`。单测：`ItemEvent{phase:Streaming, delta}` → ACP text chunk；`Failed` → ACP error。
- T2. `ApprovalEvent` → ACP request permission，**关联 turn/call id**（不建 ACP 私有授权缓存）。单测。
- T3. `Reconnected(cursor)` → 先 snapshot 再增量的映射。单测。

**阶段 B：correlation + principal**
- T4. `AcpCorrelation`：ACP session id ↔ (ConnectionId, ThreadId) 映射，有界。单测。
- T5. principal 建立：ACP connection 经 `check_peer_cred` → `PrincipalId::local_uid` → `PrincipalContext::new`。客户端给的 session id **不**作 authority。单测。

**阶段 C：请求分发（第一版子集）**
- T6. Initialize：`negotiate_protocol_version` + 返回 agent capabilities。单测。
- T7. NewSession：经 `session_service` create + 记 correlation。集成测试。
- T8. Prompt：经 `execute_turn`（第一版直提交；G3 落地后接队列）。集成测试。
- T9. Cancel：触发 `submit_with` runner 的 CancellationToken。集成测试。

**阶段 D：传输 + 事件流**
- T10. `transport.rs` stdio ACP 帧读写。单测（mock stdin/stdout）。
- T11. Aletheon `ClientEvent` 流经 `map_client_event_to_acp` 推给 ACP 客户端。集成测试：一个 turn 的 Item 流正确翻译。

**阶段 E：入口 + 收尾**
- T12. `cli.rs` `--acp` 模式；flag 关时不暴露（现有 JSON-RPC 路径不变）。
- T13. 端到端：TUI 与 ACP 对同一 turn 得等价 terminal 状态（用现有 stream 断言）。
- T14. clippy/fmt；更新 §2 漂移；标注 flag 灰度 + 许可证审查结论。

## 7. 兼容与迁移

- **flag 关闭**：无 ACP 入口，现有 Unix socket JSON-RPC 客户端完全不受影响。
- **边缘隔离**：ACP 类型只存在于 `crates/interact/src/acp/`；Executive 及以下只见 `PrincipalContext`/`TurnRequest`/`ClientEvent`。
- **分阶段**：第一版四方法；load/reconnect、permission round-trip、client FS/terminal、mode/model 逐阶段接入（各自子任务）。
- **依赖 G2/G3**：prompt 的丰富进度依赖 G2 streaming；prompt 排队/插话依赖 G3。第一版可先用现有单输入直提交。

## 8. 测试计划（映射研究文档 ../08 §7 验收方向）

| 验收方向 | 测试 |
|---|---|
| TUI 与 ACP 对同一 turn 得等价 terminal | T13 |
| ACP 不能绕过 WorkspacePolicy/approval/sandbox/budget | T5, T8（走 execute_turn 权威路径） |
| reconnect 不重复 turn、不丢 terminal event | T3, T11（EventCursor 复用） |
| 两 principal 不能 load/observe 对方 session | T5（session id 非 authority） |
| 适配器崩溃不改 Executive 权威 run 状态 | T11（适配器无领域状态） |

## 9. 可观测性

- 指标：`acp_sessions_active`、`acp_prompt_total`、`acp_reconnect_total`、`acp_map_unmapped_event_total`（未映射的 ClientEvent 计数）。
- 日志：未知 ACP 方法、principal 不可见的 load 拒绝。

## 10. 许可证

若引入外部 `agent-client-protocol` crate，需审查其许可证并更新 `THIRD-PARTY-NOTICES`。适配器与映射代码为 Aletheon 原创，不复制 Grok `xai-acp-lib` 源码（只参考其 gateway 方法集概念）。
