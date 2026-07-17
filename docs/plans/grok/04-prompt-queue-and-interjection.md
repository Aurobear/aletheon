# Prompt Queue 与 Mid-turn Interjection

## 1. 为什么是两个机制

```text
Prompt Queue                      Mid-turn Interjection
------------------------------    ---------------------------------
排队等待成为下一正式 turn          当前 turn 内在安全点注入新用户信息
支持查看、编辑、取消、排序           不抢占任意指令执行，不直接改写历史
```

Grok 的 queue item 带稳定 id、单调 version、owner、last editor、kind 和 text（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-prompt-queue/src/types.rs:3-20`）；广播还包含 session id、排队位置和 running prompt id（同文件 `:22-56`）。这套元数据非常适合 Aletheon 的多客户端/多用户场景。

## 2. Grok 的插话语义

Grok 把 mid-turn 输入先缓冲，在安全 drain point 以 FIFO、每条独立 synthetic user message 的形式注入（`/home/aurobear/Bear-ws/grok-build/crates/common/xai-interjection-core/src/buffer.rs:6-41`）。底层共享队列支持 capped push、按谓词 drain 和 FIFO 保序（`/home/aurobear/Bear-ws/grok-build/crates/common/xai-interjection-core/src/events.rs:27-79`）。格式化会在 UTF-8 边界截断大输入（`/home/aurobear/Bear-ws/grok-build/crates/common/xai-interjection-core/src/format.rs:1-32`）。

## 3. Aletheon 当前适配点

当前 `TurnRequest` 只有单一 `input`（`crates/fabric/src/types/turn.rs:8-16`），AgentControl 的 `send` 是面向 child Agent 的控制端口，不等价于 session prompt queue（`crates/fabric/src/types/agent_control.rs:506-518`）。因此建议由 session actor/Executive 持有 queue，不放进 Cognit harness。

```text
Clients (TUI/ACP/API)
   | enqueue/edit/cancel/interject
   v
Session Input Coordinator  <-- principal/thread/connection authority
   +--> PendingPromptQueue
   +--> RunningPrompt
   `--> InterjectionBuffer
           |
           v safe points only
       Unified Turn Coordinator -> Cognit
```

## 4. 建议的数据模型

候选 `PromptEnvelope`：

- `prompt_id`
- `version`
- `principal_id`
- `connection_id`
- `thread_id`
- `session_id`
- `kind`：prompt/control/interjection
- `content` / bounded attachment refs
- `created_at` / `updated_at`
- `state`：queued/running/completed/cancelled/rejected
- `idempotency_key`

候选并发规则：

1. edit/cancel 必须带 expected version；版本过期返回 conflict，不能静默覆盖。
2. owner 永不因编辑而改变，last editor 每次更新。
3. 跨 principal 编辑默认禁止；共享线程需要显式 policy。
4. running prompt 不允许原地编辑；只能转为 interjection 或 enqueue next。
5. queue 广播按 session/thread 分区，避免向其他用户泄露 prompt text。

## 5. Safe drain points

插话不应在任意 await 点注入。建议只在以下位置 drain：

- LLM response 完成、准备下一 iteration 前。
- 一组工具调用全部 settle 后。
- approval 恢复后、重新调用模型前。
- compaction/recovery 完成后。

以下位置不 drain：

- 工具写文件的中间阶段。
- Kernel lease/admission 尚未 settle。
- 正在提交 checkpoint/rewind。
- 正在持有 session state 写锁。

## 6. Interjection 与 interrupt 的区别

| 用户意图 | 机制 |
|---|---|
| “顺便也检查测试” | interjection |
| “下一步做文档” | queue next |
| “立刻停止” | cancellation/interrupt |
| “不要执行这个高风险工具” | approval deny + cancellation |

Aletheon 的 client event 已有 `Interrupted` 和 `Approval`（`crates/fabric/src/ipc/stream.rs:223-255`），所以不要把“停止”编码为文本插话。

## 7. 故障与恢复

- queue state 应可持久化，daemon 崩溃后能区分 running-but-unconfirmed 与 queued。
- running prompt 需要幂等恢复决策，不能自动重复副作用 turn。
- interjection 尚未 drain 时应持久化；已投影到 transcript 后必须标记 consumed。
- 广播失败不影响 queue 的权威状态，客户端可用 snapshot 重新同步。
- 大输入和 attachment 数量必须有界；Grok 的 queue/format 思路可借鉴，但阈值要用 Aletheon 自身合同。

## 8. 验收方向

- 两客户端同时 enqueue 时顺序确定且可观察。
- stale edit 返回 conflict。
- owner/last editor 和 principal 隔离正确。
- 插话只在 safe point 出现，且 FIFO、不合并、不重复。
- cancel 与 interjection 语义不混淆。
- daemon 重启后不会丢 queued prompt 或重复已消费插话。

