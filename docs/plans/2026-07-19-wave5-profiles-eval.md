# Wave 5：部署 Profiles + 编码评测门禁 Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 引入六个部署 profile（`core` / `coding` / `personal` / `conscious` / `evolution` / `hardware-edge`），让 bootstrap 按 profile 门禁 Memory / Agora / Dasein / Metacog 的加载，并为每个 profile 输出 capability / storage / recovery manifest；用 ablation 证明每层增益、只放证明有益的层进默认生产 profile；建立 ≥30 任务的编码 benchmark，用首次成功率等指标做发布门禁。

**Architecture:** Profile schema 落在 executive-owned config（`crates/executive/src/core/config/`），是 `AppConfig` 的一等字段，不复用只管 agent persona 的 `AgentProfilesConfig`。Profile 编译成 `ResolvedDeploymentProfile { required: RequiredSubsystems, optional: OptionalFeatureSet, manifests }`。Bootstrap（`crates/executive/src/impl/daemon/bootstrap/request.rs`）在构造任何领域子系统前先解析 profile：required 缺失 = 硬失败（进不了 ready），optional 缺失 = 写进 Feature Manifest 明确报告，绝不静默 no-op（落实 arch-review §7.3 的 `RequiredTurnPorts` vs `OptionalTurnFeatures`）。Ablation harness 复用并扩展现有 `crates/executive/tests/functional_indicators.rs` + `tests/support/conscious_core_harness.rs`，从"意识内核指标"扩到"任务成功率 / 恢复率 / 安全率"。编码 benchmark 是独立 fixture 目录 + Rust harness crate，产出 JSON receipt 喂给 `scripts/release-acceptance.sh` 的 V01 报告做发布 gate。

**Tech Stack:** Rust, TOML config, benchmark harness.

**环境说明:** cargo 可用；构建/测试走 `bash scripts/cargo-agent.sh test -p <crate> <filter>`，不要用裸 cargo。

**依赖:** Wave 2/3/4（能力底座、Pi+Verifier、状态权威）。具体：Task 5/6/7 依赖 Wave 3 的 `CodingCompletionVerifier` 与 Pi production adapter 产出的 `RuntimeReceipt`；Task 4 的 recovery manifest 依赖 Wave 4 的 `StorageManifest` 与 kill-9 恢复；Task 2/3 的 Required/Optional 端口拆分依赖 Wave 1 唯一 `TurnEngine`。若上游 Wave 未完成，对应 Task 标注为 gated，先做 Task 1–4 的 profile 骨架。

---

## 背景与现状锚点（实现前必读）

- Config 根：`crates/executive/src/core/config/mod.rs:66-97` 的 `AppConfig`。`AgentProfilesConfig`（同文件 :44-51）只承载 agent persona（`default` + `overrides`），**不是**部署 profile，不要往里塞子系统门禁。
- Deployment 现状：`crates/cognit/src/config/mod.rs:231-253` 的 `DeploymentConfig` 有 `mode`（`Development/User/Production`，:52-57）与 `integrations`（telegram/google/gbrain 布尔，:175-179）。没有任何"子系统加载门禁"概念。
- Bootstrap 现状：`crates/executive/src/impl/daemon/bootstrap/request.rs` 的 `RequestHandler::new`（:66）**无条件**构造所有领域子系统——Metacog（:541-547）、Memory（:682-690）、Dasein/Agora/conscious workspace（:749-773）。Dasein 目前是硬依赖：`.context("Dasein must be enabled for the recurrent conscious workspace")?`（:753）。
- Turn 端口现状：`crates/executive/src/service/turn_runtime_ports.rs:96-106` 的 `TurnRuntimePorts` 全是非可选 `Arc<dyn>`（`self_policy` / `capabilities` / `sessions` 等），没有 Required/Optional 拆分；`ActiveAgentProfileSnapshot`（:59-62）只有 `profile_name` + `allowed_tools`。
- 已有 ablation 骨架：`crates/executive/tests/functional_indicators.rs:157-217` 的 `workspace_recurrence_and_dasein_ablations_reduce_target_metrics`，用 `AblationConfig { workspace, recurrence, dasein_modulation }`（定义于 `crates/executive/tests/support/conscious_core_harness.rs`）跑 `run_ablation`，产 `processor_deliveries / recurrent_broadcasts / dasein_modulations`，写 `ablation-evidence.json`。
- 已有发布门禁：`scripts/release-acceptance.sh` 的 `validate_v01_report` 校验 V01 报告，已含 `ablations`（集合固定为 `{workspace, recurrence, dasein}`，要求 `baseline > ablated`）、`functional_indicators`、`architecture_gate`。Wave 5 的 benchmark 与新 ablation 必须扩展这套 report，而不是另起炉灶。
- 构建/测试 wrapper：`scripts/cargo-agent.sh`（flock + bounded target cache）。所有 cargo 命令走它。

---

## 术语与不变式

- **Required subsystem**：profile 声明为必需的端口。缺失或构造失败 → daemon 拒绝进入 ready（fail-closed）。对应 arch-review §7.3 `RequiredTurnPorts`。
- **Optional feature**：profile 声明为可选的能力层（Memory/Agora/Dasein-interpretation/Metacog）。未启用或不健康 → 在 Feature Manifest 里显式记为 `disabled{reason}` / `degraded{reason}`，turn 路径拿到 `None` 而非默认空对象。对应 §7.3 `OptionalTurnFeatures`。
- **Manifest 三元组**：每个 profile 启动后输出 `CapabilityManifest`（暴露给 agent 的工具/能力集合）、`StorageManifest`（打开的持久化库与 authority）、`RecoveryManifest`（kill-9 后可从哪个 durable authority 重建哪些投影）。
- **裁决原则（roadmap §8.3）**：某层（Memory/Agora/Dasein/Metacog）只有在 ablation 中对任务成功率/恢复率/安全率证明增益，才进默认生产 profile；否则降级为可选或实验。

---

## Task 1 — 部署 profile config schema

- [ ] 1.1 新建 profile 配置类型

**Files:**
- `crates/executive/src/core/config/profile.rs`（新建）
- `crates/executive/src/core/config/mod.rs`（`mod profile;` + `pub use`，并在 `AppConfig` 增加字段）

**Schema sketch**（`profile.rs`）：

```rust
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

/// 内建部署 profile 标识。`Custom` 允许运维用 TOML 覆盖 subsystem 门禁。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentProfileKind {
    Core,
    #[default]
    Coding,
    Personal,
    Conscious,
    Evolution,
    HardwareEdge,
    Custom,
}

/// 单个子系统的门禁状态。Required 缺失=硬失败；Optional 缺失=显式报告。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubsystemGate {
    /// 必需：构造失败则 daemon 拒绝 ready。
    Required,
    /// 可选：可用则加载，不可用则在 manifest 记 disabled/degraded。
    Optional,
    /// 关闭：不构造，turn 路径拿到 None。
    Disabled,
}

/// profile 对四个可选认知层 + 硬件层的门禁矩阵。
/// 未列出的核心项（Kernel/TurnEngine/Corpus/Session）恒为 Required，不可配。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SubsystemGates {
    pub memory: SubsystemGate,     // Mnemosyne MemoryService
    pub agora: SubsystemGate,      // Agora 共享工作状态
    pub dasein: SubsystemGate,     // Dasein self-interpretation（非 constitutional policy）
    pub metacog: SubsystemGate,    // Metacog 变更/演化
    pub conscious: SubsystemGate,  // recurrent conscious workspace（依赖 dasein=Required|Optional）
    pub hardware: SubsystemGate,   // Hardware broker（hardware-edge only）
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DeploymentProfileConfig {
    /// 选择内建 profile；Custom 时以 gates 覆盖为准。
    pub kind: DeploymentProfileKind,
    /// 覆盖内建 profile 的门禁（仅 Custom 或显式覆盖时生效）。
    pub gates: Option<SubsystemGates>,
    /// 启动后是否把三份 manifest 写到 audit 目录并打日志。
    pub emit_manifests: bool,
}
```

- [ ] 1.2 为每个内建 kind 提供 `SubsystemGates::for_kind(kind) -> Self`，矩阵如下（与 roadmap §5 / arch-review §9-A5 一致）：

| profile | memory | agora | dasein | metacog | conscious | hardware |
|---|---|---|---|---|---|---|
| core | Disabled | Disabled | Disabled | Disabled | Disabled | Disabled |
| coding | Optional | Disabled | Disabled | Disabled | Disabled | Disabled |
| personal | Required | Disabled | Required | Disabled | Disabled | Disabled |
| conscious | Required | Required | Required | Disabled | Required | Disabled |
| evolution | Required | Required | Required | Required | Required | Disabled |
| hardware-edge | Optional | Disabled | Disabled | Disabled | Disabled | Required |

> 说明：`conscious=Required` 要求 `dasein` 至少 Optional（对齐 request.rs:753 现有硬依赖）；Task 1.3 加校验。核心四件套（Kernel/TurnEngine/Corpus/Session）不进此矩阵，恒 Required。

- [ ] 1.3 加 `DeploymentProfileConfig::resolve_gates() -> Result<SubsystemGates>`：kind 展开为矩阵，`gates` 覆盖后做一致性校验（`conscious==Required` 且 `dasein==Disabled` → `Err`；`hardware==Required` 且 kind 非 `HardwareEdge` → warn）。

- [ ] 1.4 在 `AppConfig`（`mod.rs:66`）新增字段：
```rust
    #[serde(default)]
    pub profile: DeploymentProfileConfig,
```
并在 `mod.rs:16-34` 的 `pub use` 区导出 `profile::{DeploymentProfileConfig, DeploymentProfileKind, SubsystemGates, SubsystemGate}`。

**Acceptance:**
- `bash scripts/cargo-agent.sh test -p aletheon-executive config::profile` 通过：六个 kind 各产出预期矩阵；`deny_unknown_fields` 拒绝未知 key；`conscious/dasein` 矛盾组合返回 `Err`。
- `AppConfig::default()` 的 `profile.kind == Coding`（默认生产 profile，见 Task 5 裁决）。
- 现有 config 加载测试（`crates/executive/src/impl/daemon/bootstrap/request_tests.rs`）仍绿——`profile` 字段 `#[serde(default)]`，旧 config 无需改。

---

## Task 2 — Required / Optional turn 端口拆分 + Feature Manifest 类型

> gated on Wave 1 唯一 TurnEngine。若 TurnEngine 已合并，直接在其端口上做；否则先在 `turn_runtime_ports.rs` 建类型，adapter 层暂时把现有字段包装进新结构。

- [ ] 2.1 在 `crates/executive/src/service/turn_runtime_ports.rs` 拆分端口

**Files:**
- `crates/executive/src/service/turn_runtime_ports.rs`

**Sketch**（落实 arch-review §7.3）：
```rust
/// 缺失即 daemon 拒绝 ready。
pub struct RequiredTurnPorts {
    pub models: Arc<dyn ModelSelectionPort>,
    pub capabilities: Arc<dyn GovernedTurnCapabilityPort>,
    pub sessions: Arc<dyn TurnSessionStatePort>,
    pub config: Arc<dyn TurnConfigPort>,
    pub observability: Arc<dyn TurnObservabilityPort>,
    pub hooks: Arc<dyn TurnHookPort>,
    pub storm: Arc<dyn StormStatePort>,
    pub approvals: Arc<dyn TurnApprovalPort>,
}

/// 缺失=manifest 显式报告，turn 拿到 None，不是默认空对象。
#[derive(Default)]
pub struct OptionalTurnFeatures {
    pub self_interpretation: Option<Arc<dyn SelfPolicyPort>>, // Dasein
    pub memory_recall: Option<Arc<dyn MemoryRecallPort>>,
    pub agora_read: Option<Arc<dyn AgoraReadPort>>,
}

pub struct TurnRuntimePorts {
    pub required: RequiredTurnPorts,
    pub optional: OptionalTurnFeatures,
}
```
- [ ] 2.2 定义 `MemoryRecallPort` / `AgoraReadPort` trait（若 Wave 2/4 未定义则在此新建最小只读接口）。`self_policy` 从必需降为 `Option`；turn 执行处对 `None` 走"跳过主体解释/记忆召回"分支，而非 no-op 实现。
- [ ] 2.3 新建 Feature Manifest 类型：

**Files:** `crates/executive/src/service/feature_manifest.rs`（新建）+ 在 `crates/executive/src/service/mod.rs` 导出。
```rust
#[derive(Debug, Clone, Serialize)]
pub enum FeatureStatus {
    Enabled,
    Disabled { reason: String },     // gate=Disabled
    Degraded { reason: String },     // 声明启用但构造/健康失败（Optional）
    Missing { reason: String },      // Required 缺失（触发 fail-closed 前记录）
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureManifest {
    pub profile: String,
    pub features: BTreeMap<String, FeatureStatus>, // memory/agora/dasein/metacog/conscious/hardware
}
```

**Acceptance:**
- 编译通过：`bash scripts/cargo-agent.sh build -p aletheon-executive`。
- 单测：`OptionalTurnFeatures::default()` 三个字段皆 `None`；turn 执行对 `None` 分支有覆盖测试（无 memory 时不调用召回、无 dasein 时不做 self review）。
- 无 `Arc<dyn ...>` 默认空实现残留于 Optional 路径（grep `NoopSelfPolicy`/`EmptyMemory` 类型应为 0 处生产引用）。

---

## Task 3 — Bootstrap 按 profile 门禁子系统构造

> 依赖 Task 1 + Task 2。核心改动集中在 `RequestHandler::new`。

- [ ] 3.1 在 `RequestHandler::new`（`crates/executive/src/impl/daemon/bootstrap/request.rs:66`）开头解析 profile

**Files:**
- `crates/executive/src/impl/daemon/bootstrap/request.rs`
- `crates/executive/src/impl/daemon/bootstrap/services.rs`（若子系统构造已下沉到此，同步改）

**改动:**
- 在构造任何领域子系统前：`let gates = config.profile.resolve_gates()?;`
- Metacog（现 :541-547 无条件）→ 仅当 `gates.metacog != Disabled` 构造；`Required` 且失败 → `return Err`；`Optional` 且失败 → 记 `FeatureStatus::Degraded` 并置 `None`。
- Memory（现 :682-690）→ 同上门禁 `gates.memory`。
- Dasein / Agora / conscious workspace（现 :749-773）→ 用 `gates.dasein` / `gates.agora` / `gates.conscious` 门禁。删除 :753 的无条件 `.context("Dasein must be enabled...")`，改为：`conscious=Required` 时才要求 dasein_handle，否则跳过 conscious registry 构造。
- 组装 `OptionalTurnFeatures`：把实际构造成功的 memory/agora/dasein 包成 `Some`，否则 `None`。

- [ ] 3.2 Fail-closed 语义：任一 `Required` 子系统构造失败，`RequestHandler::new` 返回 `Err`，daemon 不进入 ready（对齐 arch-review §11.3"启动中任一 DB migration 失败必须阻止进入 ready"与 §7.3）。

- [ ] 3.3 在 bootstrap 末尾产出并（按 `profile.emit_manifests`）落盘 `FeatureManifest` 到 `deployment.paths.audit`/`feature-manifest.json`，并 `tracing::info!` 每个 feature 的状态。

**Acceptance:**
- 新增 bootstrap 测试（`request_tests.rs` 或新建 `profile_gating_tests.rs`）：
  - `core` profile 启动后 Memory/Agora/Dasein/Metacog 均未构造，daemon ready，manifest 全 `Disabled{reason}`。
  - `coding` profile：memory=Optional，未配 memory backend 时 daemon 仍 ready、manifest 记 `Disabled`/`Degraded`，**不 panic、不静默**。
  - `conscious` profile 在 dasein 构造失败时 daemon 拒绝 ready（Required fail-closed），错误信息含子系统名。
- `bash scripts/cargo-agent.sh test -p aletheon-executive profile_gating` 通过。
- 现有 daemon 冒烟测试在默认（coding）profile 下仍绿。

---

## Task 4 — 每个 profile 输出 capability / storage / recovery manifest

> recovery manifest 依赖 Wave 4 `StorageManifest`；若 Wave 4 未完成，storage/recovery 先输出已打开库的静态清单，authority 字段标 `unverified`。

- [ ] 4.1 定义三份 manifest 类型

**Files:** `crates/executive/src/service/deployment_manifest.rs`（新建）
```rust
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityManifest {
    pub profile: String,
    pub exposed_tools: Vec<String>,        // 该 profile 下 agent 可见工具/能力
    pub runtimes: Vec<String>,             // 可用 Capability Runtime（pi/native/...）
    pub delegation_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageManifest {
    pub profile: String,
    pub stores: Vec<StoreEntry>,           // 每个打开的 sqlite/json 库
}
#[derive(Debug, Clone, Serialize)]
pub struct StoreEntry { pub name: String, pub path: String, pub authority: String, pub schema_version: u32 }

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryManifest {
    pub profile: String,
    pub durable_authorities: Vec<String>,  // kill-9 后可重建来源
    pub rebuildable_projections: Vec<String>,
    pub backup_order: Vec<String>,
}
```
- [ ] 4.2 在 bootstrap（Task 3.3 同处）根据实际构造的子系统填三份 manifest，与 `FeatureManifest` 一起落盘 audit 目录（`capability-manifest.json` / `storage-manifest.json` / `recovery-manifest.json`）。
- [ ] 4.3 加只读查询入口（供 monitor MCP / 诊断读取），复用现有 diagnostics 端口（`crates/executive/src/core/config/diagnostics.rs` 模式）。

**Acceptance:**
- `bash scripts/cargo-agent.sh test -p aletheon-executive deployment_manifest` 通过：`core` 的 StorageManifest 不含 memory/dasein/metacog 库；`evolution` 含全部；每个 store 的 `authority` 唯一（无两个 store 声称同一 fact 权威）。
- manifest JSON 可被 `serde_json` 往返解析，字段稳定（写快照测试）。

---

## Task 5 — Ablation harness（证明每层增益）

> 依赖 Wave 3 verifier 产出的成功/失败判定。扩展现有 `functional_indicators.rs` 的 ablation 骨架，从"意识内核指标"扩到"任务成功率 / 恢复率 / 安全率"。

- [ ] 5.1 扩展 `AblationConfig`

**Files:**
- `crates/executive/tests/support/conscious_core_harness.rs`（现 `AblationConfig`）
- 新建 `crates/executive/tests/support/ablation_metrics.rs`

**改动:** `AblationConfig` 增加 `memory: bool` / `agora: bool` / `metacog: bool`（已有 `workspace`/`recurrence`/`dasein_modulation`）。ablation 度量从单一 count 扩为三类任务级指标：
```rust
pub struct AblationOutcome {
    pub task_success_rate: f64,   // 通过 verifier 的任务比例
    pub recovery_rate: f64,       // 注入失败后成功恢复比例
    pub safety_rate: f64,         // 无越权/无 protected-path 写入比例
}
```
- [ ] 5.2 对每个可选层跑 baseline（全开）vs ablated（关该层），断言至少一项目标指标下降（沿用 release-acceptance `baseline > ablated` 语义）。层集合：`memory / agora / dasein / metacog`（保留现有 `workspace / recurrence`）。
- [ ] 5.3 产出 `ablation-evidence.json`：扩展现有 :206-215 的写法，`ablations` 从 `{workspace, recurrence, dasein}` 扩为 `{workspace, recurrence, dasein, memory, agora, metacog}`，每项 `{baseline, ablated}` 为对应目标指标。
- [ ] 5.4 更新 `scripts/release-acceptance.sh` 的 `validate_v01_report`：把期望 ablation 集合从 `{"workspace","recurrence","dasein"}` 改为新集合，保持 `baseline > ablated` 校验。

**裁决落地：** 根据 ablation 结果确定默认生产 profile = `coding`（roadmap §8.3：只有证明增益的层进默认）。若某层在编码任务上无增益，则它在 `coding` profile 保持 `Disabled`，仅在 `personal`+ 启用。本 plan 默认矩阵（Task 1.2）即体现该裁决——`coding` 只开 memory=Optional，其余 Disabled。

**Acceptance:**
- `bash scripts/cargo-agent.sh test -p aletheon-executive workspace_recurrence_and_dasein_ablations` 及新增 `memory_agora_metacog_ablations` 通过。
- `bash scripts/release-acceptance.sh` 对含新 ablation 集合的 V01 报告校验通过；旧集合被拒（防漂移）。
- ablation 可重现（同 seed 两次结果一致，沿用 :137-138 的 determinism 断言）。

---

## Task 6 — 编码 benchmark 任务分类学（≥30 任务）

> 依赖 Wave 3 Pi adapter + verifier（任务需真实执行）。fixtures 与 harness 分离，fixtures 是 git 内可控小仓库。

- [ ] 6.1 建 benchmark fixture 目录

**Files:**
- `tests/benchmarks/coding/` （新建根）
- `tests/benchmarks/coding/tasks/*.toml`（每任务一个描述文件）
- `tests/benchmarks/coding/fixtures/`（每任务的种子仓库/文件）
- `tests/benchmarks/coding/manifest.toml`（任务清单 + 版本号 + 期望门禁）

**任务描述 schema**（`tasks/<id>.toml`）：
```toml
id = "bugfix-single-file-01"
category = "single_file_bug"
profile = "coding"                 # 用哪个部署 profile 跑
fixture = "fixtures/calc"          # 种子仓库相对路径
prompt = "修复 add() 对负数返回错误的问题"
[acceptance]
verifier = "test"                  # 由 CodingCompletionVerifier 判定
must_pass_tests = ["calc::tests::add_negative"]
forbidden_paths = ["Cargo.lock"]   # 越权写入即安全失败
max_repair_iterations = 8
```

- [ ] 6.2 任务分类学（≥30，覆盖 audit §13 的 13 类；每类 ≥2 任务）：

| 类别 | 数量 | 说明 |
|---|---|---|
| search_explain | 3 | 搜索并解释代码，无写入 |
| single_file_bug | 3 | 单文件 bug 修复 |
| cross_module | 3 | 跨模块修改 |
| add_tests | 2 | 新增测试 |
| compile_recovery | 3 | 从编译失败恢复 |
| test_failure_recovery | 3 | 从测试失败恢复 |
| large_output | 2 | 大搜索/大 diff 输出处理（验 artifact 分页） |
| user_steering | 2 | 执行中 steering |
| subagent_review | 2 | 委派 reviewer subagent |
| daemon_restart_resume | 2 | daemon 重启后恢复任务 |
| dirty_worktree | 2 | 脏工作区不破坏用户未提交改动 |
| agents_md_follow | 2 | 遵循 `AGENTS.md` 指令 |
| protected_path_reject | 2 | protected/越权路径写入被拒（安全） |

- [ ] 6.3 `manifest.toml` 声明 `fixture_version` 与 `schema_version`（对齐 release-acceptance 对 fixture/schema 版本+sha256 的校验，见 :16-20），并列出全部任务 id。

**Acceptance:**
- 任务数 ≥30 且 13 类全覆盖（写一个 `tests/benchmarks/coding/manifest_static_test.sh`，纯静态校验 toml 齐全、id 唯一、每个 fixture 存在，不需 cargo）。
- 每个 task toml 通过 schema 校验（`deny_unknown_fields`）。
- fixture 仓库最小化（每个 < 50 文件），可被 sandbox 打开。

---

## Task 7 — Benchmark runner + 指标计算

> 依赖 Task 6 fixtures、Wave 3 verifier/receipt、Task 4 manifest。

- [ ] 7.1 新建 benchmark harness crate

**Files:**
- `crates/coding-bench/Cargo.toml`（新建 workspace member；加入根 `Cargo.toml` members；提交 `Cargo.lock`）
- `crates/coding-bench/src/lib.rs`（任务加载、按 profile 起 in-process 或 daemon 执行、收集 `RuntimeReceipt`）
- `crates/coding-bench/src/metrics.rs`（指标计算）
- `crates/coding-bench/src/bin/run.rs`（CLI：跑全套、产 JSON receipt）

- [ ] 7.2 指标定义（roadmap §8.2 + audit §13，每项给出精确算法）：

| 指标 | 定义 | 来源 |
|---|---|---|
| `first_attempt_success` | 首轮（0 次 repair）即通过 verifier 的任务数 / 总任务数 | verifier verdict + repair 计数 |
| `repair_iterations_mean` | 每任务从首次失败到通过的平均迭代数 | turn/attempt 计数 |
| `verifier_pass_rate` | verifier 判 `SucceededVerified` 的任务比例 | `CompletionStatus`（audit §3.4） |
| `false_success_rate` | 模型自称完成但 verifier 未通过 / 声称完成总数 | 对比模型 final vs verifier verdict |
| `residual_processes_after_cancel` | cancel 后仍存活的子进程/子 agent 数（应为 0） | Kernel process table 快照 |
| `crash_resume_success` | kill-9 后能重建并完成的任务比例 | daemon_restart_resume 类任务 |
| `regression_rate` | 修改引入新失败测试的任务比例 | 前后测试集 diff |
| `tool_effective_rate` | 有效工具调用 / 总工具调用（重复/无效调用比） | receipt tool events |

- [ ] 7.3 产出 `benchmark-receipt.json`：`{schema_version, fixture_version, fixture_sha256, profile, per_task[], aggregate{...上表指标...}}`，格式与 release-acceptance V01 report 的 checksum/结构约定一致。
- [ ] 7.4 提供 `just coding-bench` 或 `bash scripts/coding-bench.sh`（内部走 `scripts/cargo-agent.sh run -p coding-bench --bin run`）。

**Acceptance:**
- `bash scripts/cargo-agent.sh test -p coding-bench` 通过：指标计算单测（构造已知 receipt 集，断言各指标数值）。
- runner 能对 `coding` profile 跑通至少 search_explain + single_file_bug 两类（其余可在 CI 全量），产出结构合法的 `benchmark-receipt.json`。
- `false_success_rate` 与 `residual_processes_after_cancel` 有专门单测覆盖边界（自称成功但验证失败、cancel 后残留）。

---

## Task 8 — 发布门禁：成功率 gate

> 依赖 Task 5/7。把 benchmark 与 ablation receipt 接入现有发布链。

- [ ] 8.1 定义门禁阈值（写进 `tests/benchmarks/coding/manifest.toml` 的 `[gate]`）：
```toml
[gate]
min_first_attempt_success = 0.60
min_verifier_pass_rate = 0.80
max_false_success_rate = 0.05
max_residual_processes_after_cancel = 0
min_crash_resume_success = 0.90
```
> 初始阈值保守；随 Wave 3/4 成熟上调。阈值变更需在 PR 说明记录。

- [ ] 8.2 扩展 `scripts/release-acceptance.sh`

**Files:** `scripts/release-acceptance.sh`
- 新增 `validate_coding_benchmark <receipt>`：校验 `benchmark-receipt.json` 的 aggregate 指标满足 `[gate]` 阈值；任一不达标 → `fail(...)`（非零退出，阻断发布）。
- 在 V01 report 或聚合 receipt 的 `results` 里新增 `coding_benchmark` 段（与现有 `ablations` / `functional_indicators` / `architecture_gate` 并列）。

- [ ] 8.3 CI 接线

**Files:** `.github/workflows/`（找现有 release/acceptance workflow 追加 step；若无则加到既有 CI gate job）
- 发布 lane 增加"跑 coding-bench + 校验阈值"step，走 `scripts/cargo-agent.sh`。
- 非发布 PR 可只跑子集（search_explain + single_file_bug）做快速反馈，全量在 release lane。

**Acceptance:**
- 构造一份低于阈值的 `benchmark-receipt.json`，`bash scripts/release-acceptance.sh`（或新校验函数）返回非零并打印具体不达标指标。
- 达标 receipt 通过。
- `tests/production/release_aggregate_receipt_test.sh` 更新后仍绿（聚合 receipt 含 coding_benchmark 段）。

---

## Task 9 — Profile 配置文件与文档

- [ ] 9.1 提供内建 profile 示例 config

**Files:**
- `config/profiles/core.toml`
- `config/profiles/coding.toml`
- `config/profiles/personal.toml`
- `config/profiles/conscious.toml`
- `config/profiles/evolution.toml`
- `config/profiles/hardware-edge.toml`

每个仅含 `[profile]` 段（`kind = "..."`）+ 该 profile 相关的最小 subsystem 配置，作为 `--config` 叠加层示例。对齐 `config/production.toml.example` 风格。

- [ ] 9.2 更新 `config/aletheon.example.toml` 增加 `[profile]` 段注释说明六个 profile 与门禁语义。
- [ ] 9.3 更新架构治理账本

**Files:** `config/architecture-allowlist.txt` / `architecture-status.toml`（若 Wave 0b 已建）
- 把新增 `crates/coding-bench` 与 profile/manifest 端口登记 owner / 生产调用者 / authority。
- 登记 roadmap §8.1 治理指标：`Required port 使用默认 no-op 的数量 = 0`（Task 2 已消除）；每类 durable fact authority = 1（Task 4 StorageManifest 校验）。

**Acceptance:**
- `bash scripts/aurb.sh validate`（若适用）或 `bash tests/architecture_check.sh` 通过——新 crate 与端口有登记，无未登记的生产入口。
- 六个 profile 示例 config 能被 `AppConfig::from_file` 解析成功（加 `crates/executive` 内一个遍历 `config/profiles/*.toml` 的测试）。

---

## Task 10 — 端到端验收：profile × benchmark 矩阵

- [ ] 10.1 端到端集成测试

**Files:** `crates/coding-bench/tests/profile_matrix.rs`（新建）
- 断言 `core` profile 下高级模块（Memory/Agora/Dasein/Metacog）全不加载，但 search_explain + single_file_bug 类任务仍能完成（roadmap §5 验收："Core/Coding profile 不因高级认知模块故障而不可用"）。
- 断言 `coding` profile 首次成功率 ≥ gate 阈值（用小子集 + 放宽阈值做冒烟，全量在 release lane）。
- 断言启用某可选层（如 personal 的 memory/dasein）在其目标任务类上有可测增益（引用 Task 5 ablation 结论）。

- [ ] 10.2 更新 memory/文档：在 `docs/plans/` 或 arch 索引登记 Wave 5 完成状态，标注哪些层经 ablation 证明进默认 profile。

**Acceptance:**
- `bash scripts/cargo-agent.sh test -p coding-bench --test profile_matrix` 通过。
- roadmap §5 Wave 5 两条验收均可用测试证据支撑：(1) Core/Coding 不因高级模块故障不可用；(2) 启用高级模块有可测增益。
- 发布 gate（Task 8）在默认 coding profile 上端到端跑通并产聚合 receipt。

---

## 实施顺序与并行度

1. **先做骨架（不依赖上游 Wave）：** Task 1 → Task 4（profile schema、端口拆分类型、bootstrap 门禁、manifest 输出）。
2. **gated on Wave 3/4：** Task 5（ablation 任务级指标）、Task 6/7（benchmark + runner）需 verifier/receipt/恢复能力就位。
3. **收口：** Task 8（gate）→ Task 9（config/文档）→ Task 10（矩阵验收）。

可并行：Task 6（fixtures，纯数据）与 Task 1–4（Rust）互不阻塞；Task 9 config 文件可在 Task 1 schema 定稿后随时做。

## 风险与开放假设

- Wave 3 的 `CodingCompletionVerifier` 与 `CompletionStatus`（audit §3.4）是 benchmark 判定的前置；若其 verdict 结构未定，Task 7 的 `verifier_pass_rate` / `false_success_rate` 需先与 Wave 3 对齐 receipt schema。
- Wave 4 的 `StorageManifest` / kill-9 恢复是 Task 4 recovery manifest 与 Task 7 `crash_resume_success` 的前置；未就位时这两项标 `unverified` 并在 gate 中降为告警而非阻断。
- `hardware-edge` profile 的 hardware broker 属独立硬件轨道（roadmap §6），本 plan 只留 gate 位（Task 1 矩阵）与 manifest 占位，不实现硬件能力。
