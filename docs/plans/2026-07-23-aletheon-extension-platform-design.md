# Aletheon 原生扩展平台重构设计与实施规格

**日期：** 2026-07-23
**状态：** 部分实现，审查未通过，等待修复阶段 R0–R6
**受众：** 实现者、审查者、发布验收人员
**范围：** 原生扩展领域模型、包管理、激活事务、隔离运行、CLI、兼容迁移与真实部署验收

## 1. 目标

Aletheon 必须在不依赖任何外部资产仓库、外部 Agent 产品或特定模型供应商的情况下，原生支持：

- Skill；
- Hook；
- Agent Profile；
- Agent Runtime；
- MCP Connector；
- 自定义可执行扩展；
- 第三方扩展包的检查、安装、启用、升级、回滚和删除。

系统必须使用通用接口与外部实现通信。外部实现的产品名、私有参数和协议细节只能存在于具体适配器和部署配置中，不得进入公共 ABI、Agent Profile、通用 Manifest 或核心状态机。

最终交付必须通过实际安装后的 `aletheon` 二进制、用户 daemon 和 TUI 验收，不能只以源码测试或仓库内临时二进制作为完成证据。

## 2. 非目标

本阶段不实现：

- 模型供应商配置向导；
- `setup.sh` 的交互式模型配置；
- 在线扩展市场；
- 自动下载不可信远程代码；
- 进程内加载第三方动态库；
- 通用 WASM 运行时；
- 扩展自动获得系统权限；
- Metacog 自动修改扩展代码。

这些能力不得作为“顺便实现”扩大本阶段范围。

## 3. 当前代码事实

### 3.1 已有能力

| 当前事实 | 精确定位 |
|---|---|
| 公共扩展身份格式为 `<kind>:<local-name>` | `crates/fabric/src/types/extension.rs:24-44` |
| 当前 `ExtensionKind` 混合 Tool、Skill、Hook、Plugin、MCP | `crates/fabric/src/types/extension.rs:46-65` |
| `ExtensionDescriptor` 同时包含资产元数据和可选 Tool 定义 | `crates/fabric/src/types/extension.rs:87-99` |
| 当前公共 Catalog 只有只读快照 | `crates/fabric/src/types/extension.rs:166-174` |
| Corpus Catalog 同时索引资产并判断可执行 Capability 冲突 | `crates/corpus/src/catalog/mod.rs:13-60` |
| Skill Manifest 已可声明 Tool 和 Hook | `crates/corpus/src/skill/manifest.rs:11-54` |
| Agent Profile 加载后在启动期解析其 Tool 授权 | `crates/executive/src/host/daemon/bootstrap/runtime.rs:24-73` |
| 未知 Tool 当前通过 `?` 传播，使整个 Profile 装载过程失败 | `crates/executive/src/host/daemon/bootstrap/runtime.rs:64-72` |
| Tool 调用仍以 `ExtensionKind::Tool` 生成身份 | `crates/corpus/src/service.rs:400-427`、`:438-467` |
| Runtime 已有基于 `RuntimeId` 的通用路由基础 | `crates/fabric/src/types/agent_control.rs`、`crates/executive/src/adapters/runtime/` |

### 3.2 旧文档与代码现实差异

| 旧文档描述 | 当前代码现实 | 一致？ |
|---|---|---|
| `Plugin` 是“历史组合资产” | 代码只定义 `Plugin` 枚举值，没有“历史组合资产”语义 | 否 |
| 新增六类 Manifest 即可形成统一平台 | 当前 Skill、Hook、Profile、Runtime 的加载和权威来源彼此独立 | 否 |
| `warn + skip` 即可修复无效 Profile | 当前装载函数返回整体 `Result`；还需要隔离状态、诊断和旧快照保留 | 否 |
| `ProcessPlugin` 应是公共扩展类型 | 进程只是执行与隔离方式，不是资产语义 | 否 |
| Tool 应从扩展模型中直接删除 | Tool 身份仍处于调用热路径和持久化兼容面 | 否 |

本规格以当前代码事实为迁移起点，不沿用上述错误假设。

### 3.3 提交 `3da4519` 后的实现审计

提交 `3da4519` 声称 Phase 0–7 全部完成，但代码审查表明该声明与实际实现
不一致。后续工作必须以本节为当前实现基线，不能继续把类型定义、未接线
模块或 CLI 打印输出视为完成。

| 阶段 | 当前实现现实 | 结论 |
|---|---|---|
| Phase 0 | Fabric 14 项契约测试、Corpus 7 项 Catalog 测试及 24 项扩展单元测试通过；真实 daemon/TUI 失败链路收据缺失 | 部分完成 |
| Phase 1 | Package/Asset/Runtime/Capability 基础类型和旧类型投影已加入 | 基础完成，仍需使用方接线 |
| Phase 2 | Inspector、Manifest、Store 类存在；安装应用服务、事务恢复和安全 entry-type 校验缺失 | 未完成 |
| Phase 3 | 未知 Tool 的 Profile 可进入内存 quarantine；默认 Profile 无效、previous-known-good、readiness/TUI 诊断仍缺失 | 未完成 |
| Phase 4 | Provider trait 已声明；Runtime 生命周期和 Catalog/权限接线不完整 | 未完成 |
| Phase 5 | 子进程类可启动和单行调用；沙箱、取消、stderr、协议校验、daemon 接线缺失 | 未完成 |
| Phase 6 | 只有 `inspect/validate` 执行真实逻辑，其他命令只打印文本 | 未实现 |
| Phase 7 | 没有安装哈希、真实 TUI、升级/回滚、故障注入和清理收据 | 未实现 |

已验证的问题证据：

- `install` 仍明确打印“后续阶段”提示：
  `crates/aletheon/src/main.rs:244-246`。
- `enable/disable/upgrade/rollback/remove/purge/doctor/import-legacy`
  只打印参数并返回成功：`crates/aletheon/src/main.rs:253-276`。
- `PackageStore` 没有生产调用者，只有自身单元测试：
  `crates/corpus/src/extension/store.rs:9-135`。
- staging 路径使用前 16 个哈希字符并对输入直接切片：
  `crates/corpus/src/extension/store.rs:85-88`。
- PID 锁采用非原子的“检查后写入”：
  `crates/corpus/src/extension/store.rs:44-71`。
- Inspector 仅区分目录与非目录，没有拒绝 symlink、hardlink、device 和 FIFO：
  `crates/corpus/src/extension/inspector.rs:61-87`、`:153-173`。
- 默认 Profile 被 quarantine 但仍存在其他有效 Profile 时，活动 Profile 解析
  仍可能失败：`crates/executive/src/host/daemon/bootstrap/agents.rs:76-82`。
- quarantine 列表被保存在组合对象中但没有进入 readiness、TUI 或持久状态：
  `crates/executive/src/host/daemon/bootstrap/agents.rs:21-25`、`:83-88`。
- `AgentRuntimeProvider` 只有 launch/health，缺少完整会话生命周期：
  `crates/fabric/src/include/extension_provider.rs:39-47`。
- 子进程的取消 token、stderr buffer 和 response ID 未参与执行：
  `crates/executive/src/extensions/runtime/subprocess.rs:13`、`:30-38`、`:51-75`。

### 3.4 完成声明规则

从本次审计开始：

1. 新文件或 Trait 存在不代表对应 Phase 完成。
2. CLI 子命令解析成功不代表产品能力完成。
3. 仅在单元测试中调用的组件视为未接线。
4. Phase 完成必须同时具备生产调用路径、负向测试和阶段收据。
5. Phase 7 必须引用安装二进制和真实 TUI 的可审计证据。
6. 未通过前置阶段门禁不得跳阶段，也不得在提交标题中宣称后续阶段完成。

## 4. 核心原则

1. **分层建模：** Package、Asset、Runtime、Capability 是四个不同概念。
2. **公共接口通用：** 核心不出现外部产品名或供应商私有字段。
3. **声明与执行分离：** Asset 描述“是什么”，Runtime 描述“如何运行”，Capability 描述“提供什么”。
4. **发现与激活分离：** 被发现不等于获准执行。
5. **失败隔离：** 单个扩展或 Profile 无效不能使 daemon 整体不可用。
6. **最小权限：** 权限从 Manifest 声明、策略审批和运行时沙箱三处共同约束。
7. **事务激活：** 新版本验证成功前不得替换旧有效版本。
8. **真实验收：** 必须测试安装二进制和真实客户端链路。
9. **兼容优先：** 旧身份与历史状态先只读兼容，满足删除条件后再移除。
10. **可观测：** 所有安装、验证、激活、隔离、失败和回滚均产生结构化证据。

## 5. 四层领域模型

```text
Distribution layer
  ExtensionPackage / PackageSource / Integrity
                    |
                    v
Asset layer
  Skill / Hook / AgentProfile / Connector / ExecutableAsset
                    |
                    v
Runtime layer
  RuntimeDescriptor / RuntimeAdapter / IsolationPolicy / RuntimeInstance
                    |
                    v
Capability layer
  ToolProvider / HookProvider / AgentRuntimeProvider / ConnectorProvider
```

### 5.1 Package

Package 是分发和版本管理容器，可以包含多个 Asset，但本身不执行。

```rust
pub struct PackageId(pub String);
pub struct PackageVersion(pub String);

pub struct PackageManifest {
    pub schema_version: u16,
    pub id: PackageId,
    pub version: PackageVersion,
    pub description: String,
    pub compatibility: CompatibilitySpec,
    pub assets: Vec<AssetRef>,
    pub requested_permissions: PermissionRequestSet,
}
```

Package 负责：

- 发布者命名空间；
- 版本；
- 完整性；
- Aletheon 兼容范围；
- Asset 索引；
- 包级权限摘要。

Package 不负责：

- Tool 参数定义；
- Runtime 进程参数；
- Hook 返回值；
- Profile 的实际 Tool 解析。

### 5.2 Asset

Asset 是用户可安装和管理的逻辑资产。

```rust
pub enum AssetKind {
    Skill,
    Hook,
    AgentProfile,
    Connector,
    Executable,
}
```

这里故意不包含：

- `Tool`：Tool 是 Capability；
- `AgentRuntime`：Runtime 是执行层；
- `ProcessPlugin`：Process 是隔离后端；
- 外部产品名称：它们属于 Adapter 配置。

公共资产描述：

```rust
pub struct AssetDescriptor {
    pub id: AssetId,
    pub package: PackageId,
    pub kind: AssetKind,
    pub version: String,
    pub description: String,
    pub origin: AssetOrigin,
    pub runtime: Option<RuntimeRef>,
    pub declared_capabilities: Vec<CapabilityDescriptor>,
    pub requested_permissions: PermissionRequestSet,
}
```

每种 Asset 使用独立 Manifest，禁止使用任意 JSON 大对象承载所有类型行为。

### 5.3 Runtime

Runtime 是 Asset 的执行方式，一个 Runtime 可以暴露多个 Capability。

```rust
pub struct RuntimeDescriptor {
    pub id: RuntimeId,
    pub class: RuntimeClass,
    pub protocol: ProtocolDescriptor,
    pub isolation: IsolationPolicy,
    pub health: HealthContract,
}

pub enum RuntimeClass {
    Native,
    Subprocess,
    Remote,
}
```

`RuntimeClass` 表达通用执行类别；具体命令、产品名称、环境变量和私有协议映射只存在于 Adapter 配置。

首版第三方可执行扩展只允许 `Subprocess`。`Native` 仅用于随正式二进制发布并经过同等代码审查的内置能力。

### 5.4 Capability

Capability 是执行和授权的最小单位。

```rust
pub enum CapabilityKind {
    Tool,
    HookProvider,
    AgentRuntimeProvider,
    ConnectorProvider,
}

pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub kind: CapabilityKind,
    pub input_schema: Option<SchemaRef>,
    pub output_schema: Option<SchemaRef>,
    pub risk: RiskLevel,
}
```

Capability 冲突必须按“可执行能力 ID”判断，而不是按 Asset 类型判断。

一个 Skill 可以不提供可执行 Capability；一个 Executable Asset 可以同时提供 Tool 和 Hook Provider；多个 Profile 可以引用同一个 Agent Runtime Provider。

## 6. 稳定公共契约与内部类型

### 6.1 Fabric 只保留跨 crate 稳定契约

建议 Fabric 拥有：

- `PackageId`、`AssetId`、`RuntimeId`、`CapabilityId`；
- `AssetKind`、`CapabilityKind`、`RuntimeClass`；
- `AssetDescriptor`、`CapabilityDescriptor` 的只读投影；
- `ActivationState`、`HealthState`；
- Runtime/Capability 的调用事件；
- 安装和运行审计事件的跨边界表示。

Fabric 不应拥有：

- 归档解压实现；
- Package Store 路径；
- CLI 参数；
- 子进程启动命令；
- 特定连接器配置；
- 具体 Manifest 文件解析器；
- 安装事务实现。

### 6.2 Corpus/扩展领域模块负责资产和目录

Corpus 或后续独立的扩展领域模块负责：

- Package 和各 Asset Manifest；
- Package Inspector；
- 安全解包；
- Store；
- Catalog 投影；
- Asset 验证；
- Capability 冲突检测；
- Skill 和 Hook 的原生适配。

### 6.3 Executive 负责编排和运行

Executive 负责：

- 安装应用服务；
- 审批编排；
- Runtime Adapter 注册和路由；
- 激活事务；
- daemon 快照切换；
- 隔离运行和断路；
- 真实运行健康验证；
- 回滚。

CLI/TUI 只能调用应用服务端口，不得直接修改 Store 或配置文件。

## 7. 扩展包格式

```text
extension-package/
├── extension.toml
├── checksums.sha256
├── assets/
│   ├── skills/<id>/SKILL.md
│   ├── hooks/<id>/hook.toml
│   ├── agents/<id>/AGENT.md
│   ├── connectors/<id>/connector.toml
│   └── executables/<id>/runtime.toml
├── payload/         # 各 Asset 的可执行脚本/二进制；路径在对应 Manifest 中声明
└── schemas/         # 可选：Asset Manifest schema 引用，供 Inspector 离线校验
```

约束：

- `schema_version = 1`；
- Package ID 使用发布者命名空间；
- 产品保留命名空间不得被第三方声明；
- Asset 路径必须是包内相对路径；
- Manifest 只能声明 `secret_ref`，不得保存真实凭据；
- 每个文件必须出现在校验和清单中；
- 未声明文件默认拒绝；
- 禁止绝对路径、`..`、符号链接、硬链接、设备文件和 FIFO；
- 必须限制单文件大小、总解压大小、文件数量和目录深度；
- Manifest 解析必须拒绝重复 ID 和未知的安全关键字段。

## 8. Store 与权威状态

```text
/usr/share/aletheon/extensions/builtin/       # 随产品安装，只读
/var/lib/aletheon/extensions/packages/        # 系统级
~/.local/share/aletheon/extensions/packages/  # 用户级
~/.local/state/aletheon/extensions/           # 收据、健康、激活状态
<workspace>/.aletheon/extensions/             # 项目候选源，默认不激活
```

Package 内容按完整 SHA-256 寻址，目录名不得只使用短哈希作为权威身份。短哈希只能用于 UI 展示。

权威状态至少包括：

- installed package versions；
- active package version；
- activated assets；
- granted permissions；
- health/quarantine state；
- transaction receipt；
- previous known-good activation；
- schema version。

状态写入采用：

```text
write temporary file
 -> fsync file
 -> atomic rename
 -> fsync parent directory
```

安装和激活按 Package ID 加锁；锁文件本身不能作为完成收据。

锁文件注册持有进程 PID。每次获取锁前检查 PID 存活：死进程持有的锁
记录日志后自动释放，恢复旧已知良好状态，不得静默丢弃或永久阻塞。

## 9. 安装和激活事务

```text
inspect
 -> validate manifest and archive
 -> extract into isolated staging
 -> verify full integrity
 -> resolve assets and runtime descriptors
 -> compute permission/capability diff
 -> request approval when required
 -> run static compatibility checks
 -> run isolated runtime health probe
 -> prepare candidate catalog snapshot
 -> atomically publish package
 -> activate candidate snapshot
 -> restart/reload installed daemon
 -> exercise installed client path
 -> commit receipt
```

失败处理：

```text
stop candidate runtime
 -> restore previous known-good activation
 -> restart/reload old daemon
 -> verify old client path
 -> mark candidate quarantined
 -> persist failure receipt and evidence
```

禁止行为：

- 验证前覆盖旧版本；
- 直接修改活动 Registry；
- 安装失败后删除所有证据；
- 健康探测失败仍标记 active；
- 使用仓库临时二进制替代安装二进制验收。

## 10. 激活状态机

```text
Discovered
  -> Validated
  -> Staged
  -> PendingApproval
  -> Probing
  -> Active

任何非 Active 状态
  -> Rejected
  -> Quarantined

Active
  -> Degraded
  -> RollingBack
  -> RolledBack
  -> Disabled
```

状态迁移必须携带：

- transaction ID；
- actor；
- old/new state；
- reason code；
- evidence references；
- timestamp；
- package/runtime hash。

未知或损坏状态不得被解释为 Active。

## 11. 资产运行模型

### 11.1 Skill

Skill 是声明式上下文资产：

- 启动时只加载 ID、名称、描述、触发器和权限摘要；
- 完整正文和 references 在匹配或显式调用时加载；
- Skill 声明的 Tool/Hook 必须转换为独立 Capability 并分别授权；
- Skill 正文不能绕过 Tool 权限；
- Skill 脚本必须通过获批 Runtime 执行。

### 11.2 Hook

Hook 模式：

```rust
pub enum HookMode {
    Observe,
    Transform,
    Guard,
}
```

约束：

- Observe 只能观察，返回的变更被忽略；
- Transform 只能修改 Hook Point 明确允许的字段；
- Guard 只能返回 allow/deny 和结构化理由；
- 每个 Hook 有执行预算、超时和输出上限；
- Hook 失败策略由 Hook Point 定义，不能由扩展自行决定；
- Guard 默认 fail-closed，纯 Observe 默认 fail-open 并记录错误；
- Hook 不得直接访问内部 Registry。

### 11.3 Agent Profile

Agent Profile 是配置资产，不是 Runtime：

- 声明角色、系统提示、Tool 授权、预算、审批策略；
- 声明 `runtime_requirements`，而不是绑定外部产品名；
- 激活前解析 Tool、模型能力、Runtime 和继承限制；
- 解析失败进入 `Quarantined`，不得成为当前 Profile；
- daemon 保留上一个有效 Profile 快照继续服务；
- TUI、healthcheck 和日志必须显示具体失败资产及原因。

`runtime_requirements` 字段骨架：

```rust
pub struct RuntimeRequirements {
    /// 需求的 Runtime 类别（Phase 4 之前仅 Native + Subprocess）。
    pub class: RuntimeClass,

    /// 最低上下文窗口 token 数（不含输出预留）。
    pub min_context_tokens: Option<u32>,

    /// 单次 Agent turn 最大输出 token 数。
    pub max_output_tokens: Option<u32>,

    /// 该 Profile 需要的最大并发 Tool 调用数。
    pub max_concurrent_tool_calls: Option<u32>,

    /// 需要的 Capability 能力标签（如 "sandboxed_exec", "network_io"）。
    pub required_capabilities: Vec<String>,
}
```

`RuntimeClass` 是公共枚举，不引用外部产品名。`required_capabilities` 用
Capability 标签而非 Runtime 名称表达需求，使 Profile 与具体 Adapter 解耦。
解析时，Router 根据 `(class, required_capabilities)` 选择满足条件的
Runtime Adapter；无匹配时 Profile 进入 Quarantined。

### 11.4 Agent Runtime Provider

统一端口至少支持：

```text
start(request) -> session
observe(session) -> event stream
steer(session, input)
follow_up(session, input)
cancel(session, reason)
wait(session)
health()
```

具体外部 Runtime 通过 Adapter 实现。公共请求和事件不得暴露外部专用方法名。

### 11.5 Connector

Connector Asset 声明：

- transport class；
- endpoint reference；
- trust level；
- capability allow/deny filters；
- timeout；
- secret references；
- health contract。

单个 Connector 失败只使自身 `Degraded/Quarantined`，不得阻止其他扩展和 daemon 核心能力启动。

### 11.6 Executable Asset

首版使用隔离子进程，但公共类型只称 `Executable`。

控制协议仅管理生命周期：

```text
initialize
capabilities.describe
health
operation.cancel
shutdown
```

业务调用按 Capability Provider 分离：

```text
ToolProvider.call
HookProvider.invoke
AgentRuntimeProvider.start/observe/steer/wait
ConnectorProvider.list/invoke
```

不能设计一个无边界的万能 `plugin.call`。

## 12. 隔离与权限

第三方可执行扩展默认：

- 禁止网络；
- 不继承 daemon 的全部环境变量；
- 不获得模型和 Connector 凭据；
- 文件系统只暴露审批路径；
- 工作目录独立；
- stdout 仅承载协议；
- stderr 脱敏并限长；
- 限制进程数、CPU 时间、内存、输出和执行时长；
- 启动、调用、取消、空闲和关闭均有超时；
- 连续失败进入断路和隔离状态。

权限变更必须重新审批：

```text
old grants vs new requests -> permission diff -> approval -> new receipt
```

升级不得继承超出旧收据范围的新权限。

## 13. 无效配置与 Daemon 健壮性

当前整体失败点位于 `crates/executive/src/host/daemon/bootstrap/runtime.rs:64-72`。修复不能只是把错误改成日志。

目标流程：

```text
load candidate profiles
 -> validate each profile independently
 -> valid profiles enter candidate registry
 -> invalid profiles create quarantine records
 -> verify configured default exists in candidate registry
 -> if valid: atomically replace active registry
 -> if invalid: retain previous known-good registry
 -> daemon continues in explicit degraded state
```

要求：

- 一个 Profile 失败不影响其他 Profile；
- 默认 Profile 失败时不得静默切换而不通知；
- 首次启动且没有任何有效 Profile 时，daemon 可以启动诊断面，但 readiness 必须为 degraded/not-ready；
- TUI 必须展示错误而不是只依赖 journal；
- 修复文件后可以重新验证并恢复；
- restart loop 必须由自动化测试覆盖。

## 14. 项目级扩展信任

`<workspace>/.aletheon/extensions/` 只提供候选发现：

1. 验证工作区身份；
2. 检查工作区是否受信任；
3. 展示 Package、Capability 和 Permission 差异；
4. 获得当前用户明确审批；
5. 仅对该工作区激活；
6. 信任撤销后立即停止项目级 Runtime。

项目资产不得：

- 覆盖内置或用户资产的同名权威记录；
- 修改全局默认 Profile；
- 注册未审批后台 Runtime；
- 将工作区信任提升为系统信任。

## 15. Metacog 接入边界

扩展平台不依赖 `metacog` 实现，但必须发布通用结构化事件，使 Metacog 可观察：

- package validation failure；
- permission denial；
- activation failure；
- runtime crash/timeout/protocol violation；
- degraded health；
- rollback；
- successful recovery。

事件必须包含证据引用、关联 ID、版本和结果。Metacog 可以评分、记录问题和提出改进建议，但不能绕过扩展审批、沙箱和激活事务直接修改或启用扩展。

## 16. CLI 与 TUI

正式产品 CLI：

```text
aletheon extension inspect PACKAGE
aletheon extension validate PACKAGE
aletheon extension install PACKAGE
aletheon extension list
aletheon extension show ID
aletheon extension enable ID
aletheon extension disable ID
aletheon extension upgrade PACKAGE
aletheon extension rollback ID
aletheon extension remove ID
aletheon extension purge ID
aletheon extension doctor ID
aletheon extension import-legacy
```

原则：

- CLI 调用应用服务，不直接写 Store；
- 破坏性操作明确确认；
- `--json` 输出稳定结构；
- `doctor` 显示状态、权限、Runtime、健康和最近失败证据；
- TUI 显示 quarantine/degraded 状态；
- 仓库脚本只编排构建、部署和真实验收，不复制领域逻辑。

## 17. 兼容迁移

### 17.1 旧 `ExtensionKind` 和身份

不能直接删除当前 `Tool/Skill/Hook/Plugin/Mcp`，因为 `tool:<name>` 仍用于调用身份：`crates/corpus/src/service.rs:407-423`、`:444-460`。

迁移策略：

1. 保持旧枚举和反序列化只读兼容；
2. 新模型写入 `AssetId/CapabilityId`；
3. 提供显式 legacy-to-new 投影；
4. 双读期间禁止继续生成新的旧格式持久化状态；
5. 收集旧格式读取计数和迁移报告；
6. 删除必须满足 §17.3。

### 17.2 旧文件系统资产

现有 Skill、Hook、Agent Profile 和 Connector 配置作为 `legacy_filesystem` 候选源：

- 可读；
- 激活前仍使用新的验证流程；
- 新安装只写 Package Store；
- `import-legacy` 在 staging 转换；
- 转换成功并真实验收后才切换权威来源；
- 失败保留旧有效状态。

### 17.3 安全删除条件

历史类型、路径和兼容读取只能在同时满足以下条件后删除：

- 支持窗口已经明确结束；
- 生产数据扫描确认没有未迁移记录；
- 连续发布周期的旧格式读取遥测为零；
- 回滚版本不再依赖旧格式；
- 迁移矩阵测试证明所有支持版本可升级；
- 删除获得单独审查和发布说明。

## 18. 审查后修复计划

修复工作必须从 `3da4519` 的实际状态继续，不得重新创建一套平行扩展平台。
每个修复阶段单独提交、单独验收。

### R0：纠正范围与完成状态

目标：建立可信基线，防止占位实现继续返回成功。

任务：

- 将未实现 CLI 子命令改为明确的 typed `Unsupported/NotImplemented` 错误和
  非零退出码；不得继续打印成功式文本；
- 删除扩展提交中混入的无关 Agent/机器人资产；
- 为每个 Phase 建立 `implemented/wired/verified` 三项状态表；
- 保存当前三组通过测试的命令和完整输出；
- 新增测试，断言占位命令不能返回成功。

门禁：

```text
inspect/validate: 可以成功执行
其余未实现命令: 明确失败且不改变状态
提交范围: 不包含无关领域资产
```

### R1：归档与 Manifest 安全修复

目标：Inspector 可以安全处理不可信归档。

必须修改：

- `crates/corpus/src/extension/inspector.rs`
- `crates/corpus/src/extension/manifest.rs`
- `crates/corpus/src/extension/validation.rs`
- `tests/fixtures/extensions/`

任务：

1. 在读取内容前按 Tar entry type 建立 allowlist，只允许 regular file 和
   directory；拒绝 symlink、hardlink、block/char device、FIFO 和未知类型。
2. 记录归档中见过的原始路径，重复路径立即失败，禁止 `HashMap` 覆盖。
3. checksum 必须是 64 位 ASCII 十六进制并规范化为小写。
4. checksum 路径重复立即失败。
5. Asset Manifest 声明路径必须存在、类型匹配且处于允许目录。
6. `extract_to_staging` 必须只解包已经验证的 regular file，不能再次依赖
   宽松的 `entry.unpack()` 类型判断。
7. 解包失败清理 staging，不能留下半完成候选。

必需负向 fixtures：

```text
symlink
hardlink
fifo
device-header
duplicate-entry
duplicate-checksum
non-hex-checksum
undeclared-file
missing-asset
asset-kind-path-mismatch
```

门禁：所有恶意 fixture 被拒绝，staging 外没有产生任何文件。

### R2：Store 和安装应用服务

目标：完成真正的 `inspect -> validate -> stage -> install -> receipt`，
安装阶段仍不激活第三方可执行内容。

任务：

1. 使用完整 SHA-256 作为权威 package/staging 路径；先验证 hash 格式，禁止
   对未验证字符串切片。
2. 使用原子互斥锁；不得采用 `exists -> write PID` 的竞态实现。
3. Package 状态路径使用无碰撞编码，禁止通过字符替换映射 ID。
4. receipt ID 使用 UUID/单调序列，不使用秒级时间戳作为唯一名称。
5. receipt 临时文件必须 `sync_all` 后 rename，再同步父目录。
6. 实现 `ExtensionApplicationService`，统一拥有 Inspector、Store、审批端口和
   状态投影。
7. `install/list/show` CLI 只调用应用服务。
8. 安装中断后重启可识别并清理或恢复未完成事务。
9. Package Store、CLI 和 receipt 增加集成测试，证明生产路径真实调用。

门禁：

- 安装合法 Package 后 `list/show` 可读取同一权威状态；
- 重复安装幂等；
- 并发安装只有一个提交者；
- 中断后没有活动半成品；
- receipt 可重放并重建 installed 投影。

### R3：Profile 隔离和 Registry 快照

目标：完整关闭 Profile 导致 daemon 崩溃循环的所有路径。

任务：

1. Profile 逐个验证并产生持久 quarantine record。
2. 如果配置的 default Profile 无效：
   - 保留 previous-known-good；
   - 没有旧快照时进入明确 degraded/not-ready；
   - 不得在存在其他有效 Profile 时再次通过 `resolve_by_name` 失败退出。
3. Registry 采用候选快照构建和原子替换，禁止在活动 Registry 上逐项修改。
4. `quarantined_profiles` 进入 daemon health、readiness、doctor 和 TUI。
5. 修复 Profile 后支持重新验证和恢复。
6. 增加以下真实 daemon 测试：

```text
one invalid + one valid
configured default invalid + another valid
all invalid on first boot
all invalid with previous-known-good
profile repaired after quarantine
```

门禁：以上五种场景均无 restart loop，且客户端能读取准确状态。

### R4：Provider 生命周期和受治理子进程

目标：Provider 契约完整，Executable Asset 只通过审批后的隔离 Runtime 运行。

任务：

1. `AgentRuntimeProvider` 补齐：

```text
start
observe
steer
follow_up
cancel
wait
health
```

2. Tool、Hook、Agent Runtime、Connector 使用独立业务端口，不增加万能调用。
3. 校验 JSON-RPC version、response ID、唯一 in-flight 请求和最大行长度。
4. 启动和调用失败均进入断路统计；成功按明确策略复位。
5. 调用超时或取消后终止相关 operation；进程失去协议同步时必须重启或隔离。
6. 启动后台 stderr drain，执行真实脱敏、限长和审计。
7. 实现 idle timeout。
8. 文件系统、网络、环境、CPU、内存和进程数按照获批
   `IsolationPolicy` 落地；缺少可用隔离后端时 fail-closed。
9. 将 Runtime 注册到 daemon 的通用 Runtime Router，并通过 Provider trait
   实际调用。

门禁：必须有真实辅助进程测试覆盖正常调用、错误 ID、超长输出、stderr
秘密、挂起、取消、崩溃、连续失败、网络/文件越权和关闭清理。

### R5：激活、升级、回滚和完整 CLI

目标：所有 CLI 通过同一个应用服务执行可逆状态事务。

任务：

- 实现 enable/disable/upgrade/rollback/remove/purge；
- 激活前计算 Capability/Permission 差异并经过审批；
- 保存 current 和 previous-known-good 指针；
- 候选健康失败自动恢复旧版本并复验；
- 实现 workspace trust；
- 实现 doctor 和 import-legacy；
- 记录旧兼容读取计数和迁移报告；
- 发布 Metacog 可消费的通用证据事件；
- TUI 显示 active/degraded/quarantined/rollback。

门禁：每个 CLI 都必须有状态前后断言，不能只检查输出字符串。升级新增权限
时必须重新审批；拒绝审批后旧版本继续可用。

### R6：真实部署和故障矩阵

目标：完成原 Phase 7，形成可审计发布收据。

收据目录必须包含：

```text
candidate-hash.json
installed-hash.json
running-process-hash.json
daemon-health.json
cli-install.json
tui-tool-task.json
subagent-runtime-task.json
profile-quarantine.json
connector-failure.json
runtime-crash.json
upgrade-rollback.json
metacog-events.jsonl
cleanup.json
```

所有收据包含 schema version、commit、命令、开始/结束时间、状态和证据摘要。
敏感值必须在持久化前脱敏。

门禁：§21 的 20 条验收标准全部有收据定位，才允许把文档状态改成“完成”。

## 19. 原目标实施阶段与文件级任务

### Phase 0：基线和保护测试

目标：先固定当前行为，避免重构破坏已运行能力。

前置依赖：无。可以直接开始。

任务：

- 为 `ExtensionId` 旧格式增加序列化兼容测试；
- 为 Catalog 冲突和 Tool 调用身份增加基线测试；
- 为无效 Agent Profile 导致整体加载失败增加复现测试；
- 记录实际 daemon/TUI 失败链路；
- 建立最小合法和恶意扩展包 fixtures。

主要文件：

- `crates/fabric/tests/extension_contract.rs`（新建或沿用现有契约测试位置）；
- `crates/corpus/tests/extension_catalog.rs`；
- `crates/executive/tests/agent_profile_isolation.rs`；
- `tests/fixtures/extensions/`。

门禁：只增加测试和 fixtures，不改变生产行为。

### Phase 1：四层通用契约

目标：引入 Package/Asset/Runtime/Capability 分层，同时保持旧读取兼容。

前置依赖：Phase 0 基线测试全部通过。

任务：

- 在 Fabric 增加新 ID、枚举、只读 Descriptor 和状态事件；
- 不把 Manifest 解析、Store 或进程命令放入 Fabric；
- 实现旧 `ExtensionDescriptor` 到新投影的兼容层；
- 新写路径不再创建旧扩展持久化状态；
- 增加 serde round-trip、未知 schema 和兼容测试。

主要文件：

- `crates/fabric/src/types/extension.rs`：保留 legacy 类型并缩小职责；
- `crates/fabric/src/types/extension_package.rs`：跨边界 Package 身份与投影；
- `crates/fabric/src/types/extension_asset.rs`：Asset/Capability/Runtime 公共投影；
- `crates/fabric/src/types/extension_state.rs`：激活和健康事件；
- `crates/fabric/src/lib.rs`、`crates/fabric/src/types/mod.rs`。

门禁：现有调用链仍通过兼容层工作，不进行目录清理式大改。

### Phase 2：Package Inspector、Manifest 和 Store

目标：可以安全检查和安装但尚不激活第三方可执行内容。

前置依赖：Phase 1 四层类型契约稳定（新 ID、枚举、Descriptor 和兼容层可用）。

任务：

- 实现 Package Manifest 和独立 Asset Manifest；
- 实现归档边界验证、安全解包和完整校验；
- 实现 content-addressed Store；
- 实现事务收据、锁、崩溃恢复和状态投影；
- 实现 `inspect/validate/install/list/show` 应用服务和 CLI。

建议按能力组织目录，避免新的 `core/bridge/impl`：

```text
crates/corpus/src/extension/
  package/
  asset/
  store/
  catalog/
  validation/
  legacy/
```

门禁：安装阶段不得启动第三方可执行内容。

### Phase 3：Profile 隔离和 Catalog 快照

目标：单资产失败不拖垮 daemon，Registry 切换原子化。

前置依赖：Phase 2 Package Store 可用（inspect/validate/install 可执行）；Phase 2 的 Asset Manifest 解析稳定。

任务：

- Profile 逐个解析和验证；
- 建立 quarantine 记录；
- 构建候选 Registry；
- 仅在默认 Profile 和核心不变量有效时替换活动快照；
- 保留 previous known-good；
- readiness、TUI、doctor 输出 degraded 原因；
- 增加 restart-loop 回归测试。

主要修改点：

- `crates/executive/src/host/daemon/bootstrap/runtime.rs`；
- `crates/executive/src/composition/agent_loader/`；
- Agent Profile Registry 所属模块；
- daemon health/readiness；
- TUI 状态展示。

门禁：故意放置未知 Tool 的 Profile，daemon 必须保持服务，错误 Profile 必须可见且不可激活。

### Phase 4：Hook 与 Capability Provider

目标：将 Hook、Tool、Agent Runtime 和 Connector 统一到明确 Provider 端口，不建立万能插件接口。

前置依赖：Phase 3 Profile 隔离可用（daemon 不因单 Profile 失败崩溃）。

任务：

- 实现 `HookMode` 和 Hook Point 允许行为矩阵；
- 增加 Capability Provider Descriptor；
- 调整 Catalog 按 Capability ID 检测可执行冲突；
- 保持旧 Tool 身份兼容；
- 为 Runtime Provider 增加 start/observe/steer/follow-up/cancel/wait/health 端口；
- 对 Connector 单点失败进行隔离。

门禁：每种 Provider 独立契约测试，权限不能通过 Asset 组合关系扩张。

### Phase 5：隔离可执行扩展

目标：第三方 Executable Asset 通过受治理子进程运行。

前置依赖：Phase 4 Provider 端口稳定（ToolProvider、HookProvider 等 trait 可用）。

任务：

- 生命周期控制协议；
- 各 Capability Provider 的独立业务协议；
- 环境变量清理、文件系统范围、网络策略和资源限制；
- 超时、取消、stderr 脱敏、断路器；
- runtime health 和 quarantine；
- 权限升级重新审批。

建议模块：

```text
crates/executive/src/extensions/
  application/
  activation/
  runtime/
    subprocess/
  health/
  recovery/
```

名称按仓库最终领域边界调整，但不得新增无意义的 `impl/` 或 `bridge/` 技术层目录。

### Phase 6：升级、回滚、项目信任和完整 CLI

目标：激活状态可逆；项目级扩展可发现但默认禁用；完整 CLI 通过应用服务操作。

前置依赖：Phase 5 子进程 Executable Asset 隔离可用。

任务：

- 原子 enable/disable/upgrade/rollback/remove/purge；
- previous known-good 恢复；
- 项目级候选发现和工作区信任；
- `doctor` 与 `import-legacy`；
- 兼容读取计数与迁移报告；
- Metacog 通用观察事件。

门禁：中断安装、daemon 崩溃、候选健康失败均能恢复旧有效状态。

### Phase 7：实际部署验收

目标：通过安装二进制和真实 TUI 完成端到端验收，证明 20 条标准全部满足。

前置依赖：Phase 6 完整 CLI 可用（enable/disable/upgrade/rollback/remove/purge/doctor/import-legacy）。

必须执行：

1. 通过仓库正式入口构建候选；
2. 安装候选二进制、service、completions 和内置资产；
3. 校验候选、安装文件和运行进程哈希一致；
4. 重启用户 daemon；
5. 使用安装后的 CLI 安装测试 Package；
6. 使用真实 TUI 启用并调用一个 Tool Provider；
7. 通过 Agent Profile 启动一个通用 Sub-Agent Runtime；
8. 注入无效 Profile、Connector 故障和 Runtime 崩溃；
9. 验证 daemon 仍可诊断、旧有效功能仍可用；
10. 执行升级和回滚；
11. 检查收据、quarantine、health 和 Metacog 观察事件；
12. 运行清理并证明没有遗留进程和活动指针。

## 20. 验证命令约束

任何 Rust 构建、检查、测试、Lint 或文档命令都不得直接调用 `cargo`，必须使用：

```bash
bash scripts/cargo-agent.sh <cargo arguments>
```

实施者必须选择验证当前变更的最窄 package 和 test target。只有集成验证负责人可以运行 workspace-wide 检查；不得并发运行 workspace build。

每个 Phase 的最低验证：

```text
format check
narrow unit tests
contract tests
negative/security tests
phase-specific integration test
```

Phase 3、5、6、7 必须增加真实进程测试；Phase 7 必须使用安装后的二进制和 TUI。

## 21. 验收标准

只有以下条件全部满足，扩展平台才能标记完成：

1. Package、Asset、Runtime、Capability 四层类型边界明确；
2. 公共 ABI、Profile 和通用 Manifest 不包含外部项目专名；
3. 一个 Package 可以包含多个 Asset；
4. 一个 Runtime 可以提供多个受独立授权的 Capability；
5. Tool 不再被错误建模为 Asset，但旧 `tool:<name>` 可兼容读取；
6. 恶意归档和路径逃逸全部被拒绝；
7. 安装、激活、升级和回滚具有持久事务收据；
8. 无效 Profile 被隔离、可诊断且不造成 daemon 崩溃循环；
9. Connector 单点失败不阻断 daemon 核心能力；
10. Executable Asset 崩溃、超时、取消和协议违规被隔离；
11. 权限升级必须重新审批；
12. 项目扩展必须经过工作区信任；
13. 旧有效快照在候选失败后可以恢复；
14. Metacog 能观察失败与恢复，但不能越权激活扩展；
15. CLI/TUI 能展示 active、degraded、quarantined 和 rollback 状态；
16. 候选、安装文件和运行进程哈希一致；
17. 真实 TUI 能完成包含 Tool 或 Sub-Agent Runtime 的实际任务；
18. 日志中不存在 restart loop、未捕获协议错误或秘密泄漏；
19. 所有 Rust 命令遵守 `scripts/cargo-agent.sh` 资源策略；
20. 旧兼容类型只有在 §17.3 条件全部满足后才删除。

## 22. 实施者约束

实现者必须遵循：

- 每个 Phase 单独提交，不把所有阶段压成一个大提交；
- 开始每个 Phase 前重新读取本规格和相关代码；
- 不修改与当前 Phase 无关的机器人、模型或 UI 行为；
- 不覆盖工作区已有未提交修改；
- 先写失败测试或基线测试，再修改生产代码；
- 不引入外部项目专用字段；
- 不用字符串替换冒充领域迁移；
- 不用 `warn + skip` 冒充完整隔离状态机；
- 不直接操作 Package Store 绕过应用服务；
- 不声称“测试通过”而未给出命令和结果；
- 每个阶段结束报告：变更文件、测试证据、未完成项、兼容风险、下一阶段入口条件；
- 任何真实部署失败都必须先恢复 previous known-good，再继续诊断。
