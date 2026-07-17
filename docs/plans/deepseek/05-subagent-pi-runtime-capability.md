# SubAgent Runtime 与 Pi Agent 集成 — 代码级分析

> **日期:** 2026-07-17
>
> **方法:** 逐行扫描 `crates/executive/src/impl/runtime/`、`crates/executive/src/service/agent_control/`、`crates/executive/src/impl/agent/`、`crates/corpus/src/tools/subagent/`、`crates/kernel/src/process/`

## 概述

分析 Aletheon 的 SubAgent 执行能力：哪些 Runtime 可用、Pi Agent 的成熟度、Agent 生命周期管理、隔离原语。

**结论：SubAgent 系统全部生产级。NativeCognitRuntime/PiRuntime/ProviderWorkerRuntime 三个 Runtime 完整运作。AgentControlService 实现标准 6 操作接口。Pi Agent 具有完整的 7 阶段 fail-closed 管线。不存在 CodexRuntime。**

---

## 1. 三种生产级 Runtime

### NativeCognitRuntime — 主 Child-Agent Runtime

**文件:** `crates/executive/src/impl/runtime/native_cognit.rs`

| 属性 | 值 |
|------|-----|
| Runtime ID | `"native-cognit"`（行 31） |
| 实现 trait | `AgentRuntimeLauncher`（行 318） |
| 状态 | **生产级** |

**执行流程** (`launch()` → `execute()`):

1. 解析并验证 AgentProfile、tools
2. 创建 `CognitiveSession` via factory
3. 运行 multi-turn loop：每轮 `session.run_turn()` + timeout + mailbox 轮询（最多 16 轮 mailbox）
4. Token 预算执行（`MeteredLlm`，行 508，原子累计 input/output tokens）
5. 输出大小预算验证（4 bytes per token）
6. 返回 `AgentResult`

**三个 stub 方法**（非 TODO，返回空默认值）:
- `recall()`（行 428）— 返回空 `RecallSet`
- `dasein_view()`（行 432）— 返回默认值
- `agora_view()`（行 436）— 返回默认值

这些不影响 Runtime 正常运作，仅意味着子 Agent 当前不接收记忆/Dasein/Agora 上下文。

---

### PiRuntime — 独立 Coding Agent

**文件:** `crates/executive/src/impl/runtime/pi.rs`

| 属性 | 值 |
|------|-----|
| Runtime ID | `"pi-coder"`（行 24） |
| 实现 trait | `SubAgentRuntime`（行 294，兼容层） |
| 状态 | **生产级**（需 `config.enabled=true` + worktree recovery 通过） |

**7 阶段 fail-closed 执行管线** (`run_attempt()`，行 302-542):

| 阶段 | 做了什么 | 行号 |
|------|---------|------|
| 1. prepare | 验证 sandbox 需 namespace 隔离、网络禁用、可执行文件在受信目录、worktree base 规范 | 86-138 |
| 2. validate_job | 拒绝命令/参数不匹配、资源超限、路径策略不匹配、网络请求 | 161-188 |
| 3. validate_sandbox | 仅接受 Namespace/Container 隔离，要求文件系统+网络隔离能力 | 211-230 |
| 4. worktree 创建 | `WorktreeManager::create()` — 独立 git worktree | 328 |
| 5. 沙箱执行 | 通过 sandbox backend 包装 argv → `CommandRunner::run()` | 353-373 |
| 6. diff 收集 | `WorktreeManager::collect()` — status + porcelain diff + SHA-256 + changed_files | 406 |
| 7. 证据报告 | `CodingJobReport` 含 4 类证据（报告、worktree 引用、base64 diff、capability audit） | 435-489 |

**Fail-closed 设计:** 每个错误路径都清理 worktree。成功的空 worktree 被删除。失败的和非空成功的 worktree 保留供 M5 审批。**零 TODO/FIXME。**

**Bootstrap 注册** (`daemon/bootstrap/request.rs:843-865`):
- Pi 通过 `RuntimeRegistry`（兼容层）注册
- 然后通过 `CompatibilityRuntimeLauncher` 包装注入 `AgentRuntimeRegistry`
- **仅在 `pi_work_allowed == true` 时注册**（需要 worktree recovery 通过）

**7 个集成测试** (`crates/executive/tests/pi_runtime.rs`，393 行)，含真实 git 操作 + test-double sandbox + 实际 bubblewrap 测试。

---

### ProviderWorkerRuntime — LLM Worker Loop

**文件:** `crates/executive/src/impl/runtime/provider_worker.rs`

| 属性 | 值 |
|------|-----|
| 实现 trait | `SubAgentRuntime`（行 117） |
| 状态 | **生产级** |

用于 Goal worker/reviewer 角色。LLM 驱动 loop：接收 task → LLM 推理 → 工具调用 → 收集 evidence → 最多 `max_steps` 步。8 个单元测试覆盖工具循环、取消、token 限制、allow-list 执行、超时。

---

### WorktreeRecoveryService — 启动协调

**文件:** `crates/executive/src/impl/runtime/worktree_recovery.rs`

**不是 Runtime。** 是 Pi coding worktree 的启动协调服务。启动时扫描受管 worktree 目录 → 隔离未知条目 → 修剪过期的失败 worktree → 匹配 recovery 记录 → 产生 `WorktreeRecoveryOutcome` 决定是否允许新 Pi 工作。

---

## 2. Agent 控制平面 (G03)

### AgentControlService

**文件:** `crates/executive/src/service/agent_control/mod.rs`

标准 6 操作接口：

| 操作 | 方法 | 行号 | 说明 |
|------|------|------|------|
| **spawn** | `spawn()` | 573 | 完整管线：验证→resolve runtime→fork context→Kernel process→admission reserve→mailbox→SQLite 持久化→lease→tokio::spawn |
| **wait** | `wait()` | 868 | 带超时的终端 snapshot 等待 |
| **send** | `send()` | 878 | 通过 mailbox 向运行中 agent 发消息 |
| **cancel** | `cancel()` | 981 | 取消 token + operation 级联 |
| **inspect** | `inspect()` | 1002 | 运行时状态查询 |
| **list** | `list()` | 1012 | 全部 agent 列表 |

### Runtime 注册架构（双层）

| 注册表 | 文件 | 行号 | 作用 |
|--------|------|------|------|
| `AgentRuntimeRegistry` | `execution.rs:229` | 新标准 | 直接注册 `Arc<dyn AgentRuntimeLauncher>`（NativeCognitRuntime 在此） |
| `RuntimeRegistry` | `runtime_registry.rs:11` | 兼容层 | 注册 `Arc<dyn SubAgentRuntime>`（Pi + ProviderWorker），通过 `CompatibilityRuntimeLauncher` 包装后注入 `AgentRuntimeRegistry` |

### 相关服务模块

| 模块 | 文件 | 行数 | 说明 |
|------|------|------|------|
| `admission.rs` | admission | ~500 | 预算控制的 spawn 准入 |
| `candidate_projection.rs` | candidate_projection | ~400 | 基于事件的候选投射 |
| `cleanup.rs` | cleanup | ~80 | 保留清理 |
| `context_fork.rs` | context_fork | ~220 | 上下文投射/fork |
| `recovery.rs` | recovery | ~180 | 启动崩溃恢复 |
| `sqlite_repository.rs` | sqlite_repository | ~900 | SQLite 持久化实现 |
| `mailbox.rs` | mailbox | 123 | Agent 消息 mailbox bridge |

**全部零 TODO/FIXME。**

---

## 3. SubAgent 隔离原语

**路径:** `crates/corpus/src/tools/subagent/`

| 原语 | 文件 | 行数 | 能力 | 测试 |
|------|------|------|------|------|
| `CommandRunner` | `command.rs` | ~330 | 有界可取消 tokio 子进程，进程组清理（SIGTERM→SIGKILL），输出上限，binary-safe | 6 |
| `WorktreeManager` | `worktree.rs` | ~670 | 所有权检查的 git worktree，磁盘预算，TTL 修剪，路径逃逸防护（symlink 检查），SHA-256 验证 | 8 |
| `ControlledApply` | `apply.rs` | ~560 | Fail-closed `git apply`，SHA-256 哈希验证，授权门控，路径范围限制，原子回滚（path snapshot/restore） | 多项 |

**全部生产级，零 TODO/FIXME。**

---

## 4. Kernel Process 管理

**文件:** `crates/kernel/src/process/table.rs`（337 行）

`ProcessTable` 实现 `ProcessManager` trait：

- `spawn()`（行 161）：创建 Process + 从 parent fork Space（context 继承）
- `transition()`（行 51）：执行合法状态转换（Created→Ready→Running→Waiting↔Running→Stopping→Exited/Failed）
- `signal()`（行 215）：分发 `ProcessSignal::Start/Wait/Resume/Terminate/Kill`
- `mark_exit()`（行 82）：设置退出状态和终端状态
- `wait()/wait_for_terminal()`（行 129/254）：基于 `Notify` 的异步等待
- `reap()`（行 118）：移除终端进程

完整状态机 + 验证。**零 TODO/FIXME。**

---

## 5. 仅有的 Scaffold

| 组件 | 文件 | 状态 |
|------|------|------|
| `AgentHarness` trait | `impl/agent/harness.rs` | **零实现** — 计划的多 provider turn 抽象，未接入生产路径 |
| `AgentRuntime` struct | `impl/agent/mod.rs` | **未接入** — 内存 `HashMap` 管理，不与 `AgentControlService` 连接 |

两者都不影响实际功能——真正的生产 agent 管理通过 `AgentControlService` + `AgentRunRepository` (SQLite) 完成。

---

## 6. "CodexRuntime" — 不存在

grep 确认：代码库中**不存在 `CodexRuntime`**。"Codex" 仅出现在：
- 文档（引用 Codex 设计模式）
- Sandbox 规则（保护 `.codex` 目录）
- 注释（"Inspired by Codex execpolicy"）

Pi 就是独立的 coding runtime，不是 Codex 的 wrapper。

---

## 总结表

| 组件 | 生产级? | 能 spawn/run 子 Agent? | 备注 |
|------|---------|----------------------|------|
| `NativeCognitRuntime` | 是 | 是 — 完整 Cognitive session 循环 | 3 context 方法返回空默认值 |
| `PiRuntime` | 是 (opt-in) | 是 — 隔离 coding 子进程 | Fail-closed，需 sandbox + worktree + enabled |
| `ProviderWorkerRuntime` | 是 | 是 — LLM worker loop + tool calls | Goal worker/reviewer |
| `AgentControlService` | 是 | 是 — 标准 spawn/dispatch/monitor/cancel | G03 权威 |
| `AgentRuntimeRegistry` | 是 | N/A — 注册表层 | RuntimeId → Launcher |
| `RuntimeRegistry` | 是 | N/A — 兼容注册表层 | RuntimeId → SubAgentRuntime |
| `ProcessTable` | 是 | N/A — 状态机 | 完整 FSM + space fork |
| `CommandRunner` | 是 | 是 — 子进程 | 有界、可取消、binary-safe |
| `WorktreeManager` | 是 | 是 — git worktrees | 所有权检查、磁盘预算 |
| `ControlledApply` | 是 | 是 — git apply | 哈希验证、授权门控、原子回滚 |
| `AgentHarness` trait | **Scaffold** | 否 — 零实现 | 计划中的多 provider turn 抽象 |
| `AgentRuntime` struct | **Scaffold** | 否 — 未接入 | 内存 HashMap，不连接真实控制平面 |
