# Aletheon 核心架构解耦与外部集成边界重构方案

**状态：** 设计基线

**日期：** 2026-07-22

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

可观测性允许记录 adapter ID 和部署实例 ID，以便诊断；这不构成业务耦合。任何日志字段都不得参与业务分支。

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
- 任何新增依赖边必须进入审查基线。

### 11.2 Rust import/module 门禁

- application 不导入 adapters；
- core/domain 不导入具体 provider；
- crate root 不公开整个 `impl` 或 adapter tree；
- repository concrete type 只能出现在 adapter/composition；
- config loader 和 environment parsing 只在 composition。

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

同时禁止核心代码：

- `contains("provider-name")`；
- `match provider_name` 决定业务策略；
- 根据 URL 识别实现；
- 根据厂商错误字符串确定重试；
- 硬编码外部 endpoint；
- 使用无边界 `serde_json::Value` 穿透 adapter。

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

---

## 13. 分阶段迁移计划

### 阶段 0：架构基线与防扩散

**目标：** 在修改现有耦合前，阻止新增同类问题。

工作内容：

1. 提交分层、目录语义和 runtime 词汇表；
2. 建立外部标识边界清单；
3. 增加依赖、import、公开导出和名称泄漏门禁；
4. 建立 compatibility/allowlist 台账；
5. 记录当前依赖基线，只允许后续减少违规。

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

验收：新增第二个同类 provider 不需要修改 Fabric enum；Fabric contract test 不引用 Google fixture。

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

验收：使用 fake coding runtime 可完成 Goal/Agent Control 全部核心测试。

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

验收：核心渠道和信息源用例全部通过 fake port 测试。

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

### 阶段 9：公共 API 收缩

**目标：** `impl`、具体 adapter 和 repository 不再作为公共稳定面。

工作内容：

1. 建立 crate facade；
2. 将实现模块改为 private/crate-private；
3. 迁移跨 crate import；
4. compatibility re-export 标记 deprecated；
5. 门禁保证旧路径调用点只减不增；
6. 调用点归零后删除旧导出。

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
2. 核心业务 service 不导入具体 adapter、repository 或 wire parser；
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
