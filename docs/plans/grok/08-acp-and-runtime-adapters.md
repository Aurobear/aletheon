# ACP 与多入口 Runtime Adapter

## 1. Grok 的入口分层

Grok 同一 composition root 可进入 TUI、headless、stdio agent 和 leader 模式；相关入口函数在 `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-pager-bin/src/main.rs:28-47`。README 明确 ACP 用于嵌入编辑器（`/home/aurobear/Bear-ws/grok-build/README.md:13-17`）。

其 `xai-acp-lib` gateway 把 initialize/authenticate/new/load session、mode、prompt、cancel 和扩展方法分发到 Agent 侧（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-acp-lib/src/gateway.rs:171-222`），反向 client capability 包括 permission、read/write file、terminal 和 session notification（同文件 `:390-463`）。

## 2. 对 Aletheon 的意义

ACP 应被视为客户端/宿主协议 adapter，不应成为新的领域运行时：

```text
IDE / ACP Client
      |
      v
Interact ACP Adapter
  - protocol session <-> Aletheon principal/thread/session
  - ACP permission <-> approval use case
  - ACP fs/terminal <-> governed capability
  - Aletheon events <-> ACP session updates
      |
      v
Executive use-case ports -> Cognit / AgentControl / Kernel
```

这样 TUI、headless、ACP、未来 Web UI 都共享同一权威路径。

## 3. 映射建议

| ACP 概念 | Aletheon 映射 |
|---|---|
| initialize/client capabilities | connection negotiation |
| authenticate | principal establishment |
| new/load session | session use cases |
| prompt | prompt queue / unified turn coordinator |
| cancel | turn cancellation token |
| session mode/model | thread/session policy，需权限校验 |
| request permission | scoped approval use case |
| fs read/write | WorkspacePolicy + governed capability |
| terminal create/output/kill | Corpus tool adapter + Kernel lease |
| session notification | `TurnEventV1`/Agent events 投影 |

Aletheon 已有结构化 turn event 流，包括 tool、usage、approval、subagent、interrupt（`crates/fabric/src/ipc/stream.rs:164-267`），因此 ACP adapter 主要工作是语义映射与恢复，而不是重新发明事件源。

## 4. 多用户与会话恢复

- 每个 ACP connection 绑定 authenticated principal；客户端给出的 session id 不能直接作为 authority。
- load session 必须验证 principal/thread 可见性。
- permission request 必须关联 turn/call id，复用 Aletheon scoped approval，不建立 ACP 私有授权缓存。
- reconnect 后先同步权威 session snapshot，再恢复增量事件；不能假设客户端收到过最后一条 notification。
- adapter 维护 protocol correlation id，领域层继续使用 Aletheon 的 TurnId/OperationId/AgentId。

## 5. 不建议直接复制的内容

- Grok 特有 leader/version/update 机制：与 Aletheon daemon supervisor 不同。
- Grok 的 session actor 内部状态：会与 Executive/AgentControl 重叠。
- Grok 的 permission mode：不能替换 Aletheon principal/thread/turn approval authority。
- Grok 特有 client extension methods：先做最小 ACP 标准面，再按需求扩展。

## 6. 分阶段建议

1. 只支持 initialize/new session/prompt/cancel/session notifications。
2. 接入 load session 和 reconnect snapshot。
3. 接入 permission round-trip。
4. 接入 client-provided FS/terminal capability，但仍经过 governed adapter。
5. 再评估 mode/model 和 extension methods。

## 7. 验收方向

- TUI 与 ACP 对同一 turn 得到等价 terminal 状态。
- ACP 不能绕过 WorkspacePolicy、approval、sandbox、budget。
- reconnect 不重复 turn，不丢 terminal event。
- 两个 principal 不能 load/observe 对方 session。
- adapter 崩溃不改变 Executive 的权威 run 状态。

