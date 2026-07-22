# Aletheon 耦合、外部接口与模块化审计

> 日期：2026-07-21
> 状态：当前代码静态审计，不是实施完成声明
> 范围：crate 依赖、Provider、外部通信、配置与凭据、网络策略、进程边界、超大模块及架构文档漂移

## 1. 执行摘要

Aletheon 已经具备可运行的 core、用户 daemon、Pi、MCP/GBrain、记忆与定时闭环，但代码结构仍处于“生产能力已接通、架构边界尚未完全收敛”的阶段。

```text
能力闭环：      已成立
安全基础：      已存在
领域 crate：    已初步成形
统一配置：      部分成立
外部接口治理：  尚未统一
模块职责：      Executive / Corpus 过重
长期扩展成本：  偏高
```

最重要的结论不是全面重写，而是按顺序完成三项收敛：

1. 统一 Provider 配置、解析与创建入口；
2. 建立 host-owned 的出站通信、端点、凭据和健康治理边界；
3. 将 Executive 从“实现中心”收缩为真正的 composition root。

## 2. 审计方法与量化信号

本次以当前工作树代码为准，检查 Cargo 直接依赖、公开 trait/config、`std::env`、HTTP client 构造、进程调用和生产文件体积。统计值是定位信号，不是单独的质量判定：

- `crates/executive/src`：约 78,893 行 Rust；
- `crates/corpus/src`：约 38,615 行 Rust；
- 生产代码直接 `std::env::var/var_os`：约 61 处；
- 独立 reqwest client 构造信号：约 14 处；
- 超过 1,000 行的 Rust 文件：26 个。

## 3. 当前依赖现实与旧文档冲突

旧架构文档是历史快照，部分结论已被代码修复，但文档没有同步。

| 旧文档描述 | 当前代码现实 | 一致？ |
|---|---|---|
| Corpus 依赖 Cognit、Mnemosyne、Platform（`docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md:46-48,66-67`） | Corpus 当前只直接依赖 Fabric、Kernel、Platform（`crates/corpus/Cargo.toml:9-12`） | 否 |
| Execd 依赖整个 Corpus（`docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md:55,68-69`） | Execd 当前直接依赖 Platform，不再依赖 Corpus（`crates/execd/Cargo.toml:8-16`） | 否 |
| MCP 配置 schema 属于 Cognit（`docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md:135-152`） | canonical `McpServerConfig` 已在 Corpus（`crates/corpus/src/tools/mcp/config.rs:11-39`） | 否 |
| Executive 是广泛 composition root（`docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md:103-123`） | Executive 仍直接依赖十个领域 crate（`crates/executive/Cargo.toml:9-19`）并承载大量实现 | 是，但实现仍过重 |

因此，后续架构决策不能继续直接引用这份旧快照，必须先刷新依赖图和已完成事项。

## 4. 当前系统结构

```text
                            +-------------------+
TUI / CLI ----------------->|     Executive     |
                            | orchestration +   |
System Core / Provider ---->| much implementation|
                            +--+---+---+---+----+
                               |   |   |   |
           +-------------------+   |   |   +----------------+
           v                       v   v                    v
        Cognit                  Corpus Mnemosyne          Dasein
     inference/reasoning       tools/MCP memory          identity
           |                      |
           v                      +--> Google/Web/Process/Filesystem
     external LLM

Executive additionally composes Pi, Goal, approvals, hooks, sessions,
worktree recovery, Google integration, channels and runtime settlement.
```

结构方向基本正确；主要问题是外部能力与领域实现通过多个独立入口接入，并在 Executive bootstrap 中集中组装。

## 5. P0：Provider 配置和创建逻辑重复

### 5.1 两份 ProviderConfig

实际应用 schema 位于 `crates/cognit/src/config/mod.rs:680-699`，字段包括：

```text
name, base_url, api_key, transport, models,
max_context_length, pricing
```

另一份 inference 配置位于 `crates/cognit/src/impl/inference/provider_config.rs:10-21`，字段包括：

```text
id, name, provider_type, model, api_url,
max_context_length, cost_per_1k_tokens, latency_ms
```

两个同名类型表达相近概念但不可互换，容易造成配置、调度和运行时指标分别演进。

### 5.2 两套 Provider factory

`ProviderRegistry::create_provider` 位于 `crates/cognit/src/impl/provider_registry.rs:149-179`：

- 支持 OpenAI/Anthropic；
- 应用 provider timeout、max tokens、max context；
- `Auto` 通过 URL 后缀识别 Anthropic。

另一套 `provider_factory::create_provider` 位于 `crates/cognit/src/impl/llm/provider_factory.rs:28-68`：

- 支持 OpenAI/Anthropic/Ollama；
- Ollama 通过 `localhost:11434` 或 `127.0.0.1:11434` 字符串判断；
- 没有与 Registry 完全相同的 timeout/token 配置路径。

同一配置经不同入口可能产生不同 Provider 类型和运行参数，这是确定的行为分叉风险。

### 5.3 URL 启发式硬编码

- `crates/cognit/src/impl/provider_registry.rs:15-26`：URL 以 `/anthropic` 结尾才视为 Anthropic；
- `crates/cognit/src/impl/llm/provider_factory.rs:11-25`：包含本机 11434 才视为 Ollama。

URL 是部署地址，不应成为协议类型的权威。项目已有显式 `Transport`（`crates/cognit/src/config/mod.rs:670-678`），生产路径应优先使用显式声明，`Auto` 仅保留为兼容模式。

### 5.4 凭据名称与 Provider 名耦合

`crates/cognit/src/impl/provider_registry.rs:181-188` 将 provider name 大写后拼成 `<NAME>_API_KEY`。`provider_factory.rs:100-106` 又实现一次相同逻辑。

这会把逻辑身份、部署命名和 secret backend 绑定在一起。推荐改成显式引用：

```toml
[[providers]]
name = "leju"
transport = "openai"
credential = "provider/leju"
```

由 `CredentialResolver` 将引用解析到 systemd credential、环境变量或 vault，领域代码不直接知道 secret 来源。

## 6. P1：外部通信缺少统一治理出口

当前不同外部能力分别创建客户端：

| 外部能力 | 当前实现位置 |
|---|---|
| LLM OpenAI/Anthropic/Ollama | `crates/cognit/src/impl/llm/` |
| MCP/GBrain | `crates/corpus/src/tools/mcp/transport.rs` |
| Google | `crates/corpus/src/tools/google/client.rs` |
| Telegram | `crates/gateway/src/telegram/mod.rs:23-65` |
| Web search/fetch | `crates/corpus/src/tools/tools/web_search.rs`, `web_fetch.rs` |
| Automation delivery | `crates/executive/src/impl/automation/delivery.rs:15-74` |
| Vector/embedding | `crates/mnemosyne/src/impl/vector_store.rs:144-146` |

这些模块分别处理 timeout、redirect、retry、endpoint、认证、错误分类和健康状态。协议适配应保留在各自领域，但横切治理不能继续复制。

### 6.1 NetworkPolicy 覆盖不完整

全局 NetworkPolicy 定义于 `crates/fabric/src/types/network_policy.rs:15-53`，默认拒绝。项目层配置不能自行提升网络权限，见 `crates/executive/src/core/config/mod.rs:206-220`，这是正确的 host-owned 权限模型。

静态代码中明确接入该策略的主要是：

- WebSearch：`crates/corpus/src/tools/tools/web_search.rs:11-22,126`；
- WebFetch：`crates/corpus/src/tools/tools/web_fetch.rs:13-24,97`；
- ToolRegistry 注入：`crates/corpus/src/tools/tools/registry.rs:143-202`。

LLM、Google、Telegram、MCP 和 embedding 使用各自的可信配置/认证模型，但没有统一经过同一个出站治理端口。不能据此断言这些路径当前不安全；可以确认的是策略语义和健康观测是分裂的。

### 6.2 推荐目标边界

```text
Domain Adapter
(Google/MCP/LLM/Telegram/Embedding)
             |
             v
+----------------------------------+
| OutboundTransport                |
| - normalized endpoint identity   |
| - connect/request/idle timeout   |
| - redirect and proxy policy      |
| - TLS requirements               |
| - retry classification           |
| - network authority decision     |
| - metrics and health             |
+----------------------------------+
             |
             v
        reqwest / socket

CredentialResolver is separate and returns short-lived secret material only
after endpoint/service identity is approved.
```

不建议创建包含全部业务协议的“万能 ExternalService”。统一的是传输治理和凭据边界，而不是 Google/MCP/LLM 的业务语义。

## 7. P1：业务模块直接读取进程环境

项目已经有 typed layered config：system → user → project → `ALETHEON__` environment → CLI，见 `crates/executive/src/core/config/mod.rs:259-301`。但生产模块仍有约 61 处直接 `std::env::var/var_os` 信号。

### 7.1 Runtime Core 的兼容变量

`crates/executive/src/core/runtime_core.rs:67-87` 直接读取：

- `AGENT_WORKING_DIR`；
- `AGENT_DATA_DIR`；
- `AGENT_SYSTEM_PROMPT`；
- `AGENT_SANDBOX_PREFERENCE`。

这些值部分覆盖已加载的 AppConfig，产生“typed config 与旧环境变量谁更权威”的双重入口。

### 7.2 Google bootstrap

`crates/executive/src/impl/daemon/bootstrap/google.rs:37-59,92-95,122-124` 直接读取 Google client、redirect、secret、Drive 开关、文件 ID 和 ingress policy。Executive 因而了解具体集成的部署变量和解析规则，增加 composition root 对 Google 细节的耦合。

### 7.3 应保留的进程协议变量

不是所有环境变量读取都应迁移。以下属于宿主协议或进程发现，保留合理：

- systemd `NOTIFY_SOCKET`、`WATCHDOG_USEC`（`crates/executive/src/host/systemd.rs:32-51`）；
- XDG/HOME 路径发现；
- `DISPLAY`、`WAYLAND_DISPLAY`、`DBUS_SESSION_BUS_ADDRESS`；
- systemd credential directory；
- 子进程一次性 capability/secret handoff。

目标规则应是：

```text
Host/bootstrap 可以读取环境；
Domain service 只能接收 typed config、SecretRef 或 capability port。
```

## 8. P1/P2：Executive 仍是实现中心

Executive 对十个领域 crate 的直接依赖见 `crates/executive/Cargo.toml:9-19`。作为 composition root，这种依赖广度可以接受；不健康的是它同时承担领域实现。

`RequestHandler::new` 从 `crates/executive/src/impl/daemon/bootstrap/request.rs:65` 开始，在同一构造流程中涉及：

- inference adapter 与 model routing；
- session store；
- SelfField 与 permission authority；
- core/recall/fact memory；
- objective、approval 和 Gmail goal store；
- worktree recovery；
- hooks、skills、tools；
- agent profiles；
- native/Pi runtime；
- runtime settlement 和服务启动。

当前调用形态：

```text
RequestHandler::new
  -> create stores
  -> restore state
  -> construct domain services
  -> register integrations
  -> register tools/runtimes
  -> construct turn runtime
  -> start workers
```

目标不是把这些内容放进更多任意文件，而是形成可独立测试的 composition units：

```text
bootstrap/
  inference.rs      -> InferenceComposition
  memory.rs         -> MemoryComposition
  integrations.rs   -> ExternalIntegrationComposition
  agents.rs         -> AgentRuntimeComposition
  tools.rs          -> ToolComposition
  sessions.rs       -> SessionComposition
  request.rs        -> only order and final wiring
```

每个 composition unit 必须有明确输入/输出资源结构，不能通过全局环境重新发现配置。

## 9. P2：超大文件与职责密度

当前主要超大生产文件：

| 文件 | 约行数 | 需要验证的拆分边界 |
|---|---:|---|
| `crates/corpus/src/security/runner.rs` | 2004 | 执行、guard、审批、审计、结果规范化 |
| `crates/executive/src/service/agent_control/mod.rs` | 1680 | API facade、状态查询、生命周期路由 |
| `crates/executive/src/service/agent_control/settlement.rs` | 1523 | 终态、证据、资源回收、记忆投影 |
| `crates/corpus/src/tools/mcp/client.rs` | 1491 | connect、discover、reconnect、状态机 |
| `crates/corpus/src/tools/mcp/auth.rs` | 1447 | bearer、OAuth discovery、token lifecycle |
| `crates/executive/src/impl/daemon/server.rs` | 1400 | socket、RPC dispatch、生命周期 |
| `crates/mnemosyne/src/service.rs` | 1394 | 记忆 facade、策略和后端协调 |
| `crates/cognit/src/harness/linear/mod.rs` | 1393 | 推理循环、工具驱动、终态 |
| `crates/executive/src/service/turn_pipeline.rs` | 1388 | turn stage orchestration |
| `crates/executive/src/service/workspace_checkpoint.rs` | 1378 | checkpoint lifecycle |
| `crates/executive/src/impl/daemon/bootstrap/request.rs` | 1373 | 全局组装 |
| `crates/cognit/src/config/mod.rs` | 1300 | 多领域 schema 聚集 |

文件大不等于错误；这里的问题是多个不同生命周期或授权阶段集中在同一模块。拆分必须以状态机和职责为依据，禁止纯按行数切文件。

推荐优先级：

1. Provider factory/config 重复；
2. `bootstrap/request.rs` composition units；
3. `agent_control/settlement.rs` 的 evidence/resource/memory 阶段；
4. MCP connection/auth state machine；
5. `security/runner.rs` 的执行与治理分离。

## 10. 硬编码分类

### 10.1 合理默认

以下值是协议或宿主默认，只要可覆盖且集中定义，就不应机械移除：

- `/run/aletheon`、`/etc/aletheon/config.toml`；
- XDG fallback；
- Google/Telegram 官方 endpoint；
- MCP wire/version defaults；
- systemd unit/socket 名；
- provider 的合理 token/context 默认。

Google 默认 endpoint 位于 `crates/corpus/src/tools/google/client.rs:62-68` 和 `google/oauth.rs:18-21`。作为 adapter 内可注入默认值是合理设计。

### 10.2 应消除或收敛

- 通过 URL 字符串猜 transport/provider；
- 通过 provider name 动态拼 secret env 名；
- 同一 socket/path 在多个模块重复定义；
- 领域代码直接使用 `/tmp` 作为运行默认；
- 外部程序名 `git`、`bash`、`cargo`、`rg`、`journalctl` 分散发现；
- Google 功能开关和文件 ID 直接从环境解析；
- 同一超时/重试语义在多个 client 重复定义。

H8 已确认 `interact::tui::cli` 是由 `crates/interact/src/lib.rs:27-28` 保留的 legacy library
入口，不是当前 `aletheon` binary 的主解析器；但它仍是公开可达代码，因此没有仅以“legacy”名义忽略。
该入口现已删除固定 `/run/aletheon/aletheon.sock` 默认，并与当前主入口复用 explicit CLI →
`ALETHEON_SOCKET` → XDG runtime resolver（`crates/interact/src/host.rs:26-46`、
`crates/interact/src/tui/cli.rs:32-35,220-258`）。daemon start/status 也使用同一已解析 socket，
不再偷偷回到 system socket（`crates/interact/src/tui/cli.rs:395-445`）。

## 11. 已经健康或已改善的边界

### 11.1 InferencePort

`InferencePort` 在 `crates/executive/src/service/inference_port.rs:27-33` 隔离用户运行时和机器 provider。`PortLlmProvider` 在 `inference_port.rs:35-107` 将该端口适配回 Cognit 使用的 `LlmProvider`。凭据不跨用户 daemon inference frame，这是正确方向。

### 11.2 Platform host traits

Platform 已提供 `ProcessHost`、`FilesystemHost`、`ServiceHost`、`SandboxHost`、`DesktopHost`、`PtyHost`。新的 OS 操作应向这些 port 收敛，而不是继续在领域模块增加直接 `Command::new()`。

### 11.3 MCP schema ownership

Canonical MCP schema 已在 Corpus（`crates/corpus/src/tools/mcp/config.rs:11-39`），Executive 只通过 `crates/executive/src/core/config/infra.rs:10` 组合/重导出。旧架构文档对应待办已完成。

### 11.4 Execd 依赖

Execd 当前依赖 Platform 而非 Corpus（`crates/execd/Cargo.toml:8-16`），独立进程边界与最小 host contract 的方向正确。

### 11.5 Host-owned network authority

项目层不能提升 NetworkPolicy（`crates/executive/src/core/config/mod.rs:206-220`），避免仓库配置给自己授予出站权限。这一不变量应扩展到所有外部 adapter。

## 12. 目标架构

```text
                    AppConfig + Provenance
                            |
              +-------------+-------------+
              |                           |
       CredentialResolver          EndpointRegistry
              |                           |
              +-------------+-------------+
                            |
                    OutboundTransport
                 timeout/TLS/proxy/policy
                            |
       +----------+---------+--------+----------+
       |          |                  |          |
   LLM Adapter  MCP Adapter     Google Adapter Telegram Adapter
       |          |                  |          |
       +----------+---------+--------+----------+
                            |
                    ExternalHealthBus

Executive only composes the above ports and domain services.
Domain adapters retain protocol semantics; host infrastructure owns authority.
```

### 12.1 核心接口建议

```text
SecretRef
  logical secret identity; no raw value in AppConfig

CredentialResolver
  resolve(SecretRef, ServiceIdentity) -> scoped secret lease

ExternalEndpoint
  normalized URL + service identity + trust class

OutboundPolicy
  endpoint authority + timeouts + redirect/proxy/TLS rules

OutboundTransport
  execute approved request and emit bounded metrics/error class

ExternalHealth
  ready/degraded/unready without exposing credentials
```

这些接口应放在有明确所有权的现有 crate 内，不能为了“统一”立即创建 `common`、`external-types` 或 `http-api` 等无领域 crate。

## 13. 分阶段收敛路线

### Phase 0：刷新事实基线

1. 更新 `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` 的 Cargo 依赖图；
2. 标记 MCP ownership 和 Execd dependency 已完成；
3. 自动生成直接本地依赖清单；
4. CI 比较生成结果与受控快照，避免再次漂移。

验收：文档中的每条当前依赖都能定位到对应 Cargo 行，旧描述与代码现实不再冲突。

### Phase 1：Provider 单一真源

1. 盘点两份 ProviderConfig 的生产调用者；
2. 定义唯一 `ProviderDefinition`；
3. 合并 Registry/factory，统一 Ollama/OpenAI/Anthropic 行为；
4. transport 在生产配置中显式声明；
5. timeout/token/context/pricing 通过同一路径；
6. 引入显式 credential reference；
7. 删除重复类型和 URL heuristic 的生产依赖。

验收：给定同一 ProviderDefinition，所有入口创建相同 transport、timeouts、context 和 credential identity。

### Phase 2：凭据与环境入口收敛

1. 将 Google、search、runtime legacy override 转成 bootstrap typed config；
2. 定义 `SecretRef` 和 `CredentialResolver`；
3. 领域模块不再直接读取业务 secret env；
4. 保留 systemd/XDG/display 等 host protocol env；
5. 加静态架构检查允许列表。

验收：新增业务环境变量只能在 config/host adapter 中解析；日志、配置诊断和错误不包含 secret value。

### Phase 3：统一出站治理

1. 定义 endpoint identity、transport policy 和 error taxonomy；
2. 先迁移 MCP 与 Google；
3. 再迁移 Telegram、automation、embedding；
4. 最后评估 LLM provider 是否共享 transport factory；
5. 全部接入统一 metrics/health；
6. 保留 adapter 自己的协议重试语义，但由 transport 限定上限。

验收：所有生产 HTTP 出口均有可定位的 host-owned policy、timeout、TLS/redirect 规则和健康信号。

### Phase 4：Executive composition 收缩

1. 为 inference/memory/integrations/agents/tools/session 定义资源结构；
2. 逐个从 `RequestHandler::new` 提取 composition unit；
3. 每个 unit 建立独立构造测试和故障注入；
4. RequestHandler 只保留顺序、依赖传递和最终 facade；
5. 禁止提取后的模块重新读取全局环境。

验收：bootstrap 顶层可以通过一张资源图说明，任一 integration 构造失败有明确错误域，其他 optional integration 可按契约降级。

### Phase 5：大模块按状态机拆分

- Settlement：terminal decision → evidence persistence → resource cleanup → memory projection；
- MCP：connection → authentication → discovery → active → reconnect/backoff；
- Tool runner：authorize → sandbox → execute → bound output → audit；
- Turn pipeline：assemble → infer → tool loop → persist → emit terminal event。

验收：每个阶段有输入、输出、失败分类和独立测试，不共享隐式可变全局状态。

## 14. 建议的架构不变量

应由 `scripts/architecture-check.sh` 或专门静态检查执行：

1. Corpus 不依赖 Cognit、Mnemosyne 或 Executive；
2. Execd 不依赖 Corpus 或 Executive；
3. Platform 不依赖领域实现；
4. Runtime manifest crate 不依赖 Executive adapter；
5. 业务模块禁止直接读取未登记环境变量；
6. 新 reqwest client 构造只能出现在受控 transport/provider adapter 目录；
7. 项目配置不能提升 network、sandbox 或 credential authority；
8. raw secret 不进入 AppConfig schema、argv、日志、evidence；
9. Executive 可以组合领域，不重新实现领域机制；
10. 新 crate 必须有真实生产调用者和不可由内部模块满足的隔离价值；
11. 超大文件增长必须解释职责边界，不能只凭阈值强拆；
12. 架构文档的当前依赖图必须由机器验证。

## 15. 风险与非目标

### 风险

- 一次性替换所有 HTTP client 会扩大回归面；
- 错误地统一协议层会形成新的“万能模块”；
- 凭据迁移若没有兼容期会破坏现有 systemd 环境文件；
- 拆 Executive 时可能改变启动顺序和恢复顺序；
- Provider 合并可能暴露历史入口行为差异。

### 非目标

- 不重写整个项目；
- 不为了减少行数创建大量 crate；
- 不删除合理的官方 endpoint 和 OS 默认值；
- 不让 repository/project config 获得 host authority；
- 不在边界收敛过程中改变 Pi、Goal、memory 的终态语义；
- 不以 mock 测试代替真实 provider/MCP/daemon 验收。

## 16. 架构收敛的首个核心工作包

仅从架构依赖关系看，首个核心工作包是“Provider 单一真源”，理由：

- 重复证据明确；
- 范围集中在 Cognit/config；
- 能立即消除行为分叉；
- 为后续 CredentialResolver 和 OutboundTransport 提供稳定输入；
- 不需要先拆 Executive 大模块。

其实施定义统一由 `docs/plans/2026-07-21-production-readiness-hardening.md` 的 H2
维护；若 H0/H1 发现阻断项，应先按该队列处理。该工作包至少覆盖：

```text
ProviderDefinition
ProviderRegistry
ProviderFactory
explicit Transport
CredentialRef compatibility
timeouts/context/pricing
OpenAI/Anthropic/Ollama contract tests
```

Provider 收敛后再做外部通信治理，避免在 Provider 类型仍分裂时搭建过早的统一传输层。
本节表达架构依赖，不建立与 hardening 文档竞争的执行优先级。

## 17. 最终结论

Aletheon 当前不是“架构不可用”，而是“运行能力领先于边界治理”：

```text
已经可以真实使用和调用 Pi；
但继续按现有方式增加外部集成，会扩大配置、凭据、网络和 bootstrap 耦合。
```

应优先停止新增第三套 Provider/HTTP/config 入口，先统一已有入口，再逐步迁移。正确的长期结构是：

```text
Host owns authority and transport governance;
Domains own protocol semantics;
Executive owns composition and global settlement;
Fabric owns only genuinely cross-domain contracts.
```

只要按 Phase 0→5 小步收敛并保持现有实机闭环验收，项目可以在不全面重写的前提下显著降低耦合和长期维护成本。

---

# Part II：运行时安全事实补充（2026-07-22 校正）

本部分是对当前工作树的可复现静态检查，不宣称运行过动态故障注入。它只记录 Part I
未覆盖的运行时安全证据；实施顺序和验收以
`docs/plans/2026-07-21-production-readiness-hardening.md` 为唯一工作队列。

## 18. 并发与异步任务生命周期

### 18.1 后台任务监督现状（P1，MCP 已收敛）

H4 已将 MCP health、notification 和 initial-reconnect 任务收敛到
`McpTaskSupervisor`：任务名、终止原因、取消、panic 降级、健康恢复和有界 shutdown 见
`crates/corpus/src/tools/mcp/supervisor.rs:14-290`，生产 MCP client 已无直接
`tokio::spawn`（静态门禁见 `scripts/architecture-check.sh:107-112`）。故障注入与正常重连证据
记录在 `docs/deployment/hardening-h4-mcp-supervision-2026-07-22.md`。

reasoning log rotation 和 perception runtime 仍存在直接后台任务路径
（`crates/executive/src/impl/session/observability/reasoning_logger.rs:99,111`、
`crates/executive/src/core/runtime_core.rs:200,213`）。它们按 H4 的明确非目标保留，后续应在各自
故障证据成立时复用同一模式，不能仅凭 `spawn` 数量判定为 P0。

`crates/mnemosyne/src/service.rs:791` 还在 async 路径执行同步 rusqlite 工作。
这不是已证实的死锁，但慢 I/O 可能占用 executor worker，属于需要压测确认的 P1 调度风险。

## 19. 持久化与数据完整性

### 19.1 GBrain migration 已事务化（H5 已收敛）

GBrain 的建表、逐列补列、数据回填和 `user_version` 更新现在位于同一个 `IMMEDIATE`
事务内（`crates/mnemosyne/src/backends/gbrain/migrations.rs:25-182`）。迁移仍保留幂等建表和
列存在性检查；高于当前二进制支持的版本会在写入前 fail closed
（`migrations.rs:33-44,185-202`）。

故障注入逐一覆盖 14 个迁移边界，证明失败后版本仍为 v1，释放连接再打开可以完成 v2
升级并保留回填值；另有更高版本不被修改的用例
（`migrations.rs:205-294`）。因此原先的 P1 中间 schema 风险已经由 H5 关闭，而不是被
重新分类为 P0。

### 19.2 SessionStore 数据库结构与记录版本已分离（H5 已收敛）

`CanonicalSessionStore` 现在以 `user_version=1` 管理数据库结构，在同一事务中完成建表和
版本推进，并在打开时拒绝更高版本或声称 v1 但缺列的数据库
（`crates/executive/src/impl/session/canonical_store.rs:19-139`）。旧的未版本化三表数据库会
原位标记为 v1，不改变 record JSON；session/item 仍分别校验 `SESSION_SCHEMA_VERSION`
（`canonical_store.rs:45-69,467-574`）。

### 19.3 完整性检查保持显式离线诊断（H5 已决策）

没有损坏样本或延迟证据支持把大小相关的扫描加入每次 open。H5 因此保持两个 open 路径只做
迁移/结构验证，并提供显式只读 `PRAGMA quick_check` 脚本
（`scripts/aletheon-sqlite-check.sh:1-37`）。脚本要求传入现存数据库，建议停止 owner service
或检查一致性备份；它不会由 daemon 高频启动路径自动调用。SQLite WAL 的崩溃恢复语义也不
被错误等同于完整性扫描。

## 20. 网络安全补充（条件性 P1）

`NetworkPolicy` 继续实现默认拒绝、host/protocol/port/DNS 检查
（`crates/fabric/src/types/network_policy.rs:53-104`）。H7 没有把业务协议合并成万能服务，而是在
Corpus 内加入最小 `EndpointPolicy`：公共、loopback 和显式可信私网是不同 trust class，公共
地址策略拒绝 loopback、link-local/metadata、RFC1918、CGNAT、IPv6 ULA/loopback
（`crates/corpus/src/tools/outbound.rs:12-145,179-225`）。

H7 先迁移 MCP 与 Google。策略在初次批准时解析并检查全部 DNS 结果，同时 reqwest 自定义
resolver 在实际连接解析时再次检查，redirect 被统一禁用，超时被封顶为 30 秒
（`crates/corpus/src/tools/outbound.rs:99-176`）。MCP 依据现有 `McpTrustLevel` 选择 endpoint
trust class，并在 bearer/OAuth 构造前批准 endpoint；OAuth endpoint 也先批准再解析环境凭据
（`crates/corpus/src/tools/mcp/client.rs:49-134,152-202`）。Google API 与 OAuth 同样在读取或发送
credential 前批准请求 endpoint（`crates/corpus/src/tools/google/client.rs:91-145,257-278`、
`crates/corpus/src/tools/google/oauth.rs:68-110,128-236`）。

其余候选按收益复核后未混入 H7：Telegram transport 仍在 Gateway 内自行持有 client，且 token
嵌入请求 path（`crates/gateway/src/telegram/mod.rs:22-80`），应作为下一次跨 crate transport port
设计的首要候选；automation delivery 明确标为 parked/future 且多个 channel 仍是 placeholder
（`crates/executive/src/impl/automation/delivery.rs:9-16,29-88`）；Qdrant 仅在
`vector-qdrant` feature 下构造 client（`crates/mnemosyne/src/impl/vector_store.rs:142-163`）。
这些路径没有被宣称已统一治理；H7 只完成计划要求的 MCP/Google 首批迁移与后续收益评估。

## 21. 错误可见性与有界存储

H6 已逐点确定 Executive 候选的错误契约：

- contributor 明确请求的 lifecycle event 发布失败向上返回；effect audit event 与原始 turn
  error 上的 abort hook 为 warn-and-continue，不能覆盖主错误
  （`crates/executive/src/service/turn_pipeline.rs:119-159,760-787,1017-1036`）；
- approval oneshot receiver 已消失时，pending 记录被终结为 `ConsumerGone`，RPC 返回 false 且
  不授予 session approval；断连 deny 的同类失败记录 warning
  （`crates/executive/src/service/admin_service.rs:97-228,544-576`）；
- budget revoke 失败不再被覆盖；terminal attempt 在 settle 前持久化 reservation identity，
  settle 精确重放幂等，重复请求从原 attempt/evidence 恢复而不再次调用 runtime/Pi
  （`crates/executive/src/impl/goal/attempt_coordinator.rs:259-518`、
  `crates/executive/src/impl/goal/budget.rs:254-308`）；
- apply receipt 在 goal 终态之前持久化，失败 receipt 保留 diff hash 并进入需要 fresh
  verification/approval 的 blocked 状态；重复 callback 读取 receipt 恢复，不重复 apply
  （`crates/executive/src/impl/approval/apply_coordinator.rs:138-270,435-493`）。

memory projection 与临时 artifact 删除是明确的 best-effort 路径，但 degraded/cleanup failure
现在会告警（`apply_coordinator.rs:532-578,646-676`）。self-evolution rollback 仍属 metacog
独立 owner，不在 H6 coding apply/settle 范围内；其原有风险不应被误报为已由本批关闭。

死信和历史事件表的长期容量上限仍需要运行数据佐证。在没有增长率与磁盘预算证据前，
它们是 P2 运维容量项，不是已发生的数据故障。

## 22. 已验证的健康行为与排除项

- H9 的 Pi capability audit 不再用“present + 两个空向量”占位，而是记录 sandbox backend
  报告的 observed/unavailable 信号，并用显式 allow-list 供 verifier 比对
  （`crates/executive/src/impl/runtime/pi.rs:285-315,685-693`）。Pi report 同时预声明与 durable
  store 一致的 `coding-diffs/<job-id>.diff` 相对引用；store 在写入前复核引用与 diff hash
  （`crates/executive/src/impl/runtime/pi.rs:641-658`、
  `crates/executive/src/impl/goal/verification.rs:82-120`）。
- 配置层级已经明确定义为 system → user → project → `ALETHEON__` environment → CLI
  （`crates/executive/src/core/config/mod.rs:259-301`）；问题是部分业务环境变量绕过该入口。
- daemon 已有 health RPC；待完善的是把外部依赖的 degraded/unready 状态统一投影到健康结果，
  不是新增一个同名健康接口。
- LLM scheduler 对 429、5xx、network、timeout 和包含 `eof` 的错误做有界重试
  （`crates/cognit/src/impl/llm/scheduler.rs:31-74`）；字符串分类可用但脆弱，属 P2 类型化改进。
- `crates/mnemosyne/src/recall/pipeline.rs:842` 的新 runtime + `block_on` 位于
  `#[cfg(test)]`/`proptest!`，排除为生产问题。
- systemd watchdog 持续 ping 到进程退出是预期行为
  （`crates/executive/src/host/systemd.rs:122`），不要求单独等待该循环结束。

## 23. 两份文档的职责边界

```text
本审计文档
  └─ 当前事实、代码证据、目标边界、风险条件
                |
                v
production-readiness-hardening.md
  └─ 唯一优先级、批次依赖、验收门槛、停止条件
```

本审计不再维护另一套实施优先级。Provider、配置/凭据、任务监督、持久化、SSRF、
Executive composition 等发现全部进入 hardening 队列统一排序；若代码变化导致锚点失效，
先更新事实基线，再调整队列。
