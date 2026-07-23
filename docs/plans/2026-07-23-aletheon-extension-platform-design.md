# Aletheon 通用扩展与隔离插件平台设计

**日期：** 2026-07-23
**状态：** 已完成方案评审，等待文档复核
**范围：** 第一阶段仅完善 Aletheon 原生扩展平台；模型配置与 `setup.sh`
向导属于后续独立阶段。

## 1. 目标

Aletheon 作为开源产品，必须在不依赖任何个人资产仓库的情况下提供原生
Skill、Hook、Agent Profile、Agent Runtime、MCP Server 和自定义插件能力。
第三方系统通过同一公开包格式安装资产，不得获得专用接口。

首版自定义插件采用隔离子进程，不将第三方动态库或解释器模块加载进
daemon。安装、启用、升级和回滚必须是可验证的事务，最终验收必须使用
实际安装后的 Aletheon daemon 和 TUI。

## 2. 当前基础与缺口

当前代码已经具备分散的运行能力：

- 公共扩展类型已有 Tool、Skill、Hook、Plugin、MCP，但缺少
  Agent Profile、Agent Runtime 和明确的隔离进程插件类型：
  `crates/fabric/src/types/extension.rs:46-64`。
- `ExtensionCatalog` 当前只有只读快照契约，没有安装、激活或回滚契约：
  `crates/fabric/src/types/extension.rs:171-174`。
- Skill 支持 `SKILL.md`、工具和 Hook 声明：
  `crates/corpus/src/skill/manifest.rs:11-54`。
- 用户 Hook 支持从 TOML 加载并注册：
  `crates/corpus/src/hook/loader.rs:27-83`。
- Agent Profile 支持 Markdown 加载、工具授权和启动期校验：
  `crates/executive/src/composition/agent_loader/mod.rs:44-123`、
  `crates/executive/src/host/daemon/bootstrap/runtime.rs:24-72`。
- MCP Server 已进入 daemon 配置和连接管理：
  `crates/executive/src/host/daemon/bootstrap/request.rs:387-390`。
- Pi 已通过通用 Sub-Agent Runtime 边界运行：
  `crates/executive/src/adapters/runtime/pi.rs:455`。

缺口是这些能力没有统一的包、Store、权限审查、安装事务和失败回滚；
无效的主动状态仍可能在 daemon 启动时造成整体不可用。

## 3. 架构边界

```text
Extension sources
  built-in / local package / project directory / external publisher
                         |
                         v
Package Inspector -> Validator -> Staging Store -> Policy Approval
                         |                 |
                         |                 v
                         |          Activation Receipt
                         v
Extension Catalog -> Typed Runtime Adapters -> Governed execution
```

### 3.1 核心与适配器

`fabric` 只拥有稳定、通用的数据契约。Corpus 拥有扩展目录、Skill、Hook、
MCP 和进程插件适配。Executive 负责编排安装、激活、Agent Runtime 路由和
真实 daemon 生命周期。CLI 只调用应用层端口。

外部运行时名称、命令参数和协议解析只能存在于适配器或部署配置。核心只
认识 `RuntimeId`、`AgentRuntimeInput`、`AgentRuntimeEvent`、
`AgentEventSink` 和取消信号。

### 3.2 Aurb 边界

Aletheon 自带足以独立工作的原生资产。Aurb 可以作为个人资产源导出标准
扩展包，但 Aletheon：

- 不依赖 Aurb 工作区路径；
- 不包含 Aurb 专用字段或命令；
- 不允许包中出现宿主绝对路径；
- 对 Aurb 与任何第三方使用同一安装和审批流程。

## 4. 通用类型

公共扩展种类调整为：

```rust
pub enum ExtensionKind {
    Skill,
    Hook,
    AgentProfile,
    AgentRuntime,
    McpServer,
    ProcessPlugin,
}
```

旧 `Plugin` 在兼容窗口内只读识别为历史组合资产；新数据不得继续写入该
类型。删除旧类型必须等待迁移证据证明没有历史激活状态依赖它。

共同元数据包括：

```text
id
version
description
origin
capabilities
risk
compatibility
integrity
activation constraints
```

每类资产拥有独立 Manifest，不能用一个无类型 JSON 对象表达所有行为。

## 5. 扩展包

标准目录：

```text
extension-package/
├── extension.toml
├── checksums.sha256
├── skills/
├── hooks/
├── agents/
├── runtimes/
├── mcp/
└── plugins/
```

`extension.toml` 使用 `schema_version = 1`，声明包 ID、版本、Aletheon
兼容范围和资产列表。资产 ID 使用反向域名或明确发布者命名空间。
`aletheon.*` 由产品保留，第三方不得声明。

扩展包不得包含真实凭据。Manifest 只能声明 Secret 名称、用途和是否必需，
运行时通过 `secret_ref` 解析凭据。

## 6. Package Store 与事务

系统级、用户级和项目级资产分离：

```text
/usr/share/aletheon/extensions/builtin/
/var/lib/aletheon/extensions/packages/
~/.local/share/aletheon/extensions/packages/
~/.local/state/aletheon/extensions/
<workspace>/.aletheon/extensions/
```

安装流水线：

```text
inspect
 -> validate schema/path/checksum/size
 -> stage into content-addressed version directory
 -> compute capabilities and conflicts
 -> request approval
 -> isolated compatibility and health probe
 -> atomically activate
 -> restart installed daemon
 -> exercise the real client path
 -> persist receipt
```

失败时停止新实例、恢复旧激活指针、重启旧 daemon、复验旧路径并保留失败
证据。解包必须拒绝绝对路径、父路径、符号链接、硬链接、设备文件、超限
单文件和超限总大小。

同一 ID 不允许静默覆盖；同一可执行 Capability 只能有一个激活实现。
替换必须有显式 `replaces` 声明和操作员审批。

## 7. 资产运行模型

### 7.1 Skill

启动时只加载名称、描述、触发器和权限摘要；完整 `SKILL.md`、reference 和
scripts 在匹配或显式调用时渐进加载。缺少必需字段的 Skill 不进入活动目录。

### 7.2 Hook

Hook 明确区分：

```rust
pub enum HookMode {
    Observe,
    Transform,
    Guard,
}
```

每个 Hook Point 固定允许的模式和结构化结果。Observe 返回值被忽略；
Transform 只能修改该事件允许的字段；Guard 只能给出允许或拒绝以及原因。

### 7.3 Agent Profile 与 Agent Runtime

Agent Profile 定义角色、提示词、工具、预算、审批策略、`runtime_class` 和
能力要求。它不直接绑定某个外部运行时名称。

```text
AgentProfile(runtime_class = coding)
              |
              v
Agent Runtime Router
  -> native adapter
  -> installed coding adapter
  -> process-plugin runtime adapter
```

Pi 继续作为内置适配器提供 coding 和 resident RPC 能力，但 Pi 名称不得
进入通用 Profile 或核心契约。

### 7.4 MCP Server

MCP 资产声明传输、端点、信任级别、工具过滤、超时和凭据引用。单个 MCP
失败不得阻止其他扩展启动。新安装的 MCP 不直接改写主配置。

### 7.5 Process Plugin

首版插件通过 JSON-RPC/stdio 隔离运行，协议方法为：

```text
initialize
describe
health
tool.call
hook.invoke
agent.launch
operation.cancel
shutdown
```

事件为：

```text
progress
log
capability.changed
health.changed
```

插件默认禁用。启用前必须审批能力。插件不继承完整 daemon 环境或 Provider
密钥，网络默认关闭，文件系统仅暴露获批路径，stderr 脱敏限长。启动、
调用、空闲和关闭都有超时；连续失败触发断路器。插件只能声明能力，由
Aletheon 创建受治理 Adapter，不能直接修改内部 Registry。

## 8. 项目级扩展

`<workspace>/.aletheon/extensions/` 支持发现，但默认禁用。只有工作区可信、
Manifest 合法、权限差异已经展示并获得当前用户明确批准后，才能在该工作区
激活。项目资产不能覆盖系统或用户资产，也不能注册未获批的后台 Runtime。

## 9. CLI

产品能力由安装后的二进制提供：

```text
aletheon extension inspect PACKAGE
aletheon extension validate PACKAGE
aletheon extension install PACKAGE
aletheon extension list
aletheon extension enable ID
aletheon extension disable ID
aletheon extension upgrade PACKAGE
aletheon extension rollback ID
aletheon extension remove ID
aletheon extension purge ID
aletheon extension doctor ID
aletheon extension import-legacy
```

仓库的 `scripts/aletheon.sh` 只负责构建、部署和真实运行门禁，不重复实现
产品领域逻辑。

## 10. 兼容迁移

现有 Skill、Hook、Agent Profile 和 MCP 配置继续作为
`legacy_filesystem` 只读来源。所有新安装写入 Package Store。
`import-legacy` 在 staging 中转换并验证旧资产，成功后才激活标准包。

Agent Profile 必须在激活前验证工具、Runtime、模型和权限。无效 Profile
只使所属扩展处于 unhealthy，不得替换旧 Registry 或使 daemon 崩溃循环。

## 11. 验收

阶段一完成必须证明：

1. 六类 Manifest 和统一 Descriptor 的序列化、校验与兼容测试通过；
2. 恶意归档、路径逃逸、链接、大小限制测试通过；
3. 安装、升级、回滚、并发锁和中断恢复测试通过；
4. Hook 三种模式的事件语义测试通过；
5. 无效 Agent Profile 不影响已部署 daemon；
6. MCP 单点失败隔离成立；
7. Process Plugin 崩溃、超时、取消和协议违规被隔离；
8. 项目扩展必须经过工作区信任和审批；
9. Pi Sub-Agent 仍通过通用 Runtime 路由完成真实任务；
10. 候选二进制、安装二进制和运行进程哈希一致；
11. 使用真实 TUI 安装/启用扩展并完成一个包含工具或 Sub-Agent 的任务；
12. 最终帧出现实质回答、输入提示符返回、日志不存在禁止错误。

所有 Rust 命令必须通过 `bash scripts/cargo-agent.sh ...` 串行执行。

## 12. 后续阶段

通用扩展平台完成后，单独设计并实施 Setup 配置向导：

```text
setup.sh
 -> install binary/services/completions
 -> initialize built-in assets
 -> optionally run provider/model/secret wizard
 -> configure machine core and current user daemon
 -> detect and configure available Agent Runtime adapters
 -> optionally install external extension packages
 -> run installed-runtime and real-TUI acceptance
```

`setup.sh` 只编排正式 CLI 和部署验证命令，不实现 Provider、扩展或密钥领域
逻辑。
