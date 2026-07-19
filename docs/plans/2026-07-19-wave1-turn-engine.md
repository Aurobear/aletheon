# Wave 1：唯一 TurnEngine 收敛 Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 daemon / CLI / child 三条 Turn 执行路径收敛为唯一 `TurnEngine`，并在此过程中一并落地 roadmap Wave 0 拆出的两项结构性修复（ResolvedTurnProfile、agent 工具可达性）——使工具授权、deadline、cancel、compaction、receipt、settlement 只有一套实现。

**Architecture:** 引入 `TurnEngine::execute` trait 作为唯一入口；daemon `TurnPipeline`、CLI `TurnService`、child `AgentControlService` 降为 adapter + policy + contributors。合并 executive 与 cognit 各自的 `CognitiveSessionFactory`。Profile 解析统一为一次性 `ResolvedTurnProfile`（携带 prompt/model/budget/verifier），agent 控制工具改为两阶段注册（稳定 definitions 先注册、profile 依赖的高层 delegate 后注册）。采用"契约先行 → 逐入口迁移 → 删除旧 facade"的增量路径，避免大合并 PR。

**Tech Stack:** Rust（executive / cognit / fabric crates）。

**环境说明:** cargo 可用；构建/测试走 `bash scripts/cargo-agent.sh test -p <crate> <filter>` 与 `bash scripts/cargo-agent.sh build -p <crate>`（共享 target + 跨 worktree 锁），不要用裸 cargo。

**依赖:** Wave 0（架构冻结门禁已就位；`combine_limits` 已修）。本 Wave 阻塞 Wave 2+ 的所有昂贵能力投入。

**granularity 说明:** 本 Wave 是结构性收敛，含新接口设计。任务写到"接口/类型 sketch + 精确文件目标 + 迁移步骤 + parity 验收"级别；具体函数体在实现每个任务时按现有代码补全。

---

## 当前状态（已代码验证）

五个 Turn 编排入口，均为独立 struct：
- `crates/executive/src/service/turn_service.rs:20` — `TurnService`，注释自承 "Compatibility facade over the canonical TurnCoordinator"（`:19`），但仍是 CLI 生产入口（`exec_session.rs:83,219`）。
- `crates/executive/src/service/turn_coordinator.rs:73` — `TurnCoordinator`，operation/session/cancel 控制外壳。
- `crates/executive/src/service/turn_pipeline.rs:43` — `TurnPipeline`，daemon 完整编排器。
- `crates/executive/src/service/daemon_turn/orchestrator.rs:35` — `DaemonTurnOrchestrator`。
- `crates/executive/src/service/agent_control/mod.rs:111` — `AgentControlService`（child agent）。

两个 `CognitiveSessionFactory`：
- `crates/cognit/src/harness/session.rs:178`（cognit trait）。
- `crates/executive/src/service/harness_factory.rs:11`（executive trait）。

Profile 与 agent 工具断点：
- `crates/executive/src/service/turn_runtime_ports.rs:59` — `ActiveAgentProfileSnapshot` 仅 `profile_name` + `allowed_tools`。
- `crates/executive/src/impl/daemon/bootstrap/turn_runtime.rs:435`（`snapshot()`）、`:450`（`constrain_profile_capabilities`）。
- `crates/executive/src/service/daemon_turn/execute.rs:175` — `model_policy: None` 硬编码。
- `crates/executive/src/impl/daemon/bootstrap/runtime.rs:153`（`register_agent_tools`）从 `services.rs:152` 于 profile 编译后调用；`AgentControlService` 依赖 profiles，存在构造顺序环。

---

## Phase A：ResolvedTurnProfile（对应 roadmap PR-01 #2）

把主 Turn 的不可变 Profile 快照从 2 字段扩成携带完整行为策略，并接进 model policy。

### Task A1：定义 ResolvedTurnProfile 类型

**Files:**
- Modify: `crates/executive/src/service/turn_runtime_ports.rs:57-67`

- [ ] **Step 1: 扩展快照类型**（保留 `ActiveAgentProfileSnapshot` 名或改名 `ResolvedTurnProfile`；此处新增字段，向后兼容）

```rust
/// Immutable authorization + behavior snapshot resolved once per turn.
#[derive(Clone, Debug)]
pub struct ResolvedTurnProfile {
    pub profile_name: String,
    pub allowed_tools: HashSet<String>,
    pub system_prompt: String,
    pub model_policy: Option<String>,     // None = router default
    pub max_iterations: usize,            // 0 = unlimited（沿用 combine_limits 语义）
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,
    pub approval_policy: fabric::AgentApprovalPolicy,
    pub tool_timeout_ms: u64,
}
```

保留旧 `ActiveAgentProfileSnapshot` 作为 `type ActiveAgentProfileSnapshot = ResolvedTurnProfile;` 别名，或分两步迁移，避免一次性改所有 import。

- [ ] **Step 2: 编译**

Run: `bash scripts/cargo-agent.sh build -p executive`
Expected: 类型定义编译通过（消费者尚未用新字段，先不接）。

- [ ] **Step 3: Commit**

```bash
git add crates/executive/src/service/turn_runtime_ports.rs
git commit -m "feat(executive): expand turn profile snapshot to ResolvedTurnProfile

Carry system_prompt/model_policy/budget/approval alongside allowed_tools so
the main turn honors the full agent profile, not just the tool allowlist.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task A2：填充快照 + 消费 model_policy

**Files:**
- Modify: `crates/executive/src/impl/daemon/bootstrap/turn_runtime.rs:435-447`（`snapshot()`）
- Modify: `crates/executive/src/service/daemon_turn/execute.rs:169-177`（`model_policy: None`）

- [ ] **Step 1: 写失败测试** — 断言 `snapshot()` 返回的 profile 携带非空 `system_prompt` 且 `model_policy` 与 profile 的 `model` 一致（在 `turn_runtime.rs` 测试模块，用一个含 `model:` 的临时 profile）。

- [ ] **Step 2: 运行确认失败**：`bash scripts/cargo-agent.sh test -p executive turn_runtime` → FAIL。

- [ ] **Step 3: 填充 snapshot()** — 从 `resolved.profile` 取全部字段填入 `ResolvedTurnProfile`（`resolve_by_name` 已返回完整 `AgentProfile`，字段现成）。

- [ ] **Step 4: 接 model_policy** — `execute.rs:175` 把 `model_policy: None` 改为从当前 turn 的 `ResolvedTurnProfile.model_policy` 取值（经由已有的 active profile port 读取）。

- [ ] **Step 5: 运行确认通过** + **Commit**（scoped add 两文件）。

**验收:** 主 Turn 的 model/prompt/budget 来自 profile；有 E2E 测试证明切换 profile 改变 model_policy。

---

## Phase B：agent 工具可达性（对应 roadmap PR-01 #3）

拆两阶段注册，解开 `AgentControlService` 依赖 profiles 的构造环。

### Task B1：稳定 agent 控制 definitions 前置注册

**Files:**
- Modify: `crates/executive/src/impl/daemon/bootstrap/runtime.rs:152-182`（`register_agent_tools`）
- Modify: `crates/executive/src/impl/daemon/bootstrap/request.rs`（注册时序，`:869-877` profile 编译点附近）
- Modify: `crates/executive/src/impl/daemon/bootstrap/services.rs:152-153`

- [ ] **Step 1:** 把 `register_agent_tools` 拆成两个函数：
  - `register_agent_control_definitions(tools)` —— 仅注册 `agent_spawn/wait/send/cancel/list` 的**稳定 definitions**（不依赖 `AgentControlService`，可用延迟绑定的 port 占位或 definition-only 注册），在 **profile 编译前**调用，使 profile 的 `allowed_tools` 校验（`runtime.rs:53-60`）能引用它们。
  - `bind_agent_control_runtime(tools, agent_control, profiles)` —— profile catalog 完成、`AgentControlService` 构造后，绑定实际执行体 + 注册依赖 profile 的高层 `agent` delegate 工具（现 `:166-181`）。

- [ ] **Step 2:** 引入高层 delegate 工具 `delegate_code` / `delegate_review` / `delegate_research`（主模型优先看到语义接口，而非底层 `AgentSpawnRequest`）。底层 `agent_spawn` 保留给受信任 orchestrator/高级 profile。

- [ ] **Step 3:** 更新一个默认 profile（如 `code-agent`）的 `allowed_tools` 使其能引用 `delegate_*`，验证 profile 编译不再因未知工具失败。

- [ ] **Step 4:** 构建 + 测试：`bash scripts/cargo-agent.sh test -p executive`（含新注册顺序的单测：主 Agent 的 constrained 工具集包含委派入口）。

- [ ] **Step 5: Commit**（scoped add 三文件 + delegate 工具新文件）。

**验收:** 默认主 Agent 能看到并调用委派入口；未授权工具仍不可见；启动不因 profile 引用 agent 工具而失败。

---

## Phase C：TurnEngine 契约与迁移（对应 roadmap PR-03/04/05）

### Task C1：提取 TurnEngine trait + parity harness（PR-03，不删旧 facade）

**Files:**
- Create: `crates/executive/src/service/turn_engine.rs`
- Test: `crates/executive/src/service/turn_engine.rs`（parity harness）

- [ ] **Step 1: 定义 trait**

```rust
#[async_trait]
pub trait TurnEngine: Send + Sync {
    async fn execute(
        &self,
        request: TurnRequest,
        context: TurnExecutionContext,   // principal/workspace/operation/deadline/policy
        events: Arc<dyn TurnEventSink>,
    ) -> Result<TurnExecution, TurnError>;
}
```

`TurnExecutionContext` 收敛现有 daemon/CLI 各自散落的 pre/post、budget、cancel、session 绑定。`TurnPolicy`（daemon vs CLI vs child 的差异）+ 有序 `contributors` 列表注入 pre/post，取代两套编排。

- [ ] **Step 2: parity harness** — 一组同输入用例，断言经 TurnEngine 与经现有 daemon 路径产生一致的：工具授权集、deadline、terminal settlement、receipt 结构。此步只建 harness + 让现有 daemon 适配到 harness 断言，不迁移实现。

- [ ] **Step 3:** 构建 + 运行 parity harness（此时可 `#[ignore]` 待 C2/C3 迁移后启用）。

- [ ] **Step 4: Commit**。

### Task C2：daemon 迁移到 TurnEngine（PR-04）

**Files:**
- Modify: `crates/executive/src/service/turn_pipeline.rs`（拆为 contributors）
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs`

- [ ] 把 `TurnPipeline::run`（~1,000 行）拆解为 `TurnEngine::execute` + 一组 daemon contributors（Agora/Dasein/session/event streaming 各成一个有序 contributor）。保持流式协议字节兼容（现有 daemon streaming 测试须绿）。
- [ ] 启用 C1 的 parity harness（去掉 `#[ignore]`）。
- [ ] 构建 + 全 daemon turn 测试 + parity。
- [ ] Commit。

**验收:** daemon turn 经 TurnEngine 执行；流式协议不变；parity 绿。

### Task C3：CLI / Native child 迁移 + 删除 facade（PR-05）

**Files:**
- Modify: `crates/executive/src/service/exec_session.rs:83,219`（CLI 入口改用 TurnEngine）
- Delete/Reduce: `crates/executive/src/service/turn_service.rs`（compatibility facade）
- Modify: `crates/executive/src/service/agent_control/mod.rs`（child 进入同一 TurnEngine，或明确作为外部 Runtime 返回标准 receipt）

- [ ] CLI `ExecSessionBuilder::build` 返回/驱动 TurnEngine，而非 `TurnService`。
- [ ] 删除 `TurnService` facade（或缩为 0 逻辑的 re-export 并标删除期限，登记进 `architecture-status.toml`）。
- [ ] child agent 路径：`AgentControlService` 调 TurnEngine 或经 Runtime contract（与 Wave 2 Runtime API 对齐）返回标准 receipt。
- [ ] 统一 deadline / cancel / settlement 语义（三入口共用一套）。
- [ ] 构建 + CLI/child E2E + parity。
- [ ] Commit。

**验收:** daemon/CLI/child 同输入 semantic parity test 全绿；`TurnService` 已删除或标记删除期限；deadline/cancel/settlement 单实现。

---

## Phase D：合并 CognitiveSessionFactory

### Task D1：统一 factory 概念

**Files:**
- Modify: `crates/executive/src/service/harness_factory.rs:11`
- Modify: `crates/cognit/src/harness/session.rs:178`

- [ ] 保留一个权威 `CognitiveSessionFactory`（建议 cognit 侧为核心 trait，executive 侧改为其实现/适配），消除 executive 与 cognit 的概念重复。
- [ ] `LinearCognitiveSessionFactory`（`harness_factory.rs`）成为唯一生产实现，被 TurnEngine 持有。
- [ ] 构建 + 测试。
- [ ] Commit。

**验收:** 只有一个生产 `CognitiveSessionFactory`；TurnEngine 通过它构造认知循环。

---

## 自审对照（roadmap Wave 1 + 并入项）

| roadmap 项 | 覆盖 |
|---|---|
| 唯一 TurnEngine（A1 arch-review） | Phase C |
| 合并两个 CognitiveSessionFactory | Phase D |
| ResolvedTurnProfile（Wave 0 #2） | Phase A |
| agent 工具可达性（Wave 0 #3） | Phase B |
| parity（工具授权/deadline/cancel/compaction/receipt/settlement 单实现） | Task C2/C3 验收 |

---

## 建议 PR 切分

- PR-W1-A：Phase A（ResolvedTurnProfile）—— 独立可先落。
- PR-W1-B：Phase B（agent 工具两阶段注册 + delegate 工具）。
- PR-W1-C1：TurnEngine 契约 + parity harness（不删旧路径）。
- PR-W1-C2：daemon 迁移。
- PR-W1-C3：CLI/child 迁移 + 删 facade。
- PR-W1-D：合并 factory。

每个 PR 都要么减少兼容面，要么在 parity 断言下等价替换实现；不做大合并 PR。

## 下一步

Wave 1 完成（parity 绿、单一 settlement）后进入 Wave 2「能力底座」——Workspace Tools V2 + Corpus 依赖倒置 + Runtime API/Broker 均建在此收敛后的唯一主链上。
