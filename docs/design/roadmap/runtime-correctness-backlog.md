# 运行时正确性问题台账

> 更新日期：2026-07-26
> 性质：已确认但本轮不实现的问题记录，不是实施计划。完成项必须以
> `docs/testing/runtime-correctness.md` 定义的安装态证据关闭。

## 状态定义

| 状态 | 含义 |
|---|---|
| Confirmed | 已有代码或运行证据，尚未进入实现 |
| Needs evidence | 现象或风险已知，但需要可重复运行证据再决定是否实现 |
| Closed | 已完成安装态验收，并链接验收记录 |

## 已确认问题

### RC-001：Provider 背压不是机器级统一机制

- **状态：** Confirmed
- **优先级：** P0
- **现状：** `LlmScheduler` 在
  `crates/cognit/src/adapters/inference/scheduler.rs:69` 管理其自身调度；provider
  错误能够携带 `Retry-After`（
  `crates/cognit/src/adapters/inference/provider.rs:28`），调度器也会采用该值（
  `crates/cognit/src/adapters/inference/scheduler.rs:288`）。但主会话、多 Session
  和外部 subagent runtime 还没有共享一个 provider 级并发额度与冷却状态。
- **风险：** 多实例或多运行时可在短时间内叠加请求，引发 429、重复重试和长尾延迟；
  单个 Session 内退避正确并不能消除机器级请求风暴。
- **约束：** 不得硬编码一个通用并发数或固定等待时间。配置必须按 provider/model
  可解析，并允许 provider 响应动态更新冷却期限。
- **关闭证据：** 并发多 Session 与 subagent 压测中分别记录 inference rounds、
  provider attempts、retries 和 admission wait；证明全局额度生效且单 Session 长任务
  不被饿死。

### RC-002：Monitor 健康检查使用了错误的 systemd 作用域

- **状态：** Confirmed
- **优先级：** P1
- **现状：** monitor 的 health 路径调用系统作用域的
  `systemctl is-active aletheon`（`tools/aletheon-monitor/src/tools/health.py:45`），
  但其他场景明确查询用户作用域 `aletheon.service`（
  `tools/aletheon-monitor/src/scenarios.py:41`）。2026-07-26 安装态测试中 daemon
  socket 可达，而 health 仍返回 `systemd.active=false`。
- **风险：** 测试器会把健康运行的 user daemon 判为失败，或者诱导不必要的重启。
- **关闭证据：** health 同时报告 machine core 与 user daemon 的 unit、PID、
  `ActiveState` 和 `NRestarts`，并有 system/user unit 分离测试。

### RC-003：TUI settle timeout 存在终态误判

- **状态：** Confirmed
- **优先级：** P1
- **现状：** settle 逻辑位于 `tools/aletheon-monitor/src/tools/tui.py:186`。
  2026-07-26 的第三次连续 TUI 验收中，返回的三帧字节相同、`❯` 已出现且 session
  JSONL 已有 `turn_done`，但 capture 仍返回 `stable=false, timeout=true`；证据已记录在
  `docs/testing/runtime-correctness.md` 的安装态验收章节。
- **风险：** 正确完成被误报为超时，自动 debug loop 可能重复发送任务或错误重启服务。
- **关闭证据：** settle 判定联合 frame hash、prompt 可见性和 durable terminal event；
  添加“最后三帧相同且 turn_done 已到达”的回归测试。

## 需要更多运行证据

### RC-004：长时间与并发 Session 回归覆盖不足

- **状态：** Needs evidence
- **优先级：** P1
- **依据：** 项目状态仍将 long-duration 与 concurrent-session coverage 标记为 Partial
  （`docs/design/README.md` 的 Testing 行）。现有验收覆盖了三轮持续会话和三次新会话，
  还不能证明小时级任务、resume、取消以及多 TUI 同时运行时不会串流或污染历史。
- **取证要求：** 使用独立 session ID 和事件文件运行多客户端；记录每轮上下文、延迟、
  推理次数、重试次数和最终 session ownership，不用总 token 或 tool count 代替这些指标。

### RC-005：探索停止策略缺少稳定基准

- **状态：** Needs evidence
- **优先级：** P1
- **依据：** tester 已要求项目概览先批量读取入口文件并分别统计 inference/tool/retry，
  但还没有一组版本化的普通项目、陌生项目和复杂深挖任务基准。
- **风险：** 为降低普通概览 token 而设置过紧阈值，可能损害长期复杂分析；阈值过松又会
  恢复无界 glob、逐扩展名扫描和重复读取。
- **取证要求：** 采用 token-budget 驱动的停止原因，分别比较入口批读、定向发现、复杂
  深挖三类场景；不以固定文件数、固定轮数或固定 50/100 次上限作为通用策略。

### RC-006：运行事实尚未投影到所有客户端/API

- **状态：** Needs evidence
- **优先级：** P2
- **依据：** TUI 推理上下文已经使用 host-owned `ModelRuntimeFacts`，但项目状态仍将
  “Unified runtime-fact projection to every UI/API” 标为 Partial
  （`docs/design/README.md` 的 Observability 行）。
- **取证要求：** 清点 TUI、snapshot、monitor、session inspect 和 daemon API 的模型、
  上下文、session 与 budget 字段；只有存在真实消费者缺口时再扩展协议。

## 维护规则

1. 本文只记录可验证的问题，不展开文件级实施步骤。
2. 新问题必须包含运行证据或精确代码位置；推测项标记为 Needs evidence。
3. 不因 Codex、Claude 或 Pi 存在某功能就自动列为缺口，必须证明 Aletheon 的用户场景需要。
4. 问题关闭前必须通过系统安装态验收；开发二进制、直接 RPC 和 isolated daemon 仅作诊断。
5. 已关闭条目移动到文末并链接 commit、二进制 hash 与真实 TUI/session 证据。
