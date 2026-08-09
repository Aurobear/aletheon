# Composition Root、Gateway、TUI/ACP 提取计划

> 状态：Draft，只有迁移设计，不包含生产代码变更
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
> 上位计划：`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`
> 目标：让 `aletheon` 成为唯一 composition root；把 daemon transport 收进 Gateway；把 TUI、CLI、ACP 收缩为 presentation/client

## 1. 结论

Agent Core 并不是主要构造在 TUI 中，但也没有真正独立出来。当前 canonical Session、Self、Memory、Kernel、Agent runtime、Pi、Robot、stores 和 workers 的装配集中在 `executive::host::daemon::bootstrap::request::RequestHandler::new`；TUI 和 ACP 又绕过清晰的 Gateway/Application 边界直接接入 Executive。

本计划不把 `RequestHandler` 改名为 `AgentRuntime`，也不把它整块搬到 `aletheon`。目标调用路径是：

```text
TUI / CLI / ACP / external channel
        -> gateway-client / gateway transport adapter
        -> typed Gateway protocol + authenticated request context
        -> narrow Application use case
        -> one canonical Runtime

aletheon binary
        -> preflight config
        -> open concrete adapters
        -> compose domain/runtime/application/gateway components
        -> bootstrap supervised workers
        -> serve one authoritative user daemon
```

只有 `aletheon` 可以看见完整 concrete component graph；它只 wiring 和管理进程 lifecycle，不实现领域规则。

## 2. 当前耦合证据

| 当前位置 | 观察到的耦合 | 为什么必须拆 |
|---|---|---|
| `executive/src/host/daemon/bootstrap/request.rs` | `RequestHandler::new` 有 16 个显式入参、文件约 1590 行，内部创建 data dirs、Session/Self/Memory/Agora/tools/MCP/Google/Pi/Robot/stores/workers | transport handler 实际是 composition root、migration/recovery runner 和 worker supervisor |
| `executive/src/host/daemon/handler/ports.rs` | `HandlerPorts` 聚合 Kernel、Approval、Goal、Admin、legacy Session、Health、Google、Workflow、Turn、Evaluation、Checkpoint、Memory、Inference、Extensions 等 28 个依赖 | 名称虽叫 ports，实质是跨领域 service bag；route 可以绕过 Application 直达 Kernel/domain/concrete service |
| `executive/src/host/daemon/handler/mod.rs` | `RequestHandler` 持有 thread authority、workspace trust、MCP；同时适配 JSON、执行业务推导和 error shaping | request adaptation、host policy、application orchestration 混在同一类型 |
| `executive/src/core/runtime_core.rs` | `RuntimeCore::bootstrap` 加载 config/provider、启动 pulse/perception，再创建 RequestHandler | 与 `UserRuntime` 并存，重复定义“runtime bootstrap” |
| `executive/src/composition/user_runtime/mod.rs` | `UserRuntime::bootstrap` 再次创建 clock、RequestHandler、Unix server 和 shutdown orchestration | 第二条 user-daemon composition 路径 |
| `executive/src/core/system_core_runtime.rs` | `SystemCoreRuntime` 其实是 machine-wide inference server | 名称冒充 Agent Runtime；目标名应为 `InferenceBroker`/`InferenceHost` |
| `executive/src/composition/exec_session.rs` | one-shot exec 自行组装 Session/connection/store | 可能成为第二个 Session/Turn authority |
| `crates/aletheon/src/main.rs`、`workspace.rs` | binary 入口大多委托 `executive::host::launcher::*`，workspace 参数返回 `Executive` 类型 | 真正 composition owner 仍是库内 Executive，而不是 binary |
| `crates/aletheon/src/acp.rs` | `ExecutiveAcpBackend` 直接持有 `RequestHandler`，手工构造 `new_session/chat/session.interrupt` JSON-RPC，并直接打开 canonical Session store 做 recovery | ACP 既是协议 adapter，又知道 Executive RPC alias 和 persistence |
| `crates/interact/Cargo.toml` | `interact` 直接依赖 `executive` | Presentation 无法独立编译，也可跨过 Gateway |
| `interact/src/tui/mod.rs`、`tui/app/submit.rs`、`tui/rpc_client.rs` | 大型 `App` 持有 `UnixStream`；自行 framing/JSON；pending/compat/projection/terminal state 混合 | view、controller、transport 和 compatibility lifecycle 未分离 |
| `interact/src/intent.rs` | 客户端读取本地 UID，并提交 principal、workspace、permission、task/runtime hints | 不可信客户端数据被塑造成 effective authority；peer identity 应由 host 建立 |

现有 `gateway` crate 主要包含 channel dispatch、goal/approval/chat handler 和 SQLite/Telegram adapter，它还不是 daemon public protocol/client/server 的物理边界。不能只把 Executive handler 文件复制到该 crate。

### 2.1 现有 Gateway 不是中性 transport

对 `dev@bd1ceac` 的静态 census 得到 17 个 `crates/gateway/src/**/*.rs` 文件、约 2,900 行（含 Telegram 子目录）。其中存在五个必须先冻结语义、再迁移的 authority 泄漏：

- `handlers/chat.rs` 从 channel binding 字符串重新构造 `PrincipalId`，把 principal 文本同时当作 `SessionId`，并在 Gateway 内填充 `WorkspacePolicy`、`HostPermissionMode::Safe` 和默认 execution target；这会让 channel adapter 替 Runtime 决定身份、Session 和 effective execution policy。
- `dispatcher.rs` 直接拥有 concrete `ChannelStore`、`ChannelTurnExecutor`、`ChannelGoalExecutor`、approval repository port/resolver；它既推进 inbox/outbox/cursor，又启动 Turn、Goal 和 Approval lifecycle。
- `handlers/goal.rs` 直接 create/pause/resume/cancel Goal，`handlers/approval.rs` 直接 resolve approval 后执行 resolver；它们是业务 use case，不是 channel parsing。
- `adapters/sqlite_store.rs::ChannelStore::open` 会隐式执行 `migrate`；这破坏 `aletheon migrate -> open -> compose` 的唯一迁移顺序。
- `lib.rs::build_telegram_transport` 在库 public API 内构造 concrete Telegram HTTP adapter；provider selection 和 secret/lifecycle wiring 因而没有留在 composition root。

另外，`registry.rs::CapabilityRegistry` 把 channel intent、external event ingest 和 approval resolver 放进同一“capability”命名空间；它与 Agent/tool capability 概念冲突，也掩盖三种不同 authority。目标名固定为 `ChannelIntentRouter`（或仅当确实支持动态注册时为 `ChannelIntentRegistry`），不得继续使用无边界的 `CapabilityRegistry`。

### 2.2 Gateway 全量 K/M/D census

记号：`K` 表示该行为仍属于 Gateway，`M` 表示迁到既有 Runtime/Application/adapter owner，`D` 表示完成等价切换后删除旧 symbol/file；`SPLIT` 和 `INVESTIGATE` 不等于预先判定删除。Gateway 最终只拥有 channel transport、authenticated identity、typed Application command adapter，以及可由 Runtime facts 重建的 channel projection/cursor。

| 当前文件/文件族 | 当前责任与问题 | K/M/D | 动作 | 目标 owner、阶段与 gate |
|---|---|---:|---|---|
| `adapters/mod.rs` | 同时暴露 SQLite 与 Telegram concrete adapter | M+D | `DELETE` | 物理 adapter 分包后删除旧 module shell；`CGP-03/CGP-04`，`G0-STORE/G0-TELEGRAM` |
| `adapters/sqlite_store.rs` | binding、inbox/outbox/cursor 是 channel state；`open` 隐式 migrate，dispatcher 还直接访问 `db` | K+M | `SPLIT` | schema/migration bundle -> `aletheon migrate`；store adapter -> Gateway-owned `ChannelProjectionStore` port；`CGP-02/CGP-03/CGP-04`，`G0-STORE` |
| `adapters/telegram/{mod.rs,types.rs}` | Telegram long-poll、HTTP DTO、token 与 provider conversion | K+M | `INVESTIGATE` | 先以当前 Linux installed daemon 的 Telegram receive/send/restart 证据判定受支持面；保留时迁 `gateway-channel-telegram`，由 `aletheon` 构造；`CGP-00/CGP-04`，`G0-TELEGRAM` |
| `dispatcher.rs` | concrete store、intent dispatch、Turn/Goal/Approval executor、recovery/send 混合 | K+M+D | `SPLIT` | 仅保留 channel dedup/cursor/outbox pump；业务调用变成窄 typed Application command/query ports，旧 `ChannelDispatcher` 删除；`CGP-02/CGP-03/CGP-04`，`G0-ROUTES/G0-STORE` |
| `effect.rs` | channel outbound effect 闭集 | K | `KEEP` | 收进 Gateway channel protocol/internal effect；不得加入 Goal/Approval/Turn transition；`CGP-02`，`G0-ROUTES` |
| `handlers/chat.rs` | 自建 principal、principal-as-session、workspace/permission/target 并直接执行 Turn | M+D | `SPLIT` | Gateway 只传 `AuthenticatedPrincipal + SubmitPromptRequest`；Application 调 Runtime，Runtime 创建/选择 Session 并产 receipt；`CGP-02/CGP-03`，`G0-IDENTITY/G0-ROUTES` |
| `handlers/goal.rs` | Gateway 驱动 Goal draft 与 pause/resume/cancel，`/edit` 进入 approval resolver | M+D | `MIGRATE` | 迁到既有 optional Application Goal use case；Gateway 只解析命令并调用 typed port；`CGP-02/CGP-03`，`G0-ROUTES` |
| `handlers/approval.rs` | Gateway 读取/resolve repository 并执行业务 resolver | M+D | `MIGRATE` | 迁到既有 Application approval decision use case；Gateway callback 只提交 typed decision request；`CGP-02/CGP-03`，`G0-ROUTES` |
| `handlers/external_read.rs` | channel 层识别外部账号语义并改写 prompt | M+D | `INVESTIGATE` | Gmail/外部读取能力保留，但账号选择与可信上下文由可选 Application extension 决定；无生产调用则删旧 preprocessor；`CGP-00/E2/CGP-03`，`G0-CALLERS` |
| `handlers/greeting.rs` | 无业务状态的 channel-local greeting；当前效果又被 dispatcher hardcode 覆盖 | K+D | `SPLIT` | 保留一个 pure Gateway response renderer，删除重复 hardcode/handler 之一；`CGP-02/CGP-03`，`G0-CALLERS` |
| `handlers/mod.rs` | 旧 handler namespace shell | M+D | `DELETE` | route 完成 typed port cutover 后删除，不在 Gateway 重建业务 handlers 目录；`CGP-03`，`G0-ROUTES` |
| `intent.rs` | 纯分类，但直接绑定 Fabric command surface 和 Goal 命令族 | K+M | `SPLIT` | channel syntax classification 留 Gateway protocol；业务 command mapping 由 Application route catalog 提供；`CGP-02/CGP-03`，`G0-ROUTES` |
| `lib.rs` | facade 同时 re-export store 并构造 Telegram | K+M+D | `SPLIT` | 新 facade 只导出 protocol/router/ports；store/Telegram 由 `aletheon` 选择并注入，删除 `build_telegram_transport`；`CGP-02/CGP-04`，`G0-TELEGRAM` |
| `notify.rs` | 把 Approval projection 渲染成 channel message | K | `KEEP` | 输入必须是只读、已脱敏 Application projection；不得反查 approval store 或推进状态；`CGP-03`，`G0-ROUTES` |
| `ports.rs` | `ChannelApprovalPort` 直接暴露 get/resolve/delivery repository 操作 | K+M+D | `SPLIT` | delivery receipt 留 Gateway projection port；approval decision/get/list 改为窄 Application command/query port，旧 repository-shaped port 删除；`CGP-02/CGP-03`，`G0-ROUTES` |
| `registry.rs` | intent handler、external event ingest、approval lifecycle resolver 混为 capability registry | K+M+D | `SPLIT` | channel dispatch 改 `ChannelIntentRouter/Registry`；external ingest 和 approval resolver 迁各自 Application extension/use case；`CGP-02/CGP-03/E2`，`G0-NAMES/G0-ROUTES` |

覆盖 gate：基线 `rg --files crates/gateway/src` 的 17 个文件已在上表逐文件或逐文件族覆盖，计数为 `17/17`；任何新增文件必须先登记 owner、K/M/D、迁移阶段和删除/保留 gate。这个 census 只决定边界，不授权把 Goal、Approval、Gmail 或 Agent orchestration 复制成第二个 `gateway-application`。

### 2.3 CGP-00 内部证据门（`G0-*`）：先消除歧义，不切 writer

`G0-*` 只是 `CGP-00` PR 内部的 evidence-ledger/gate code，不是独立 PR、branch 或 owner namespace。`CGP-00` 必须先于 CGP-01 完成这些只读 census/contract 证据，不得改 official socket、schema、worker 或运行路径。

#### G0-ROUTES：route 与 side-effect ledger

- 逐 `ChannelTurnExecutor/ChannelGoalExecutor/ChannelApprovalPort/ApprovalResolver` 方法登记 production/test caller；
- 记录每次调用读取/写入的 authoritative store、外部 effect、幂等键和当前失败语义；
- 映射到既有 Application command/query、Runtime receipt 或只读 projection；
- 未有唯一 owner、receipt 和 rollback boundary 的 route 不准迁移。

#### G0-IDENTITY：身份、Session 与请求偏好

- `AuthenticatedPrincipal` 只能来自 peer credentials 或验证过的 channel binding；
- 目标 contract 删除 client-provided effective `PrincipalId`、workspace、permission 和 execution target；
- 禁止把 principal、conversation id 或 sender id 直接转换成 canonical `SessionId`；
- 新 channel conversation 首次提交由 Runtime 原子创建 Session 并返回 canonical `SessionId`；
- 后续请求只携带 Runtime receipt 中的 Session reference 与 channel correlation。

#### G0-STORE：projection store 与 migration

- channel binding/inbox/outbox/cursor 是 Gateway-owned projection/store state，不是 Agent authority；
- 建立窄 `ChannelProjectionStore` port，dispatcher 不再取得 `rusqlite::Connection`；
- 将 schema DDL 抽成 owner migration bundle，由 `aletheon migrate` 按顺序执行；
- 证明 `open` 在旧/新 schema 上均不写 DDL、user_version 或业务 row；
- recovery 只重发 durable outbox，不重新执行已经 receipt-confirmed 的 Runtime command。

#### G0-TELEGRAM：保存舱判定

- 检查当前 Linux installed binary、systemd topology、配置入口和 secret reference；
- 采集真实 binding、receive/send、cursor recovery、restart 后不重跑 Turn 的证据；
- 证据存在则 preservation ledger 标 `KEEP+MIGRATE`，锁定 installed equivalence；
- 证据不足标 `INVESTIGATE` 并列明缺口，不能因“应用层冻结”直接删除；
- `CGP-00/G0-TELEGRAM` 不新增 channel provider，也不扩大 Telegram feature surface。

#### G0-NAMES：消除 capability 命名冲突

- 完成全仓 `CapabilityRegistry`、`CapabilityHandler` 和相关 re-export symbol census；
- channel 类型改 `ChannelIntentRouter`，仅确有 runtime registration 才使用 `ChannelIntentRegistry`；
- Agent/tool capability owner 与领域名保持不变，不为迁就 Gateway 反向改名；
- external-event ingest 和 approval resolver 迁到既有 Application extension/use case；
- 禁止用通用 `Capability*` bag 再次聚合 channel、Agent、tool 与 lifecycle authority。

#### G0-CALLERS：保留、调查与删除证据

- 列出 `external_read`、greeting、Telegram builder 的 production/test-only callers；
- 列出 Goal progress notification、event registry 和 approval delivery 的 producer/consumer；
- 对同名 handler/registry 做语义比较，不以名称相同作为 merge/delete 证据；
- 只有 production caller 为零、preservation ledger 无承诺、替代路径无缺口时才提出 `DELETE`；
- 其他不确定项保持 `INVESTIGATE`，在 writer cutover 前必须归零。

#### G0-EXECUTIVE：拆旧 facade，不搬 god object

- transport adaptation -> Gateway；use-case orchestration -> 既有 Application；
- Session/Turn authority -> Runtime；policy enforcement -> Kernel；wiring/lifecycle -> `aletheon`；
- 逐 `RequestHandler` route 与 `HandlerPorts` field 建 source-to-target ledger；
- 每条 route 切换并取得 equivalence 后删除对应 field/method；
- 禁止迁移、改名或重新生成同型 `HandlerPorts/Services/Dependencies` getter bag。

`CGP-00` 的 `G0-*` 交付物固定为七份小型 ledger，而不是新 crate：route、authority、store/schema、installed Telegram、naming、caller、Executive-field。每份 ledger 都必须带 source location、现 owner、target owner、writer、迁移 PR、rollback 和 deletion gate；缺列视为未完成。

后续 Gateway 工作不再建立局部编号；全部统一落入规范 crosswalk 已登记的 PR：

- `CGP-02` 建立 typed channel protocol、authenticated context、projection store port 和 `ChannelIntentRouter`；
- `CGP-03` 逐 route 切到既有 Application command/query，Runtime 仍是唯一 Session/Turn writer；
- `CGP-03/CGP-04` 迁 channel SQLite migration/adapter 与已确认保留的 Telegram physical adapter，并切 official daemon route、recovery 和 cursor/outbox pump；
- Gmail/external-event 等可选 seam 由 Gmail owner 的 `E2` prep、唯一 writer cutover `E2-K6a` 与 `CGP-03` 共同交付；GBrain `E3` 不进入该 channel seam，也不扩大 frozen Application surface；
- 每阶段未满足对应 `G0-*` evidence gate 时只能继续调查，不能用 compatibility bag 跨过边界。

`CGP-00` exit gate：17/17 Gateway 文件、全部 Gateway-facing Executive adapter/caller、全部 route 和 schema 均有唯一 owner；`ChatHandler` 目标 contract 不再能构造 core ID/effective policy；migration 与 `open` 已分契约；Telegram 有明确 preservation decision；`RequestHandler/HandlerPorts` 字段数只能下降。未满足任一项时，CGP-01 只能建立空 skeleton，不能开始 writer cutover。

## 3. 目标边界

### 3.1 唯一 composition root

目标结构：

```text
crates/aletheon/src/
├── main.rs                 # parse CLI only
├── composition/
│   ├── config.rs           # collect typed owner configs
│   ├── core.rs             # compose Kernel + domains + Runtime
│   ├── application.rs      # optional use cases
│   ├── adapters.rs         # open SQLite/Linux/provider/extension adapters
│   ├── gateway.rs          # bind typed Gateway server
│   └── topology.rs         # user daemon / inference broker process plan
├── migrate.rs              # only migration executor
└── lifecycle.rs            # start handles, drain, shutdown order
```

`aletheon` 允许依赖所有需要装配的 concrete crates，但有四条硬限制：

- 不定义 aggregate transition、permit policy、Self/Metacog/Memory 规则；
- 不实现 repository，只调用 owner adapter 的 `open`；
- 不隐藏全图到 `Components/Services/Container` getter bag；
- 不把 fallible I/O、migration 或 worker spawn 塞进 `new/compose`。

生命周期动词固定：

```text
preflight  解析/归一化配置和 secret refs，不启动组件
open       打开并检查 durable/host 资源，不 migrate
compose    纯装配已构造依赖，不 spawn
bootstrap  recovery/reconcile 后启动 supervised workers，返回 handles
serve      运行 transport loop 直到 shutdown
drain      拒绝新命令，等待/取消 operation，停止外部 effect
shutdown   按反向依赖顺序终止并等待 handles
```

### 3.2 Gateway 物理拆分

采用独立 package，避免 feature 把 server 依赖带入 TUI：

```text
gateway-protocol  # versioned command/query/event DTO、GatewayErrorCode
gateway-client    # typed request、subscription、cursor、reconnect、framing
gateway-server    # peer auth、rate limit、dispatch、Application error mapping
gateway-channel   # Telegram/Gmail 等 transport adapter，可选
```

`gateway-protocol` 不依赖 Runtime repository、Unix socket、SQLite 或 presentation widget。`gateway-client` 可有 Unix transport adapter，但它只返回 typed result/event，不公开 raw JSON。`gateway-server` 不持有 Kernel/Runtime concrete type；业务 route 只持有对应的窄 Application use-case trait，运维 health route 只持有只读 host snapshot port。

可信请求路径：

```text
peer credentials / authenticated channel identity
-> Gateway 建立 AuthenticatedPrincipal
-> requested workspace/permission/target 仅作为 RequestPreference
-> Application/Runtime/Kernel 各自求 Effective* 并持久化依据
-> Gateway 返回 projection；客户端不能提交 grant/permit/effective principal
```

### 3.3 Handler 不是 composition

目标不再有持有完整组件图的 `RequestHandler`。按 route family 建窄 handler，例如：

- `SessionCommandHandler`：只依赖 Session use cases；
- `TurnCommandHandler`：只依赖 Turn use cases；
- `ApprovalCommandHandler`：只依赖 Approval use cases；
- `ProjectionQueryHandler`：只依赖 query facade；
- `HealthQueryHandler`：只依赖 host health snapshot；
- extension-specific handler 只在对应 profile composition 中注册。

`GatewayRouter` 只做 method/version 到 handler 的 typed dispatch，不提供跨 route getter。每个 handler 最多 3 个业务依赖；不能直接 import `kernel`、`rusqlite`、Cognit/Dasein/Metacog/Mnemosyne concrete service。

旧 JSON-RPC alias 暂时由 `LegacyJsonRpcAdapter` 单向翻译到新 command。它不持有 store、worker、policy、ID generator 或 Runtime handle；每个 alias 有调用计数和删除 PR。

### 3.4 TUI/CLI 是 Presentation

目标依赖：

```text
interact -> gateway-client + gateway-protocol + presentation libraries
interact -X-> executive/runtime/kernel/domain store/concrete adapter
```

把当前 `App` 分成：

| 类型 | 唯一职责 | 禁止职责 |
|---|---|---|
| `TuiModel` | projection、input draft、selection、cursor/modal、`UiOverlayId` | socket、clock/env 读取、core ID mint、业务推导 |
| `TuiController` | 把 key/command 转成 typed client call，把 result/event 送 reducer | JSON-RPC、effective policy、terminal 推断 |
| `TuiRenderer` | 从 model 生成 frame | 发 command、改 authority state |
| `GatewayClient` | typed commands、subscription、cursor/reconnect | widget、local policy、repository |

`streaming/turn_active/status` 等状态必须由 authoritative event/projection 驱动；本地只能持有 pending/optimistic overlay。临时 row 使用 `UiOverlayId`，收到 Runtime ID 后替换；TUI/CLI 不调用 `TurnId::new/SessionId::new/AgentId::new`。

`ClientIntent` 删除 effective principal、permission、workspace policy 和 authority grant；runtime hint 只是 `RequestedExecutionTarget`，客户端不得把某 runtime 映射成 `TaskKind::Coding`。错误分类只看 `GatewayErrorCode`，禁止 `message.contains(...)` 推断 denial/success/failure。

### 3.5 ACP 只做协议翻译

保留 `interact::acp::AcpAdapter` 的 request/event translation 价值，但替换：

```text
ExecutiveAcpBackend
    -> GatewayApplicationClient / typed Session+Turn client

ExecutiveEvents + direct CanonicalSessionStore
    -> Gateway subscription + projection recovery API
```

ACP `NewSession/Prompt/Cancel` 不再手写 `new_session/chat/session.interrupt` JSON；不再调用 `RequestHandler` 或打开 Session DB。session correlation 是 presentation/protocol state，不成为 authority。ACP、TUI、CLI 最终共享同一 gateway-client 和同一 Runtime。

## 4. Process topology

生产 topology 固定为：

```text
optional machine InferenceBroker  # 只拥有 provider admission/routing，不是 Agent Runtime
             ^
             | governed InferencePort
authoritative user daemon          # 每个 user state root 只有一个 Runtime writer
             ^
             | official typed socket
TUI / CLI / ACP / Gmail worker clients
```

- system daemon 不持有第二套 Agent/Session/Turn 状态。
- one-shot `exec` 也通过同一 Runtime command boundary；若使用临时 isolated runtime，必须明确是 diagnostic/ephemeral，不写 official store。
- socket、data-dir、schema、running PID 与 installed binary provenance 必须一致。
- `InferenceBroker` 不读取 user-scoped Gmail/MCP/GBrain credential，也不持有 Self/Memory。

## 5. 分 PR 执行顺序

### CGP-00：Protocol、route、composition census

- 列出所有 CLI/TUI/ACP/daemon RPC method、alias、DTO、caller 和 authority side effect；
- 固化 [`Interact authority census`](./2026-08-09-interact-authority-census.md)，逐一覆盖 56 个 `interact/src/**/*.rs` 文件中的 Session/Turn/Agent ID mint、字符串重建、permission/workspace/effective-policy 推导、runtime requirement、`TaskKind` 推导、raw framing、terminal/error inference 与 Executive import；
- 列出 `RuntimeCore/UserRuntime/SystemCoreRuntime/ExecSession/RequestHandler` 的每个构造/启动责任；
- 建 route 到新 Application use case 的一一映射；
- 建旧 alias 使用计数和删除 deadline。

验收：新 route/composition path 必须先登记；Interact census 必须保持源码文件一一对应，所有 authority point 恰好归类且计数只能下降；无行为变化。回滚删除清单/gate。

### CGP-01：建立 `aletheon` composition skeleton

前置：Kernel/Runtime/domain 已有可注入的 owner API；Application 最小 facade 可用。

- 在 binary crate 实现 preflight/open/compose/bootstrap/serve/drain 阶段；
- 将 config 按 owner 归一化，不再传播一个 Executive `AppConfig`；
- 为每个 worker 返回显式 lifecycle handle；
- 先通过旧 launcher 单向调用新 composition，不切 official socket。

验收：无 `ComponentGraph/ServiceBag`；compose 不 I/O/spawn；旧生产 path 不变。回滚回到旧 launcher。

### CGP-02：提取 `gateway-protocol` 与 `gateway-client`

- 固定 versioned commands、queries、events、cursor 和 error codes；
- 迁内层 Gateway framing、JSON serialization、request correlation、subscription/reconnect；外层 ACP framing、ACP connection correlation 与 DTO translation 留 `interact::acp` adapter，但该 adapter 不得出现 Gateway JSON-RPC/business method；
- 为 legacy wire 提供单向 translation adapter；
- 客户端 authority fields 改为 requested preferences。

验收：client contract 可用 in-memory transport 验证；TUI 尚可通过 compatibility client 工作。回滚协议 additive 字段，不复用已发布 tag 改语义。

### CGP-03：按 route family 提取 Gateway server handlers

- 先 Session/Turn/Approval/Projection，再 Health/Admin，再可选 extension；
- 每个 handler 只调 Application trait；
- peer identity、rate limit、protocol version 和 typed error mapping 收进 Gateway；
- `LegacyJsonRpcAdapter` 只翻译，不执行业务。

验收：handler 无 Kernel/domain store/concrete adapter import；unknown/old version fail typed；旧/new route 对相同 command 产生同一 Runtime receipt。回滚逐 route 切回旧 adapter，仍只有一个 writer。

### CGP-04：official user daemon 与 socket cutover

- `aletheon` 新 composition 绑定 official socket；
- drain 旧 daemon，确认无 remaining writer/PID/socket；
- 启动单一 Runtime、Gateway server 和 supervised workers；
- 将 `RuntimeCore/UserRuntime` 调用者迁到新 topology，`SystemCoreRuntime` 收缩/改名为 InferenceBroker host。

验收：socket/PID/binary/data-dir/schema provenance 一致；restart counter 稳定；真实 `/usr/bin/aletheon` Turn 成功。回滚先 drain 新 daemon，再启动旧 binary，禁止并行写。

### CGP-05：ACP typed client cutover

- 删除 `ExecutiveAcpBackend`、手写 RPC alias 和 direct Session store recovery；
- ACP 使用 typed Session/Turn commands 和 Gateway subscription/recovery；
- reconnect/cancel 在 active prompt 期间可并发处理。

验收：create/prompt/cancel/recover 均走同一 Runtime；ACP 无 `executive::`、repository 或 raw JSON business method。回滚到旧 ACP adapter，不回滚 Runtime state。

### CGP-06：TUI/CLI command-only cutover

- `interact` 改依赖 gateway-client/protocol；
- 删除 raw `UnixStream`、manual framing、local principal/effective policy、runtime-to-task mapping 和 core ID mint；
- 所有 terminal/status 从 projection 得出；旧 transcript 只读显示后删除。

验收：Interact 不依赖 Executive/Runtime/Kernel/concrete adapters；断线重连能从 cursor/snapshot 重建；provider rejection 显示失败而非 false success。回滚 client implementation，不恢复本地 authority。

### CGP-07：拆 `App` 并删除 Presentation compatibility state

- 提取 `TuiModel/TuiController/TuiRenderer/GatewayClient`；
- 删除 legacy Session RPC branch、compat terminal inference 和 durable conversation mirror；
- 本地 persistence 只允许 input draft/history/display preferences。

验收：renderer 零 I/O/command；reducer 零 socket/clock/env；controller 零 authority constructor。回滚 UI component 组合，不回滚 server。

### CGP-08：Executive host/composition deletion-ready gate

前置：所有 supported route/host/extension 已从新 composition 运行并通过 installed equivalence。

- 将 `executive/src/host/**`、`composition/**` 和 `core/{runtime_core,system_core_runtime}.rs` 的生产 caller、writer、socket bind、worker spawn 与 business dependency 清零；旧代码只能保持 inert、单向、可计数的 rollback seam；
- 固化 `executive::host::launcher`、RequestHandler、HandlerPorts 与每个 host/composition remnant 的 exact deletion inventory，证明新 `aletheon` wiring/official socket 不再依赖它们；
- 更新 examples/CLI 调用新 public surface。

验收：生产路径无 Executive host/composition symbol、writer、bind 或 spawn；`aletheon` 是唯一 full graph composition root；本阶段不物理删除 host/composition tree、RequestHandler 或 compatibility facade。`AK2-25` rename 完成后，唯一由 `XRET-04` 按 inventory 删除这些 remnants。回滚只启用已有单向 launcher seam，不能恢复第二 composition graph。

## 6. Fitness gates

逐 PR 加严，不等待最终清理：

- 只有 `crates/aletheon` 可 import 两个以上 concrete adapter crates；任何领域对 `aletheon` 反向依赖失败。
- `RequestHandler`/transport handler 不得持有 Runtime/Kernel/domain component graph；constructor 业务依赖上限 3。
- 新 composition function 显式依赖上限 5；禁止 `Services/Dependencies/Container` getter bag。
- `interact` manifest/source 禁止 `executive`、`runtime`、`kernel`、`rusqlite`、`UnixStream` 和 concrete adapter。
- TUI/CLI 禁止手写 Gateway JSON-RPC method name、raw newline framing、core aggregate ID mint；ACP 也禁止手写内层 Gateway method/framing 或 mint core ID，但可在 `interact::acp` 边界保留外层 ACP framing、connection correlation 与 DTO translation。
- Gateway server 禁止 authority repository write、Agent decision、Turn settlement、Kernel table 和 provider call；Gateway-owned cursor/projection 只能经明确的 projection port 写入，并且必须可由 Runtime facts 重建。
- composition `new/compose` 禁止 `std::fs`、socket bind、DB open、spawn；这些只在命名明确的 open/bootstrap/serve 阶段。
- official user socket 只允许一个 writer topology；system inference process 禁止 Agent/Session/Turn store。
- source 中 `RuntimeCore/UserRuntime/SystemCoreRuntime/ExecutiveAcpBackend/HandlerPorts` 计数只能下降，目标为零。

## 7. 验收与回滚矩阵

| 场景 | 验收证据 | 回滚边界 |
|---|---|---|
| typed Gateway | version/error/auth/reconnect focused contract | additive protocol；旧 client adapter 可临时恢复 |
| Session/Turn | TUI、CLI、ACP 命中同一 Runtime journal/receipt | 切 client/route，不复制 store |
| daemon restart | recovery 后无 duplicate Turn/worker，projection 可重建 | drain 新 PID 后回旧 binary |
| TUI | 多 turn 同 Session、cancel、reconnect、provider rejection 均与 journal 一致 | UI 可回滚，authority 不回滚 |
| ACP | prompt streaming 时 cancel 可达，cursor recovery 不重放旧 event | ACP adapter 可回滚，DB 不回滚 |
| topology | installed binary、running PID、socket、data dir、schema digest 一致 | 严禁 old/new daemon 共写 |
| extensions | core profile 无扩展；full profile Gmail/Pi/GBrain/Robot/Hardware 各自等价 | 按 extension facade 切换，不整体回滚核心 |

涉及真实 socket、daemon、persistence 或 client 行为的 PR，必须按仓库 installed runtime policy 完成 system deployment、digest 对齐、稳定 restart counter 和真实 LLM request。开发 binary 和临时 socket 只算诊断证据。

## 8. 完成定义

- `aletheon` 是唯一完整 composition root，其他 crate 无法取得 full component graph。
- official user daemon 只有一个 Runtime writer；machine process 只提供 inference broker。
- Gateway 拥有 protocol/client/server、peer identity、subscription 和 error mapping，但不拥有 Agent 决策。
- TUI/CLI/ACP 只通过 typed Gateway client；零 Executive、Runtime、Kernel、repository 和 raw socket framing 依赖。
- `RequestHandler`、`HandlerPorts`、`RuntimeCore`、`UserRuntime`、`SystemCoreRuntime`、`ExecutiveAcpBackend` 均已删除而非改名。
- 删除 Executive host/composition 后，Gmail/Pi/GBrain/Robot/Hardware 的受支持路径仍由显式 composition 插入并独立通过保存舱验收。
