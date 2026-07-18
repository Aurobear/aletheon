# G2 可执行 Spec：Streaming Tool Runtime

> 对应研究文档 `../03-streaming-tool-runtime.md`。优先级 P0。
> **状态（2026-07-18）**：T1–T18 的实现与定向验证已完成；§8 属性测试及 §9 全部协议违规日志仍待收口。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：让工具在执行期间可发出零到多个 `Progress` 事件、最后恰好一个 `Terminal` 结果；progress 桥接到现有 turn event 流用于 TUI/远程可观测性；不改变受治理边界与权威终态。

**非目标**：
- 不复制 Grok 的 `ContentBlock`（二进制/image 走后续 artifact store，本期只做 text/structured/resource-ref）。
- 不改 Cognit 的 `CapabilityCall` 提交模型。
- 不引入 per-tool 动态 manifest（Grok 有，本期不做）。
- 不做 backpressure 之外的流控优化。

## 2. 当前代码锚点（已复核 @ current branch）

| 符号 | 位置 | 当前事实 |
|---|---|---|
| `Tool::execute_streaming` | `crates/fabric/src/types/tool.rs:155-175` | 旧工具继承 terminal-only adapter；无需修改即可进入 G2 路径 |
| `ToolEventSink` | `crates/fabric/src/types/tool_stream.rs` | progress/notification 有界 try-send；terminal 唯一且保留供 governed executor 校验 |
| `TurnEventV1::ToolProgress` | `crates/fabric/src/ipc/stream.rs:171-216` | canonical turn spine 的附加 progress 事件 |
| `GovernedCapabilityInvoker` | `crates/executive/src/service/governed_capability.rs:176-375` | flag 开走 streaming bridge；settled `CapabilityResult` 仍是 usage/audit 权威终态 |
| `bridge_tool_stream` | `crates/executive/src/service/tool_stream_bridge.rs:173+` | 4 KiB/100 ms 合并、64 event 上限、drop/cancel fail-closed、按 tool 计数 |
| guarded executor | `crates/corpus/src/security/runner.rs:223-244` | policy/approval/loop/output/audit 同一路径调用 `execute_streaming`，不重复副作用 |
| sandbox streaming | `crates/fabric/src/types/sandbox.rs:269+` + `crates/corpus/src/security/sandbox/streaming.rs` | bash stdout/stderr 在子进程运行期间逐行发 progress；完整结果仍被捕获 |
| client projection | `crates/executive/src/service/turn_pipeline.rs:755-765` | progress 投影为 `ClientEvent::ToolProgress`，TUI/CLI/ACP 可观察 |
| feature flag | `crates/executive/src/core/config/grok_hardening.rs:20` | `streaming_tools` 默认 false；关闭态保留 legacy invoke/execute 路径 |

**关键约束**：progress 不写入模型输出或 canonical ToolResult；唯一权威终态仍由 admit → execute → settle 产生，bridge 只负责可观测性与缺失 terminal 的 fail-closed 检测。

## 3. 权威归属决策（doc10 §6 八问）

1. **权威 owner**：Fabric 定义 stream 契约与 invariant；Executive（`GovernedCapabilityInvoker`）拥有 settle/audit，是唯一 terminal 权威。
2. **scope**：progress 无持久状态；terminal 归属现有 `CapabilityResult`（已带 call_id/turn 归属）。
3. **crash 恢复**：progress 不持久、不参与恢复；terminal 走现有 capability 恢复路径。producer 中断无 terminal → runtime 合成 terminal error。
4. **fail 模式**：progress 发送失败 = 降级（丢弃/采样），不影响 terminal；terminal 发送失败 = fail closed（记审计错误）。
5. **上限**：per-call progress channel 容量独立（默认 32）；text progress 按字节/频率合并；超限采样丢弃 progress，**绝不丢 terminal**。
6. **兼容**：旧工具经 adapter 自动生成 terminal-only stream，零改动。
7. **进 event spine**：progress 桥接为新增 `TurnEventV1::ToolProgress`；terminal 仍走既有 `ToolResult`。
8. **许可证**：重新实现，不复制 Grok `xai-tool-runtime` 代码。

## 4. 类型定义

### 4.1 新增 Fabric 类型 — `crates/fabric/src/types/tool_stream.rs`（新文件）

```rust
//! 流式工具执行契约。工具可发出零到多个 Progress，最后恰好一个 Terminal。
//! 受治理边界不可被 stream 绕过：terminal 仍须经 Executive settle/audit。

use serde::{Deserialize, Serialize};
use crate::types::tool::ToolResult;

/// 单次工具调用的执行事件。invariant：0..N 个 Progress/Notification，然后
/// 恰好 1 个 Terminal，其后不得再有任何事件。
#[derive(Debug, Clone)]
pub enum ToolExecutionEvent {
    /// 进度：默认不进入模型上下文，可压缩为最终摘要。
    Progress(ToolProgress),
    /// 通知：UI 状态 / 后台句柄 / 监控，永不进入模型上下文。
    Notification(ToolNotification),
    /// 唯一权威终态。Ok = 正常结果，Err = 执行错误。
    Terminal(Result<ToolResult, ToolExecutionError>),
}

/// 进度事件负载。本期只支持 text 与 structured；二进制走后续 artifact store。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolProgress {
    /// 文本片段（stdout chunk、阶段描述）。已在工具侧按行/字节合并。
    Text(String),
    /// 结构化进度（下载 %、搜索阶段、子任务计数）。
    Structured(serde_json::Value),
    /// 资源引用（已落地文件/artifact 的 host-minted 引用，不含内容）。
    ResourceRef { uri: String, mime: Option<String> },
}

/// 非模型内容的通知（不进入上下文，不影响 terminal）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolNotification {
    pub kind: ToolNotificationKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolNotificationKind {
    BackgroundTaskStarted,
    MonitorEvent,
    UiStatus,
}

/// 工具执行错误（区别于 ToolResult{is_error:true} 的"工具正常返回了错误内容"）。
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum ToolExecutionError {
    #[error("tool cancelled: {0}")]
    Cancelled(String),
    #[error("tool panicked or stream ended without terminal")]
    NoTerminal,
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error("execution failed: {0}")]
    Failed(String),
}
```

### 4.2 Stream 句柄与 invariant 强制 — 同文件

```rust
use tokio::sync::mpsc;

/// 工具侧发送端。工具用它 emit progress，最后 emit 一次 terminal。
/// Drop 时若未发 terminal，runtime 侧的 recv 端会观察到 channel 关闭并合成 NoTerminal。
pub struct ToolEventSink {
    tx: mpsc::Sender<ToolExecutionEvent>,
    terminal_sent: bool,
}

impl ToolEventSink {
    /// 发送 progress。channel 满时按策略丢弃（返回 false），绝不阻塞 terminal。
    pub async fn progress(&self, p: ToolProgress) -> bool {
        if self.terminal_sent { return false; }
        // try_send：满则丢弃并计数，不阻塞工具完成。
        self.tx.try_send(ToolExecutionEvent::Progress(p)).is_ok()
    }

    pub async fn notify(&self, n: ToolNotification) -> bool {
        if self.terminal_sent { return false; }
        self.tx.try_send(ToolExecutionEvent::Notification(n)).is_ok()
    }

    /// 发送唯一 terminal。第二次调用是协议违规（debug_assert + 记录）。
    pub async fn terminal(&mut self, result: Result<ToolResult, ToolExecutionError>) {
        debug_assert!(!self.terminal_sent, "second terminal is a protocol violation");
        if self.terminal_sent { return; }
        self.terminal_sent = true;
        // terminal 用有界 send（等待消费），保证不丢。
        let _ = self.tx.send(ToolExecutionEvent::Terminal(result)).await;
    }
}

/// per-call progress channel 默认容量。独立于 turn stream 的 64。
pub const TOOL_PROGRESS_CHANNEL_CAP: usize = 32;

pub fn tool_event_channel() -> (ToolEventSink, mpsc::Receiver<ToolExecutionEvent>) {
    let (tx, rx) = mpsc::channel(TOOL_PROGRESS_CHANNEL_CAP);
    (ToolEventSink { tx, terminal_sent: false }, rx)
}
```

### 4.3 Tool trait 扩展（附加，非破坏） — `crates/fabric/src/types/tool.rs`

在现有 `Tool` trait 追加**默认方法**，旧实现零改动：

```rust
// 追加到 Tool trait（tool.rs:154-176 内）：

/// 流式执行。默认实现调用非流式 execute() 并包装为 terminal-only stream。
/// 需要 progress 的工具 override 本方法。
async fn execute_streaming(
    &self,
    input: serde_json::Value,
    ctx: &ToolContext,
    sink: &mut crate::types::tool_stream::ToolEventSink,
) {
    let result = self.execute(input, ctx).await;
    sink.terminal(Ok(result)).await;
}
```

### 4.4 新增 turn 事件变体 — `crates/fabric/src/ipc/stream.rs:169-277`

在 `TurnEventV1` enum 追加（放在 `ToolResult` 之后）：

```rust
/// 工具执行进度。默认不进入模型上下文；客户端可展示。
ToolProgress {
    call_id: String,
    name: String,
    /// "text" | "structured" | "resource_ref"
    kind: String,
    /// text 的 chunk 或 structured 的 JSON 字符串或 resource uri
    payload: serde_json::Value,
},
```

> 注：新增变体是**附加**，`#[serde(tag="type")]` 下旧客户端遇未知 type 应忽略（确认客户端反序列化容错，见任务 T14）。

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 新增 | `crates/fabric/src/types/tool_stream.rs` | stream 契约类型 |
| 修改 | `crates/fabric/src/types/mod.rs` | 导出 `tool_stream` |
| 修改 | `crates/fabric/src/types/tool.rs:154-176` | 追加 `execute_streaming` 默认方法 |
| 修改 | `crates/fabric/src/ipc/stream.rs:169-277` | 追加 `TurnEventV1::ToolProgress` |
| 修改 | `crates/executive/src/service/governed_capability.rs:109-191` | invoke 内改用 streaming：起 recv 循环，桥接 progress→turn event，terminal 走原 settle |
| 修改 | `crates/executive/src/service/turn_pipeline.rs:357-372` | 把 progress 事件接入既有 Event→TurnEventV1 桥 |
| 修改 | `crates/executive/src/impl/daemon/handler/tool_executor.rs:374-423` | executor 调用 `execute_streaming` 并持有 progress accumulator |
| 新增 | `crates/executive/src/service/tool_stream_bridge.rs` | progress accumulator + turn event 桥接（有界、合并、采样） |
| 修改 | feature flag config | `grok_hardening.streaming_tools` 默认关；关闭时 executor 走旧 `execute()` |

## 6. 任务分解（TDD，2-5 分钟粒度）

**实现证据**：T1–T18 的提交与定向测试映射见 `00-EXECUTION-INDEX.md §1.4`；最终关闭仍以 §8–§9 为准。

**阶段 A：Fabric 契约（无行为变更）**
- T1. 新建 `tool_stream.rs`，写 `ToolExecutionEvent`/`ToolProgress`/`ToolNotification`/`ToolExecutionError` 类型。`cargo check -p fabric`。
- T2. 写 `ToolEventSink`/`tool_event_channel`。单测：progress 满 channel 返回 false，terminal 后 progress 返回 false。
- T3. 单测：`terminal()` 二次调用被吞掉且 debug_assert 触发（`#[should_panic]` in debug）。
- T4. `mod.rs` 导出；`Tool::execute_streaming` 默认方法。`cargo check -p fabric`。
- T5. 单测：默认 `execute_streaming` 对一个 mock 非流式工具产出恰好一个 Terminal(Ok)。

**阶段 B：turn 事件变体**
- T6. `TurnEventV1::ToolProgress` 变体 + serde roundtrip 单测。`cargo test -p fabric`。

**阶段 C：桥接（有界/合并/采样）**
- T7. 新建 `tool_stream_bridge.rs`：`ToolProgressAccumulator`，输入 `ToolExecutionEvent` 流，输出 (turn events, 最终 terminal)。先写测试：喂 10 条 text progress + 1 terminal → 合并后的 progress 事件数 ≤ 上限，terminal 精确透传。
- T8. 实现 text 合并（按字节阈值，默认 4KB 或 100ms 窗口，取先到）。测试通过。
- T9. 测试：progress 洪水（1000 条）不 OOM、不丢 terminal（channel 满时采样丢弃 progress）。
- T10. 测试：producer 中途 drop（无 terminal）→ 桥接合成 `Terminal(Err(NoTerminal))`。
- T11. 测试：cancellation token 触发后 → 合成 `Terminal(Err(Cancelled))`，且此后不接受成功 terminal。

**阶段 D：Executive 集成（feature flag 后）**
- T12. `governed_capability.rs::invoke`：flag 开时走 streaming（起 `tool_event_channel`，spawn 工具，recv 循环喂 accumulator）；flag 关时走原 `execute()`。关闭态回归测试（行为等价）。
- T13. `tool_executor.rs` 调 `execute_streaming`；progress 经桥转成 `TurnEventV1::ToolProgress` 注入 turn stream（复用 `turn_pipeline.rs:365-372` 的 Event→TurnEventV1 桥）。
- T14. 客户端反序列化容错验证：确认未知 `TurnEventV1` type 被忽略而非 panic（若不容错，先加 `#[serde(other)]` fallback 或 Generic 兜底）。

**阶段 E：首个真实流式工具**
- T15. 选 `bash`/terminal 工具 override `execute_streaming`：stdout 按行 emit `ToolProgress::Text`，退出 emit terminal。集成测试：长命令产出多 progress + 一 terminal。
- T16. 端到端：TUI/监控侧观察到 progress（用现有 monitor MCP 或 stream 断言）。

**阶段 F：收尾**
- T17. `cargo clippy -p fabric -p executive`；`cargo fmt --all`。
- T18. 更新 spec §2 若有签名漂移；标注 flag 默认值与灰度计划。

## 7. 兼容与迁移

- **旧工具**：不实现 `execute_streaming`，走默认包装 → terminal-only stream，行为不变。
- **flag 关闭**：`invoke` 走原 `execute()` 直返 `CapabilityResult`，完全等价当前路径（A/B 回归）。
- **迁移顺序**：Fabric 契约 → 桥接 → Executive（flag 后）→ bash 工具 → 逐步迁移长时工具（web fetch、长 MCP 调用）。
- **客户端**：新变体附加式；`ClientEvent::decode_if_known` 对未知事件返回 `None`，老客户端事件循环不中断（T14 验证）。
- **灰度**：默认保持关闭；先在监控环境按 daemon 开启并观察 `tool_progress_dropped_total`/`tool_no_terminal_total`，两项无异常后再扩大启用范围；发现缺 terminal 或持续 overflow 时关闭 flag 即回到 legacy 路径。

## 8. 测试计划（映射研究文档 §7 验收方向）

| 验收方向（../03 §7） | 测试 |
|---|---|
| 任意正常调用恰好一个 terminal | T5, T7 |
| cancellation 后无成功 terminal | T11 |
| progress 洪水不丢 terminal/不 OOM | T9 |
| 旧工具无需修改可执行 | T5, flag-off 回归 |
| progress 不计入模型上下文 | 桥接单测：progress 不进 CapabilityResult.output |
| usage/audit 只以 settle 后 terminal 为准 | T12（terminal 走原 settle 路径断言 audit_id） |

属性测试（proptest）：随机序列 `[Progress*, (Notification|Progress)*, Terminal]` → 桥接输出恒满足"≤1 terminal 且在最后"。

## 9. 可观测性

- 新事件：`TurnEventV1::ToolProgress`。
- 新指标：`tool_progress_dropped_total{tool}`（采样丢弃计数）、`tool_no_terminal_total{tool}`（合成 terminal error 计数）。
- 日志：协议违规（第二 terminal / terminal 后 progress / call_id 不匹配）以 `warn` 记录并带 call_id。

## 10. 许可证

重新实现契约与语义，不复制 Grok `xai-tool-runtime` 源码。无需 NOTICE 变更。若后续参考其 `ContentBlock` 具体结构，另行审查。
