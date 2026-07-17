# 流式工具运行时

## 1. Grok 的关键合同

Grok 的统一 `Tool` 允许工具实现简单的 `run` 或流式 `execute`，而 runtime 始终调用 `execute`；默认实现把非流式结果包装为单 terminal item（`/home/aurobear/Bear-ws/grok-build/crates/common/xai-tool-runtime/src/tool.rs:32-112`）。流必须满足：零到多个 `Progress`，最后恰好一个 `Terminal`（同文件 `:10-13`、`:114-125`）。

Grok 还支持：

- 类型化 `Args` 与 `Output`（同文件 `:36-47`）。
- 根据 per-turn context 动态生成 tool description 和决定是否列出（同文件 `:52-77`）。
- `Progress` 可为 text、rich content 或自定义 payload（同文件 `:135-153`）。
- per-call typed extensions，可装 cwd、session、trace、cancellation（`/home/aurobear/Bear-ws/grok-build/crates/common/xai-tool-runtime/src/context.rs:10-98`、`:112-142`）。

## 2. Aletheon 当前差距

Aletheon 的 `Tool` 已有 permission level、exposure 和 concurrency class，但 `execute` 一次性返回 `ToolResult`（`crates/fabric/src/types/tool.rs:153-175`）；`ToolResult` 只有 content、is_error、execution time 和 truncated（同文件 `:100-112`）。

Aletheon 的 turn/client 协议已经可以流式传输 text、tool start/complete/result、usage、approval、subagent status 和 interruption（`crates/fabric/src/ipc/stream.rs:164-267`）。因此缺口主要位于“工具执行内部 -> turn event spine”这一段，而非客户端传输层从零开始。

## 3. 建议的候选合同

```text
Tool::execute(...)
    -> ToolExecutionStream
         Progress(ToolProgress)
         Notification(ToolNotification)   // 可选、非模型内容
         Terminal(Result<ToolResult, ToolExecutionError>)
```

建议区分三类事件：

| 类别 | 用途 | 是否进入模型上下文 |
|---|---|---|
| `Progress` | stdout chunk、下载进度、搜索阶段、子任务状态 | 默认不进入；可压缩为最终摘要 |
| `Notification` | UI 状态、后台任务句柄、监控事件 | 不进入 |
| `Terminal` | 唯一权威最终结果/错误/usage/audit | 进入，且只能有一个 |

不要把 Grok 的所有 `ContentBlock` 原样复制。Aletheon 应先覆盖 text、structured JSON、resource reference；二进制 image 应进入受控 artifact/media store，避免 base64 大块直接冲击 turn buffer。

## 4. 运行时数据流

```text
Tool implementation
  | progress/terminal
  v
GovernedCapabilityInvoker
  | enforce authority, cancellation, budget, lease, audit
  v
Tool stream adapter
  +--> TurnEventV1 / EventSpine --> TUI / ACP / headless
  +--> bounded progress accumulator
  `--> final CapabilityResult / ToolResult
```

受治理边界不能被 stream 绕过。Executive 当前是 canonical capability entry point（`crates/executive/src/service/governed_capability.rs:49-61`），因此 stream 的每个 terminal 仍必须经过同一 settle/audit 路径；progress 不能自行代表成功。

## 5. Backpressure 与取消

- 每个 tool call 使用有界 channel；Aletheon turn stream 当前构造容量为 64（`crates/executive/src/service/turn_pipeline.rs:360-362`），工具层应有独立上限。
- 文本 progress 应按字节/频率合并，不能每字符发事件。
- 慢客户端不应阻塞工具完成；超过缓冲后丢弃或采样 progress，但绝不丢 terminal。
- cancellation 应是 per-call token；Grok 把 `CancellationToken` 放入 typed context（`/home/aurobear/Bear-ws/grok-build/crates/common/xai-tool-runtime/src/context.rs:138-142`），Aletheon 已在 `InvocationControl` 中持有 token（`crates/fabric/src/include/turn.rs:70-80`）。
- producer panic/stream end without terminal 必须由 runtime 合成 terminal error。
- 第二个 terminal、terminal 后 progress、call_id 不匹配必须作为协议错误记录。

## 6. 兼容迁移

建议保留非流式工具实现的低成本路径：

```text
legacy ToolResult
   -> terminal_only(Ok(result))
```

迁移顺序：

1. Fabric 定义事件和 stream invariant。
2. Adapter 为所有旧工具自动生成 terminal-only stream。
3. 先迁移 terminal/bash、web fetch、长时 MCP 调用。
4. Executive bridge progress 到现有 `TurnEventV1` 或新增 tool-progress variant。
5. TUI/ACP/headless 分别决定展示策略。

## 7. 验收方向

- 任意正常调用恰好产生一个 terminal。
- cancellation 后不会再产生成功 terminal。
- progress 洪水不会导致 terminal 丢失或 OOM。
- 旧工具无需修改仍可执行。
- progress 不计入模型上下文，除非显式投影。
- usage/audit 只以 settle 后 terminal 为准。
