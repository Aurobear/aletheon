# G5 可执行 Spec：Lifecycle Contributors 与 Hook 系统

> 对应研究文档 `../05-typed-lifecycle-extensions.md`。优先级 P1。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：分两条线补强 Aletheon 已有的生命周期扩展能力：
- **命令 hook 线（增强现有）**：Aletheon **已有两条可用 hook 路径**——corpus `execute_hook`（结构化，可 Block/ModifyInput/Inject）与 `run_hook_scripts`（config 驱动脚本，fire-and-forget）。本期：丰富脚本 hook 的 envelope（对齐 Grok），修正 `start` 只跑 corpus hook 而不跑脚本 hook 的不对称，扩展 `HookPoint` 覆盖至对齐 Grok 的事件集（session/turn/tool/subagent/compaction），接入 G1 信任门控（未信任 repo 的 hook 不跑）。
- **typed contributor 线（新增）**：在**进程内**加一个有序、类型化、返回声明式 effect 的 contributor 注册表，用于 authority/安全/记忆投影等不可外置的逻辑；contributor 永不拥有 loop control。

**非目标**：
- 不移除现有 `HookRegistry`/`Plugin` 机制（复用/扩展）。
- 不让 contributor 直接调 tool（只返回 effect，Executive 重新授权/执行）。
- 不一次实现所有 hook 点（先补已有调用点：session start/end、turn start/end、tool terminal）。

## 2. 当前代码锚点（已验证 @ commit bec15695）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| `SessionLifecycleUseCases` | `crates/executive/src/service/request_use_cases.rs:331-335` | `reset_turn_token`/`finish`/`start` |
| `ProductionSessionLifecycle::finish` | 同上 `:394-417` | 调 `execute_hook(OnSessionEnd)`(:405) **+** `run_hook_scripts(config.on_session_end)`(:411) |
| `ProductionSessionLifecycle::start` | 同上 `:419-434` | 只调 `execute_hook(OnSessionStart)`(:433)；**不跑脚本 hook**（与 finish 不对称） |
| `run_hook_scripts` | `crates/executive/src/impl/daemon/handler/mod.rs:17-52` | **已实现**：spawn 脚本、JSON stdin、stdout/stderr **丢弃**、30s timeout、fail-open。纯 fire-and-forget |
| envelope | `request_use_cases.rs:407-410` | 仅 `{ session_id, cwd }`——**比 Grok envelope 简陋**（无 hook_event_name/workspace_root/tool 信息） |
| Corpus `execute_hook` | `crates/corpus/src/service.rs:140-142` | `execute_hook(&HookContext) -> HookResult`（结构化，可 Block） |
| `HookRegistry` | `crates/corpus/src/hook/registry.rs:35,81-104` | `HashMap<HookPoint, Vec<RegisteredHook>>`；execute 聚合 Block>ModifyInput>Inject |
| hook 进程执行（corpus 侧） | 同上 `:125-142` | `tokio::process::Command` + 30s timeout + JSON stdin |
| `HooksConfig` | `crates/executive/src/core/config/agent.rs:64-79` | `{ pre_turn, post_tool, on_session_end, pre_tool }` |
| 配置分层加载 | `crates/executive/src/core/config/mod.rs:153-194` | system/user/project/env/CLI |
| turn 生命周期点 | `crates/executive/src/service/turn_pipeline.rs:189-213,215,417,638-647` | PreTurn hook、begin_user、tool terminal observe、finish |
| `CapabilityExecutionContext` | `crates/executive/src/service/governed_capability.rs:21-37` | 完整可信字段（principal/thread/turn/workspace/sandbox/cancel/session_id/... ） |
| `Plugin` trait | `crates/fabric/src/include/plugin.rs:31-62` | init/run/shutdown/additional_capabilities |
| `publish_event_v2` | `crates/fabric/src/ipc/bus/communication_bus.rs:164-179` | 事件发布 |
| `TurnEvent` | `crates/fabric/src/types/turn.rs:62-74` | Started/Finished/ToolCall |

**核心事实（已核实纠正）**：Aletheon 已有**两条可用 hook 路径**——corpus `execute_hook`（结构化，可 Block/ModifyInput/Inject）与 `run_hook_scripts`（脚本，fire-and-forget，已实现且被正确调用）。真实缺口是：(a) 脚本 envelope 简陋（仅 session_id+cwd）；(b) `start` 不跑脚本 hook（与 finish 不对称）；(c) HookPoint 覆盖窄（~6 点）；(d) 无 typed contributor 层；(e) 无信任门控。

> **勘误**：本 spec 早期草稿曾称 `run_hook_scripts` "未实现"——经核实该函数存在于 `handler/mod.rs:17-52` 并在 `request_use_cases.rs:411` 被正确调用。无此 bug。

## 3. 权威归属决策（doc10 §6 八问）

1. **owner**：Executive/UnifiedTurnCoordinator 是唯一 loop owner；Corpus 拥有命令 hook 执行；新 contributor 注册表在 Executive。
2. **scope**：contributor 无用户状态；hook 输入带只读 principal/thread/session 标识。
3. **crash 恢复**：hook/contributor 无持久态；失败不影响 turn 权威态。
4. **fail 模式**：observability contributor 失败降级；authority/security contributor 失败 **fail closed**；命令 hook 默认 fail-open（对齐现有 + Grok）；blocking hook（PreTool）失败按策略。
5. **上限**：contributor effect 有大小/数量上限；hook 输入 128KB 截断（对齐现有 payload 上限）；hook 超时 30s（已有）。
6. **兼容**：contributor 注册表 flag 后；命令 hook 只补缺口，现有行为不变。
7. **进 event spine**：新 hook 点触发 + contributor effect 应用经 `publish_event_v2`。
8. **许可证**：hook 事件命名/语义参考 Grok 但重新实现，不复制源码。

## 4. 类型定义

### 4.1 扩展 HookPoint — `crates/corpus/src/hook/`（对齐 Grok 事件集）

```rust
/// 扩展后的 hook 点。前 6 个已存在；其余为新增（对齐 Grok，映射 Aletheon 域）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPoint {
    // 已存在
    OnSessionStart,
    OnSessionEnd,
    PreTurn,
    PostTurn,
    PreTool,      // blocking：可 Block（deny）
    PostTool,
    // 新增
    PostToolFailure,
    PermissionDenied,
    UserPromptSubmit,
    Notification,
    SubagentStart,
    SubagentStop,
    PreCompact,
    PostCompact,
}

impl HookPoint {
    /// 仅 PreTool 是 blocking（对齐 Grok：一个门，其余 fire-and-forget）。
    pub fn is_blocking(&self) -> bool { matches!(self, Self::PreTool) }
}
```

### 4.2 命令 hook script runner（补缺口） — `crates/executive/src/service/hook_scripts.rs`（新文件）

```rust
use std::path::PathBuf;

/// 增强现有 run_hook_scripts（handler/mod.rs:17-52）：承载更丰富的 envelope
/// 构造与信任门控。复用现有进程执行模式（tokio::process::Command + 30s + JSON stdin）。
/// 信任门控：未信任 workspace（G1 决策）的 repo-local hook 不执行。
pub struct HookScriptRunner {
    /// G1 trust decision 查询（flag 关时恒 trusted）。
    trust: std::sync::Arc<dyn crate::service::workspace_trust::TrustQuery>,
}

impl HookScriptRunner {
    /// 逐条执行 script 路径；非 blocking 点 fail-open；blocking 点（PreTool）
    /// 返回 aggregate 决策。envelope 为 JSON（带 hook_event_name/session_id/cwd/...，
    /// 对齐 Grok HookEventEnvelope），payload 超 128KB UTF-8 安全截断。
    pub async fn run(
        &self,
        point: HookPoint,
        scripts: &[String],
        envelope: serde_json::Value,
        workspace_trusted: bool,
    ) -> HookResult {
        // repo-local 且未信任 → 跳过并记事件（restricted）。
        // 逐条 spawn，聚合 Block>ModifyInput>Inject>Continue（复用 registry 语义）。
        unimplemented!()
    }
}
```

### 4.3 Typed contributor 注册表 — `crates/executive/src/service/lifecycle_contributors.rs`（新文件）

```rust
use async_trait::async_trait;

/// contributor 阶段（与 HookPoint 对应，但进程内、类型化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LifecyclePhase {
    BeforeSessionStart, AfterSessionStart, BeforeSessionEnd, AfterSessionEnd,
    BeforeTurnInput, AfterContextProjection, BeforeModelCall, BeforeToolBatch,
    AfterToolTerminal, AfterTurnTerminal, OnAbort,
}

/// 不可变的 dispatch 输入（只读可信标识，不含可伪造 approval authority）。
#[derive(Debug, Clone)]
pub struct LifecycleInput {
    pub phase: LifecyclePhase,
    pub principal_id: fabric::PrincipalId,
    pub thread_id: fabric::ThreadId,
    pub turn_id: Option<fabric::TurnId>,
    pub session_id: String,
    /// 阶段相关的只读快照（如 tool terminal 的 call_id/name）。
    pub detail: serde_json::Value,
}

/// contributor 只能返回声明式 effect，不推进 turn、不调 tool。
#[derive(Debug, Clone)]
pub enum LifecycleEffect {
    Continue,
    /// 追加上下文片段（有字节上限，带来源）。
    AddContextFragment { source: String, content: String },
    EmitEvent { schema: String, payload: serde_json::Value },
    RequestCheckpoint,
    RequestCancellation { reason: String },
    /// 仅 blocking 阶段有效（如 BeforeToolBatch）。
    RejectInput { reason: String },
}

#[async_trait]
pub trait LifecycleContributor: Send + Sync {
    fn id(&self) -> &str;
    /// 稳定顺序：阶段 + priority + id。
    fn priority(&self) -> i32 { 0 }
    /// 是否 security-critical：失败时 fail closed（否则降级）。
    fn is_critical(&self) -> bool { false }
    async fn on_lifecycle(&self, input: &LifecycleInput) -> Vec<LifecycleEffect>;
}

/// 注册表：安装时注入 capability，dispatch 时只给 data-only input。
pub struct LifecycleRegistry {
    /// 按 phase 分桶，桶内按 (priority, id) 排序。
    contributors: std::collections::BTreeMap<LifecyclePhase, Vec<std::sync::Arc<dyn LifecycleContributor>>>,
}

impl LifecycleRegistry {
    /// 注册。重复 id 报错。
    pub fn register(&mut self, phase: LifecyclePhase, c: std::sync::Arc<dyn LifecycleContributor>)
        -> Result<(), String> { unimplemented!() }
    /// 有序 dispatch；收集 bounded effects；记录每个 contributor 耗时与 outcome。
    /// critical 失败 → 传播 fail-closed；非 critical 失败 → 记录并继续。
    pub async fn dispatch(&self, input: LifecycleInput) -> Vec<LifecycleEffect> { unimplemented!() }
}

/// effect 上限。
pub const MAX_EFFECTS_PER_DISPATCH: usize = 32;
pub const MAX_CONTEXT_FRAGMENT_BYTES: usize = 8 * 1024;
```

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 修改 | `crates/corpus/src/hook/` HookPoint 定义 | 扩展事件集 + `is_blocking` |
| 修改 | `crates/corpus/src/hook/registry.rs` | 新 HookPoint 的注册/匹配；envelope 对齐（hook_event_name 等） |
| 修改/新增 | `crates/executive/src/impl/daemon/handler/mod.rs:17-52` 或新 `hook_scripts.rs` | 把现有 `run_hook_scripts` 抽为 `HookScriptRunner`：丰富 envelope + 信任门控入口 |
| 修改 | `crates/executive/src/service/request_use_cases.rs:411,433` | finish 调增强后的 runner；start 补跑脚本 hook（修不对称） |
| 新增 | `crates/executive/src/service/lifecycle_contributors.rs` | typed contributor 注册表 |
| 修改 | `crates/executive/src/service/turn_pipeline.rs` | 在已有生命周期点 dispatch contributor + 解释 effect |
| 修改 | `crates/executive/src/core/config/agent.rs:64-79` | HooksConfig 补新点（subagent/compact 等）字段 |
| 修改 | feature flag | `grok_hardening.lifecycle_contributors` 默认关；脚本 hook 增强（envelope/对称化）无 flag（向后兼容增强） |

## 6. 任务分解（TDD）

**阶段 A：增强现有脚本 hook（无 flag，行为增强）**
- T1. envelope 对齐：把 `run_hook_scripts` 的 `{session_id, cwd}`（request_use_cases.rs:407-410）扩为对齐 Grok 的 envelope（加 `hook_event_name`/`workspace_root`/`timestamp`/`tool_*`）；payload 128KB UTF-8 截断。单测。
- T2. 修正 `start`/`finish` 不对称：`start`（:419-434）也跑 `config.on_session_start` 脚本 hook（当前只跑 corpus hook）。集成测试：session start 触发配置的 script。
- T3. 把 `run_hook_scripts` 抽为 `HookScriptRunner`（承载 envelope 构造 + 后续信任门控入口），保持现有 fire-and-forget 语义。回归测试：现有 on_session_end 脚本行为不变。

**阶段 B：扩展 HookPoint**
- T4. HookPoint 加新变体 + `is_blocking`。`cargo check -p corpus`。
- T5. registry 支持新点注册/匹配；只 PreTool blocking。单测。
- T6. 在 turn_pipeline 已有点补发新 hook（PostToolFailure、SubagentStart/Stop 等挂到对应位置）。集成测试。

**阶段 C：信任门控（依赖 G1；G1 未落地则 stub trusted）**
- T7. `HookScriptRunner` 查询 G1 `TrustQuery`：repo-local hook 且未信任 → 跳过 + 记 restricted 事件。单测（mock trust）。

**阶段 D：typed contributor（flag 后）**
- T8. `lifecycle_contributors.rs` 类型 + 注册表。`cargo check -p executive`。
- T9. `register` 重复 id 报错；`dispatch` 稳定顺序（priority+id）。单测。
- T10. effect 上限 + context fragment 字节上限。单测。
- T11. critical 失败 fail-closed、非 critical 失败降级并可观察。单测（两个 mock contributor）。
- T12. contributor 无法自行推进/重入 turn loop（类型上只返回 effect，无 loop handle）。编译期保证 + 单测断言。

**阶段 E：turn_pipeline 集成（flag 后）**
- T13. 在 session start/end、turn start/end、tool terminal 点 dispatch contributor；Executive 解释 effect（AddContextFragment 注入、RequestCancellation 走已有 cancel、EmitEvent 走 publish_event_v2）。flag 关 → 不 dispatch（等价当前）。集成测试。
- T14. RejectInput 仅 blocking 阶段生效；非 blocking 阶段返回 RejectInput 记 warn 并忽略。单测。

**阶段 F：收尾**
- T15. clippy/fmt；更新 §2 漂移；标注 flag 灰度。

## 7. 兼容与迁移

- **脚本 hook 增强无 flag**：envelope 丰富、start/finish 对称化是对已工作路径的增强，向后兼容（现有 on_session_end 脚本行为不变），无需 flag。
- **contributor flag 关闭**：不 dispatch，turn 行为等价当前。
- **两条线并存**：命令 hook（外部、可不信任、只 PreTool 阻塞）用于用户集成；typed contributor（进程内、可信、可 fail-closed）用于 authority/安全/记忆。选择规则见研究文档 ../05 §8.4。
- **HookPoint 扩展向后兼容**：现有 6 点行为不变，新点默认无注册 = no-op。

## 8. 测试计划（映射研究文档 ../05 §7 验收方向）

| 验收方向 | 测试 |
|---|---|
| contributor 无法自推进/重入 turn loop | T12 |
| 固定输入产生固定 contributor 顺序 | T9 |
| non-critical 故障隔离可观察 | T11 |
| security contributor 故障 fail closed | T11 |
| context fragment 有字节上限带来源 | T10 |
| session/turn/tool terminal 现有行为迁移后不变 | T13 flag-off 回归 |
| （补充）start/finish 脚本 hook 对称执行 | T2 |
| （补充）未信任 repo hook 不跑 | T7 |

## 9. 可观测性

- 事件：新 HookPoint 触发、contributor effect 应用（AddContextFragment 来源、RejectInput 原因）、hook restricted（未信任跳过）。
- 指标：`hook_script_failed_total{point}`、`contributor_dispatch_ms{phase,id}`、`contributor_critical_failclosed_total`、`hook_restricted_untrusted_total`。
- 日志：慢 contributor（超阈值）warn；重复 id 注册 error。

## 10. 许可证

hook 事件命名与 blocking 语义参考 Grok `xai-grok-hooks`/`xai-agent-lifecycle` 概念，重新实现，不复制源码。typed contributor 的 effect 模型是 Aletheon 设计。无 NOTICE 变更。
