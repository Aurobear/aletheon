# Agent Production Closure Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Aletheon 用真实 Executive 完成三类编码任务，并以可重放 evidence、verification 与 receipt 证明完成，而不是凭最终文本自证成功。

**Architecture:** Executive 保持 admission、cancel、verification 和 settlement 权威；Pi 只是实现 Runtime contract 的外部执行者；Corpus/execd 执行获准工具。编码 harness 启动真实 `aletheon` 入口，在一次性 workspace 副本内运行，并用独立命令验收。

**Tech Stack:** Rust、Tokio、Serde JSON、现有 `runtime`/`executive`/`aletheon` crates、shell fixture repositories。

---

## 需求锚点

- Agent 结果必须携带真实 output/usage/evidence：`crates/fabric/src/types/agent_control.rs:538-559`。
- Coding diff/report 由 Executive Goal verification 持久化并校验 hash：`crates/executive/src/impl/goal/verification.rs:85-159`。
- Runtime selector 必须保持 Executive 权威：`crates/executive/src/service/agent_control/execution.rs:397-444`。
- 三个真实 fixture 与故障门禁：`docs/arch/aletheon-agent-production-audit.md:99-120`。

### Task 1: 收敛结构化编码证据 owner

**Files:**
- Inspect: `crates/fabric/src/types/agent_control.rs`
- Inspect: `crates/executive/src/impl/runtime/pi.rs`
- Inspect: `crates/executive/src/impl/goal/verification.rs`
- Delete: 无 production caller 的 `crates/runtime/src/{receipt,events,work_order}.rs`

- [x] 审计确认生产主链已经以 `AgentResult` 保存 bounded output/usage/tool evidence，以 `CodingJobReport` + diff artifact 保存 workspace change。
- [x] 删除没有 production caller 的第二套 `RuntimeReceipt`、`RuntimeEvent`、`WorkOrder` 与仅测试调用的 `CodingVerifier`，不维护平行 completion authority。
- [x] 独立 coding harness 以 operation id、workspace fingerprints、command digest 与验收结果形成可重放 receipt；Runtime/模型不能自证 verified。
- [x] `bash scripts/cargo-agent.sh test -p runtime` 通过（2 passed），`bash scripts/cargo-agent.sh check -p executive` 通过。
- [x] **裁决:** 按生命周期 owner 收敛：Runtime 只描述可选择能力，Executive/Fabric 拥有 Agent result/verification/settlement。

### Task 2: 让 Pi session 产生真实 AgentResult 与 coding evidence

> **实码裁决（2026-07-20）:** 原计划引用的 `runtime::CapabilityRuntime`
> prepare/start/settle 是无生产 caller 的平行生命周期，而且 `start` 没有 workspace、
> operation 或执行输入，无法产生真实 receipt。已删除该空实现；唯一生产主链是
> Executive `AgentRuntimeRegistry -> AgentRuntimeLauncher -> AgentControl settlement`。
> 本任务使用该主链既有 `AgentResult` 与 Goal coding evidence，禁止重建第二套 receipt/session state。

**Files:**
- Modify: `crates/executive/src/impl/runtime/pi_rpc.rs:584-639`
- Modify: Pi session state in `crates/executive/src/impl/runtime/pi_rpc.rs`
- Test: `crates/executive/tests/pi_rpc_runtime.rs`

- [x] Executive AgentControl handle 关联 operation/process；Pi 累积真实 output、usage、tool evidence，coding worktree 保存 diff/hash/report。
- [x] 删除无 caller 的空 `CapabilityRuntime` session/settle；不再允许未知 handle 返回空成功。
- [x] 修复 `CompatibilityRuntimeLauncher` 丢弃 `RuntimeResult` usage/evidence 的问题，生产 AgentResult 现在保留真实结果。
- [x] Pi/AgentControl 定向测试覆盖真实 output/usage/diff、unknown runtime/handle、重复终态拒绝与 cancel 后资源清理；不存在返回空成功的 settle API。
- [x] 运行 `bash scripts/cargo-agent.sh test -p executive --test pi_rpc_runtime`（4 passed）。
- [x] **裁决:** 用户授权自主继续；保持 Executive `AgentResult` 主链，删除无 caller 的通用 Runtime receipt 投影。

### Task 3: 通过 Runtime registry/selector 选择 Pi

**Files:**
- Modify: `crates/executive/src/impl/daemon/bootstrap/runtime.rs`
- Modify: `crates/executive/src/impl/daemon/bootstrap/services.rs`
- Modify: `crates/runtime/src/selector.rs`
- Test: `crates/executive/tests/pi_runtime.rs`

- [x] Runtime crate 只拥有 manifest/selector contract；Executive `AgentRuntimeRegistry` 是唯一实例与生命周期 owner，bootstrap 将 Pi RPC manifest 与 launcher 原子注册。
- [x] selector 对 alias 也强制满足全部 required capabilities；无匹配 fail closed 并返回可诊断错误。
- [x] 测试覆盖 alias、capability、无匹配与现有 Pi RPC cancel 传播；最终 verification 仍归 Executive。
- [x] `bash scripts/cargo-agent.sh test -p runtime`（5 passed）、`--test pi_rpc_runtime`（4 passed）与 `--test pi_runtime`（7 passed）通过。
- [x] **裁决:** 用户已授权自主继续；保留唯一生产调用链 `bootstrap -> Executive AgentRuntimeRegistry -> selected Pi launcher -> AgentControl settlement`。

### Task 4: 删除文本启发式并使用生产 verification owner

**Files:**
- Delete: 无 production caller 的 `crates/executive/src/service/verifier/coding_verifier.rs`
- Keep: `crates/executive/src/impl/goal/verification.rs`
- Keep: `tests/coding/harness/{run,replay}.py`

- [x] 删除 `messages.len()`/最终文本启发式及其无 caller replacement；不保留第二套 verifier。
- [x] Executive Goal verification 校验 attempt identity、bounded diff 与 diff hash；独立 harness 再要求 workspace change、成功验收命令、operation 匹配和完整性摘要。
- [x] replay 测试拒绝篡改、错误 operation、缺失 evidence 与 false-success。
- [x] `bash scripts/cargo-agent.sh check -p executive` 与 coding replay/static tests 通过。

### Task 5: 建立 tests/coding harness 与 receipt replay

**Files:**
- Create: `tests/coding/Cargo.toml`
- Create: `tests/coding/src/lib.rs`
- Create: `tests/coding/src/harness.rs`
- Create: `tests/coding/src/receipt.rs`
- Create: `tests/coding/tests/coding_tasks.rs`
- Create: `tests/coding/tasks/*.toml`
- Create: `tests/coding/receipts/.gitkeep`
- Modify: root `Cargo.toml`

- [x] 不新增 workspace crate；`tests/coding/harness/run.py` 复制 fixture、隔离 HOME/config/runtime、启动真实 `aletheon exec`、施加 wall-clock timeout并清理进程组。
- [x] task schema 固定输入、超时、独立验收命令、forbidden path；隐藏验收仅在 Agent 结束后注入。
- [x] replay schema 保存 task/operation、事件、diff、bounded 命令输出与 digest、usage、verification、terminal status 和完整性摘要。
- [x] replay 测试检测篡改 diff、缺失 command evidence、错误 operation id 和 false-success。
- [x] 验收 Cargo 命令统一通过 `scripts/cargo-agent.sh`；静态门禁通过，未用闭包伪造成功。

### Task 6: Fixture 1 — 局部 Rust Bug 修复

**Files:**
- Create: `tests/coding/fixtures/rust_bugfix/`
- Create: `tests/coding/tasks/rust_bugfix.toml`

- [x] 小仓库包含一个 off-by-one 缺陷和失败单元测试；prompt 只描述行为，不给出目标行；初始测试确认失败。
- [x] 独立验收配置运行完整测试并验证 forbidden path 不变；真实 Agent 修改 `src/lib.rs`。
- [x] receipt 含 diff、成功命令、匹配 operation id 与 `verified` terminal status。

### Task 7: Fixture 2 — 跨文件功能实现

**Files:**
- Create: `tests/coding/fixtures/rust_multifile/`
- Create: `tests/coding/tasks/rust_multifile.toml`

- [x] 小仓库包含 parser、domain、CLI 三层，要求新增跨文件功能；隐藏验收覆盖规范化、错误路径，独立 CLI 命令覆盖输出。
- [x] harness 记录 changed files 并拒绝 forbidden path 变化；真实 Agent 修改 parser 层并通过隐藏验收。
- [x] 保存可重放 receipt，Cargo 验收与独立 CLI 命令全部成功。

### Task 8: Fixture 3 — 诊断与约束修复

**Files:**
- Create: `tests/coding/fixtures/rust_diagnosis/`
- Create: `tests/coding/tasks/rust_diagnosis.toml`

- [x] 小仓库提供表面失败测试，根因位于配置与实现约束不一致；prompt 不直接暴露根因。
- [x] 验收覆盖 deterministic test、harness wall timeout 和进程组清理；真实 Agent 完成最小修复。
- [x] replay false-success 测试确认最终声明不能覆盖失败验收；失败 receipt 必须标记 `failed_verification`。

### Task 9: 生产故障矩阵

**Files:**
- Modify: `tests/production/failure_matrix.sh`
- Modify: `tests/production/failure_matrix_static_test.sh`
- Test: `tests/coding/tests/coding_tasks.rs`

- [x] 定向门禁覆盖 restart（coding_goal_flow）、cancel/残留进程（Pi/Pi RPC process-group tests）、timeout（Pi structured failure）、orphan（worktree_recovery）、重复 verification/settlement（idempotency tests）与 false-success（receipt replay）。
- [x] 运行三个 coding fixture；每个均由真实 `aletheon exec` 产生独立 receipt（期间一次 provider 429 被记录为失败后重跑成功）。
- [x] 运行 `bash scripts/architecture-check.sh` 与 `git diff --check`（2026-07-20 通过）。

## 完成条件

- [x] 三个 fixture 全部通过真实 Executive 路径。
- [x] Executive `AgentResult` + Goal coding report + harness receipt 携带真实 output、usage、diff 和关联 evidence；不存在平行 Runtime receipt。
- [x] verifier 不再依据消息数量或最终文本判定完成。
- [x] Runtime selector 是 Pi 生产选择入口，最终验证仍归 Executive。
- [x] restart/cancel/timeout/orphan/false-success 的 deterministic production-path tests 全部 fail closed；installed multi-user destructive matrix 仍要求 disposable host，作为外部验证门禁保留。

## 外部验证门禁

- `tests/production/failure_matrix.sh` 必须在安装了 candidate binary、两个 disposable
  users、V01 aggregate receipt 和真实 failure driver 的主机运行；当前开发容器不具备
  这些前置条件，因此只运行并通过 `failure_matrix_static_test.sh`，不伪造 destructive
  installed-host 结果。
