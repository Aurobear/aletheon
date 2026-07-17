# Aletheon 工程成熟度综合评估

> **日期:** 2026-07-17
>
> **方法:** 8 个独立 Agent 逐行扫描 `crates/` 全部 11 个 crate + `tests/` + `scripts/` + CI workflows
>
> **验证范围:** 架构耦合、事务完整性、Dasein 状态、CI 执行、SubAgent 能力、工具/测试、外部集成

> **校正 (audit follow-up):** 本文原判「Dasein-Agora 意识闭环不存在于生产路径」**已被后续代码级复查推翻**。闭环实际已无条件接线在 turn path 中（证据见 §三.1 与 `2026-07-17-conscious-core-engineering-plan.md`）。真实 gap 不是闭环缺失，而是闭环只「观察-提交」、未「仲裁」。相关段落已就地校正并标注「（校正）」。

## 一、综合评分

| 维度 | 评分 | 说明 |
|------|------|------|
| **工具执行** | ⭐⭐⭐⭐⭐ | 21 工具 + 7 阶段安全管线 + 5 sandbox 后端 + 4 级权限 |
| **SubAgent/Pi** | ⭐⭐⭐⭐⭐ | 3 生产 Runtime + G03 标准控制平面 + fail-closed Pi + worktree 隔离 |
| **LLM 集成** | ⭐⭐⭐⭐⭐ | 3 provider 全生产级 + auto-detect + prompt cache + streaming |
| **测试覆盖** | ⭐⭐⭐⭐⭐ | 2,766 tests + 真实 daemon 测试 + 零 ignored + 仅 7 TODO |
| **外部集成** | ⭐⭐⭐⭐ | MCP/Google/Telegram/channels 生产级；Discord/Slack stub |
| **CI 执行** | ⭐⭐⭐⭐⭐ | 485 行 fitness gate + 20+ 删除门 + baseline 回归防护 |
| **IPC** | ⭐⭐⭐⭐ | Unix socket 生产级；io_uring/shared_mem 部分 |
| **Agora 事务** | ⭐⭐⭐⭐⭐ | 5/5 文档 bug 已修复 + competition + broadcast 生产级 |
| **Dasein 状态** | ⭐⭐⭐⭐ | Event-sourced ledger 完整，但 determine_action 空转 |
| **意识闭环** | ⭐⭐⭐ | Dasein→Agora 循环已接线在 turn path（observe-and-commit），但非仲裁；SelfField 被排除、determine_action 空转（校正） |
| **文档准确度** | ⭐⭐ | 大面积过时，低估了代码成熟度 |

---

## 二、现在就能做的事情

### 1. 单 Agent ReAct 循环 ✅
Daemon 接收任务 → TurnPipeline → SelfField review → LLM 推理 → 工具调用（7 阶段安全审查）→ Agora commit evidence → 结果返回。**完全可用。**

### 2. 多 Agent 协调 ✅
AgentControlService spawn/wait/send/cancel/list。NativeCognitRuntime 可运行多个子 Agent。PiRuntime 可运行独立 coding Agent（需 `config.enabled=true`）。

### 3. 代码生成/修改（Pi Agent）✅
独立 coding Agent 在 git worktree + bubblewrap sandbox 中执行编码任务。7 阶段 fail-closed 管线：配置验证 → job 验证 → sandbox 验证 → worktree → 沙箱执行 → diff 收集 → 证据报告。**需 `config.enabled=true` 且 worktree recovery 通过。**

### 4. 外部通道 ✅
- Gmail ingress：收邮件 → 分类 → 创建 Goal/Task
- Telegram：long-poll 接收指令 + 发送结果
- Channel Router：SQLite-backed at-least-once delivery

### 5. 自动化 ✅
- Cron 调度（5 字段 parser）
- Webhook 触发（HMAC-SHA256 验证）
- Script 预处理

### 6. MCP 生态连接 ✅
- MCP Client：连接外部工具服务器（3 种传输 + Bearer/OAuth2.0）
- MCP Embedded Server：将 daemon 工具暴露为 MCP 协议

### 7. 记忆系统 ✅
- SQLite-backed episodic/semantic/procedural memory
- GBrain 外部知识图谱（默认关闭）
- Recall → Agora competition → Dasein interpretation 管线（部分实现）

---

## 三、还做不了的事情

### 1. 意识闭环：已接线，但只「观察-提交」，未「仲裁」 ⚠️（校正）
**校正（audit follow-up）：** 原判「闭环不存在于生产路径」**不准确**。Dasein→Agora 闭环实际已无条件接线：bootstrap 构建（`impl/daemon/bootstrap/request.rs:657-676`，注入 `request.rs:1060`）→ 每 turn `observe_turn`（`turn_pipeline.rs:215-225`）→ 每个受治理工具调用 `select_action`+`observe_outcome`（`governed_capability.rs:148-188`）→ `coordinator.run_cycle` 把 Dasein 状态注入 Agora 竞争/广播（`conscious_core_coordinator.rs:404-446`：signals→Concern、concerns→CareConcern、projection→Goal、protentions→Prediction）。

真实 gap 更精细，也更值得工程化：
- **(a) 只提交不仲裁：** `select_action` 恒定 `confidence:1.0 + max_salience()`（`conscious_action.rs:125-126`）总是胜出，`GovernedCapabilityInvoker` 无视选择结果照常执行 `inner.invoke(...)`（`governed_capability.rs:164-172`）——意识状态无法否决/改序真实调用。
- **(b) care 决策空转：** `CareStructure::determine_action()` 只在单测调用（`care_structure.rs:324/328/333/337`），生产 `reducer.rs:408-417` 不调它——`Deliberate/Direct/Wait/Negate` 决策从未影响行为。
- **(c) SelfField 被排除：** 闭环桥接的是 DaseinModule→Agora，SelfField（8 层策略）不是参与者，与 DaseinModule 无因果连接。

工程化方案见 `2026-07-17-conscious-core-engineering-plan.md`（R1–R4）。

### 2. 自进化（默认关闭）❌
Metacog `MorphogenesisPipeline` 存在但需要 `--enable-evolution` flag。Candidate→sandbox test→evaluate→migrate/rollback 管线完整，但非默认启用。

### 3. MCP Resources/Prompts ❌
Client 不支持 `resources/list`、`resources/read`、`prompts/list`、`prompts/get`。

### 4. Android 平台 ❌
Driver 是显式 stub（`platform/android.rs:3,25` — "Currently a stub. Full implementation requires Android NDK compilation"）。

### 5. 跨进程 Shared Memory IPC ❌
仅有单进程实现（`shared_mem.rs` — `memfd_create` + `mmap`，无跨进程 fd 传递）。

### 6. io_uring IPC 的 recv 路径 ❌
Ring setup/write 可用（`io_uring.rs`），但 `recv` 从 eventfd 读而非实际 IPC channel。

### 7. Discord/Slack/Email Delivery ❌
仅 `info!` 日志（`delivery.rs:47-55`），无实际投递。

### 8. NativeCognitRuntime 的 Context 注入 ⚠️
子 Agent 的 `recall()`/`dasein_view()`/`agora_view()` 返回空默认值。子 Agent 无法接收记忆/Dasein/Agora 上下文。这不是 bug——方法正确实现，但上下文集成未完成。

---

## 四、架构问题优先级（修正后）

基于代码实际状态（非文档声称）重新排序：

### P0 — 安全/稳定性（无阻塞项）

当前的 ToolRunnerWithGuard + bubblewrap sandbox + 4 级权限 + AgentControlService fail-closed 设计已经覆盖了核心安全需求。

### P1 — 架构收紧

| 优先级 | 问题 | 位置 | 影响 |
|--------|------|------|------|
| 1 | `TurnPipeline` 半具体耦合 (7/14 concrete) | `turn_pipeline.rs:42-59` | 测试/替换困难 |
| 2 | `DaemonTurnOrchestrator` 纯 God Object (7/7 concrete) | `orchestrator.rs:22-30` | 公开 `Arc<Mutex<>>` |
| 3 | `TurnRuntimeResources` 17 concrete + 8 Mutex | `turn_runtime_ports.rs:105-135` | `pub(crate)` 全部泄漏 |
| 4 | `KernelRuntime` 3 个 getter 返回 `Arc<具体可变类型>` | `runtime.rs:189-209` | 文档声称 "immutable snapshots" 不实 |

### P2 — 语义完整性

| 优先级 | 问题 | 位置 | 影响 |
|--------|------|------|------|
| 1 | `CareStructure::determine_action()` 空转 | `reducer.rs:408-417` | 关心的决策无行为效果 |
| 2 | Agora `Attention` 死状态 | `attention/mod.rs:7-31` | 声明但从未被驱动 |
| 3 | SelfField ↔ DaseinModule 因果断裂 | 两套系统，零连接点 | 有 "思考" 无 "自我感" |
| 4 | NativeCognitRuntime context 注入返回空 | `native_cognit.rs:428-436` | 子 Agent 无记忆/Dasein/Agora 上下文 |

### P3 — 集成补齐

| 优先级 | 问题 |
|--------|------|
| 1 | MCP Resources/Prompts 支持 |
| 2 | Discord/Slack/Email delivery 实现 |
| 3 | io_uring recv 路径完善 |
| 4 | corpus 9 处 `SystemClock` 纳入 CI enforcement scope |
| 5 | dasein 2 处直接 `Tool::execute` 纳入 CI enforcement scope |

### P4 — 文档同步 ✅ (已完成 2026-07-17)

架构文档已根据代码实际状态更新（原始计划内容完整保留）：
- `2026-07-16-a01-agora-transaction-integrity.md` — 顶部新增 "Code-Reality Update"，5 个 bug 全部标记为 "已修复（A01 计划完成）" + 3 个新 gap
- `2026-07-15-dasein-agora-conscious-core-plan.md` — 顶部新增 "Code-Reality Update"，5 个 bug 全部标记为 "已修复（event-sourced ledger）" + 2 个新 gap
- `2026-07-15-architecture-coupling-optimization-plan.md` — 顶部新增 "Code-Reality Update"，Executive 耦合修正（TurnRuntimeResources 等新增发现） + KernelRuntime/Bootstrap/CodexRuntime 修正

更新依据：`docs/plans/deepseek/` 下的 01-04 号代码级验证报告。

---

## 五、工程硬指标总览

| 指标 | 数值 |
|------|------|
| Crate 数量 | 11 |
| Rust 源文件 | ~600+ |
| 测试数量 | 2,766 |
| 测试代码行数 | 37,316 |
| 测试文件数 | 172 |
| 无条件 ignored tests | 0 |
| `#[should_panic]` | 0 |
| TODO/FIXME/HACK/XXX/WORKAROUND | 7 |
| 生产代码 `unimplemented!()` | 0 |
| 内置工具 | 21 |
| LLM Provider | 3 |
| Sandbox 后端 | 5 |
| RPC Handler 模块 | 11 |
| SubAgent Runtime | 3 生产 + 2 scaffold |
| CI Architecture Gates | 20+ 删除门 + 7 扫描规则 + 依赖图 + 路径 inventory |
| Platform Driver | 10/13 完整实现 |
| MCP 传输类型 | 3 (Stdio/HTTP/SSE) |
| Feature Flags | ~15 |

---

## 六、详细报告索引

- [01 — Executive Crate 耦合现状](./01-executive-coupling-reality.md)
- [02 — Agora 事务完整性](./02-agora-transaction-verification.md)
- [03 — Dasein SelfField/DaseinModule 内部分裂](./03-dasein-split-reality.md)
- [04 — CI 架构执行机制](./04-ci-architecture-enforcement.md)
- [05 — SubAgent Runtime 与 Pi Agent 集成](./05-subagent-pi-runtime-capability.md)
- [06 — 工具执行、测试覆盖与工程硬实力](./06-tool-execution-testing-maturity.md)
- [07 — MCP、Google、外部集成与 IPC](./07-external-integration-maturity.md)
- [08 — 工程成熟度综合评估](./08-engineering-maturity-assessment.md) ← 本文档

---

## 七、核心结论

**Aletheon 是一个工程成熟度极高的 AI Agent 运行时，其实际代码质量远超架构文档的自我描述。**

1. **核心执行路径全部生产级。** ReAct 循环、21 个工具、7 阶段安全管线、5 种 sandbox 后端、3 个 LLM provider、AgentControlService 标准控制平面——全部完整运作，零 stub。

2. **测试纪律出色。** 2,766 个测试，零无条件 ignored，零 should_panic，全代码库仅 7 个 TODO。

3. **文档大面积过时。** Agora 的 5 个 "CRITICAL bug" 已在代码中修复，Dasein 的 event-sourced ledger 完整运作，但文档从未更新。另一方面，Executive 的 concrete type 耦合比文档描述的更严重。

4. **真正缺失的不是工程能力，也不是闭环本身，而是闭环的「仲裁质量」。**（校正）Dasein→Agora 意识核心闭环已接线在生产 turn path，但目前只「观察-提交」：`select_action` 恒定胜出且不改变真实调用、`determine_action()` 空转、SelfField 未参与闭环。让 care 状态真正能改序/软否决行为、并把 SelfField 纳入闭环，是从 "功能完整的 Agent 运行时" 进化为 "具有持续自我觉察的 Agent 运行时" 的关键 gap。详见 `2026-07-17-conscious-core-engineering-plan.md`。

5. **Pi Agent 已具备实际编码能力。** Fail-closed 7 阶段管线 + git worktree 隔离 + bubblewrap sandbox + SHA-256 验证 + ControlledApply 原子回滚——这是一个完整的、安全的独立 coding agent 实现。
