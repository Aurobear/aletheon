# S1 可执行 Spec：Sandbox Profile 层

> 对应研究文档 `../11-sandbox-enforcement.md`。优先级 P2。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：在 Aletheon **已有的命令级 sandbox**（`SandboxExecutor`/`SandboxBackend`）之上，增加 **profile 层**——把命名 profile（workspace/read-only/strict/custom）解析为 deny 路径 + 网络限制 + 读写根，喂给现有 `SandboxConfig` 和 backend 选择；支持分层配置（daemon 全局 > 项目附加，项目不可覆盖全局同名 profile）；deny 集合 fail-closed。

**非目标**：
- 不替换现有 `SandboxBackend`/`SandboxExecutor`（它们已解决"如何隔离执行一条命令"）。
- 本期不做 Grok 的 bwrap 进程级 re-exec（daemon 不能自我 re-exec；deny 通过 backend 的 namespace/mount 能力或 Landlock 实现）。
- 不做 Grok 的 `/data` devbox 约定（Aletheon 无此惯例）。
- profile 配置**不**来自不可信 repo 的 `.grok/`；只来自 daemon 可信配置（与 G1 Folder Trust 区分：G1 门控 repo 配置加载，S1 约束进程能力）。

## 2. 当前代码锚点（已验证 @ commit bec15695）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| `SandboxBackend` trait | `crates/fabric/src/types/sandbox.rs:80-112` | `execute(cmd, config, timeout)`、`wrap_argv`、`capabilities()`、`isolation_level()` |
| `IsolationLevel` | 同上 `:11-21` | None/Process/Namespace/Container |
| `SandboxConfig` | 同上 `:24-37` | `{ workspace: WorkspacePolicy, environment: BTreeMap }`——**当前无 deny/network 字段** |
| `SandboxCapabilities` | 同上 `:40-52` | filesystem_isolation/network_isolation/resource_limits/seccomp_filter |
| `SandboxExecutor` | 同上 `:145-227` | 按 `SandboxPreference` 选 backend；Require→noop 时 fail-closed（`:205-209`） |
| `SandboxPreference` | 同上 `:115-138` | Auto/Require/Forbid/BestEffort |
| `SandboxRequirement` | `crates/fabric/src/types/admission.rs:77-84` | NotRequired/Required/RequiredThenPromote |
| `SandboxDecision` | 同上 `:88-94` | NotApplicable/Required/Passed/Failed/Unavailable |
| `CapabilityExecutionContext.sandbox` | `crates/executive/src/service/governed_capability.rs:33` | 已注入 `SandboxRequirement` |
| `WorkspacePolicy` | `crates/fabric/src/types/local_authority.rs:71-76` | cwd/writable_roots/protected_paths |
| `ProtectedPathPolicy` | 同上 `:125-153` | credential_paths 只读 |

**核心缺口**：`SandboxConfig` 只携带 workspace + env，没有 profile 解析出的 deny 路径与网络策略；backend 拿不到"禁止读 `**/.env`"这类信息。

## 3. 权威归属决策（doc10 §6 八问）

1. **owner**：Fabric 定义 `SandboxProfile`/`ResolvedSandboxPolicy` 类型 + profile 解析纯函数；Executive/daemon 拥有 profile 配置装配；backend 消费解析后的 policy。
2. **scope**：profile 选择按 principal + workspace；child Agent 只能收窄。
3. **crash 恢复**：profile 无持久运行态；每次执行按当前配置重新解析。
4. **fail 模式**：deny glob 展开超上限 / profile 未找到 → fail closed（拒绝执行）；backend 不支持所需隔离且 `Require` → 已有 fail-closed（`:205-209`）。
5. **上限**：deny glob 展开有 max depth/matches/entries（对齐 Grok）。
6. **兼容**：flag 关闭 → profile 解析返回空 policy，`SandboxConfig` 行为等价当前。
7. **进 event spine**：profile applied / violation / deny 经 `publish_event_v2` 发布。
8. **许可证**：重新实现 profile 解析语义，不复制 Grok `xai-grok-sandbox` 源码。

## 4. 类型定义

### 4.1 扩展 Fabric — `crates/fabric/src/types/sandbox.rs`（追加）

```rust
use std::collections::BTreeMap;

/// 命名 sandbox profile 配置（来自 daemon 可信配置，非 repo）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SandboxProfileConfig {
    /// 继承的内建基线（"workspace"|"read-only"|"strict"）。custom 不可继承 custom。
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub restrict_network: Option<bool>,
    /// 额外只读根（绝对路径）。
    #[serde(default)]
    pub read_only: Vec<String>,
    /// 额外读写根（绝对路径）。
    #[serde(default)]
    pub read_write: Vec<String>,
    /// deny 条目：精确路径或 glob（`**/*.pem`）。读+写都禁止。
    #[serde(default)]
    pub deny: Vec<String>,
}

/// 分层 sandbox 配置。全局先加载，项目**附加**（不可覆盖全局同名）。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SandboxProfiles {
    #[serde(default)]
    pub profiles: BTreeMap<String, SandboxProfileConfig>,
}

impl SandboxProfiles {
    /// 项目 profiles 附加合并：仅新增名字，已存在名字保留全局定义（防 hollowing）。
    pub fn merge_project_additive(&mut self, project: SandboxProfiles) {
        for (name, cfg) in project.profiles {
            self.profiles.entry(name).or_insert(cfg);
        }
    }
}

/// 内建 profile 名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileName {
    Workspace,
    ReadOnly,
    Strict,
    Off,
    Custom(String),
}

/// 解析后的执行策略。喂给 backend；backend 按能力施加。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSandboxPolicy {
    pub name: String,
    pub read_only_roots: Vec<PathBuf>,
    pub read_write_roots: Vec<PathBuf>,
    /// 精确 deny 路径（已 canonicalize）。
    pub deny_exact: Vec<PathBuf>,
    /// deny glob（backend 侧尽力施加；展开在解析时有上限）。
    pub deny_globs: Vec<String>,
    pub restrict_network: bool,
}

/// deny glob 展开上限（对齐 Grok，fail-closed）。
pub const DENY_GLOB_MAX_DEPTH: usize = 8;
pub const DENY_GLOB_MAX_MATCHES: usize = 4096;
pub const DENY_GLOB_MAX_ENTRIES: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum ProfileResolveError {
    #[error("custom profile '{0}' not found")]
    NotFound(String),
    #[error("custom profile cannot extend another custom profile")]
    ExtendsCustom,
    #[error("'off' is not a valid base profile")]
    ExtendsOff,
    #[error("deny glob expansion exceeded caps (fail-closed)")]
    GlobOverflow,
}
```

### 4.2 profile 解析纯函数 — 同文件

```rust
/// 把 ProfileName + 配置解析为 ResolvedSandboxPolicy。纯函数（glob 展开除外，
/// 需读 FS——拆出 expand_globs 单独测）。protected_paths 恒并入 deny_exact。
pub fn resolve_profile(
    name: &ProfileName,
    workspace: &WorkspacePolicy,
    profiles: &SandboxProfiles,
) -> Result<ResolvedSandboxPolicy, ProfileResolveError> {
    // Workspace: 默认读全盘，写 writable_roots，无 deny（credential 仍由 protected 并入）。
    // ReadOnly:  写最小集，restrict_network=true。
    // Strict:    读白名单（系统 + workspace），写最小集，restrict_network=true。
    // Custom:    从 extends 基线起，叠加 read_only/read_write/deny/network 覆盖。
    // 所有分支末尾：把 workspace.protected_paths().credential_paths() 并入 deny_exact。
    // ... 实现见任务 T2-T6 ...
    unimplemented!()
}
```

### 4.3 扩展 `SandboxConfig` — 同文件 `:24-37`

```rust
pub struct SandboxConfig {
    pub workspace: crate::WorkspacePolicy,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// 新增：解析后的策略。None = flag 关或无 profile（等价当前行为）。
    #[serde(skip)]
    pub policy: Option<ResolvedSandboxPolicy>,
}
```

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 修改 | `crates/fabric/src/types/sandbox.rs` | 追加 profile 类型 + `resolve_profile` + `SandboxConfig.policy` |
| 修改 | `crates/fabric/src/lib.rs:188` | 导出新类型 |
| 新增 | `crates/fabric/src/types/sandbox_glob.rs` | glob 展开（有上限，fail-closed） |
| 修改 | 各 `SandboxBackend` 实现（bubblewrap/namespace） | 消费 `config.policy`：deny→mount/Landlock，network→隔离 |
| 新增 | daemon profile 配置加载 | 读全局 + 项目附加，构造 `SandboxProfiles` |
| 修改 | Executive sandbox 装配点 | 执行前 `resolve_profile` 填 `SandboxConfig.policy` |
| 修改 | feature flag | `grok_hardening.sandbox_profiles` 默认关 |

## 6. 任务分解（TDD）

**阶段 A：类型 + 附加合并**
- T1. 追加 profile 类型。`cargo check -p fabric`。单测：`merge_project_additive` 全局同名保留、项目新名加入。

**阶段 B：profile 解析**
- T2. `resolve_profile(Workspace)`：读全盘/写 writable_roots/protected 并入 deny。单测。
- T3. `resolve_profile(ReadOnly)`：restrict_network=true，写最小集。单测。
- T4. `resolve_profile(Strict)`：读白名单 + workspace。单测。
- T5. `resolve_profile(Custom extends workspace)`：叠加覆盖。单测。
- T6. 错误分支：Custom not found / extends custom / extends off → 对应 Err。单测。
- T7. **credential 恒 deny**：任意 profile 解析后 `deny_exact` 含 `protected_paths` 的 credential_paths。单测。

**阶段 C：glob 展开（fail-closed）**
- T8. `sandbox_glob.rs`：展开 glob 到现存匹配，超 depth/matches/entries → `Err(GlobOverflow)`。单测含超限用例。

**阶段 D：backend 消费**
- T9. 扩展 `SandboxConfig.policy`（默认 None）。`cargo check`。确认现有构造点默认 None（等价当前）。
- T10. 选一个 backend（bubblewrap/namespace）消费 `policy.deny_exact`（bind-over 或 Landlock）+ `restrict_network`。集成测试：deny 路径读失败，非 deny 路径可读。
- T11. `SandboxCapabilities` 与 policy 一致性：backend 不支持 network 隔离但 policy `restrict_network` 且 `Require` → fail-closed（复用 `:205-209` 模式）。

**阶段 E：装配 + 事件（flag 后）**
- T12. daemon 加载 profile 配置（全局 + 项目附加）。
- T13. Executive 执行前 `resolve_profile` 填 policy；flag 关 → policy=None（回归等价当前）。
- T14. profile applied / deny violation 事件经 `publish_event_v2`。事件断言测试。

**阶段 F：收尾**
- T15. clippy/fmt；更新 §2 漂移；标注 flag 灰度。

## 7. 兼容与迁移

- **flag 关闭 / policy=None**：`SandboxConfig` 行为完全等价当前（现有 backend 忽略 None）。
- **现有 executor/backend 不动**：只在 config 增字段、在 backend 增消费分支。
- **与 G1 区分**：profile 配置来自 daemon 可信源，绝不来自未信任 repo；G1 管 repo 配置加载，S1 管进程能力。
- **与 admission 的 `SandboxRequirement` 关系**：Requirement 决定"是否必须沙箱"，profile 决定"沙箱里能做什么"，正交。

## 8. 测试计划（映射研究文档 ../11 §6 验收方向）

| 验收方向 | 测试 |
|---|---|
| strict 下不能读白名单外路径 | T4, T10 |
| deny glob 阻止匹配路径 | T8, T10 |
| daemon profile 不被 repo 覆盖 | T1 |
| violation 事件带 principal/agent 归属 | T14 |
| child Agent 沙箱 ≤ parent | 装配层单测（child 解析继承 parent profile 名，只收窄） |
| 平台不支持隔离 → 降级告警不静默全放开 | T11（复用 BestEffort warn / Require fail-closed） |

## 9. 可观测性

- 事件：`sandbox.profile.applied`（profile 名、read/write/deny/network）、`sandbox.violation`（target、operation、principal）。
- 指标：`sandbox_fs_violation_total`、`sandbox_glob_overflow_total`（fail-closed 计数）。

## 10. 许可证

重新实现 profile 解析与分层合并语义，不复制 Grok `xai-grok-sandbox` 源码。glob 上限常量为独立选值。无 NOTICE 变更。
