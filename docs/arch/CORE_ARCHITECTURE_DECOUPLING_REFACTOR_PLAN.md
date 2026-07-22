# Aletheon 核心架构解耦与外部集成边界重构方案

**状态：** 设计基线

**日期：** 2026-07-23

**适用范围：** Aletheon 主仓库
**目标：** 在保持现有业务能力、部署兼容性和安全约束的前提下，建立低耦合、可替换、可验证的核心架构，并将外部项目专属名称与接口限制在明确的边界适配器内。

---

## 1. 决策摘要

本方案采用“**核心隔离，而非全仓库机械去名称**”原则：

> 核心领域、共享契约和应用编排只认识领域能力、稳定协议与通用接口；外部项目名称、专属类型、wire 字段和行为差异只能存在于边界适配器、部署配置、兼容迁移及其专项测试中。

标准协议名称是系统边界的一部分，可以保留，包括 HTTP、OAuth 2.0、PKCE、MCP、gRPC、Protobuf、SSE、JSON、SQLite。外部服务或产品名称也可以出现在对应 adapter、部署实例、用户选择的 provider/model 值和 contract test 中。

禁止的不是“文字本身”，而是以下结构性耦合：

1. 共享 DTO 包含某个供应商专属类型或 scope；
2. 业务流程根据 provider/runtime 名称决定行为；
3. 核心模块直接解析外部 wire payload 或错误文本；
4. 具体 adapter 类型成为跨 crate 公共 API；
5. 替换外部实现时必须修改核心领域枚举、协调器或持久化模型；
6. 配置读取、secret 解析、adapter 构造分散到领域模块。

本重构采用“**先固定边界，再迁移依赖，最后收缩接口**”的渐进方式，不进行一次性大爆炸重写。

---

## 2. 当前架构事实与主要问题

### 2.1 Fabric 的职责超过共享契约边界

Fabric 自身将其定义为子系统之间的共享契约层，见 `crates/fabric/src/lib.rs:3-7`。但它目前同时公开共享类型、协议、IPC、事件、运行时辅助和大量兼容导出，见 `crates/fabric/src/lib.rs:21-87`。

外部平台细节已经进入共享契约：

- `IdentityProvider::Google` 位于 `crates/fabric/src/types/external_identity.rs:47-57`；
- Gmail、Calendar、Drive OAuth scope 位于 `crates/fabric/src/types/external_identity.rs:61-104`；
- 共享层直接返回 Google OAuth URL，见 `crates/fabric/src/types/external_identity.rs:74-85`；
- `ExternalEvent` 直接导入 Google 类型，见 `crates/fabric/src/types/external_event.rs:3-7`；
- `MailChange` 直接持有 `GmailMessageSummary`，见 `crates/fabric/src/types/external_event.rs:71-75`；
- Fabric 公开 `types::google`，见 `crates/fabric/src/lib.rs:67-72`。

这使新增外部身份或信息源时必须修改共享层，违反开放扩展边界。

### 2.2 Executive 同时承担组合、业务、基础设施和外部适配

Executive 直接依赖 Gateway、Kernel、Agora、Cognit、Corpus、Mnemosyne、Runtime、Dasein、Metacog、Hardware，见 `crates/executive/Cargo.toml` 的 `[dependencies]`。作为 composition root，依赖这些 crate 本身合理；问题在于 Executive 还同时实现：

- daemon/host 生命周期；
- 应用用例与业务协调器；
- 外部身份和渠道集成；
- 具体数据库 repository；
- 外部 coding runtime 协议；
- 补充记忆 adapter；
- 公开实现模块。

`crates/executive/src/impl/mod.rs:1-25` 同时公开 `daemon`、`external`、`gbrain`、`google`、`runtime` 等性质不同的模块，表明 `impl` 已成为无明确边界的容器。

### 2.3 `impl` 被当作公共 API

以下 crate 直接公开实现层：

- Cognit：`crates/cognit/src/lib.rs:20-24`、`crates/cognit/src/lib.rs:47-50`；
- Executive：`crates/executive/src/lib.rs:14`；
- Mnemosyne：`crates/mnemosyne/src/lib.rs:14`、`crates/mnemosyne/src/lib.rs:113-119`；
- Metacog：`crates/metacog/src/lib.rs:4`、`crates/metacog/src/lib.rs:9-10`；
- Dasein：`crates/dasein/src/lib.rs:71`。

一旦调用方可以导入 `crate::impl::*`，实现目录就不再是实现细节，内部重构会变成跨 crate API 迁移。

### 2.4 配置所有权混乱

部署配置、领域策略和 adapter 配置尚未清晰分离：

- `PiRuntimeConfig` 位于 Cognit，见 `crates/cognit/src/config/mod.rs:353-392`；
- `TelegramConfig` 也位于 Cognit，见 `crates/cognit/src/config/mod.rs:989-1024`；
- `AppConfig` 聚合 provider、goal runtime、Pi runtime 和 integration，见 `crates/executive/src/core/config/mod.rs:74-90`；
- Executive 再次重导出 Cognit provider 配置，见 `crates/executive/src/core/config/provider.rs:1-6`；
- `UserRuntimeConfig` 从应用配置中重新抽取部分字段，见 `crates/executive/src/user_runtime/mod.rs:22-52`。

这混合了三种不同概念：原始部署配置、规范化应用配置、领域运行策略。

### 2.5 Runtime 概念存在多个所有者

仓库同时存在独立 `runtime` crate、Kernel runtime、Executive runtime、runtime core、system core runtime 和 user runtime。它们分别表达 agent 执行、进程生命周期、领域对象集合或组合状态，但命名没有形成唯一词汇表，增加理解成本和错误依赖风险。

### 2.6 大模块聚合过多职责

当前多个生产模块超过千行，例如：

- `crates/corpus/src/security/runner.rs`；
- `crates/executive/src/service/agent_control/mod.rs`；
- `crates/executive/src/service/agent_control/settlement.rs`；
- `crates/corpus/src/tools/mcp/client.rs`；
- `crates/corpus/src/tools/mcp/auth.rs`；
- `crates/executive/src/service/turn_pipeline.rs`；
- `crates/executive/src/impl/daemon/server.rs`；
- `crates/mnemosyne/src/service.rs`；
- `crates/cognit/src/harness/linear/mod.rs`；
- `crates/cognit/src/config/mod.rs`。

行数不是独立的违规指标，但这些文件同时持有状态、策略、I/O、协议和生命周期时，应按状态机 owner 与端口边界拆分，而不是仅按长度切文件。

### 2.7 架构门禁有基础，但仍偏路径与文本启发式

`scripts/architecture-check.sh` 已经保护若干重要边界，包括配置加载归 Executive 所有、Interact 不依赖 Kernel/Corpus、具体 LLM provider 构造集中到 factory、部分 service 不直接导入具体 store。现有限制需要保留。

不足之处是规则较依赖具体文件路径和 grep 文本，尚未覆盖：

- 外部项目名称进入核心路径；
- `impl` 的跨 crate 公共导出；
- domain/application 对 reqwest、rusqlite 等基础设施的直接依赖；
- 通过 provider/runtime 名称或 URL 猜测实现；
- 兼容 allowlist 只增不减。

---

## 3. 外部项目名称与接口耦合专项

### 3.1 Pi coding runtime

Pi 协议适配器内部知道 Pi 是必要的；Goal、Agent Control 和通用配置知道 Pi 则没有必要。

现状包括：

- `PiRuntimeConfig` 位于 Cognit：`crates/cognit/src/config/mod.rs:353-392`；
- `PiAttemptRequest`、`PiRuntime` 位于 Executive runtime：`crates/executive/src/impl/runtime/pi.rs:88-102`；
- `ParsedPiOutput` 和 `parse_job_jsonl`：`crates/executive/src/impl/runtime/pi_protocol.rs:11-23`；
- `PiRpcCommand`、`PiRpcRecord`：`crates/executive/src/impl/runtime/pi_protocol.rs:219-264`；
- 通用 runtime 模块公开 Pi 类型：`crates/executive/src/impl/runtime/mod.rs:12-14`；
- Goal attempt coordinator 直接导入 Pi request/runtime ID：`crates/executive/src/impl/goal/attempt_coordinator.rs:11`；
- Agent Control 通过 runtime ID 是否包含 `pi` 选择行为：`crates/executive/src/service/agent_control/mod.rs:747`。

目标是业务层只认识 `CodingRuntime`、`CodingAttemptRequest`、`RuntimeManifest`、`RuntimeEvidence` 和 `VerificationPolicy`。Pi argv、JSONL/RPC、生命周期字段、版本与 executable hash 均留在私有 adapter。

### 3.2 Google/Gmail 共享契约污染

Google adapter 内保留 Google API 类型是必要的；Fabric、Executive application 层持有 Google 类型是不必要的。

目标是将共享契约改为：

- `ExternalProviderId`；
- `ExternalCapabilityId`；
- `MailQuery`、`MailMessageSummary`、`MailMessage`；
- `CalendarQuery`、`CalendarEntry`；
- `ExternalFileMetadata`、`ExternalChangeBatch`；
- opaque provider object ID 与 cursor。

Google OAuth scope、history ID、etag、API endpoint 只由 Google adapter 解释。

### 3.3 GBrain supplemental memory

当前已存在通用 `McpMemoryConfig`，但专属名称仍贯穿配置和模块：

- `MemoryConfig.gbrain`：`crates/cognit/src/config/mod.rs:751-778`；
- `GbrainMemoryConfig` 兼容别名：`crates/cognit/src/config/mod.rs:826-827`；
- 默认 server name 硬编码：`crates/cognit/src/config/mod.rs:829-836`；
- Mnemosyne 公开 `backends::gbrain`：`crates/mnemosyne/src/backends/mod.rs:9-10`；
- Executive 公开 `impl::gbrain`：`crates/executive/src/impl/mod.rs:15`。

目标是核心只认识 `SupplementalMemoryTransport` 和 `SupplementalMemoryConfig`。旧 `gbrain` 字段仅保留在配置兼容层，部署中的实例名可以继续存在。

### 3.4 具体 LLM provider 公开范围过大

具体 provider adapter 可以保留，但不应成为 Cognit 公共 API：

- `pub mod anthropic`、`ollama`、`openai_provider`：`crates/cognit/src/impl/llm/mod.rs:1-3`；
- crate root 公开整个 `impl::llm`：`crates/cognit/src/lib.rs:47-50`；
- `Transport` 直接枚举具体实现：`crates/cognit/src/config/mod.rs:674-684`。

目标是公共 API 只导出 `InferenceProvider`、通用 DTO、通用错误、capability 和 registry/factory contract。具体 adapter 默认 `pub(crate)`。部署配置中的 provider/model 名称仍可保留。

### 3.5 Telegram 与渠道职责

现有通用渠道 DTO 边界基本正确：`InboundMessage` 和 `OutboundMessage` 位于 `crates/fabric/src/types/channel.rs:38-57`。问题是渠道专属配置位于 Cognit，且具体 transport 被 Gateway 公开，见 `crates/gateway/src/lib.rs:1-18`。

目标是 Cognit 完全不知道渠道；Gateway application 只认识 `ChannelTransport`，Telegram polling、bot token 和 callback payload 位于 adapter。

### 3.6 Embodiment 下游细节

通用 gRPC embodiment provider 应保留。需要清除的是下游实现泄漏：

- 通用配置注释出现具体机器人/仿真 bridge：`crates/executive/src/core/config/integrations.rs:113-130`；
- Hardware 通用错误包含 `ROS master unreachable`：`crates/hardware/src/grpc/error.rs:45`。

核心错误应归一化为 `ProviderUnavailable`、`ControlPlaneUnavailable` 或 `UpstreamDisconnected`。ROS、topic、service、action 和机器人产品类型不得进入 Hardware core 或 Fabric DTO。

---

## 4. 目标分层架构

```text
┌─────────────────────────────────────────────────────────────┐
│ Host                                                        │
│ daemon / CLI / RPC / process lifecycle                      │
└──────────────────────────────┬──────────────────────────────┘
                               │
┌──────────────────────────────▼──────────────────────────────┐
│ Composition                                                 │
│ config normalization / secret resolution / factories / DI   │
└───────────────┬──────────────────────────────┬──────────────┘
                │                              │
┌───────────────▼──────────────┐  ┌────────────▼──────────────┐
│ Application                  │  │ Adapters                  │
│ use cases / coordinators     │◄─┤ HTTP/MCP/gRPC/DB/process │
│ application ports           │  │ provider conversion       │
└───────────────┬──────────────┘  └───────────────────────────┘
                │
┌───────────────▼─────────────────────────────────────────────┐
│ Domain + Stable Contracts                                  │
│ rules / state / IDs / DTOs / ports / events / errors       │
└─────────────────────────────────────────────────────────────┘
```

唯一允许的源码依赖方向：

```text
host -----------┐
composition ----┼----> application ----> domain/contracts
adapters -------┘
```

运行时控制流可以由 application 调用注入的 adapter port，但源码依赖仍然是 adapter 实现 domain/application 定义的 trait，而不是 application 导入 adapter。

### 4.1 统一目录语义

逐步采用以下命名：

| 目录 | 唯一职责 |
|---|---|
| `domain` | 领域状态、值对象、纯规则和状态转换 |
| `contract` | 跨边界 trait、稳定 DTO、错误与事件 |
| `application` | 用例、协调器、事务边界、应用端口 |
| `adapters` | 外部 API、数据库、进程、wire protocol |
| `composition` | 配置规范化、factory、依赖注入 |
| `host` | daemon、CLI、RPC、OS 生命周期 |
| `compatibility` | 旧配置、旧路径和持久化迁移 |

不得继续把互不相关的实现放入含义模糊的顶层 `impl`。迁移期间可以保留旧路径，但新代码必须进入目标分层。

**统一的是职责分类和依赖方向，不是目录模板。** `fabric`、`executive`、`cognit`、`mnemosyne`、`hardware`、`gateway` 中的每个模块都必须能唯一归入 domain、contract、application、adapter、composition、host 中的一类，并符合 §4 的依赖方向；crate 只创建实际需要的层，不要求五类目录齐全。例如 Fabric 可以是 contract-only，Executive 需要 application/composition/host，而 Hardware 未必需要独立 application 层。各 crate 的目标内部结构见 §20。

**`impl` 治理分为公共边界和物理目录两个目标。** 当前存在 `impl/` 目录的 crate 有 5 个：`cognit`、`executive`、`mnemosyne`、`dasein`、`metacog`。所有 crate root 都必须停止公开 `r#impl`，跨 crate 的 `impl` 路径必须归零；本轮只要求 `cognit`、`executive`、`mnemosyne` 将杂物容器式顶层 `impl/` 物理拆入明确职责层。`dasein`、`metacog` 属于 §17.1 的非主动重构范围，本轮收缩其公共 facade，是否物理拆目录另行评估。

> 附注：`impl` 是 Rust 关键字，`src/impl/` 目录实际依赖 `r#impl` 或路径属性才能编译，本身即是应当改名的信号之一。目标目录名（`adapters`、`composition` 等）不与关键字冲突。

### 4.2 进程与 wire 边界

分层图中的 `Host` 是架构角色，不等于一个具体进程。当前仓库至少包含 `aletheon` host/CLI 进程和 `execd` 执行进程；`interact` 是由上层 host 使用的客户端/UI library，`platform` 是提供文件系统、进程、PTY、沙箱等 OS capability 的 library，而不是独立进程。证据分别见 `crates/aletheon/Cargo.toml:9-11`、`crates/execd/src/main.rs`、`crates/platform/src/lib.rs:1-20`。

**这样区分的原因：** 如果把 library 误写成进程，就会把普通 Rust 调用误当作 wire contract，导致错误的版本升级、序列化兼容和部署假设；反过来，如果把真实进程边界当成内部调用，则可能在重构 DTO 时静默破坏旧客户端或 sidecar。

当前需要分别治理的边界包括：

| 边界 | 参与方 | 协议 owner / 版本依据 | 变更纪律 |
|---|---|---|---|
| 客户端协议 | `aletheon`/`interact` ↔ Executive daemon | `crates/fabric/src/protocol/client.rs` 的 `CLIENT_PROTOCOL_VERSION` | 修改该协议暴露类型时 bump 客户端协议版本，并兼容或明确拒绝旧版本 |
| 执行进程协议 | Executive/host ↔ `execd` | `crates/execd/src/protocol.rs` | 由 execd 协议 owner 定义兼容和版本策略，不复用客户端协议版本 |
| Embodiment 协议 | Hardware provider ↔ 外部 bridge | Hardware protobuf/gRPC contract | 按 protobuf service/message 兼容规则演进 |
| MCP 协议 | Corpus ↔ MCP server | MCP 标准协议及本项目工具 schema | 标准协议版本与工具 schema 分别治理 |
| 持久化格式 | service/repository ↔ SQLite/spool/artifact | 各 schema/version/migration owner | 使用数据库或事件 schema migration，不使用客户端协议版本 |

因此 Fabric 类型必须先分为 `wire-exposed` 和 `internal-shared`，并为每个 `wire-exposed` 类型记录具体 `protocol_owner`：

| 分类 | 含义 | 改动纪律 |
|---|---|---|
| `wire-exposed` | 出现在某一明确跨进程或持久化序列化路径 | 跟随其 `protocol_owner` 的版本与兼容策略，不统一绑定 `CLIENT_PROTOCOL_VERSION` |
| `internal-shared` | 仅在同一进程内跨 crate 共享，不越过 wire/持久化边界 | 可按内部 DTO 迁移，无需协议版本流程 |

Phase 1 / Phase 6 改动 Fabric DTO 前，必须先产出该分类（见 §19 wire 面清单）。未分类的 Fabric 类型默认按 `wire-exposed` 从严处理，原因是漏做一次兼容迁移的损失高于多做一次边界核对。

### 4.3 端到端数据流与待清洗耦合点

分层图(§4)描述**依赖方向**,本节描述**运行时数据流**,并把 §3 的泄漏点标注在流上——这是重构要"清洗"的对象。两条主链路:

**链路 A：入站消息 → 回复**

```text
外部渠道 ──[InboundMessage]──▶ Gateway ��─▶ TurnRuntime / turn_pipeline
   (adapter)                   (app)          (app, executive)
        │                                         │
        │                                   [统一 Message/ContentBlock]
        ▼                                         ▼
  ⚠️渠道配置在 Cognit                       InferenceProvider ──[wire 转换]──▶ LLM adapter
  ⚠️具体 transport 由 Gateway 公开            (contract)                        (adapter)
                                                  │
                                            工具调用 ▼
                                        Corpus 工具 / MCP client ──▶ 外部工具/MCP server
                                                  │                    ⚠️MCP 名称贯穿 config
                                            [记忆读写] ▼
                                        Mnemosyne(supplemental) ──▶ 记忆后端
                                                  │                    ⚠️backends::gbrain 公开
                                                  ▼
                                        [OutboundMessage] ──▶ 渠道 adapter ──▶ 外部
```

**链路 B：Goal → 编码执行 → 验证**

```text
Goal ──▶ AgentControl(admission / lifecycle / settlement) ──▶ CodingRuntime(port)
(app)        (app, executive)                                    (contract)
  │             ⚠️runtime_id.contains("pi") 决定存储                  │
  │             ⚠️Goal coordinator 直接导入 Pi request               ▼
  │                                                        CodingRuntime adapter
  ▼                                                          (Pi argv/JSONL/RPC)
[RuntimeEvidence / RuntimeManifest] ◀────[归一化 outcome]────────┘
  │
  ▼
存储配额 / 验证策略  ⚠️应由 manifest 显式字段驱动,而非名称
```

**链路 C（旁路）：身份 / 外部信息源**

```text
ExternalIdentity/Mail/Calendar/File(port) ──▶ Google adapter ──▶ 外部 API
   (contract)          ⚠️IdentityProvider::Google、Gmail scope、GmailMessageSummary 在 Fabric
        │                ⚠️ExternalEvent 内嵌 Google 类型(且已持久化)
        ▼
  归一化 ExternalEvent ──▶ 消费方(app)
```

图中每个 ⚠️ 对应 §3 的一处泄漏,是各阶段的清洗目标:链路 A 的渠道/记忆/MCP 名称(Phase 4/5/6),链路 B 的 `contains("pi")` 与 Pi 类型(Phase 3),链路 C 的 Google 类型与持久化事件(Phase 1)。清洗的判定标准:**每条箭头两端只应看到通用 DTO 与 port,任何厂商类型/名称都必须被挡在最外层 adapter 之内。**

数据流治理原则:

1. 每条箭头必须能标出 owner crate 与所处层(adapter/app/contract);标不出的说明职责不清,需先拆分;
2. 跨 wire/持久化的箭头(链路 A 的入站/出站、链路 C 的事件落盘)按 §4.2 分类并版本化;
3. 同一数据在链路上只有一个归一化点(如 InboundMessage、ExternalEvent、CodingAttemptOutcome),不得多处各自解析外部 payload;
4. 反向依赖禁止:下游(Mnemosyne、Corpus)不得回指 Executive application 内部类型,只经 port/contract 交互。

---

## 5. 核心端口与通用模型

### 5.1 Inference

核心能力：

```text
InferenceProvider
├── complete(InferenceRequest) -> InferenceResponse
└── stream(InferenceRequest)   -> InferenceStream
```

核心只认识统一 Message、ContentBlock、ToolCall、Usage、StopReason、capability 和 `InferenceErrorKind`。具体 messages/chat-completions/native-chat wire format由 adapter 转换。

### 5.2 External identity

```text
ExternalIdentityProvider
├── begin_authorization
├── complete_authorization
└── revoke
```

身份提供方和 capability 使用受边界校验的开放 ID，而非每增加一个 provider 就修改共享 enum。credential 永远不进入共享 DTO。

### 5.3 External information sources

按能力拆分，而不是建立一个平台巨型 integration：

```text
MailSource
CalendarSource
FileSource
ExternalEventSource
```

provider cursor、object ID 和版本标识在核心中保持 opaque；adapter 负责解释。

### 5.4 Channel

```text
ChannelTransport
├── receive -> InboundMessage
├── send    -> DeliveryReceipt
└── health  -> ChannelHealth
```

核心消息 DTO 不包含 bot API、webhook 或平台 callback 类型。

### 5.5 Supplemental memory

```text
SupplementalMemoryTransport
├── recall
├── read
└── write
```

MCP 可以作为一个标准协议 adapter；某个具体 MCP server 名称不是领域类型。

### 5.6 Coding runtime

```text
CodingRuntime
AgentRuntimeLauncher
RuntimeProtocolAdapter
VerificationPolicy
```

业务 DTO 为 `CodingAttemptRequest`、`CodingAttemptOutcome`、`RuntimeManifest`、`RuntimeEvidence`、`RuntimeCapabilityAudit`。具体 executable、argv 和 RPC event 只在 adapter 内存在。

### 5.7 Embodiment

保留通用 DeviceProvider、permit、lease、observation、command、capability 与 gRPC contract。下游 ROS 或厂商映射由独立 bridge/adapter 负责。

---

## 6. 配置架构

配置必须经过单向管线：

```text
DeploymentConfig
  文件 / env / alias / 旧字段
          │ parse + merge
          ▼
NormalizedConfig
  schema / URL / secret ref / adapter registration
          │ validate + resolve
          ▼
DomainConfig + AdapterConfig
  只包含各自真正消费的已验证字段
```

约束：

1. 只有 composition 层读取文件和业务环境变量；
2. secret 以 `SecretRef` 表示，只在构造 adapter 时解析；
3. domain 不接收部署路径、环境变量名或 endpoint 推断规则；
4. adapter config 不进入不使用它的领域模块；
5. provider/adapter 是注册数据，不是核心业务分支；
6. 不通过 URL 后缀猜测 provider；
7. 旧字段通过 compatibility normalization 转换为新模型；
8. 日志输出只包含脱敏后的规范化配置。

**adapter-id → 构造器匹配的唯一合法位置（消除歧义）：** 静态注册表不可避免存在一处 `match adapter_id { "messages-http" => ... }`。规则是：**该匹配只允许出现在 composition 的 registry/factory 模块中，且只用于选择构造器，不得据此改变业务语义。** application、domain、host 层出现任何按 adapter/provider 字符串分派的 `match`/`contains` 均视为违规。这条与 §11.2 门禁对应，避免"要么把 match 塞进 application、要么把合法 registry 也当违规删掉"两种误读。本项目不引入动态插件 ABI（见 §17），注册表是编译期静态表。

**`SecretRef` 归属（消除歧义）：** `SecretRef` 属于 deployment/composition config contract，不属于 domain contract。DeploymentConfig、NormalizedConfig 和构造期 AdapterConfig 可以持有它；DomainConfig、application port、领域事件和共享业务 DTO 不得持有它。明文只在 composition 构造 adapter 时解析，解析结果不得写回 config 或事件；需要刷新 credential 的 adapter 应持有受限 credential handle，而不是让 domain/application 接触 `SecretRef` 或明文。

推荐配置形态：

```toml
[[integrations]]
id = "primary-inference"
kind = "inference"
adapter = "messages-http"

[integrations.settings]
base_url = "https://example.invalid"
credential_ref = "primary-provider"

[[integrations]]
id = "supplemental-memory"
kind = "memory"
adapter = "mcp"
```

`adapter` 是 composition registry 的 ID，核心 service 不得匹配其具体值。

---

## 7. 错误与诊断约束

核心不得通过外部错误文本判断业务行为。所有 adapter 必须归一化为稳定分类：

```text
InvalidConfiguration
AuthenticationRequired
PermissionDenied
RateLimited
InvalidRequest
ResponseTooLarge
ProtocolViolation
IncompatibleVersion
TemporarilyUnavailable
PermanentlyUnavailable
Timeout
Cancelled
```

错误可携带通用 category、retry disposition、sanitized message、opaque diagnostic code 和 correlation ID。不得包含 credential、未经限制的外部响应正文，也不得要求核心理解厂商错误文本。

上述 12 类定义的是外部传输和集成边界的稳定 `IntegrationFailureKind`，不替代领域错误。`AdmissionError`、`MemoryContractError`、`CognitError`、repository error 等领域错误仍由各自 owner 定义；当它们包裹 adapter 失败时，只能依赖 `IntegrationFailureKind`，不能暴露 Google、Pi、ROS 或其他厂商错误类型和文本。

**与 §3.6 命名的关系（消除歧义）：** 上面 12 类是 `IntegrationFailureKind` 的唯一稳定枚举，不是全系统唯一错误枚举。§3.6 出现的 `ProviderUnavailable`、`ControlPlaneUnavailable`、`UpstreamDisconnected` 不是新增集成失败变体，而是 adapter 归一化时对以下变体的语义描述，必须映射到本枚举：

| §3.6 描述 | 归一化目标（§7 枚举） |
|---|---|
| `ProviderUnavailable` | `TemporarilyUnavailable` 或 `PermanentlyUnavailable`（按可重试性） |
| `ControlPlaneUnavailable` | `TemporarilyUnavailable` |
| `UpstreamDisconnected` | `TemporarilyUnavailable`（携带 opaque diagnostic code 区分场景） |

任何外部 adapter 的传输失败不得向核心暴露这 12 类之外的**集成失败种类**；领域 port 可以在其上定义稳定领域错误。需要区分的厂商细节放进 opaque diagnostic code，不进入领域枚举变体。

可观测性允许记录 adapter ID 和部署实例 ID，以便诊断；这不构成业务耦合。任何日志字段都不得参与业务分支。

该约束同样适用于 metrics 与 trace span：span 名称、metric label 可以携带 adapter ID / 部署实例名以便排障，但告警规则、重试逻辑或任何业务分支都不得以这些名称为条件。

---

## 8. 公共 API 与可见性约束

### 8.1 可以公开

- 领域值对象和 DTO；
- 通用 trait/port；
- 通用错误；
- capability 与 registry contract；
- 用户需要的 host/client facade；
- 测试用 fake port。

### 8.2 默认不得公开

- 具体 provider struct；
- 外部 wire request/response；
- adapter parser 和状态机；
- 外部 endpoint 常量；
- 具体数据库 repository；
- 专属 adapter config；
- 顶层 `impl` 模块。

具体 adapter 默认使用 `pub(crate)`。跨 crate 使用必须经稳定 facade。兼容 re-export 必须登记规范路径、调用点数量和删除条件；新代码不得使用旧路径。

**"稳定 facade"的统一形态（消除歧义）：** facade = 每个 crate 根部一个显式的公开表面，仅由以下构成：(a) `pub use` 重导出的领域 DTO / 通用 trait / 通用错误 / capability；(b) 用户需要的 host/client 门面类型；(c) 测试用 fake port。**不是** newtype 包装、**不是** 直接 `pub` 内部模块树。具体 adapter、repository、wire 类型、parser 一律不进 facade。各 crate 的 facade 形态必须一致（同一套 `pub use` 约定），避免每个 crate 各自发明一种。

---

## 9. Runtime 术语约束

统一采用：

| 名称 | 含义 |
|---|---|
| `HostRuntime` | daemon/process/OS 生命周期 |
| `AgentRuntime` | Agent 执行环境 |
| `TurnRuntime` | 单轮推理执行所需能力集合 |
| `CodingRuntime` | 代码任务执行能力 |
| `DomainServices` | 长生命周期领域服务集合 |
| `RuntimeRegistry` | Agent runtime 注册与解析 |
| `CompositionState` | 构造期已解析依赖集合 |

禁止新增没有明确语义的 `RuntimeCore`、`CoreRuntime`、`SystemRuntime` 组合。现有命名在对应模块发生实质重构时迁移，不进行无收益的全局一次性改名。

上表是目标词汇表，落地前必须先把**现状名映射到目标名并指定 owner crate**，否则本节仍是愿望而非计划。映射基线（在 Phase 0 产出、随迁移更新）：

| 现状（crate/模块） | 目标名词 | owner crate | 处置 |
|---|---|---|---|
| 独立 `runtime` crate | `RuntimeContract`（crate 名暂不改） | runtime | 保留 manifest、capability、interaction/workspace/tool-governance 描述与 deterministic selector；不拥有实例、进程生命周期、准入或 adapter 构造 |
| Kernel runtime（`crates/kernel`） | `KernelLifecycle` / `KernelServices` | kernel | 保留 process/operation/supervision/admission/budget/lease 的唯一治理 owner；实质重构时再迁移类型名 |
| Executive runtime（`crates/executive/src/impl/runtime`） | `AgentRuntime` + adapter | executive | Phase 3 迁移 |
| `UserRuntimeConfig`（`crates/executive/src/user_runtime`） | 归入 `DomainConfig` | executive composition | Phase 4 迁移 |
| system core runtime / runtime core | `CompositionState` / `DomainServices` | executive composition | Phase 2 迁移 |
| Executive daemon/host 生命周期（`crates/executive/src/impl/daemon/server.rs`）+ `aletheon` bin bootstrap | `HostRuntime` | executive host / `aletheon` | Phase 2 迁移；只做进程 bootstrap/RPC/信号，不持有领域规则或治理 |

**保留独立 runtime crate 的原因：** 当前该 crate 只定义 `RuntimeManifest`、`RuntimeCapability` 和 `RuntimeSelector`，且 selector 明确声明 Executive 仍是 registry/admission authority，见 `crates/runtime/src/lib.rs:1-9`、`crates/runtime/src/manifest.rs:40-49`、`crates/runtime/src/selector.rs:10-13`。它已经接近一个稳定、无基础设施依赖的 contract crate；把它并入 Executive 会让通用能力描述反向依赖 application/composition，增加而不是减少耦合。是否将 crate 物理改名为 `runtime-contract` 只在 Phase 10 根据收益复评，不阻塞 Phase 2。

**不把 KernelRuntime 改称 HostRuntime 的原因：** `KernelRuntime` 当前拥有 process/operation table、supervision、admission、budget、lease 和 mailbox，见 `crates/kernel/src/runtime.rs:25-48`；这些是受治理的生命周期权威，而 OS 文件系统、PTY、进程后端和沙箱 capability 已由 Platform 拥有，见 `crates/platform/src/lib.rs:1-37`。使用 `HostRuntime` 会混淆治理权威与 OS host adapter。

---

## 10. 大模块重构原则

文件拆分必须以“唯一状态 owner、唯一策略 owner、明确端口”为依据，不设置机械行数上限。

优先模块：

1. Agent Control；
2. Turn Pipeline；
3. Mnemosyne service；
4. MCP client/auth；
5. Daemon server；
6. Cognit config。

以 Agent Control 为例，目标结构可以是：

```text
agent_control/
├── service.rs
├── lifecycle.rs
├── settlement.rs
├── admission.rs
├── messaging.rs
├── runtime_port.rs
├── persistence_port.rs
└── error.rs
```

拆分验收不是“文件变短”，而是：

- 状态转换只有一个入口；
- I/O 通过端口隔离；
- lifecycle 和 settlement 规则可纯测试；
- service 不导入具体 repository/runtime adapter；
- 不产生新的循环依赖或重复 facade。

---

## 11. 自动架构门禁

扩展 `scripts/architecture-check.sh`，并保留现有有效规则。门禁分三层。

### 11.1 Cargo 依赖门禁

- domain/contracts crate 不依赖 Executive、host 或 adapter crate；
- UI/Interact 只通过 Fabric protocol 与 host 通信；
- binary crate 不构造领域内部对象；
- domain module 不直接依赖 reqwest、rusqlite、dirs 等基础设施库；
- 任何新增依赖边必须进入审查基线（见下方"新增边"判定）。

**contract/domain 层允许依赖白名单（消除歧义）：** 只有明确列出的"无 I/O、无 OS、无运行时"库允许出现在 contract/domain：

```text
允许：serde / serde_json（仅作 DTO 派生，不做业务分派）/ uuid / chrono / thiserror / bytes / 纯数据结构库
禁止：reqwest、rusqlite/sqlx、tokio(full)、nix、libc、dirs、toml（config 解析属 composition）、
      png/base64 等编解码（属 adapter）、任何具体 provider/DB/HTTP client
灰色（需登记理由）：tracing（允许 span/event，但不得据字段分派）、async-trait、futures
```

> 现状警示：`crates/fabric/Cargo.toml` 目前依赖 `tokio(full)`、`nix`、`libc`、`toml`、`png`、`base64`、`bincode`、`dashmap`，**Fabric 现在并不是纯契约 crate**。这些超标依赖必须在 Phase 1/2 登记为待清理项：运行时/OS 相关下沉到 host/platform，config 解析下沉到 composition，编解码下沉到 adapter。清理前它们进入依赖基线并只减不增。

**"新增依赖边"判定（消除歧义）：** "只减不增"针对的是**违规边**（domain/application → 基础设施或 adapter crate）。合法方向的新边——如新增 `adapters/<x>` 依赖 reqwest、composition 依赖具体 crate——允许，但必须在审查基线登记，且不得反向让 domain/application 因此获得对基础设施的传递依赖。

### 11.2 Rust import/module 门禁

- application 不导入 adapters；
- core/domain 不导入具体 provider；
- crate root 不公开整个 `impl` 或 adapter tree；
- repository concrete type 只能出现在 adapter/composition；
- config loader 和 environment parsing 只在 composition；
- 按 adapter/provider 字符串分派的 `match`/`contains` 只允许出现在 composition registry/factory；application、domain、host 出现即违规（对应 §6"唯一合法匹配位置"）。

### 11.3 外部标识与语义门禁

以下路径禁止外部产品/项目专属标识：

```text
crates/fabric/**
crates/*/src/domain/**
crates/*/src/contract/**
crates/*/src/application/**
crates/executive/src/service/**
crates/executive/src/impl/goal/**   # 迁移完成前保护旧路径
crates/cognit/src/harness/**
crates/kernel/**
```

允许区域：

```text
*/adapters/<provider>/**
*/compatibility/**
deploy/**
config examples
adapter contract tests
明确登记的迁移文件
```

每条例外必须精确记录：

```text
规则 | 文件 | 原因 | 规范替代 | 删除条件
```

不允许整目录永久放行。allowlist 必须满足“调用点只减不增”。

**"调用点"计数法（消除歧义）：** 一个调用点 = 一条对旧路径/兼容 re-export 的**导入或引用行**（`use 旧路径`、限定路径引用、或命中禁用标识的一行）。计数由门禁脚本对每条 allowlist 条目 grep 得到整数,基线数字随条目一起提交到 `config/architecture-allowlist.txt`(或同类台账)。CI 规则:任一条目的当前计数 > 基线即失败;计数下降时必须同步下调基线(棘轮),使其不可回升。计数归零则删除该条目及其允许区域。

同时禁止核心代码：

- `contains("provider-name")`；
- `match provider_name` 决定业务策略；
- 根据 URL 识别实现；
- 根据厂商错误字符串确定重试；
- 硬编码外部 endpoint；
- 使用无边界 `serde_json::Value` 让 provider 语义穿透 adapter 进入核心分支。

> `serde_json::Value` 豁免：MCP tool 调用参数/结果等本质开放的 JSON 允许作为 **adapter 内部载荷** 或 **显式类型化的 passthrough 信封**（如 `OpaqueToolPayload(Value)`）存在。禁止的是：核心 service 直接 `match`/读取 `Value` 内部字段来决定业务行为，或用 `Value` 规避定义稳定 DTO。门禁按“核心路径是否读取 Value 内部结构”判定，而非“是否出现 Value 类型”。

---

## 12. 测试策略

### 12.1 核心测试

只使用通用 fake port：

- fake inference provider；
- fake channel；
- fake supplemental memory；
- fake external identity/source；
- fake coding runtime；
- fake embodiment provider。

核心测试不得要求具体外部项目 fixture。

### 12.2 Adapter unit test

覆盖：

- 外部 payload 与内部 DTO 转换；
- 错误归一化；
- credential 脱敏；
- 响应大小和数量边界；
- timeout/cancellation；
- 未知字段和协议版本漂移；
- cursor/object ID opaque round-trip；
- 外部不可用时的 retry disposition。

### 12.3 Adapter contract test

允许使用真实外部名称和协议 fixture，但必须放在明确的 adapter contract test 路径，不得成为核心测试依赖。

### 12.4 迁移测试

每个旧配置和持久化格式都必须验证：

```text
legacy input -> compatibility normalization -> canonical model -> equivalent behavior
```

若无法兼容，必须 fail closed 并返回明确迁移错误，不能静默选择默认实现。

### 12.5 Rust 资源策略

所有 Rust build/check/test/lint/docs 命令必须通过：

```bash
bash scripts/cargo-agent.sh <cargo arguments>
```

每阶段使用最窄 package/test target；只有最终集成验证 owner 可以运行 workspace-wide 检查；不得并发运行 Executive 或 workspace build。

**"集成验证 owner"定义（消除歧义）：** 指被显式指派运行 workspace-wide 构建/测试的**单一角色**——在多 agent 协作中即协调者(coordinator)或人类维护者本人,而非任意 developer/fixer 子代理。子代理只跑自己 package 的最窄 target;workspace-wide 检查串行、由该单一 owner 触发,以遵守"不并发运行 Executive/workspace build"的资源约束。

---

## 13. 分阶段迁移计划

各阶段并非等量，也不是纯线性。下表给出体量、blast-radius 与并行性，用于排期与多任务分工；实际依赖 DAG 见其后。

| 阶段 | 体量 | blast-radius | 硬前置 | 可并行 |
|---|---|---|---|---|
| 0 架构决策、风险盘点与防扩散门禁 | 中 | 全仓库（不改业务逻辑） | 无 | — |
| 1 Fabric 契约纯化 | 大 | 高（跨进程 wire + 持久化事件） | 0 | 与 8 部分并行 |
| 2 Executive 分层 | **特大** | 高（几乎所有下游阶段的地基） | 0 | 不建议与他阶段并行 |
| 3 Coding runtime 解耦 | 中 | 中 | 2 | 与 5 并行 |
| 4 配置所有权 | 大 | 中 | 2 | 与 5、6 部分并行 |
| 5 Supplemental memory | 中 | 低 | 2 | 与 3、4 并行 |
| 6 Channel/Identity/信息源 | 大 | 高（含安全敏感 OAuth） | 1、2 | 与 3、5 部分并行 |
| 7 Inference adapter 私有化 | 中 | 中 | 2 | 与 5、6 部分并行 |
| 8 大模块状态机化 | **特大** | 中（局部但深） | 对应模块的端口稳定阶段完成 | 逐模块实施，非整体并行 |
| 9 公共 API 收缩 | 中 | 高（跨 crate import） | 1–7 | 收尾，不并行 |
| 10 全局验证/复评 | 小 | — | 9 | — |

依赖 DAG（→ 表示硬前置）：

```text
0 → 1 ─────────────┐
0 → 2 → 3          ├→ 9 → 10
        2 → 4      │
        2 → 5      │
    1,2 → 6        │
        2 → 7      │
8（拆为 8a–8e，各自位于对应端口稳定阶段之后）┘
```

要点：**Phase 2 是关键路径上体量最大的单点**，其余多数阶段等它；而 3/5/4/7 在 2 完成后彼此高度独立，应并行以缩短总工期，不要按 3→4→5→6→7 串行执行。

Phase 8 不是一个整体节点，应拆成独立子任务：Phase 2、3 → 8a Agent Control；Phase 2 → 8b Turn Pipeline；Phase 5 → 8c Mnemosyne service；对应 MCP adapter 边界稳定阶段 → 8d MCP client/auth；Phase 2 → 8e Daemon server。端口尚未稳定就先拆状态机，会在后续阶段被推倒重来。

### 阶段 0：架构决策、风险盘点与防扩散

**目标：** 在修改现有耦合前，确定 owner 与兼容边界、量化当前风险，并阻止新增同类问题。

**这样安排的原因：** 仅增加名称 grep 无法保护持久化、跨进程协议和安全语义；但 Phase 0 也不应改变业务行为，否则基线和迁移混在同一阶段，后续无法判断回归来自门禁还是逻辑变化。

工作内容：

1. 提交分层、目录语义和 runtime owner 词汇表；
2. 建立外部标识边界清单；
3. 产出 wire surface 与 protocol owner 清单；
4. 产出 persistence surface 与 migration owner 清单；
5. 增加依赖、import、公开导出和名称泄漏门禁；
6. 建立 compatibility/allowlist 台账；
7. 记录量化基线，只允许后续减少违规；
8. 为每类门禁提供会失败的 fixture，证明 CI 能识别回退。

验收：架构检查能够在测试 fixture 中捕获新增违规，并且不要求先完成后续迁移。

### 阶段 1：Fabric 契约纯化

**目标：** 共享契约不再包含具体外部平台类型。

工作内容：

1. 引入开放 provider/capability ID；
2. 引入通用 mail/calendar/file DTO；
3. adapter 完成 Google payload 转换；
4. ExternalEvent 改用通用 payload；
5. 保留旧序列化格式的显式兼容转换；
6. 禁止新代码导入 `fabric::google`。

**持久化前置：** `ExternalEvent` 当前带 `EXTERNAL_EVENT_SCHEMA_VERSION` 且内嵌 `GmailMessageSummary`（见 `crates/fabric/src/types/external_event.rs`），已落盘的事件/spool 会被本阶段打穿。开工前必须完成 §19 持久化面清单中受本阶段影响的条目，并实现双读或版本化迁移。

**安全前置：** `ExternalScope` 内嵌 `is_write()` / `is_m6_allowed()` 安全语义（`crates/fabric/src/types/external_identity.rs`）。scope 迁入 adapter 时，写权限门控与 M6 白名单必须一并迁移且行为不变，需配套 scope 门控回归测试。本阶段结束前需一次安全评审 checkpoint。

验收：新增第二个同类 provider 不需要修改 Fabric enum；Fabric contract test 不引用 Google fixture；scope 门控回归测试通过；受影响持久化格式的迁移测试通过。

### 阶段 2：Executive 分层

**目标：** application、composition、adapter、host 边界可被源码路径和门禁识别。

工作内容：

1. 建立目标目录 facade；
2. 将 bootstrap/factory 收口到 composition；
3. 将 repository/外部 client 收口到 adapters；
4. 将 Goal/Turn/Agent use case 收口到 application；
5. host 只处理 daemon/RPC/process lifecycle；
6. 保留旧路径兼容 re-export，禁止新增使用。

验收：application 不导入具体 store、provider 或协议 parser。

### 阶段 3：Coding runtime 解耦

**目标：** Goal、Agent Control 和通用配置不知道具体 coding runtime 项目。

工作内容：

1. 建立通用 coding request/outcome/evidence；
2. 建立 runtime manifest 和 verification policy；
3. 将专属 JSONL/RPC parser 移入 adapter；
4. 移除 Goal coordinator 对专属 request/runtime ID 的依赖；
5. 移除 `runtime_id.contains(...)` 业务判断；
6. 以 capability/manifest 决定存储与验证策略。

**替换 `contains("pi")` 的具体字段（消除歧义）：** 当前 `agent_control/mod.rs:747` 用 `runtime_id.contains("pi")` gate 1GB 存储配额，而现有 `RuntimeManifest`（`crates/runtime/src/manifest.rs:40-49`）没有存储维度。本阶段必须给 manifest 增加显式 `resource_requirements`（如请求的 storage bytes/items），由 adapter 注册时声明；它只是需求，不是授权。Agent Control 将声明转换为 `AgentStorageRequirement`，再由 admission/quota policy 按系统上限 clamp 或拒绝，最终产生 `AgentStorageReservation`。禁止用 `workspace_mode` 等无关字段隐式代表存储需求，也禁止 adapter 自行决定最终配额。该字段属于 runtime contract 变更，按 §4.2 的 contract/wire 纪律处理。

验收：使用 fake coding runtime 可完成 Goal/Agent Control 全部核心测试；存储需求来自 manifest 显式字段，最终配额由 admission/policy 裁决，代码中不再有任何 `contains("pi")` 或等价名称判断。

### 阶段 4：配置所有权与规范化

**目标：** Deployment、Normalized、Domain、Adapter config 分离。

工作内容：

1. 将渠道配置移出 Cognit；
2. 将 coding adapter 配置移出 Cognit；
3. 建立统一 integration registry config；
4. secret resolution 集中于 composition；
5. 移除 URL/provider 自动猜测；
6. 为旧字段建立 deterministic normalization。

验收：领域 crate 不读取业务环境变量，不持有不属于其策略的 adapter 配置。

### 阶段 5：Supplemental memory 通用化

**目标：** Memory core 不知道具体补充记忆产品名。

工作内容：

1. 将核心字段迁移为 `supplemental`；
2. 通用 transport/service/status/error 命名；
3. MCP adapter 私有化；
4. 旧字段和旧路径由 compatibility 层读取；
5. schema fixture 改由 adapter config 声明。

验收：替换 MCP server 实例不修改 Mnemosyne 或 Executive application。

### 阶段 6：Channel、Identity 与信息源收边

**目标：** Executive application 只持有通用 port。

工作内容：

1. 将具体 OAuth/client/store 移入 adapters；
2. 按 Mail/Calendar/File/Identity 能力拆 port；
3. Gateway 具体 transport 私有化；
4. 业务层只使用 normalized identity、message、event；
5. provider cursor 与错误保持 opaque/normalized。

**安全前置：** 本阶段搬迁 OAuth 授权、token store 与 credential 处理，属安全敏感改动。credential 边界、fail-closed、token 落盘加密/权限必须保持不变；结束前需一次安全评审 checkpoint，并验证 credential 不出现在共享 DTO、事件与诊断中。

验收：核心渠道和信息源用例全部通过 fake port 测试；credential 边界与 fail-closed 行为回归测试通过。

### 阶段 7：Inference adapter 私有化

**目标：** Cognit 公共 API 不暴露具体 provider 实现。

工作内容：

1. 只公开通用 inference contract；
2. 具体 provider 模块降为 crate-private；
3. factory 只在 composition 使用；
4. transport/adapter ID 从业务 enum 迁出；
5. scheduler 只依据 capability、health、policy 路由；
6. 保留用户 provider/model 配置值。

验收：外部 crate 无需导入具体 provider struct；新增 adapter 不修改 scheduler 业务分支。

### 阶段 8：高风险大模块状态机化

**目标：** 降低单文件隐式状态和旁路路径风险。

按以下顺序逐个设计、逐个实施：

1. Agent Control；
2. Turn Pipeline；
3. Mnemosyne service；
4. MCP client/auth；
5. Daemon server。

每个模块必须先画出状态、事件、owner、I/O port，再进行拆分。不得在一个提交中同时重构多个状态机。

**排序约束（与 §13 DAG 一致）：** 上述顺序是优先级，不是可提前执行的许可。每个模块必须排在**改动其端口的那个阶段之后**——Agent Control 在 Phase 2、3 之后；Mnemosyne service 在 Phase 5 之后；MCP client/auth 在其 adapter 化之后；Daemon server 在 Phase 2 之后。端口未稳定就先做状态机拆分，会在后续阶段被推倒重来。

### 阶段 9：公共 API 收缩

**目标：** `impl`、具体 adapter 和 repository 不再作为公共稳定面。

工作内容：

1. 建立 crate facade（形态见 §8）；
2. 将实现模块改为 private/crate-private；
3. 迁移跨 crate import；
4. compatibility re-export 标记 deprecated；
5. 门禁保证旧路径调用点只减不增；
6. 调用点归零后删除旧导出；
7. **拆除主要范围内的 `impl/` 顶层容器**：`cognit`、`executive`、`mnemosyne` 必须完成（拆入其实际需要的职责层，不强制创建空目录）；`dasein`、`metacog` 只要求收敛公共导出，完整物理拆分不属于本轮强制范围。

验收：以上 5 个 crate 的 crate root 都不再出现 `pub mod impl` 或 `pub use impl::*`；跨 crate 不存在对任何 `impl` 路径的依赖（对应 §15.1.8）。

### 阶段 10：全局验证与是否拆 crate 的复评

**目标：** 验证架构收益，而不是预设必须物理拆分。

评估指标：

- Cargo 依赖边是否减少；
- domain/application 的基础设施依赖是否归零；
- 公开 API 面积是否下降；
- 外部 adapter 是否可独立 contract test；
- 核心测试是否全部使用 fake port；
- 修改一个 provider 是否只影响 adapter；
- 编译时间和变更影响范围是否改善。

只有证据表明独立编译、所有权或发布边界有实际收益时，才进一步拆分 Fabric 或 Executive crate。

---

## 14. 迁移与兼容策略

1. 不在同一阶段同时改变核心 DTO、数据库 schema、配置 schema 和外部 wire protocol；
2. 先引入 canonical model 和双读转换，再迁移调用方，最后删除旧入口；
3. 旧配置 alias 只存在于 compatibility parser，不传播到运行时对象；
4. 持久化迁移必须幂等、可检测版本、失败时保持原数据；
5. wire protocol 变更必须版本化；
6. 公开 API 先提供 facade，再迁移调用方，最后收缩可见性；
7. 每个兼容层必须有调用点计数和退出条件；
8. 不允许以长期 `serde_json::Value` 作为迁移捷径；
9. 不允许静默 fallback 到其他 provider/runtime；
10. 所有安全相关行为保持 fail closed。

**fail-closed 与 degrade 的边界（消除歧义）：** integration 必须在配置中显式标注 `required` 或 `optional`：

- `required`（如主推理 provider、身份、授权、安全策略）：配置缺失或校验失败时 **fail closed = 拒绝启动/拒绝该请求**，并给出明确错误，绝不静默降级或换实现。
- `optional`（如默认关闭的 supplemental memory）：缺失时**允许降级运行**，但必须记录明确的 degraded 状态；一旦配置存在却校验失败，则按 `required` 处理（fail closed），不得因"可选"而吞掉错误配置。
- 判定"安全相关"的口径：凡影响 credential、授权 scope、sandbox/network policy、workspace trust 的行为一律按 `required` fail closed，不因所属 integration 标为 optional 而放宽。

---

## 15. 项目级强制约束

### 15.1 依赖与所有权

1. 每个状态、配置、协议和 repository 必须有唯一 owner；
2. 外层实现依赖内层接口，内层不得依赖外层实现；
3. application 只通过 port 访问 I/O；
4. composition 是具体对象构造和 secret resolution 的唯一位置；
5. host 不实现领域规则；
6. adapter 不决定业务授权，只执行已授权能力并转换协议；
7. Fabric 不承载外部供应商模型；
8. 不新增跨 crate `impl` 路径依赖。

### 15.2 外部集成

1. 外部项目名称仅允许存在于 adapter、部署、兼容与专项测试；
2. 标准协议名称允许存在于协议实现和通用配置；
3. 外部错误必须在 adapter 边界归一化；
4. 外部 ID/cursor 对核心保持 opaque；
5. credential 不进入共享 DTO、事件或诊断；
6. provider 名称、URL 和日志字段不得参与核心业务分支；
7. 新 provider 不得要求修改核心领域枚举；
8. adapter 的 wire 类型不得从 crate root 导出。

### 15.3 变更纪律

1. 每阶段必须独立可编译、可测试和可回滚；
2. 不得混入无关重命名或格式化；
3. 不得覆盖用户已有未提交修改；
4. 每次先写窄范围失败测试，再完成最小迁移；
5. 非平凡阶段使用独立、可审查的提交；
6. 提交正文必须说明问题、方案和具体变更；
7. Rust 命令必须通过 `scripts/cargo-agent.sh`；
8. 只有集成验证 owner 运行 workspace-wide 检查；
9. 架构门禁失败不得通过扩大目录 allowlist 绕过；
10. 任何例外必须有退出条件。

---

## 16. 总体验收标准

完成本方案后，必须满足：

1. Fabric 不包含外部供应商/项目专属类型、scope 或错误；
2. application 层（协调器/用例）只通过 port 访问 I/O，不导入具体 adapter、repository 或 wire parser；domain 层不含任何 I/O；（"核心"一词在本方案统一指 domain + contract + application 三层，不含 adapter/composition/host）
3. Goal 与 Agent Control 不知道具体 coding runtime 名称；
4. Cognit 不知道具体消息渠道；
5. Memory core 不知道具体补充记忆产品；
6. Hardware core 不知道 ROS、机器人产品或仿真器名称；
7. 新增同类 provider 不修改核心领域枚举；
8. provider 故障只通过通用错误分类影响核心；
9. 配置读取、secret 解析和 adapter 构造集中于 composition；
10. domain/application 不直接依赖 HTTP、数据库或用户目录基础设施；
11. crate root 不公开整个实现/adapter tree；
12. 旧配置、旧持久化和旧 API 均有明确兼容或明确拒绝行为；
13. architecture check 能阻止同类耦合重新进入；
14. 核心用例可完全使用 fake ports 测试；
15. 替换一个外部实现只修改对应 adapter、配置和 contract test；
16. 大状态机模块具有唯一状态 owner 和明确事件转换；
17. 每个迁移阶段都有窄范围验证证据；
18. workspace 最终验证遵守仓库 Rust 资源策略。

---

## 17. 明确不做

本方案不要求：

- 删除 MCP、gRPC、OAuth、HTTP 等标准协议名称；
- 删除部署配置中的 provider/model 实例值；
- 把所有 adapter 拆成独立仓库；
- 立即物理拆分 Fabric 或 Executive crate；
- 引入动态插件 ABI 或依赖注入框架；
- 一次性重写所有配置和数据库；
- 为文件变短而机械拆文件；
- 全局替换所有 `anyhow`；
- 重命名所有领域 crate；
- 在一次提交中重构多个状态机；
- 为追求“零名称”牺牲部署可读性和诊断能力。

### 17.1 范围边界

本方案按改动性质划分范围，而不是简单划分“在/不在”。这样做的原因是：外部耦合横跨 contract、adapter 和 wire；完全排除 Corpus、Runtime 或 Kernel 会与 Phase 3/8 冲突，而把所有 crate 都列为主要重构范围又会造成无边界扩张。

| 范围类别 | crate | 允许的改动 |
|---|---|---|
| 主要重构范围 | `fabric`、`executive`、`cognit`、`mnemosyne`、`hardware`、`gateway` | 按 Phase 1–9 调整契约、application、composition、adapter 和公开 API |
| 专项触及范围 | `corpus` | 仅处理 Google/外部信息源 adapter、MCP adapter 以及 Phase 8 明确列出的 MCP client/auth 状态机 |
| 专项触及范围 | `runtime` | 固定 RuntimeContract owner，按 Phase 3 调整 manifest/capability contract；不并入 Executive |
| 专项触及范围 | `kernel` | 仅核对生命周期治理 owner 和必要的术语/facade；不把 KernelRuntime 重构成 Host adapter |
| 兼容验证范围 | `interact`、`aletheon`、`execd` | 仅在所属 wire contract 改动时适配、验证或明确拒绝旧版本 |
| 兼容验证范围 | `platform` | 作为 OS host capability owner 核对依赖边；除非具体 adapter 迁移需要，不主动重构内部结构 |
| 明确不主动重构 | `agora`、`dasein`、`metacog` | 仅在编译或稳定 facade 迁移所必需时做最小调用点调整，并单独登记原因 |

---

## 18. 后续文档与实施方式

本文件是总设计和项目约束，不应直接作为一个巨型实施任务。后续应为阶段 0 至阶段 10 分别编写文件级实施计划，每份计划包含：

- 精确修改路径与符号；
- 兼容输入和 canonical 输出；
- 失败测试与最小实现；
- 最窄 Cargo 验证命令；
- 架构检查更新；
- 独立提交边界；
- 回滚方式；
- 进入下一阶段的门槛。

实施顺序不得跳过阶段 0 的防扩散门禁。阶段 1 至阶段 7 可以在依赖明确时细分，但共享契约迁移必须先于删除兼容类型，应用端口必须先于移动具体 adapter，稳定 facade 必须先于收缩公共 API。

---

## 19. 实施前必产出的基线（Phase 0 前置产物）

以下三张表是本方案从“原则”落到“可执行”的关键，必须在 Phase 0 产出并随迁移更新。缺任何一张，对应阶段不得开工。

Phase 0 的机器可读冻结产物位于 `config/architecture/`：

- `module-boundaries.txt`：workspace crate、公开模块、依赖与 Runtime/Kernel/Executive/Platform 所有权；
- `wire-surfaces.tsv`：wire/internal-shared 分类、参与方、协议 owner 与兼容规则；
- `persistence-surfaces.tsv`：schema/version owner、读写方与迁移规则；
- `external-identifiers.txt`：受保护外部标识、合法 adapter 区域与中性替代项；
- `compatibility-debt.tsv`：逐文件兼容债务、计数基线及退出阶段；
- `metrics.env`：由 `scripts/architecture-check.sh` 强制执行的精确棘轮指标。

这些文件均记录冻结提交；降低债务必须同步降低基线，增加或遗漏 owner 会直接导致架构门禁失败。对应正反 fixture 由 `tests/architecture_check.sh` 维护。

### 19.1 持久化面清单（最高风险）

枚举所有已落盘/跨会话格式，标注受影响阶段与迁移策略。候选面（Phase 0 补全并核对）：

| 持久化面 | 位置/证据 | 版本机制 | 受影响阶段 | 策略 |
|---|---|---|---|---|
| 外部事件 | `crates/fabric/src/types/external_event.rs`（`EXTERNAL_EVENT_SCHEMA_VERSION`，内嵌 `GmailMessageSummary`） | 有 schema version | 1 | 双读 + 版本化迁移 |
| gbrain spool | 配置：`crates/cognit/src/config/mod.rs`；schema owner：`crates/mnemosyne/src/backends/gbrain/migrations.rs:92` | 已有 SQLite migration；完整版本路径由 Phase 0 清点 | 5 | compatibility 读旧、写新，并验证现有 migration 幂等性 |
| agent run repo | `crates/executive/src/service/agent_control/sqlite_repository.rs:31` | Phase 0 检查 schema/migration 机制并记录结果 | 2、3 | 先冻结现有 schema；只有 DTO 落盘形态变化时增加版本化 migration |
| OAuth token store | Corpus credential/token store 与 Executive external composition | Phase 0 追踪实际表/文件 owner，不凭配置推断 | 6 | 加密、文件权限和 fail-closed 行为不变，增加迁移与泄漏回归测试 |
| memory store | Mnemosyne repository/backends | Phase 0 按 backend 列出 schema/version owner | 5 | 仅迁移被 supplemental-memory 命名或 DTO 变化影响的格式 |

每条必须满足 §14 第 4 项：幂等、可检测版本、失败保持原数据；同时满足 §14 第 9–10 项：不可静默 fallback，安全相关行为保持 fail closed。

### 19.2 wire 面分类

把 Fabric 中所有跨边界类型分为 `wire-exposed` / `internal-shared`（判据见 §4.2）。至少覆盖 `protocol::client`、`external_event`、`external_identity`、`channel`、`llm_types`。每个 `wire-exposed` 条目必须记录参与方、序列化路径、`protocol_owner`、当前版本机制和兼容策略；只有属于客户端协议的类型才绑定 `CLIENT_PROTOCOL_VERSION`，其他类型跟随各自的 execd、protobuf、MCP、事件或数据库 schema owner。

### 19.3 量化基线指标

Phase 0 提交一次快照，每阶段出 delta，用数字而非清单勾选证明耦合下降。至少包含：

| 指标 | 采集方式 | 目标趋势 |
|---|---|---|
| core 路径外部产品名命中数 | 门禁 grep（§11.3 路径集） | → 0 |
| `pub` 的 `impl`/adapter 项数 | 模块可见性扫描 | 下降 |
| domain/application 对 reqwest/rusqlite/dirs 的依赖边 | cargo 依赖分析 | → 0 |
| `contains(provider)` / `match provider_name` 命中数 | 门禁 grep（当前已知：`agent_control/mod.rs:747`） | → 0 |
| Fabric 外部供应商类型数 | 类型扫描 | → 0 |
| 兼容 allowlist 条目数与调用点数 | allowlist 台账 | 只减不增 |

这些指标应接入 `scripts/architecture-check.sh` 的基线，使“回退”在 CI 中可被机械拦截。

---

## 20. 附录：各 crate 目标内部结构

以下是“主要重构范围”六个 crate 的目标态内部结构，基于当前真实布局给出迁移方向。目标不是照抄目录名，而是让每个模块唯一归入一个职责层并满足依赖方向；只创建实际需要的层。迁移期间旧路径可经 compatibility re-export 保留，新代码进入目标层。

附录以 Phase 0 inventory 所在提交为冻结基线。Phase 0 前产生的新模块（包括当前工作区中的 Cognit ports/policy/proto 和 embodiment 相关模块）必须重新归类，不能仅凭临时工作区路径直接认定最终 owner。

### 20.1 fabric（纯契约,收缩为 contract-only）

现状顶层:`contract/ types/ include/ events/ ipc/ kernel/ policy/ protocol/ primitives/ security/ dasein/`。问题:承载了运行时/OS 依赖(§11.1 现状警示)与 `types::google` 等外部类型(§2.1)。

```text
fabric/                      # 只保留稳定契约，无 tokio(full)/nix/libc/toml/png
├── contract/                # 跨边界 trait/port（inference/channel/identity/memory/coding/device）
├── types/                   # 通用 DTO（移除 google、去除 external_event 对 Google 的内嵌）
├── protocol/                # wire-exposed 协议（客户端协议在此,标 protocol_owner）
├── events/                  # 通用事件（ExternalEvent 改通用 payload）
├── errors/                  # §7 的 IntegrationFailureKind；领域错误仍归各 owner
└── primitives/policy/security（仅保留稳定契约部分）
```

Fabric IPC 必须在 Phase 0 分类后迁移，不能整体下沉 Platform：DTO/envelope/protocol 留在 Fabric；进程内 mailbox/bus 由 Kernel 或 Executive runtime owner 承接；Unix socket、文件描述符等 OS transport 由 Platform/Host adapter 承接。原因是 Platform 只拥有 OS capability，不能反向承载应用通信语义。

### 20.2 executive（拆分最重,§13.2 主战场）

现状顶层：`core/ host/ impl/ service/ tools/ user_runtime/`，其中 `impl/` 混装 daemon/external/gbrain/google/runtime/goal/channel 等模块（§2.2）。

```text
executive/
├── domain/                  # Goal/Turn/Agent 领域状态与规则（来自 service/ 的纯逻辑）
├── application/             # 用例与协调器 + 应用 port（service/ 的协调部分、impl/goal、impl/orchestration）
├── adapters/                # impl/{external,google,gbrain,runtime,channel,daemon 的 client 部分}
│   ├── google/  identity/  channel/  coding_runtime/  supplemental_memory/
├── composition/             # core/config 归一化、factory、secret 解析、registry（唯一 match adapter-id 处）
├── host/                    # impl/daemon/server + bin bootstrap → HostRuntime（进程/RPC/信号）
└── compatibility/           # 旧 config/路径/持久化迁移;旧 impl 路径 re-export（登记退出条件）
# user_runtime/ 归入 composition 的 DomainConfig（§9 映射）;顶层 impl/ 消除
```

### 20.3 cognit（推理域）

现状:`config/ core/ harness/ impl/ ports/ bridge/ testing/`,`impl/llm` 公开具体 provider(§3.4),渠道/Pi 配置混在 `config`(§2.4/§3.5)。

```text
cognit/
├── domain/                  # core/ 的推理领域逻辑、harness 纯策略
├── contract/                # ports/ 中的 InferenceProvider 等通用 trait + 通用 DTO/错误
├── adapters/                # impl/llm/{anthropic,ollama,openai_provider} 降为 crate-private
├── composition/             # provider_factory、scheduler 路由（按 capability/health,不按名称）
└── compatibility/           # 渠道配置迁出 cognit（→ gateway）、Pi 配置迁出（→ executive adapter）
# impl/ 消除;crate root 不再导出 impl::llm
```

### 20.4 mnemosyne（记忆域）

现状:`impl/{core_memory,fact_store,vector_store,recall_memory,…}`、`backends/{gbrain,…}`、`service.rs`(1394 行)、`recall/ consolidation/ retention/ projection`。`backends::gbrain` 公开(§3.3)。

```text
mnemosyne/
├── domain/                  # 记忆领域模型 model/、consolidation/retention/promotion 规则
├── contract/                # SupplementalMemoryTransport 等 port（去 gbrain 命名）
├── application/             # service.rs 按状态机/端口拆分（Phase 8 第 3 项）
├── adapters/                # backends/{mcp（原 gbrain）, vector_store, fact_store} → crate-private
└── composition/             # backend 选择与构造
# impl/ 消除;backends::gbrain 改 backends::mcp,旧名走 compatibility
```

### 20.5 hardware（本体域,已较干净)

现状无 `impl/`,但通用错误含 `ROS master unreachable`(§3.6)。

```text
hardware/
├── domain/                  # device/observation/command/lease/safety/emergency_stop 领域
├── contract/                # provider.rs（DeviceProvider）、capability、gRPC contract
├── adapters/                # grpc/（错误归一化为 ProviderUnavailable 等,清除 ROS 字符串）、simulator
└── composition/             # registry、broker、deployment_gate 的构造部分
# ROS/topic/厂商类型不得进入 domain/contract
```

### 20.6 gateway（渠道域,已具 ports.rs)

现状:`ports.rs store.rs registry.rs dispatcher.rs telegram/ handlers/`,具体 telegram transport 由 crate 公开(§3.5)。

```text
gateway/
├── domain/                  # intent/effect/notify 领域逻辑
├── contract/                # ports.rs（ChannelTransport）已接近目标,保留并收敛可见性
├── adapters/                # telegram/ 降为 crate-private（polling/token/callback 在此）
├── composition/             # registry/ dispatcher 构造
└── store.rs                 # 归入 adapters（具体持久化）
# crate root 不再公开具体 transport（§3.5 目标）
```

> 这些树是方向而非硬性文件清单;每个 crate 的精确拆分在其所属阶段的文件级实施计划(§18)中细化,并遵守 §14"先 canonical 后迁移、最后收缩"的顺序。
