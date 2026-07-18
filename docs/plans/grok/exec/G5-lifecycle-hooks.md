# G5 可执行 Spec：Lifecycle Contributors 与 Hook 系统

> 对应研究文档 `../05-typed-lifecycle-extensions.md`。优先级 P1。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：分两条线补强 Aletheon 已有的生命周期扩展能力：
- **命令 hook 线（增强现有）**：保留 **Corpus `HookRegistry`/`execute_hook` 单一权威路径**。配置脚本在 bootstrap 注册进同一 registry，不恢复 Executive 自有 subprocess runner。本期：丰富 Corpus hook envelope（对齐 Grok），补齐配置的 session start/end 对称注册，扩展 `HookPoint` 覆盖至 session/turn/tool/subagent/compaction，并接入 G1 信任门控（未信任 repo 的 hook 不跑）。
- **typed contributor 线（新增）**：在**进程内**加一个有序、类型化、返回声明式 effect 的 contributor 注册表，用于 authority/安全/记忆投影等不可外置的逻辑；contributor 永不拥有 loop control。

**非目标**：
- 不移除现有 `HookRegistry`/`Plugin` 机制（复用/扩展）。
- 不让 contributor 直接调 tool（只返回 effect，Executive 重新授权/执行）。
- 不一次实现所有 hook 点（先补已有调用点：session start/end、turn start/end、tool terminal）。

## 2. 当前代码锚点（重新验证 @ 2026-07-18，含用户裁定）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| `SessionLifecycleUseCases` | `crates/executive/src/service/request_use_cases.rs:368-435` | start/end 均调用 Corpus `execute_hook` |
| 配置脚本注册 | `crates/executive/src/impl/daemon/bootstrap/turn_runtime.rs:21-46` | `HooksConfig` 脚本统一注册进权威 Corpus registry；当前缺 `on_session_start` |
| Corpus `execute_hook` | `crates/corpus/src/service.rs:376-378` | 唯一命令 hook 执行入口，返回结构化 `HookResult` |
| `HookRegistry` | `crates/corpus/src/hook/registry.rs:19-171` | priority 有序执行、30s timeout、聚合 Block/ModifyInput/Inject |
| hook envelope | `crates/fabric/src/types/hook.rs:32-51` | 当前直接序列化 `HookContext`；缺稳定 event name/timestamp/workspace root，且未做 128KB 上限 |
| `HooksConfig` | `crates/executive/src/core/config/agent.rs:79-92` | `{ pre_turn, post_tool, on_session_end, pre_tool }` |
| 配置分层加载 | `crates/executive/src/core/config/mod.rs:153-194` | system/user/project/env/CLI |
| turn 生命周期点 | `crates/executive/src/service/turn_pipeline.rs:189-213,215,417,638-647` | PreTurn hook、begin_user、tool terminal observe、finish |
| `CapabilityExecutionContext` | `crates/executive/src/service/governed_capability.rs:21-37` | 完整可信字段（principal/thread/turn/workspace/sandbox/cancel/session_id/... ） |
| `Plugin` trait | `crates/fabric/src/include/plugin.rs:31-62` | init/run/shutdown/additional_capabilities |
| `publish_event_v2` | `crates/fabric/src/ipc/bus/communication_bus.rs:164-179` | 事件发布 |
| `TurnEvent` | `crates/fabric/src/types/turn.rs:62-74` | Started/Finished/ToolCall |

**用户裁定（2026-07-18）**：保留单一 Corpus hook 权威路径。`run_hook_scripts` 已由 `42888eb8` 删除，配置脚本已迁入 Corpus registry；G5 不恢复第二条执行路径。

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

### 4.2 Corpus 命令 hook runner（增强单一权威路径） — `crates/corpus/src/hook/registry.rs`

```rust
use std::path::PathBuf;

/// Registry 在 spawn 前构造稳定 envelope（hook_event_name/session_id/
/// workspace_root/timestamp/tool_*），以 UTF-8 边界截断至 128KB。
/// config 来源是 host-configured；其它位于 workspace_root 内的脚本只有
/// `repo_hooks_trusted=true` 时执行，否则跳过并记录 restricted。
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
| 修改 | `crates/corpus/src/hook/registry.rs` | 在单一 runner 内丰富 envelope、限制 payload、执行信任门控 |
| 修改 | `crates/executive/src/core/config/agent.rs` + `impl/daemon/bootstrap/turn_runtime.rs` | 增加 `on_session_start` 并与 end 一样注册进 Corpus |
| 新增 | `crates/executive/src/service/lifecycle_contributors.rs` | typed contributor 注册表 |
| 修改 | `crates/executive/src/service/turn_pipeline.rs` | 在已有生命周期点 dispatch contributor + 解释 effect |
| 修改 | `crates/executive/src/core/config/agent.rs:64-79` | HooksConfig 补新点（subagent/compact 等）字段 |
| 修改 | feature flag | `grok_hardening.lifecycle_contributors` 默认关；脚本 hook 增强（envelope/对称化）无 flag（向后兼容增强） |

## 6. 任务分解（TDD）

**阶段 A：增强现有脚本 hook（无 flag，行为增强）**
- T1. Corpus envelope 对齐：加 `hook_event_name`/`workspace_root`/`timestamp`/`tool_*`；payload 128KB UTF-8 截断。单测。
- T2. 配置注册对称：增加 `config.on_session_start`，与 `on_session_end` 一样注册进 Corpus。集成测试。
- T3. 回归证明配置脚本仅经 Corpus registry 执行，不恢复 Executive subprocess runner。

**阶段 B：扩展 HookPoint**
- T4. HookPoint 加新变体 + `is_blocking`。`cargo check -p corpus`。
- T5. registry 支持新点注册/匹配；只 PreTool blocking。单测。
- T6. 在 turn_pipeline 已有点补发新 hook（PostToolFailure、SubagentStart/Stop 等挂到对应位置）。集成测试。

**阶段 C：信任门控（依赖 G1；G1 未落地则 stub trusted）**
- T7. Corpus registry 使用 host 注入的 trust metadata：非 config 的 repo-local hook 且未信任 → 跳过 + 记 restricted 事件。单测。

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

- **脚本 hook 增强无 flag**：envelope 丰富、start/end 配置注册对称化是单一 Corpus 路径的向后兼容增强，无需 flag。
- **contributor flag 关闭**：不 dispatch，turn 行为等价当前。
- **两类扩展并存但不形成双 runner**：Corpus 命令 hook（外部）用于用户集成；typed contributor（进程内）用于 authority/安全/记忆。
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
