# D2 合并可执行 Spec：Multi-User Runtime M3–M5 × Grok G1/G3/G5

> 合并 DeepSeek `../../deepseek/2026-07-17-codex-inspired-multi-user-runtime-design.md`（M3–M5 权威源，§10 milestones / §11 acceptance）+ `../../deepseek/2026-07-17-multi-user-runtime-m0-m2.md`（M0–M2 已完成的类型）与 Grok `G1-folder-trust.md` / `G3-prompt-queue.md` / `G5-lifecycle-hooks.md`。
> 执行前按 `00-EXECUTION-INDEX.md §0` 重新核对锚点。

## 1. 一句话

M0–M2 **已实现**（principal/workspace 契约、per-user runtime、任意 cwd）。M3–M5 是剩余工作，全部 Executive 接线。Grok G1/G3/G5 已交付 M3–M5 需要的 **fabric 原语**（trust decide、prompt-queue 并发规则、lifecycle effect）——**零冲突，纯 build-on**。

## 2. 现状锚点（合并，需重新核对）

| 事实 | 锚点 |
|---|---|
| M0–M2 COMPLETE | `../../deepseek/2026-07-17-multi-user-runtime-m0-m2.md`（Task 1–16）；`PrincipalContext`/`WorkspacePolicy`/`ConnectionId`/`ThreadId` 在 `fabric::local_authority` |
| 全局 default-session 切换（M3 要移除） | `crates/executive/src/service/legacy_session_service.rs:338-360` |
| turn 重读共享 default session | `crates/executive/src/service/daemon_turn/execute.rs:62-71` |
| 版本化协议类型（M3 扩展非替换） | `crates/fabric/src/protocol/client.rs:10-145` |
| legacy wire events | `crates/fabric/src/events/ui_event.rs:204-249` |
| M5 config 面 | `crates/executive/src/core/config/mod.rs:94-193` + `provenance.rs:57-125` |
| **Grok G1 已提交** `workspace_trust::decide()` + `TrustReceipt`/`WorkspaceIdentity{canonical_path,repo_fingerprint}` | `fabric::types::workspace_trust`（`df90b775`） |
| **Grok G3 已提交** `prompt_queue::evaluate_edit/evaluate_cancel`（乐观版本 + 跨 principal 拒绝）+ `PromptEnvelope` | `fabric::types::prompt_queue`（`d3023c04`） |
| **Grok G5 已提交** `lifecycle::validate_effects` + `LifecyclePhase`(11) + `LifecycleEffect` | `fabric::types::lifecycle`（`6b5cb929`） |

## 3. M3 —— 显式 thread + 单客户端协议（消费 G3 + G1）

DeepSeek M3 任务（design §10 `:346-350`）：移除全局 default-session 切换；chat/approval/cancel/snapshot/subscribe 请求显式带 thread 身份；把 live turn/tool 投影成版本化 Item 事件；TUI resume/replay/interrupt/terminal 迁到版本化协议。

**合并任务**：
- **D2-M3-T1**：移除 `legacy_session_service.rs:338-360` 的全局 default-session 切换；`execute.rs:62-71` 不再重读共享 default，改用请求显式 `thread_id`。
- **D2-M3-T2（消费 G3）**：把 cancel/interrupt 的前置条件建成 `(thread_id, turn_id, operation_id)`（design `:243`），用 G3 `evaluate_cancel` 的版本检测 + owner 保持保证「interrupt 只命中指定 operation」（design §11）。edit/enqueue 走 G3 `evaluate_edit`（跨 principal 拒绝、running 不可原地编辑已测）。**剩余接线**：G3 spec §5 的 `SessionInputCoordinator` + 持久化 + safe-point drain（在 Executive）。
- **D2-M3-T3**：live turn/tool 投影为版本化 Item 事件（扩展 `protocol/client.rs`，design §6「extended rather than replaced」）；新增 `TurnCompleted{turn_id,status,error,retryable,usage}` + item 状态机（design `:224-241`）。
- **D2-M3-T4（关联 G1）**：把 M2 canonical cwd 喂进 G1 `WorkspaceIdentity{canonical_path,repo_fingerprint}`（= design §6.2/§7.2「normalized workspace identity + optional repo identity」）。folder-trust（G1 `decide()` 门控 repo hooks/MCP 加载）是 M0–M5 未列的**新增关切**，在此接入：加载 repo 可执行配置前查 `decide()`。**剩余接线**：G1 spec §5 的 discovery/digest 生产者 + trust-store 持久化 + 交互 prompt（Executive/Interact 边缘）。

**验收（design §11 M3）**：并发 thread 互不改对方 session/workspace；每个 turn 恰好一个 terminal；每个 item 恰好 completed/failed 一次；从 cursor 重连无缺失/重复；interrupt 只命中指定 operation。

## 4. M4 —— 持久恢复 + 上下文完整性（消费 G5 + C1）

DeepSeek M4 任务（design `:353-357`）：可观测的有界 writer 失败 + terminal flush 排序；恢复时关闭未完成 turn；每 turn 持久化最新 workspace context；tool call/result 归一化 + compaction lineage。

**合并任务**：
- **D2-M4-T1**：持久化顺序状态机（design §7.1）+ 有界 writer 失败在 doctor 可见（阻止假成功）。
- **D2-M4-T2**：恢复扫描「有起始边界无 terminal」的 turn（design §7.2），标记 recoverable interrupted/failed。
- **D2-M4-T3（关联 C1）**：tool call/result 归一化 + compaction lineage —— compaction 侧复用 Grok **C1** 的 `safe_tail_cut`（tool-pair 不切分）+ `is_degenerate_summary`（`fabric::include::compaction`，已提交）。注意：C1 是 token-budget 轴，与 conscious-core 仲裁轴正交（勿混）。
- **D2-M4-T4（关联 G5）**：每 turn 持久化最新 workspace context —— 注入的 context fragment 用 G5 `LifecycleEffect::AddContextFragment` + `validate_effects`（`MAX_CONTEXT_FRAGMENT_BYTES`，满足 design §7.3「no injected context fragment is unbounded」）。**剩余接线**：G5 spec §4.3 的 contributor 注册表 + dispatch（Executive，各 turn-loop phase 调用）。

**验收（design §11 M4）**：流式/工具/compaction/terminal-persist 期间杀 runtime → recoverable interrupted/failed；durable canonical history 不被 model-context compaction 改动；归一化不暴露 orphan result 给模型；writer 失败在 doctor 可见并阻止假成功。

## 5. M5 —— 诊断 + 运维硬化

DeepSeek M5 任务（design `:360-363`）：effective-config + doctor 命令；overload/backpressure + connection-owned 进程清理；精确部署校验 + core/user runtime 版本不匹配的回滚检查。

**合并任务**：
- **D2-M5-T1**：`aletheon config effective|layers` + `aletheon doctor [--json]`（建在 `config/mod.rs:94-193` + `provenance.rs:57-125`）。
- **D2-M5-T2**：overload/backpressure 行为 + connection-owned 进程清理。
- **D2-M5-T3**：部署校验 installed SHA + core/user 双 runtime 版本；回滚恢复双 binary/config。

**验收（design §11 M5）**：`doctor --json` schema 稳定、有界、密钥脱敏；部署校验双 runtime 版本；回滚恢复。

## 6. 依赖与顺序

```
M3（消费 G3 cancel/edit + G1 workspace identity/trust）
  └─ M4（消费 C1 compaction + G5 context fragment）
       └─ M5（诊断/运维）
```
G1/G3/G5 fabric 原语已就绪；本文各 T 的「剩余接线」= 各 grok spec §4/§5 的 Executive consumer 层。

## 7. 分工

- **G1/G3/G5 spec**：fabric 原语 + Executive consumer 设计（coordinator/registry/store/prompt）。
- **DeepSeek design**：M3–M5 里程碑、协议扩展、持久化/恢复/诊断的权威任务与验收。
- 协调点：G1 `WorkspaceIdentity` 复用 design §6.2/§7.2；G3 cancel-version 路由 design M3 interrupt 前置；G5 fragment 上限满足 design §7.3。
