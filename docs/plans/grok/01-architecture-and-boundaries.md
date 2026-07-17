# Grok Build 与 Aletheon：总体架构及可借鉴边界

## 1. 两个项目解决的问题不同

Grok Build 是终端 coding-agent 产品：同时提供交互式 TUI、脚本化 headless 和 ACP 编辑器嵌入（`/home/aurobear/Bear-ws/grok-build/README.md:13-17`）。其仓库把 composition root、TUI、agent runtime、tools、workspace 分为独立 crate（`/home/aurobear/Bear-ws/grok-build/README.md:95-105`）。

Aletheon 是带治理、认知、记忆、Agora 和多 Agent 权威模型的执行系统。当前 AgentControl 请求包含可信工作区、上下文 fork、广播引用、工具 allowlist 和层级预算（`crates/fabric/src/types/agent_control.rs:189-207`）；Capability 调用由 Executive 注入 principal、thread、turn、workspace、sandbox 和 cancellation（`crates/executive/src/service/governed_capability.rs:20-37`）。

因此，二者的正确组合不是：

```text
Aletheon -> 调用 Grok Agent -> 让 Grok 接管执行
```

而是：

```text
                    可借鉴的宿主/runtime 机制
Grok Build ------------------------------------------------+
  folder trust / tool stream / prompt queue / rewind / ACP |
                                                           v
Client -> Interact -> Executive -> Cognit -> Capability -> Kernel/Tools
                         |           |             |
                         +-> AgentControl           +-> governed authority
                         +-> Mnemosyne/Agora/Dasein

                    Aletheon 继续拥有所有领域权威
```

## 2. 可借鉴层与禁止替换层

| 层 | Grok 强项 | Aletheon 当前基础 | 决策 |
|---|---|---|---|
| 客户端接入 | TUI/headless/stdio/ACP/leader 多入口；入口导入见 `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-pager-bin/src/main.rs:28-47` | Interact CLI/TUI 解析 workspace 后发起 turn（`crates/interact/src/tui/cli.rs:218-243`） | 借鉴 adapter 分层 |
| 工作区宿主 | folder trust、FS/VCS、checkpoint | `WorkspacePolicy` 管 cwd、writable roots、protected paths（`crates/fabric/src/types/local_authority.rs:70-117`） | 补充信任维度，不替换 policy |
| 工具运行时 | 类型化参数/输出、动态 manifest、流式 progress | `Tool` 有权限、exposure、并发分类，但只返回最终 `ToolResult`（`crates/fabric/src/types/tool.rs:100-175`） | 引入流式执行契约 |
| 输入协调 | 版本化 prompt queue、mid-turn interjection | 当前 turn 请求是单一 input（`crates/fabric/src/types/turn.rs:8-16`） | 新增 session actor/队列 |
| 生命周期扩展 | contributor 不拥有主循环 | Aletheon 有 session lifecycle use cases 和 Corpus hooks（`crates/executive/src/service/request_use_cases.rs:330-377`） | 抽象 typed contributor |
| 多 Agent | 隐藏 child sessions，共享 FS/terminal/hunk/env | AgentControl 已有强权威、预算、恢复和隔离 | 仅吸收资源生命周期 |
| 记忆 | FTS5 + vector KNN + fallback | Mnemosyne 已有 authority/scope/retention/promotion（`crates/mnemosyne/src/lib.rs:23-60`） | 不替换，只吸收检索和凭证策略 |

## 3. Grok 的 AgentBuilder 值得借鉴什么

`AgentBuilder` 把真实 cwd 与 model-facing cwd 分开，避免 overlay/worktree 物理路径泄露给模型，同时工具仍使用真实路径（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-agent/src/builder.rs:42-52`）。它还集中装配 terminal、filesystem、owner session、parent scheduler、tool allow/deny、permission mode、memory、subagents、plugins、context window 和 API key provider（同文件 `:53-135`）。

可借鉴点不是复制一个巨型 builder，而是明确三种数据：

```text
AuthorityContext     可信、不可由模型伪造
RuntimeResources     FS/terminal/scheduler/cancel/notification
PromptProjection     可呈现给模型的 cwd、skills、tools、memory snippets
```

Aletheon 已经把可信 authority 放在 `CapabilityExecutionContext`（`crates/executive/src/service/governed_capability.rs:20-37`），因此候选改进是把 runtime resources 和 prompt projection 也做成显式、有界的装配对象，而不是把 authority 回收到 builder 内。

## 4. 保留 Aletheon 的核心权威

以下部分不应被 Grok 对应模块替换：

1. **AgentControlPort**：Aletheon 的 spawn/wait/send/cancel/inspect/list 是跨 runtime 的领域端口（`crates/fabric/src/types/agent_control.rs:506-528`）。
2. **层级预算与恢复**：`AgentBudget` 包含 token、tool call、elapsed、cost、depth 限制（`crates/fabric/src/types/agent_control.rs:157-186`），`RuntimeResumability` 与 recovery receipt 已是正式合同（同文件 `:26-60`）。
3. **受治理 Capability**：Executive 是授权边界，Cognit 只提交 `CapabilityCall`（`crates/executive/src/service/governed_capability.rs:1-5`）。
4. **Memory authority**：Mnemosyne 暴露 authority、scope、sensitivity、status、retention 和 promotion（`crates/mnemosyne/src/lib.rs:23-56`）。
5. **Conscious-core/Agora/Dasein**：这些是 Aletheon 的认知域，不是 coding-agent shell 的交互机制。

## 5. 迁移原则

- **Adapter first**：新协议/客户端在 Interact/Executive 边缘适配，不污染 Cognit。
- **Authority stays typed**：来自 cwd、客户端或模型的字符串不能直接升级为 Workspace/Principal authority。
- **Single terminal truth**：工具流、turn 流和 Agent run 都必须只有一个权威终态。
- **Session-scoped multi-user state**：队列、interjection、通知、approval 必须以 principal/session/thread 归属。
- **Additive rollout**：所有高风险机制先做 feature flag + telemetry + fallback。
