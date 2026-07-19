# Wave 3：Pi 默认 Coding Runtime + Verifier Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Pi 做成 Aletheon 受治理的默认 Coding Runtime（合并 pi-coder 的隔离 worktree + pi-rpc 的交互式 RPC），并引入独立的 CodingCompletionVerifier——使编码任务的完成状态由证据（diff/命令/测试）判定，而非"模型停止说话"。

**Architecture:** 新建 `crates/runtime-pi/`（实现 Wave 2 的 `CapabilityRuntime`）：resident Pi RPC session + 每个 coding job 一个 Aletheon 管理的 worktree/checkpoint；映射 model/prompt/tools/budget；捕获 stderr/tool events/diff/artifacts；Pi 结束后进入统一 Verifier；验证失败把结构化错误经 `RuntimeMessage::VerificationFailure` 送回同一 session；最终输出标准 `RuntimeReceipt`。Verifier 独立于 Pi，消费标准 RuntimeEvent/Receipt。

**Tech Stack:** Rust（新建 runtime-pi crate；executive runtime adapters；Pi 官方 RPC 协议 JSONL over stdio）。

**环境说明:** cargo 可用；构建/测试走 `bash scripts/cargo-agent.sh test -p <crate> <filter>`，不要用裸 cargo。

**依赖:** Wave 2（runtime-api：RuntimeManifest/WorkOrder/CapabilityRuntime/RuntimeEvent/RuntimeReceipt；runtime-broker）。本 Wave 不重复定义这些类型，只实现与消费。

**granularity 说明:** 涉及对现有 Pi runtimes 的 adapter 合并 + 新 Verifier。任务写到"改哪些函数 + 接口 sketch + contract/verifier 测试 + 验收"级别。

---

## 当前状态（已代码验证）

- `crates/executive/src/impl/runtime/pi.rs` — `PI_CODER_RUNTIME_ID = "pi-coder"`（`:27`），one-shot，实现 `SubAgentRuntime`；每请求新进程，`WorktreeManager` 隔离（`:150-158`），网络关闭，接 Goal verification。
- `crates/executive/src/impl/runtime/pi_rpc.rs` — `PI_RPC_RUNTIME_ID = "pi-rpc"`（`:23`），resident，实现 `AgentRuntimeLauncher`；JSONL stdin/stdout，`PiRpcCommand::Prompt/FollowUp/Steer/Abort/GetState`（`:153-298`）；缺口：直接操作当前 workspace（无隔离 worktree）、丢 stderr、`artifacts` 为空、不产 authoritative diff、不建 checkpoint、不执行 Verifier、不注入完整 Profile prompt、不完整映射 allowed tools/budget。
- `crates/executive/src/impl/runtime/pi_protocol.rs` — Pi 协议投影。
- `crates/executive/src/impl/goal/attempt_coordinator.rs:231` — Goal 特判 `PI_CODER_RUNTIME_ID`（Wave 2 已删，本 Wave 依赖其结果）。
- ReActLoop verifier seam 默认 `None`（`crates/cognit/src/harness/linear/`）——主生产 bootstrap 未安装 Coding Verifier。
- `crates/cognit/src/config/mod.rs` `PiRuntimeConfig` `enabled = false`（默认关闭）。

Pi 官方 RPC/工具/压缩为 IDE 嵌入设计，成熟机制（read/bash/edit/write/grep/find/ls、Session JSONL tree、自动压缩）不重写，只在其上加治理/证据/验证。

---

## Phase A：runtime-pi 生产 adapter

### Task A1：crate 骨架 + Manifest

**Files:** 新增 `crates/runtime-pi/`（`lib.rs`/`manifest.rs`/`session.rs`/`transport.rs`）

- [ ] 实现 `CapabilityRuntime`（来自 runtime-api）；提供 `RuntimeManifest`：
  - `id: "pi/coding"`，`aliases: ["pi"]`
  - `capabilities: {CodeRead, CodeSearch, CodeEdit, Shell, Test}`
  - `interaction_modes: {OneShot, Resident, Steering, FollowUp}`
  - `transports: {JsonlStdio}`
  - `workspace_mode: IsolatedWorktree`，`resumability: Session`，`tool_governance: Observed`
- [ ] 依赖规则：`runtime-pi` 只依赖 `runtime-api` + Pi transport；不被 Executive/Goal 反向 import 具体类型。
- [ ] 构建 + manifest 序列化测试。Commit。

### Task A2：resident RPC session + 隔离 worktree

**Files:** `crates/runtime-pi/src/session.rs`（复用 `pi_rpc.rs` 的 JSONL 协议逻辑 + `pi.rs` 的 `WorktreeManager`）

- [ ] `prepare`：为每个 coding job 创建 Aletheon 管理的 worktree + 初始 checkpoint。
- [ ] `start`：拉起 resident Pi 进程，JSONL 通信；把 Pi tool events 投影为标准 `RuntimeEvent`（ToolRequested/Started/Completed、FileChanged、CommandStarted/Output、Diagnostic、TestResult）。
- [ ] `send`：`RuntimeMessage::Steer/FollowUp/ProvideContext/VerificationFailure` 映射到 `PiRpcCommand::Steer/FollowUp/Prompt/Abort`。
- [ ] 捕获 stderr（现丢弃）；收集 workspace delta（authoritative diff）与 artifacts（现为空）。
- [ ] `checkpoint`/`cancel`/`settle`：cancel 终止整个进程树；settle 产出标准 `RuntimeReceipt`。
- [ ] contract tests（Wave 2 的共用 Runtime contract test 套件）：start 只发一次 Started、tool event 有序、cancellation 终止进程树、terminal event 唯一、workspace policy 不可绕过。
- [ ] 构建 + contract tests（需完整 Rust env + 可用 Pi 可执行文件；无 Pi 时用 mock transport）。Commit。

### Task A3：策略映射（model/prompt/tools/budget）

**Files:** `crates/runtime-pi/src/session.rs`

- [ ] 把 `RuntimeLaunchSpec` 的 model_policy / instructions(prompt) / tool_policy(allowlist) / budget 完整映射到 Pi 启动参数（现仅用 task/workspace/elapsed timeout）。
- [ ] 注入完整 Agent Profile prompt（来自 Wave 1 的 `ResolvedTurnProfile`）。
- [ ] TDD：断言 allowlist/budget 被传递并生效（mock）。构建 + 测试。Commit。

### Task A4：生产配置 + health probe

**Files:** Pi 配置（`crates/cognit/src/config/mod.rs` `PiRuntimeConfig`）、部署模板

- [ ] 保留 fail-closed（executable/version/SHA-256/固定 argv/namespace sandbox/worktree/路径 policy）；补完整部署模板 + `health()` probe。
- [ ] Broker 注册 `pi/coding`，默认在 `coding` profile 下启用（配合 Wave 5 profiles）。
- [ ] 构建 + health 测试。Commit。

**Phase A 验收:** Pi 能完成真实文件修改 + 测试；执行中可 steering；workspace 外写入被拒；结果含 authoritative WorkspaceDelta + artifacts + 标准 Receipt。

---

## Phase B：CodingCompletionVerifier

### Task B1：Verifier 类型与完成状态

**Files:** 新增 `crates/executive/src/service/verifier/`（或独立 crate `runtime-verifier`）

- [ ] 定义 `CompletionStatus`（audit §3.4）：`SucceededVerified / SucceededUnverified / FailedVerification / Blocked / BudgetExhausted / Cancelled`。
- [ ] `CodingCompletionVerifier`：消费标准 `RuntimeReceipt`（diff/commands/tests/diagnostics），产出 `VerificationReceipt`。
- [ ] 构建 + 类型测试。Commit。

### Task B2：结构化 command/test/diagnostic receipts + 窄测试选择

**Files:** verifier 模块 + 与 runtime-pi 的 event 消费

- [ ] 从 RuntimeEvent 流聚合结构化 `CommandReceipt`/`TestReceipt`/`Diagnostic`。
- [ ] 自动选择最窄相关测试（基于改动文件 → 对应测试目标的启发式；Rust 下可按 crate/module 选 `cargo test -p <crate> <module>`，走 `scripts/cargo-agent.sh`）。
- [ ] TDD：给定一组改动文件，断言选出的测试集最窄且相关。构建 + 测试。Commit。

### Task B3：验证失败回送同一 session + 主回路接入

**Files:** verifier ↔ runtime-pi session；主生产 bootstrap（安装 Verifier，取代默认 `None` seam）

- [ ] 验证失败 → `RuntimeMessage::VerificationFailure`（结构化证据）送回同一 Pi session 继续修复，而非结束。
- [ ] 主生产 bootstrap 安装 CodingCompletionVerifier（替换 ReActLoop 默认 `None` verifier seam）。
- [ ] 完成语义：无证据不得返回 `SucceededVerified`；测试失败继续修复直到 verified 或 budget/blocked。
- [ ] TDD：注入一个"编译失败→修复→通过"用例，断言不提前 SucceededUnverified。构建 + E2E（完整 Rust env）。Commit。

**Phase B 验收:** 没有证据不能返回 Verified Success；测试失败会继续修复而非结束；用户能看到命令/测试/diff/最终 verdict；区分 verified/unverified/blocked/budget-exhausted。

---

## 自审对照（roadmap Wave 3）

| roadmap 项 | 覆盖 |
|---|---|
| 生产 Pi adapter（audit PR4） | Phase A |
| CodingCompletionVerifier（audit PR5） | Phase B |
| Pi 成默认 Coding Runtime | Task A4 + Wave 5 profiles |
| 失败回送同一 session | Task B3 |

---

## 建议 PR 切分

- PR-W3-A1..A4：runtime-pi 骨架/manifest → session/worktree → 策略映射 → 配置/health。
- PR-W3-B1..B3：Verifier 类型 → 结构化 receipts + 窄测试 → 回送 + 主回路接入。

## 下一步

Wave 3 完成后，Wave 4 补状态权威与 trajectory/恢复；Wave 5 用 profiles 把 Pi+Verifier 收进默认 `coding` profile 并建编码成功率 gate。
