# Grok-inspired Hardening：可执行实现 Spec

> 文档性质：**可执行实现文档**。与上级 `../` 研究文档不同，这里的类型定义、文件路径、任务分解是面向实施的。研究文档回答"为什么/是什么"，本目录回答"改哪个文件、写哪段代码、怎么验证"。

> **📌 执行者入口（deepseek）：先读 [`00-EXECUTION-INDEX.md`](00-EXECUTION-INDEX.md)。**
> 它把本目录的 Grok exec spec 与 `../../deepseek/*` 的 DeepSeek 计划合并为一条依赖排序的执行序列，标注已完成项、重叠协调（含一处 S1 命名冲突）、以及每个任务的源文档。合并桥接文档：
> - [`D1-tool-execution-integration.md`](D1-tool-execution-integration.md) — tool-exec × S1/G2
> - [`D2-multi-user-m3-m5.md`](D2-multi-user-m3-m5.md) — M3–M5 × G1/G3/G5
> - [`D3-mcp-integration.md`](D3-mcp-integration.md) — mcp × G7

## 1. 与研究文档的关系

```text
../01..12  研究与差距分析（候选设计，非代码事实）
   |
   v
exec/Gn    可执行 spec（具体类型 + 文件变更 + 任务分解 + 测试）
   |
   v
plans skill -> 实施
```

每份 exec spec 对应一个研究文档的 G-item。实施前仍须按 §5 重新核对当前代码锚点。

## 2. 索引与状态

全部 10 份 spec 的"当前代码锚点"章节已由并行 repo-researcher 对当前分支（含未提交修改）核实，标注了 verbatim 签名与 `path:line`。

| Spec | 对应研究 | 优先级 | 状态 |
|---|---|---|---|
| [G1 Folder Trust](G1-folder-trust.md) | ../02 | P0 | 草稿（锚点已核实） |
| [G2 Streaming Tool Runtime](G2-streaming-tools.md) | ../03 | P0 | 草稿（锚点已核实） |
| [G3 Prompt Queue / Interjection](G3-prompt-queue.md) | ../04 | P1 | 草稿（锚点已核实） |
| [G5 Lifecycle & Hook System](G5-lifecycle-hooks.md) | ../05 | P1 | 草稿（锚点已核实） |
| [G4 Workspace Checkpoint / Rewind](G4-checkpoint-rewind.md) | ../06 | P1 | 已实施并通过聚焦验收（2026-07-18） |
| [G6 Subagent Resource Settlement](G6-subagent-settlement.md) | ../07 | P2 | 草稿（锚点已核实） |
| [G8 ACP Adapter](G8-acp-adapter.md) | ../08 | P2 | 草稿（锚点已核实） |
| [S1 Sandbox Enforcement](S1-sandbox.md) | ../11 | P2 | 草稿（锚点已核实） |
| [G7 Memory Search Hardening](G7-memory-search.md) | ../09 | P3 | 草稿（锚点已核实） |
| [C1 Compaction Engine](C1-compaction.md) | ../12 | P3 | 草稿（锚点已核实） |

### 各 spec 发现的现状要点（影响实施策略）

- **G1**：Aletheon **无任何 trust 层**——全新实现；`WorkspaceSelection`/`WorkspacePolicy` 已具备任意 cwd + canonicalize + 隐式 `/` 拒绝。
- **G2**：`TurnEventV1` 已有 23 变体但**无 progress 变体**；channel 容量 64、overflow=BlockProducer；`ToolResult` 现带 `ToolResultMeta`。
- **G3**：**无任何 queue/interjection**；`ActiveTurnKey=(principal,thread)` 是天然 session 键；多连接共享 thread。
- **G5**：**已有两条可用 hook 路径**——corpus `execute_hook`（结构化，可 Block/ModifyInput/Inject）+ `run_hook_scripts`（脚本，fire-and-forget，已实现且正确调用）；真实缺口是 envelope 简陋、start/finish 不对称、HookPoint 覆盖窄、无 typed contributor 层、无信任门控。
- **G4**：`FileSnap`/`RuntimeResumability`/`AgentRecoveryReceipt` 均**已定义但零集成**；`LeaseManager` 就绪；无持久 checkpoint。
- **G6**：**已有实质结算**（usage settle/revoke、lease 幂等删除、budget drop、memory promotion receipt）；缺显式状态机 + 后台分类 + reparent。
- **G8**：Unix socket + JSON-RPC；已有 `ClientEvent`/`ItemEvent`/`ApprovalEvent` + `EventCursor` 重连；**无 ACP 依赖**。
- **S1**：**已有命令级 sandbox**（`SandboxBackend`/`SandboxExecutor`/`IsolationLevel`）；缺 profile 层 + deny glob + 分层配置。
- **G7**：FTS5 已用；vector 基建存在但**未接入**；authority/scope **检索后**才应用（应前置）；无 endpoint-scoped 凭证。
- **C1**：**已有 tail-keep compaction**（`CompactorTrait`+`AdvancedCompressor`+3-pass prune+threshold）；缺多策略 + guardrails + tool-pair 原子性。

## 3. 每份 Spec 的固定结构

每份可执行 spec 必须包含以下章节，缺一不可：

```text
1. 目标与非目标          一句话目标；明确不做什么
2. 当前代码锚点           验证过的 path:line + 签名引用（实施前重新核对）
3. 权威归属决策           owner 层 / scope / 恢复决策 / fail 模式 / 上限（对应 doc10 §6 清单）
4. 类型定义              完整 Rust 类型（struct/enum/trait），可直接���地
5. 文件变更计划           新增/修改文件清单，每项标明动作与理由
6. 任务分解              2-5 分钟粒度的有序任务，标注依赖与 TDD 测试先行
7. 兼容与迁移            feature flag 位置；旧路径如何不中断
8. 测试计划              单元/集成/属性测试用例，映射到研究文档的"验收方向"
9. 可观测性              新增事件、指标、日志
10. 许可证              是否复制 Apache-2.0 代码；NOTICE 处理
```

## 4. 共享工程约束（所有 spec 遵守）

沿用研究文档 `../10 §5`，此处给出可执行的落地规则：

### 4.1 Feature flag

- 每个高风险机制通过 **单一** 配置开关门控，默认关闭。
- 开关命名：`grok_hardening.<item>`（如 `grok_hardening.folder_trust`）。
- 关闭时代码路径必须等价于当前行为（no-op adapter），可用 A/B 回归测试证明。

### 4.2 多用户 scope

- 所有新持久状态必须绑定 `principal_id` +（`session_id` / `thread_id` / `agent_id` 之一）。
- 禁止进程级全局可变状态（`static mut`、无 principal 的 `OnceLock` 缓存）代表用户数据。
- 复用 `CapabilityExecutionContext` 已注入的可信身份，不新造字符串身份。

### 4.3 权威终态

- Tool call / turn / Agent attempt 各自只有一个 terminal truth。
- progress、notification、client disconnect 不得改变 terminal。
- 所有 terminal 经过同一 settle/audit 路径。

### 4.4 恢复与幂等

- 每个持久状态更新带 idempotency key 或 receipt。
- daemon crash 后能区分：running-but-unconfirmed / queued / completed。
- 重放不产生重复副作用。

### 4.5 有界性

- 队列长度、interjection bytes、progress buffer、checkpoint 磁盘、后台资源、memory candidate 全部有硬上限。
- 超限行为明确：丢弃采样 / 拒绝 / fail-closed，不静默增长。

### 4.6 许可证

- **优先重新实现接口与语义**，不复制 Grok 源码。
- 若必须复制/改写 Apache-2.0 代码：逐文件记录来源 commit、变更说明，更新 `THIRD-PARTY-NOTICES`。
- Grok 许可证证据：`/home/aurobear/Bear-ws/grok-build/README.md:127-139`；Aletheon 为 MIT（`Cargo.toml:19-23`）。

## 5. 实施前核对（每次进入实施必做）

因为 Aletheon 主分支持续变动，且本仓库有未提交修改，每份 spec 的"当前代码锚点"章节可能过期。进入实施前：

1. `git log --oneline -5` 确认基线未大幅偏移。
2. 对 spec §2 引用的每个符号重新 `grep` / LSP 定位，更新 `path:line`。
3. 若签名已变，先更新 spec §2/§4，再动手。
4. 对照 doc10 §6 八问清单确认权威归属未变。

## 6. 验证命令约定

- 单 crate 快速校验：`cargo check -p <crate>`
- 单 crate 测试：`cargo test -p <crate>`
- 全量（慢，CI 用）：`cargo test --workspace`
- lint：`cargo clippy -p <crate>`；格式：`cargo fmt --all`
- 已知 flaky：`corpus execute_script_hook_inject` 在并行下偶发，非回归。
