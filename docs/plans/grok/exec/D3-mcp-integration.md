# D3 合并可执行 Spec：MCP Integration × Grok G7

> 合并 DeepSeek `../../deepseek/2026-07-17-mcp-integration-plan.md`（4 PR）与 Grok `G7-memory-search.md`（`mnemosyne::credential` 已提交）。
> 执行前按 `00-EXECUTION-INDEX.md §0` 重新核对锚点。

## 1. 一句话

MCP client 已工作（3 传输 + bearer/OAuth + discovery + ToolRegistry 接线）。DeepSeek 计划 = 统一/硬化/补 resources+elicitation。Grok **G7** 已交付 endpoint-scoped 凭证守卫（`approved_for`），正好补上 MCP 的 token 释放缺乏 endpoint-scoping 的缺口——**build-on**。（Grok G8 `interact::acp` 与 MCP **无关**：ACP 是出站 UI/session 协议，MCP 是入站工具-server 协议。）

## 2. 现状锚点（合并，需重新核对）

| 事实 | 锚点 |
|---|---|
| 两处 `McpServerConfig` 并存（§3.1 HIGH） | cognit TOML 型 `crates/cognit/src/config/mod.rs:664`（string-based）；corpus 运行时型 `crates/corpus/src/tools/mcp/config.rs:23`（enum-based） |
| 内联转换 | `crates/executive/src/impl/daemon/bootstrap/request.rs:385`（ToolRegistry 接线 `:382-417`） |
| §3.7 **疑似 trust 映射反转** | `wrapper.rs` `Untrusted => PermissionLevel::L2`（system 级）——计划疑应 L1，**先验证再改** |
| `health_check_interval_sec:30` 定义但无重连循环用 | `corpus/tools/mcp/config.rs:8,17`（§3.5） |
| 无 `list_resources`/`read_resource`/`elicitation` 符号 | mcp 模块内不存在 |
| **Grok G7 已提交** `EmbeddingCredentialGrant::approved_for(request_base_url, now_unix)`（精确 base-url 匹配、拒 hostname-suffix 扩展、Debug 隐藏 secret） | `mnemosyne::credential`（`299f9d68`） |

## 3. Phase 1 —— 统一 + 硬化（采纳 G7）

DeepSeek Phase 1（1 PR）：
- **D3-T1**：统一 `McpServerConfig` 到单一类型（DeepSeek §6：新 `McpServerConfig`+`McpTransport`+`McpTrustLevel` 放 `cognit/src/config/mod.rs`；corpus 侧改用之）；消除 `request.rs:385` 内联转换。
- **D3-T2**：allowlist/denylist 在注册时生效；per-tool `permission_overrides`。
- **D3-T3（先验证）**：核实 §3.7 trust 映射——读 `wrapper.rs` 确认 `Untrusted=>L2` 是否真反转；若确认，改 `Untrusted=>L1`。**这是安全项，改前必须验证语义**（对齐 grok 教训：强断言先复核）。
- **D3-T4**：接线 `health_check_interval_sec` 的后台健康检查 + 重连。
- **D3-T5（采纳 G7）**：把 G7 `approved_for` 语义引入 `corpus/tools/mcp/auth.rs` / `token_store.rs`：bearer/OAuth token 仅在**请求 base_url 与授权 endpoint 精确匹配**时释放（拒绝重定向/错误 host）。当前 `TokenStore`（keyed by `TokenKey`）**无 endpoint-scoping**——这是缺口。G7 的 `approved_for` 正是所缺守卫。**测试**：token 不发往 hostname-suffix 扩展的 host；重定向到新 origin 不携带凭证（G7 已有等价测试可参照）。

**验收（DeepSeek §8 DoD 相关项 + G7）**：单一 `McpServerConfig`；allowlist/denylist 生效；trust→permission 正确且可 override；health-check 重连；**token 释放受 endpoint-scoping 约束**。

## 4. Phase 2 —— Resources + Notifications

- **D3-T6**：`list_resources()`/`read_resource()`/`list_resource_templates()`；新 `mcp_resource_read` 工具 + `McpResourceProvider`。
- **D3-T7**：响应 `tools/list_changed`（刷新 registry）。

## 5. Phase 3 —— Elicitation + Parallel

- **D3-T8**：`elicitation/create` 路由到 `SocketApprovalGate` + `ApprovalRepository`（复用现有 approval 系统，勿建 MCP 私有授权缓存）。
- **D3-T9**：检测 `supports_parallel_tool_calls` → `ConcurrencyClass::ReadOnly`。

## 6. Phase 4 —— HTTP/OAuth polish（再用 G7）

- **D3-T10**：连接池 + retry/backoff + per-server timeout。
- **D3-T11**：OAuth metadata discovery（RFC 8414）。**重定向/新 origin 时，D3-T5 的 G7 endpoint-scoping 必须继续生效**（Phase 4 引入 redirect，凭证泄漏风险最高处）。
- **D3-T12**：`GbrainMemoryConfig` → `McpMemoryConfig`（向后兼容）。

## 7. 约束（DeepSeek §9）

无 `rmcp` 依赖；不重写传输；不新建 crate；MCP 保持 optional。

## 8. 依赖与顺序

```
Phase 1: T1→T2→T3(先验证)→T4→T5(采纳 G7)
  └─ Phase 2: T6, T7
       └─ Phase 3: T8, T9
            └─ Phase 4: T10→T11(G7 持续生效)→T12
```

## 9. 分工

- **DeepSeek mcp 计划**：4 Phase 的权威任务、验收、约束、类型统一设计（§6）。
- **Grok G7**（`mnemosyne::credential`，已提交）：`approved_for` fail-closed endpoint-scoping —— 本文 D3-T5/T11 采纳到 MCP auth。
- **注意**：G8（ACP）不参与本流；MCP 与 ACP 是不同方向的协议。
