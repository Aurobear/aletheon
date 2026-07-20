# Architecture Boundary Convergence Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除当前已确认的错误所有权、宽依赖和无生产调用者抽象，同时保持 16 个领域 crate，不用拆 crate 掩盖耦合。

**Architecture:** 类型归其生命周期 owner；Executive 只组合领域服务；Fabric 只承载稳定跨领域协议；实验抽象逐个以“真实接入或删除”裁决。

**Tech Stack:** Rust workspace、Cargo dependency graph、architecture ledger/check。

---

## 需求锚点

- MCP owner：`docs/arch/aletheon-current-architecture-review-and-optimization.md:57-61`。
- `execd -> corpus` 收敛：`docs/arch/aletheon-current-architecture-review-and-optimization.md:95-103`。
- 禁止横向拆 crate：`docs/arch/aletheon-current-architecture-review-and-optimization.md:105-113`。
- 待裁决抽象：`architecture-status.toml:38-63`。

### Task 1: MCP 配置归 Corpus

**Files:**
- Modify: `crates/corpus/src/tools/mcp/config.rs`
- Modify: `crates/corpus/src/tools/mcp/client.rs`
- Modify: `crates/cognit/src/config/` 中定义 MCP 类型的现有文件
- Modify: `crates/executive/src/impl/doctor.rs`
- Modify: `crates/corpus/Cargo.toml`
- Test: `crates/executive/tests/gbrain_mcp_adapter.rs`

- [x] 使用 `rg -n 'Mcp[A-Z]' crates/cognit/src/config crates/corpus crates/executive` 建立类型和 caller 清单。
- [x] 在 Corpus MCP config 中建立 canonical 类型，保留 legacy transport、OAuth、tool policy serde 兼容测试。
- [x] 迁移 Corpus client、Executive doctor/config caller，并删除 Cognit 中仅服务 MCP 的定义。
- [x] Corpus 不再使用 Cognit 符号并删除 Cargo 依赖；未恢复 `mcp-types`。
- [x] 运行 Corpus MCP config、Cognit config 和 Executive doctor 定向测试；architecture check 在账本更新后运行。
- [x] **裁决:** MCP 生命周期由 Corpus 持有；用户已授权按生命周期 owner 自主裁决。

### Task 2: MCP credential grant 归属审计

**Files:**
- Inspect: `crates/corpus/src/tools/mcp/auth.rs:7`
- Inspect: `crates/corpus/src/tools/mcp/client.rs`
- Inspect: `crates/mnemosyne/src/credential.rs`
- Modify: 仅在用户裁决后的 owner 文件

- [x] Caller 审计确认 Mnemosyne grant 服务 embedding，而 Corpus MCP 仅借用其 endpoint comparison，生命周期不同。
- [x] Corpus 定义不含 secret 的 `McpEndpointCredentialGrant`；Mnemosyne 保留 embedding 私有 grant。
- [x] Credential grant 未搬入 Fabric；Corpus 已删除 Mnemosyne 依赖。
- [x] **裁决:** endpoint grant 分属各自领域，避免伪通用共享类型；用户已授权自主裁决。

### Task 3: 缩窄 execd structured patch 依赖

**Files:**
- Modify: `crates/execd/Cargo.toml`
- Modify: `crates/execd/src/filesystem.rs:276-280`
- Modify: `crates/corpus/src/tools/tools/structured_patch.rs`
- Test: `crates/execd/tests/protocol_integration.rs`

- [x] 依赖测量确认 execd 只调用 Corpus 的 structured patch parse/execute；移除 Corpus 后 normal dependency tree 从 532 行降至 74 行。
- [x] 纯 parser/hunk engine 归 `platform::structured_patch`，Corpus 提供兼容 facade，execd 不再引用 `corpus::tools::tools::*`。
- [x] Execd structured/unified patch 均由 Platform parser 和 operation-scoped FilesystemHost 执行，不再启动 ambient `patch` 子进程；未新增 crate。
- [x] 测试合法 structured/unified patch、路径逃逸、symlink、profile deny、Platform hash precondition 和 bounded RPC response。
- [x] **裁决:** 根据依赖测量选择 Platform owner；用户已授权按生命周期 owner 自主裁决。

### Task 4: 删除或接入 RuntimeOps

**Files:**
- Modify: `crates/fabric/src/include/runtime.rs:13`
- Modify: `crates/fabric/src/lib.rs:175`
- Modify: `crates/fabric/tests/mock_subsystems.rs`
- Update: `architecture-status.toml`

- [x] 搜索确认仅有定义/re-export、没有生产 caller；删除 `RuntimeOps` 及其专属 `StepResult`，保留真实使用的调度数据类型。
- [x] 未发现不能由现有 Runtime 生命周期 contract 表达的语义，因此不保留平行 runtime facade。
- [x] `bash scripts/cargo-agent.sh test -p fabric` 通过（307 passed，4 ignored；集成测试同时通过）。
- [x] **裁决:** 用户已授权无生产 caller 的实验抽象优先删除；本项独立审计后执行。

### Task 5: 删除或接入 AletheonExecutive::step

**Files:**
- Modify: `crates/executive/src/core/orchestrator.rs:125-151`
- Update: `architecture-status.toml`

- [x] 搜索确认 `step` 无 caller 且仅递增 iteration；删除方法和专属导入，保留被状态端口调用的 `iteration()`。
- [x] 未建立第二条 Turn 主链；真实 turn 继续走 `TurnEngine`/`TurnPipeline`。
- [x] Executive 定向构建与 `bash scripts/cargo-agent.sh check -p executive` 通过。

### Task 6: 裁决 CognitCore

**Files:**
- Inspect/Modify: `crates/cognit/src/core/mod.rs:75-`
- Inspect/Modify: `crates/cognit/src/core/cognit_ops.rs`
- Inspect/Modify: `crates/cognit/src/core/brain_core_subsystem.rs`
- Modify: `crates/cognit/src/lib.rs`
- Update: `architecture-status.toml`

- [x] 构造点审计确认聚合 `CognitCore` 仅在自身测试构造、没有生产 trait-object caller；生产路径直接使用 Linear harness、awareness signal、reflector 等聚焦模块。
- [x] 删除聚合外壳、适配 trait 与仅验证外壳的测试；保留 ReAct/Linear session 使用的算法模块。
- [x] 不保留或新增平行 facade；组合责任仍在 harness/session 边界。
- [x] `bash scripts/cargo-agent.sh test -p cognit` 通过（295 passed），Executive 定向检查通过。
- [x] **裁决:** 用户已授权按生产 caller 裁决；依据调用图删除无生产入口的聚合外壳。

### Task 7: 更新可执行架构门禁

**Files:**
- Modify: `architecture-status.toml`
- Modify: `scripts/architecture-check.sh`
- Modify: `config/architecture-dependencies.txt`（仅当真实依赖改变）

- [x] 从 `architecture-status.toml` 删除三个已完成收敛项；账本只保留仍存在的边界。
- [x] 增加禁止恢复 `*-api`、`*-types`、`*-broker`、`platform-*` 以及任意含连字符 workspace package 的门禁。
- [x] `bash scripts/architecture-check.sh` 与 `git diff --check` 通过。

## 完成条件

- [x] MCP canonical config 只有一个 owner。
- [x] Execd 不再引用 Corpus 深层实现路径。
- [x] 三个实验抽象均有生产 caller 或已删除。
- [x] Workspace 保持收敛后的 18 个 package，package 名不含连字符。
