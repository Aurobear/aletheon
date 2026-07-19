# Aletheon 统一执行总纲与优先级设计

> 文档版本：1.0
>
> 更新日期：2026-07-19
>
> 审计快照：`dev` / `294e76c`
>
> 定位：四份 arch 文档之上的**唯一执行总纲**。不重复各文档细节，只做三件事——把所有工作项映射成一条依赖驱动的关键路径、给出优先级裁决、把最近能动的 Wave 0–1 深化到可直接进 `/plans` 的粒度。
>
> 执行方式：**单人 + Claude 代理**。关键路径按串行设计，Host/Hardware 为可插入的序列化后续，而非同时开工的并行轨。

---

## 0. 上游文档

本总纲统合并去重以下四份文档（位于 `docs/arch/`），所有工作项均已对 `dev` 分支源码逐行验证：

- `aletheon-agent-production-audit.md` —— 工具/Runtime 层的具体缺陷与 PR1–7。
- `aletheon-current-architecture-review-and-optimization.md` —— 整体架构收敛与 A0–A6。
- `aletheon-host-platform-plan.md` —— Linux/Windows/macOS 宿主能力 H0–H5。
- `aletheon-hardware-control-platform-plan.md` —— 设备/机器人控制 D0–D6。

arch-review 的 A0–A5 与 agent-audit 的 PR1–7 存在大量重叠（如 A4 ≈ PR3+PR4、A2 部分 ≈ PR2）。本总纲将二者合并去重为 6 个 Wave，并在 §7 给出"文档工作项 → Wave"映射表，保证不丢项、不重复。

---

## 1. 核心问题：两套排序哲学的取舍

四份文档给出了两套并不完全一致的排序哲学：

- **arch-review（A0–A6）** 主张*先收敛后扩张*：冻结横向扩张 → 唯一 TurnEngine → Capability 收敛 → 状态权威 → 再加 Runtime/Host/Hardware。
- **agent-audit（PR1–7）** 主张*先能力后架构*：先修 P0 bug → Workspace Tools → Runtime → Pi → Verifier。

二者各有其正确之处，分歧的真正解法不是二选一，而是**按成本和解锁关系分波**：

- 便宜且高价值的 P0 修复（`max_iterations` 死配置、`file_search` cwd）几十行、隔离、立刻见效——不应押后。
- 昂贵的能力投入（Workspace Tools V2 / Pi adapter / Verifier）若建在当前**双轨主链**上（daemon `TurnPipeline` + CLI `TurnService` + child 直连 `CognitiveSession`），会各建一遍并语义漂移——必须等主链收敛后再做。

因此采用**依赖驱动的分波混合**方案。

---

## 2. 三条排序裁决规则

用于解决后续所有排序争议：

1. **便宜且解锁全员的先做。** 隔离、低风险、立刻见效的 P0 修复不等任何架构工作（Wave 0）。
2. **昂贵能力必须建在收敛后的主链上。** Workspace Tools V2 / Pi adapter / Verifier 一律排在 Wave 1（唯一 TurnEngine）之后，避免在 daemon/CLI/child 三条路径上各建一遍。
3. **单人执行 = 关键路径串行。** 不假设并行团队；Host/Hardware 写成"可插入的序列化后续"，只在被 Wave 2 真实依赖时才提前（H1 Linux）。

---

## 3. 关键路径全景

```text
关键路径 = coding-agent 收敛线

Wave 0  止血 + 冻结      [P0，天级，无前置]
Wave 1  唯一 TurnEngine  [关键乘数，阻塞 Wave 2+]
Wave 2  能力底座         [Workspace Tools V2 + Runtime API/Broker]
Wave 3  Pi + Verifier    [Pi 成默认 Coding Runtime]
Wave 4  状态权威         [Session/Event 唯一 authority + kill-9 恢复]
Wave 5  Profiles + 评测  [部署 profile + 编码成功率 gate]

序列化后续（不在关键路径，按需插入）：
  Host Platform:  H0/H1(Linux) 在 Wave 2 需要 "Dasein/Executive 命令
                  走 Host Capability" 时插入；H2+(Win/mac) 押到 Wave 5 后
  Hardware:       完全独立，sim-first，coding 线稳定后再单开 spec
```

依赖关系：Wave 1 是关键乘数，阻塞 Wave 2 及之后的所有昂贵能力。Wave 2 的"依赖倒置 + Host Capability 化"部分依赖 Host H1（Linux ProcessHost/FilesystemHost）。其余为线性递进。

---

## 4. Wave 0–1 深化（可直接进 /plans）

### Wave 0 —— 止血 + 冻结（P0，天级）

#### 0a 能力止血

全部已代码验证，隔离、低风险，可并行小改：

| 项 | 改动位置 | 验收 |
|---|---|---|
| `max_iterations` 死配置 | `crates/executive/src/impl/daemon/bootstrap/request.rs:502` 把 `AgentConfig.max_iterations` 接进 `ExecutiveConfig`；放宽 `crates/fabric/src/types/agent_control.rs:167` 与 `crates/executive/src/impl/agent_loader/mod.rs:192` 的 `.max(1)` clamp | `0 = unlimited` 语义可达；profile 的 20/50 迭代真正生效 |
| Profile 快照残缺 | `crates/executive/src/service/turn_runtime_ports.rs:59` 的 `ActiveAgentProfileSnapshot` 扩成 `ResolvedTurnProfile`（system_prompt / model_policy / budget / verifier 一并落地）；消除 `execute.rs:175` 的硬编码 `model_policy: None` | model_policy 不再恒为 None；profile 的 prompt/model/budget 有 E2E 测试 |
| agent 工具不可达 | 把 agent 控制工具注册**移到 profile 编译前**（调整 `bootstrap/request.rs:869-877` 与 `bootstrap/services.rs:152-153` 的顺序） | 主 Agent 能看到委派入口；未授权工具仍不可见 |
| `file_search` cwd | `crates/corpus/src/tools/tools/file_search.rs` 三个子进程（`try_ripgrep`/`try_grep`/`try_find_grep`）加 `working_dir` 参数并 `.current_dir(&ctx.working_dir)`，对照 `grep.rs` 的正确实现 | daemon cwd ≠ workspace 时搜索结果正确 |
| 搜索全局 limit | `grep.rs` 与 `file_search.rs` 的 ripgrep 路径在收集 stdout 后加 `.take(max_results)`（`--max-count` 只限每文件） | 大仓库搜索严格遵守总上限 |

> 说明：`max_iterations` 的真实机制是 `AgentConfig.max_iterations=0` 从未被接入 `ExecutiveConfig`（后者默认 50），而非文档最初描述的 `min(0).max(1)==1`。修复以"接线 + 放宽 clamp"为准。

#### 0b 架构冻结（arch-review A0）

几乎无运行时改动，防止边修边退化：

- 新建 `architecture-status.toml`：每个 Runtime/Turn/Session 接口标注 owner / 生产调用者 / authority / 兼容删除期限。
- 修 `scripts/architecture-check.sh` 的 cargo 探测（当前 `:571` 直接调 `cargo metadata`，无 Cargo 时诊断不清）与 fail-fast 诊断。
- 门禁新增：禁止新增 fabric 根级重导出、禁止 executive 新增具体 domain state。

**Wave 0 验收**：每个公开 Runtime/Turn/Session 接口都有 owner、生产调用者和状态；没有调用者的接口必须标 experimental/deprecated；P0 五项均有回归测试。

### Wave 1 —— 唯一 TurnEngine（关键乘数）

当前至少存在 `TurnService`、`TurnCoordinator`、`TurnPipeline`、`DaemonTurnOrchestrator`、`AgentControlService` 五个编排入口（均已验证为独立 struct），且 `TurnService` 自承是 `TurnCoordinator` 的 compatibility facade（`turn_service.rs:19`）却仍是 CLI 生产入口。

- 提取 `TurnEngine::execute(request, ctx, events)` trait。
- daemon `TurnPipeline` / CLI `TurnService` / child `AgentControlService` 三路降为 **adapter + policy + contributors**。
- 合并 executive（`harness_factory.rs:11`）与 cognit（`harness/session.rs:178`）**两个** `CognitiveSessionFactory` 概念。
- Native child Agent 也进入同一 TurnEngine，或明确作为外部 Runtime 并返回标准 receipt。

**Wave 1 验收**：工具授权 / deadline / cancel / compaction / receipt / terminal settlement **只有一套实现**；daemon/CLI/child 同输入的 semantic parity test 通过。

---

## 5. Wave 2–5 概要（后续各自开 spec 深化）

### Wave 2 —— 能力底座

- **Workspace Tools V2**（agent-audit PR2）：统一 `read/ls/find/grep/edit/write/bash` 语义；修 cwd/相对路径/symlink confinement；搜索全局 limit + cursor + 结构化结果；编辑支持 `expected_sha256`；长输出进 Artifact Store；旧工具名保留 alias。
- **Corpus 收敛 + 依赖倒置**（arch-review A2）：Corpus Core 只保留 catalog/executor/schema/receipt；MCP 配置从 cognit 移出（当前 `corpus/mcp/config.rs:54` `pub use cognit::config`）；credential grant 从 mnemosyne 移到 Credential Port（当前 `corpus/mcp/auth.rs:7`）；Exec Server 改依赖 platform/sandbox contract（当前 `exec-server/Cargo.toml:15` 依赖整个 corpus）。
- **Runtime API + Broker**（agent-audit PR3 / arch-review A4a）：引入 Manifest / WorkOrder / LaunchSpec / Event / Receipt；删除 Goal 的 `PI_CODER_RUNTIME_ID` 特判（`attempt_coordinator.rs:231`）与 AgentControlService 的 `.contains("pi")` 字符串判断（`agent_control/mod.rs:740`）。

**验收**：所有副作用都有 operation/permit/receipt；`corpus` 不再依赖 `cognit`/`mnemosyne`；Executive/Goal 不 import Pi 具体类型；可通过 alias/capability 选 Runtime。

**前置依赖**：依赖倒置中"Dasein/Executive 的直接 `Command::new`（`dasein/.../rollback/mod.rs:459-639` 跑 btrfs/systemctl）走 Host Capability"依赖 Host H1。

### Wave 3 —— Pi + Verifier

- **生产 Pi adapter**（agent-audit PR4）：合并 pi-coder（隔离 worktree）与 pi-rpc（resident JSONL RPC）优点；支持 steer/follow-up/abort；映射 model/prompt/tools/budget；捕获 stderr/diff/events/artifacts。
- **CodingCompletionVerifier**（agent-audit PR5）：结构化 command/test/diagnostic receipts；自动选最窄相关测试；验证失败发 `RuntimeMessage::VerificationFailure`；区分 verified/unverified/blocked/budget-exhausted。

**验收**：Pi 能完成真实文件修改 + 测试；执行中可 steering；失败后同一 session 继续修复；无证据不能返回 Verified Success。

### Wave 4 —— 状态权威

- 确定 Session/Event 唯一 authority（当前有 CanonicalSessionStore / EventSourcedSessionStore / SessionService / SessionGateway 多重表示）。
- **Trajectory / 压缩 / 恢复**（agent-audit PR6）：持久保存完整 tool call/result pairs；token-based compaction 取消固定 6 条历史（当前 `daemon_turn/helpers.rs:12` `MAX_HISTORY_MESSAGES = 6`）；session branching/checkpoint；daemon 重启后恢复支持 resumability 的 Runtime。
- 建立 `StorageManifest`、schema version、migration coordinator；定义多库备份顺序、恢复点、reconciliation。
- 恢复测试覆盖 turn / agent run / lease / approval / checkpoint。

**验收**：kill -9 后可从单一 durable authority 重建 projection；没有两个 store 互相宣称权威。

### Wave 5 —— Profiles + 评测门禁

- 部署 profile：`core` / `coding` / `personal` / `conscious` / `evolution` / `hardware-edge`；每个输出 capability/storage/recovery manifest。
- ablation：只有能在任务成功率/恢复率/安全率上证明增益的层（Memory/Agora/Dasein/Metacog）才进默认生产 profile。
- 编码 benchmark（≥30 任务）成功率 gate。

**验收**：Core/Coding profile 不因高级认知模块故障而不可用；启用高级模块有可测增益。

---

## 6. 序列化后续轨道

### Host Platform（`aletheon-host-platform-plan.md`）

- **H0/H1（Linux）** 在 Wave 2 需要 "Dasein/Executive 命令走 Host Capability" 时插入。H1（迁移 process/fs/pty/service/sandbox）是 Agent 生产化的实际前置。
- **H2+（Windows/macOS）** 押到 Wave 5 之后，不阻塞 Linux coding 线。
- 工期提醒：原计划 H1 估 4 周偏乐观，单人 + 代理下建议预留 6–8 周当量。

### Hardware Control（`aletheon-hardware-control-platform-plan.md`）

- 完全独立于 coding 线，sim-first。coding 主线（Wave 0–4）稳定后再单开 spec。
- 最小纵向切片：虚拟移动机器人 → ROS 2 仿真 → 只读 CAN/Serial。
- 真实执行器写入必须通过 sim → SIL → HIL 门槛，不提前。

两条轨道只通过 Kernel Capability 和受治理 Host 原语连接。

---

## 7. 文档工作项 → Wave 映射表

| 原文档工作项 | 归入 |
|---|---|
| audit P0（max_iterations / profile / agent 工具可达 / file_search cwd / 搜索 limit / verifier 接入 / Pi 可达） | Wave 0（bug 类）+ Wave 1/3（结构类） |
| audit PR1（修 agent 基础闭环） | Wave 0 + Wave 1 |
| audit PR2（Workspace Tools V2） | Wave 2 |
| audit PR3（Runtime API + Broker） | Wave 2 |
| audit PR4（Production Pi Adapter） | Wave 3 |
| audit PR5（Verifier + Receipt） | Wave 3 |
| audit PR6（Trajectory / 压缩 / 恢复） | Wave 4 |
| audit PR7（生产编码评测） | Wave 5 |
| arch A0（冻结 + 架构账本） | Wave 0（0b） |
| arch A1（唯一 TurnEngine） | Wave 1 |
| arch A2（Capability + Corpus 收敛） | Wave 2 |
| arch A3（状态权威与存储运营） | Wave 4 |
| arch A4（通用 Runtime + 编码能力） | Wave 2（API/Broker）+ Wave 3（Pi） |
| arch A5（部署 profiles） | Wave 5 |
| arch A6（Host / Hardware 双轨） | §6 序列化后续 |
| Host H0–H5 | §6（H0/H1 随 Wave 2；H2+ 押后） |
| Hardware D0–D6 | §6（coding 线稳定后单开） |

---

## 8. 治理与成功指标

### 8.1 架构指标（可机器验证，写入 architecture-status.toml 校验）

- 生产 TurnEngine 实现数：目标 **1**。
- Capability execution syscall 数：目标 **1**。
- 每类 durable fact 的 authority 数：目标 **1**。
- `corpus → cognit/mnemosyne`：目标 **0**。
- Fabric 新增根级 re-export：目标 **0**。
- 兼容 facade 有删除版本比例：目标 **100%**。
- Host 直接 process/fs 调用 allowlist：只减不增并最终归零。
- Required port 使用默认 no-op 的数量：目标 **0**。

### 8.2 任务指标（证明 Agent 真变强）

- 编码 benchmark 首次成功率。
- 平均修复迭代数。
- 工具调用有效率与重复调用率。
- verifier 通过率与 false-success 率。
- cancel 后残留进程数。
- crash/resume 成功率。

### 8.3 裁决原则

任何一层（Dasein/Agora/Memory/Metacog）只有能在任务成功率/恢复率/安全率上证明增益，才进默认生产 profile。控制平面复杂度必须转化为任务指标，否则不进关键路径。

---

## 9. 建议 PR 顺序

每个 PR 都要么减少兼容面/forbidden dependency，要么增加可验证能力；不以"大合并 PR"同时改协议、数据库和主链。

| PR | 内容 | 对应 Wave |
|---|---|---|
| PR-01 | P0 能力止血（五项 bug）+ 回归测试 | Wave 0a |
| PR-02 | 架构状态账本 + architecture-check 修复 + 门禁 | Wave 0b |
| PR-03 | TurnEngine contract + parity harness（不删旧 facade） | Wave 1 |
| PR-04 | daemon 迁移到 TurnEngine（拆 TurnPipeline 为 contributors） | Wave 1 |
| PR-05 | CLI / Native Agent 迁移，删除 TurnService facade | Wave 1 |
| PR-06 | Corpus 依赖倒置（MCP-owned config / CredentialPort / 删 corpus→cognit/mnemosyne） | Wave 2 |
| PR-07 | Workspace Tools V2 | Wave 2 |
| PR-08 | Runtime API + Broker（删 Pi 字符串特判） | Wave 2 |
| PR-09 | 生产 Pi adapter + CodingCompletionVerifier（编码纵向切片） | Wave 3 |
| PR-10 | Session/Event 权威收敛 + StorageManifest + kill-9 恢复 | Wave 4 |
| PR-11 | 部署 profiles + 编码成功率 gate | Wave 5 |

Host H0/H1 在 PR-06 前后按需插入；Windows/macOS 与 Hardware 在 Wave 5 后单开。

---

## 10. 明确不做

- 不推倒重写 Rust workspace；不把每个领域拆成微服务。
- 不先新建十几个 `*-api` crate 再找调用者。
- 不在 Wave 1 收敛前对 daemon/CLI/child 各建一遍昂贵能力。
- 不因名称优雅而保留没有生产调用者的抽象（`RuntimeOps` / 旧 `CognitCore` / `AletheonExecutive::step` 占位实现）。
- 不让 Dasein 或 Metacog 成为权限授予者。
- 不在 coding 闭环稳定前铺开 Host 多平台或真实机器人执行器。
- 不以单元测试数量代替唯一主链、真机恢复和端到端任务成功率。

---

## 11. 下一步

本总纲经 review 后，Wave 0–1 已达可执行粒度，建议直接进入 `plans` skill 生成 Wave 0（PR-01 / PR-02）的文件级实现计划。Wave 2–5 各自在启动前开独立 spec → plan → 实现循环。
