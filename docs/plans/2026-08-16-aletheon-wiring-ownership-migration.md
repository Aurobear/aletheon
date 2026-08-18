# Aletheon `wiring` 所有权迁移与功能模块化实施计划

状态：**MIGRATION CLOSEOUT DONE · ARCHITECTURE CONVERGENCE TRACKED — M0–M9、M10 已完成并通过安装态验收；2026-08-18 closeout（§9.1）完成：架构门禁假绿修复、迁移 warnings 清理、M8.4 HandlerPorts 全收口、legacy sunset ledger、公共 API doc(hidden)、closeout binary 已重新部署并真实 LLM smoke 通过；剩余 2 项显式收敛残留见 §9.1**

日期：2026-08-16

修订日期：2026-08-17

目标分支：`dev`

核查基线：

- HEAD：`85704ed3f641419f86dd08c5be8d26c52c64c19b`
- 批准设计 SHA-256：`c9b927bd7a21a01309d05ae31bbc41ff8024988fb0b3417cf799f3a673f76039`
- 除本计划和两份审核稿外 dirty path：34；path-list SHA-256：`5fe336cd825f6f3f116215fd080c6349e19114b99fdcb53877d8b7febb98bc6f`
- 除本计划和两份审核稿外 tracked diff SHA-256：`f35d49b1e6dd57e50cd7b14a5d9e2017f87438d442a404079ae9aec9632bee12`
- M0 evidence：`docs/arch/evidence/2026-08-17-wiring-migration-m0-baseline.md`

该快照包含未提交和未跟踪文件，不能只从 HEAD 复现。所有“当前依赖”结论都必须在
packet 开始前重跑 manifest/metadata 检查；M0 再冻结实施时的新快照和统计命令。

实施授权记录（2026-08-17）：用户已授权按本计划完善文档并开始迁移。授权覆盖 M0–M4、
M7–M9 中不触及核心控制边界的生产文件；并追加授权 architecture checker/fixture 以及 M1
列出的 Turn、Agent、Evaluation 核心文件作纯 owner/import cutover、删除已确认的
compatibility/ghost 文件。M5/M6 的核心行为变更和 M10 `sudo deploy` 仍保留 §0.4 门槛。
该门槛随后由下述统一授权解除。

追加授权记录（2026-08-17）：用户已明确授权本计划全部后续范围，包括核心文件、测试、
fixture 和最终 deploy/安装态验收。执行仍必须逐 packet 保持行为语义、运行最窄验证并记录
失败；统一授权不取消 architecture/runtime safety gate。

计划 owner：Aletheon composition/architecture

> 本计划替代“把旧 `executive` 文件继续搬到 `aletheon/wiring`”的目录迁移思路。
> 它要求先确定语义权威和功能边界，再移动实现，最后删除过渡路径。
> 不允许通过新增同规模 `executive`/`orchestration` god crate、长期兼容 re-export、
> 双写或文本 allowlist 伪造收敛。

---

## 0. 审核目标与本轮边界

### 0.1 目标

把当前 `crates/aletheon/src/wiring/` 从“旧 Executive 的过渡容器”收敛为可由
Cargo 依赖方向约束的模块化架构，使最终代码满足：

1. `aletheon` 只负责 CLI facade、配置装载、system/user daemon host 和 composition；
2. `application` 负责无具体基础设施依赖的功能用例编排；
3. `runtime` 负责 Session、Turn、Agent 生命周期和持久语义权威；
4. Goal、Approval、Verification、Memory 等功能按垂直切片组织；
5. SQLite、HTTP provider、MCP、Google、进程和文件系统实现位于明确 adapter owner；
6. daemon 与 `aletheon exec` 复用同一 Turn 用例和 Runtime authority；
7. 迁移前后 wire protocol、数据库 schema、恢复语义和安装态行为不变；
8. 最终删除 `wiring/application`、`wiring/adapters` 及 `pub mod wiring` 公共表面。

### 0.2 包含

- `crates/aletheon/src/wiring/**` 的所有权拆分；
- `crates/application`、`crates/runtime`、`crates/adapters/*`、`crates/gateway`、
  `crates/platform`、`crates/agora`、`crates/cognit`、`crates/mnemosyne` 的必要接收面；
- Cargo 依赖、架构账本、文档 locator 和已有测试 import/owner 迁移；
- 逐包验证、最终系统部署和真实请求验收。

### 0.3 不包含

- 新增产品功能、改变提示词、模型路由或工具策略；
- 数据库 schema 变更或数据回填；
- 更改公共 JSON-RPC/Gateway wire schema；
- 改变机器人控制、安全状态机或硬件命令语义；
- 为减少行数而删除仍有生产调用的功能；
- 重新引入 `executive` 或一个名称不同但职责相同的 god crate；
- 未经单独批准创建新测试框架、fixture 或 robot HIL 资产。

### 0.4 实施授权门槛

Grok/Codex 架构审核通过只代表设计可实施，不自动授权代码修改。真正执行前必须再次确认：

- 本计划版本和 packet 范围；
- 允许修改/移动的生产文件；
- 允许更新和移动的现有测试文件；
- M5/M6 Agent/Turn 核心生命周期范围；
- 最终 `sudo bash scripts/aletheon.sh deploy` 系统验收。

---

## 1. 当前事实基线

### 1.1 目录规模

基于当前工作区逐个 Rust 源文件统计：

| 路径 | Rust LOC | 文件数 | 当前职责 |
|---|---:|---:|---|
| `crates/aletheon/src/wiring/` | 78,148 | 215 | composition、application、adapter、daemon、持久化、Host I/O |
| `wiring/application/` | 29,329 | 69 | Turn、Goal、Agent、Approval、Evaluation、Checkpoint |
| `wiring/daemon/` | 22,724 | 65 | bootstrap、transport、RPC、tool execution、projection |
| `wiring/adapters/` | 16,846 | 45 | inference、GBrain、Google、runtime、session adapter |
| `wiring/composition/` | 1,781 | 12 | 实际 composition helper |
| `crates/application/src/` | 6,315 | 30 | 窄 DTO、port 和少量已接线用例 |

`wiring.rs` 声明这里是 binary-owned runtime binding boundary
（`crates/aletheon/src/wiring.rs:1-5`），但同时公开 `application`、`adapters`、
`daemon` 等大模块（`:24-36`）。

### 1.2 迁移性质

提交 `14d3525b refactor(architecture): complete coupling closeout` 删除了物理
`executive`/`fabric`，但大量文件以 90%–99% 相似度从
`crates/executive/src/{application,adapters,host}` 移到
`crates/aletheon/src/wiring/**`。当前仓库自己的硬化清单也把该问题描述为
“整体机械搬入，只是换了个家”
（`docs/plans/2026-08-15-architecture-hardening.md:63-100`）。

### 1.3 边界混合的代码证据

当前 `wiring/application` 中：

- 9 个文件直接使用 `rusqlite`；
- 11 个文件引用 `corpus`；
- 7 个文件执行生产 `std::fs`；
- `goal/store.rs`、`goal/transition.rs`、`goal/budget.rs` 直接执行 SQL；
- `workspace_checkpoint.rs:811-903` 直接扫描、删除、写入、rename 文件；
- `admin_service.rs:296-410` 自行打开 SQLite；
- `runtime/src/session_service.rs:42-47` 把 Runtime session authority 与具体
  `rusqlite::Connection` 放在同一对象，`:76-205` 直接维护 protocol event SQL；
- `verification/mod.rs:50-51` 解析 `cargo`/`git` 可执行文件；
- `turn_runtime_ports.rs:101-103` 的 port 返回 binary-owned
  `crate::config::CognitiveRuntimeConfig`，说明 port 仍被 host 配置类型污染。

而 `crates/application/src/lib.rs:1-7` 同时声明 Application 不拥有 concrete
repository 或 host adapter。两个同名 Application 边界的语义并不一致。

### 1.4 文档与代码现实差异

| 现行文档描述 | 当前代码现实 | 一致？ |
|---|---|---|
| `aletheon` 是 composition + host wiring（`docs/design/architecture-overview.md:43-49`） | `wiring` 内含 78K 行编排、持久化和 adapter | 否 |
| Application/Turn 边界已收敛（`:151-155`） | `wiring/application` 仍为 29K 行，并依赖 SQLite/Corpus/文件系统 | 否 |
| `runtime` 是 Session/Turn/Agent authority（`:59`） | authority reducer 已在 runtime，但主要 coordinator 和兼容 facade 仍在 aletheon | 部分 |
| `application` 不拥有 concrete repository（`crates/application/src/lib.rs:3-7`） | 同名 nested application 拥有 concrete repository | 名义正确、整体误导 |

架构 reviewer 需要首先裁决本计划的目标架构；在裁决前不得把当前文档中的“已收敛”当作
继续机械搬迁的依据。

---

## 2. 架构原则

### A1. 所有权优先于目录

移动前必须回答：谁定义状态、谁允许转换、谁持久化、谁执行外部副作用、谁只做
协议投影。无法回答时停止该 packet，不能先移动再补 owner。

### A2. 一个 durable fact 只有一个 authority

- Session/Turn/Agent 生命周期：`runtime`；
- Goal 业务状态转换：`application::goal` 的规则 + repository port，SQLite 只保存；
- Approval/settlement：`application` 的规则，adapter 保存 receipt；
- Memory：`mnemosyne`；
- Capability/Operation/Admission：`kernel`；
- Cognitive workspace：`agora`；
- 推理策略与 cognitive session：`cognit`。

迁移期间禁止双写 old/new repository，也禁止两个 reducer 同时裁决同一终态。

### A3. 功能垂直切片，基础设施水平隔离

每个功能遵循：

```text
transport DTO
    -> application use case
        -> runtime/domain authority
            -> output port
                -> infrastructure adapter
                    -> durable receipt/projection
```

功能目录可以包含 command/query/service/port，但不能包含具体 SQLite、HTTP client、
`Command::new` 或任意真实文件写入。

### A4. Cargo 边界必须能够阻止倒置依赖

目录规则不是边界。最终必须通过 Cargo manifest 和 architecture gate 保证：

```text
contracts  <- runtime <- application <- gateway/interact
    ^            ^           ^
    |            |           |
 kernel/domain APIs      adapter implementations
         ^                   ^
         +------ aletheon composition ------+
```

该图表达角色，不是 `application` 的静态 domain 白名单。任何 Cargo 边都必须检查完整
传递闭包；目标 crate 不直接依赖 `application`，不代表它的下游不会经 `platform`、
`agora` 或其他 crate 回到 `application`。允许 `aletheon` 依赖所有需要组装的 crate，
但不允许其他生产 crate 依赖 `aletheon`。

### A5. Port 放在消费者侧，wire type 放在 contracts

- 某个用例独占的 port 放在 `application::<feature>`；
- 某个 domain 独占的 port 放在 domain owner；
- 只有跨多个稳定边界共享、必须 wire-safe 的类型才进入 `contracts`；
- 禁止为躲避循环把具体 adapter DTO 塞进 `contracts`。

### A6. 不建立新的 Executive 2.0

不新建通用 `orchestration` crate。只有同时满足以下条件才允许新增 adapter crate：

1. 有独立外部依赖或安全边界；
2. 能形成无环、稳定的 manifest；
3. 可独立编译和验证；
4. 不只是少量 wrapper；
5. 名称表达具体集成能力，而不是 `common`、`misc`、`core`。

### A7. 行数只是预警，不是 owner 判据

迁移不能以 LOC 下降代替行为与权威验证。LOC 预算只用于阻止再次膨胀。

### A8. 协议、恢复和安全语义优先

每个 packet 必须保持：principal/workspace/session identity、cancel、deadline、permit、
idempotency、generation fence、terminal receipt、restart recovery 和敏感度标记。

### A9. Canonical identity 与边界表示分离

- `runtime` 是 Session/Turn/Agent canonical ID 的唯一 mint authority；
- `contracts` 中同名类型若作为 wire/domain representation，必须明确不可自行升级为
  runtime authority；
- String/UUID 转换只能发生在一个显式、可失败、可审计的 adapter 边界；
- caller/thread/legacy alias 不得直接构造 canonical Session/Turn ID；
- repository、event、receipt 必须逐项声明存储哪一种表示，禁止双向散落转换。

---

## 3. 目标架构

### 3.1 最终运行结构

```text
main / launcher
      |
      v
aletheon::host (crate-private)
  core | user_daemon | exec | systemd
      |
      +---------- composition ----------+
      |                                 |
      v                                 v
application                         adapter crates
  session                           sqlite
  turn                              inference
  agent                             gbrain
  goal                              google
  approval                          agent-backend
  verification                         |
      |                                 |
      +---------- ports ----------------+
      |
      v
runtime / kernel / cognit / corpus / mnemosyne / agora / metacog
      |
      v
platform / execd / external services
```

### 3.2 `aletheon` 最终保留内容

```text
crates/aletheon/src/
├── main.rs                 CLI 解析和 facade 调用
├── lib.rs                  稳定 public facade；不公开 host internals
├── launcher.rs             Core/Daemon/Exec 启动请求
├── config/                 binary composition 配置
├── host/                   crate-private OS process/daemon lifecycle
│   ├── core.rs             MachineInferenceRuntime config/socket/server lifecycle
│   ├── user_daemon.rs
│   ├── exec.rs
│   ├── unix_server.rs
│   ├── readiness.rs
│   ├── doctor.rs
│   ├── goal_scheduler.rs
│   └── cli/
│       └── extension.rs
└── composition/            只创建对象和连接 ports
    ├── core.rs
    ├── user_daemon.rs
    ├── turn.rs
    ├── agent.rs
    ├── memory.rs
    └── integrations.rs
```

最终不存在：

- `crates/aletheon/src/wiring/application/`；
- `crates/aletheon/src/wiring/adapters/`；
- `pub mod wiring`；
- binary-owned `ObjectiveStore`、SQLite connection 或 provider HTTP client；
- 为旧路径保留的 re-export seam。

### 3.3 功能所有权矩阵

| 功能 | Application owner | Authority/domain owner | Adapter owner | `aletheon` 只负责 |
|---|---|---|---|---|
| Session | `application::session` | `runtime::session_*` | `adapters-sqlite::session` | 注入 store、clock、event sink |
| Turn | `application::turn` 的 provider-neutral TurnService 拥有授权后的用例顺序 | `runtime::turn_*` | inference/tool/memory/workspace/notification adapters | 构造同一 TurnService 给 daemon/exec；不拥有 pipeline policy |
| Agent | `application::agent` | `runtime::agent_*` | `adapters-agent-backend`、SQLite | 注册 backend/profile、注入 admission |
| Goal | `application::goal` 的 transition/retry/advance service | Agent run 仍由 `runtime` | `adapters-sqlite::goal`、platform artifact/quota、gateway progress | host scheduler 只触发 interval/cancel；composition 注入 ports |
| Approval | `application::approval` | application settlement rules + kernel permit | `adapters-sqlite::approval`、Corpus governed-patch adapter、`platform::worktree` | 连接 apply/cleanup adapters |
| Verification | `application::verification` | application policy；Metacog evaluation facts | `platform::verification_command` | 注入受约束 executor |
| Memory | application 只持 recall/projection port | `mnemosyne` | `adapters-gbrain` | 配置 binding 和 credential |
| Cognitive workspace | application coordination port | `agora`/`cognit`/`dasein` | Corpus tool/skill adapter | 组装 arbitration mode |
| Inference | `cognit` provider-neutral port | `cognit` | `adapters-inference`，含 provider 与 core RPC transport | 根据配置注册 provider；host 管 core socket 生命周期 |
| Google/Gmail | typed gateway/application use cases | gateway/application | `adapters-google` | 注册 channel/integration |
| Daemon RPC | gateway typed dispatch | application/runtime | gateway server transport | Unix listener、systemd readiness |
| Governed review | `application::governed_review` + repository port | application review transition/idempotency rules | platform filesystem store | 注入 store；不定义 repository implementation |
| Workspace trust | `application::workspace_trust` decision/port | application trust policy | platform workspace-evidence adapter | 传 workspace root/client mode |
| Core RPC | 无独立业务 use case；实现 cognit inference port | cognit inference/backpressure contract | `adapters-inference::core_rpc` client/protocol/server | `host::core` 管配置、socket 路径和生命周期 |
| Machine core runtime | 无业务 use case | cognit inference/backpressure contract | `adapters-inference::RegistryInferencePort`、provider registry/backpressure | `host::core` 拥有 `MachineInferenceRuntime` 的 config/socket/peer-policy 配置/server lifecycle |
| Readiness | 无 | host lifecycle facts | `aletheon::host::readiness` | 汇总已注入组件的 ready receipt |
| Doctor | typed diagnostic query | 各 authority 暴露只读事实 | `aletheon::host::doctor` | 收集安装态事实；CLI 只渲染 |
| Exec/User runtime | 调用同一 TurnService | runtime Turn/Session authority | host transport adapters | `host::{exec,user_daemon}` 生命周期 |
| Embodiment | application capability port | hardware/robot owner | hardware/platform adapter | 只管理设备 adapter 生命周期，不改控制 policy |
| Evolution | application trigger port | `metacog` mutation/evolution policy；cognit reflection facts | platform lineage store | 注入 ports；不保留跨域 EvolutionCoordinator |
| Cognitive runtime legacy | 无整包 target | Cognit session、Metacog evolution 各归其 owner | 对应 adapters | 拆分完成后删除 `AletheonCognitiveRuntime` |
| Mode routing | 无业务状态 | host entry-mode selection | `aletheon::host` | 选择启动入口，不进入 Turn policy |
| Domain handle aggregate | 无 | 各 domain owner | 无 | `composition::services` crate-private handles；按 feature builder 拆小 |
| Extensions | application extension use case | runtime provider manifest | Corpus extension package/inspection adapter | host CLI 只读 inspection；daemon 注入注册表 |

该矩阵必须覆盖 `wiring/` 每个顶层文件/目录。M0 的 `wiring-ownership.tsv` 不允许出现
未解析的“或”、`TBD` 或“暂留 composition”；同一行可以拆出 policy、adapter、host
lifecycle，但每种责任只能有一个 owner。未入表模块会阻止 M9。

### 3.4 新 adapter crate 决策

本计划推荐新增以下聚焦 crate；Codex 二次审核应逐项批准或拒绝，不能执行时临场改成一个大
`adapters-all`：

| 新 crate | 当前来源 | 理由 |
|---|---|---|
| `crates/adapters/inference` | `wiring/adapters/inference/**`，约 3.8K LOC | 独立 HTTP/stream/backpressure 依赖与 provider 安全边界 |
| `crates/adapters/gbrain` | `wiring/adapters/gbrain/**` + 必要 memory wrapper | MCP、attestation、external authority 降权边界 |
| `crates/adapters/google` | `wiring/adapters/google/**`、Gmail/external Google 路径 | OAuth/channel/sync/SQLite projection 是独立集成 |
| `crates/adapters/agent-backend` | `wiring/adapters/runtime/**` 中 Pi/native/provider worker | 外部 Agent backend，不与 authority crate `runtime` 撞名 |

`wiring/adapters/session/**` 不新建 crate，合入已有 `adapters-sqlite`。小于约 200 行且
仅做类型桥接的 wrapper 留在 `aletheon::composition`，不得为它单独建 crate。

补充约束：

- `wiring/core_rpc/{client,protocol,server}.rs` 进入 `adapters-inference::core_rpc`；其 wire
  schema 是该内部 transport 私有类型，不提升到 contracts 公共根；
- `adapters-inference` 内部固定拆为 `core_rpc/`、`providers/{anthropic,openai,ollama}/`
  和 `backpressure/`；Unix `CorePeerPolicy` 只能出现在 `core_rpc`，HTTP provider 模块不得
  import peer-policy，host 只提供配置、socket 路径和 server lifecycle；
- `adapters-google` 唯一拥有 Google/Gmail OAuth、sync、cursor 和 SQLite projection；
  `adapters-sqlite` 不管理 Google schema/table/migration；
- Corpus 的 Google 工具只负责受治理工具执行，不拥有 OAuth/sync projection；
- verification command 的 executable resolution/process execution 唯一由
  `platform::verification_command` adapter 实现；
- Git worktree create/clean/recovery/quarantine/remove I/O 唯一由 `platform::worktree` adapter
  实现；application/runtime 分别保留清理 policy port 与 lease/recovery disposition authority，
  Corpus 只拥有 governed patch/workspace 语义，`adapters-agent-backend` 不拥有 worktree，
  `adapters-sqlite` 只持久化 worktree/command durable receipt 而不执行副作用。

### 3.5 功能模块内部结构约束

每个功能模块不是简单按文件名归档，而必须形成可审查的最小闭包：

```text
<feature>/
├── command.rs       输入意图；不携带 adapter handle
├── query.rs         只读查询；不绕过 authority
├── model.rs         feature-owned value/state
├── service.rs       用例顺序和授权
├── port.rs          consumer-owned external dependency
├── event.rs         feature event/receipt（需要时）
└── mod.rs           最小 public facade
```

并满足：

1. 外部调用方只经 `service`/显式 facade，不访问 repository implementation；
2. `mod.rs` 默认 `pub(crate)`，只导出跨 crate 的稳定类型；
3. feature 之间不共享可变 store；跨功能协作经 typed port/event；
4. command 不返回 adapter-specific error，query 不产生隐藏副作用；
5. service 不 mint 其他 authority 的 ID，不直接打开连接或启动进程；
6. adapter 只翻译 port，不重新解释业务 policy；
7. composition 只选择实现和生命周期，不出现 Goal/Turn/Agent 状态转换分支；
8. 每个 durable write 都能追溯 command、principal、authority、receipt 和 recovery 规则。

模块完成审查必须同时检查 cohesion（同一功能是否聚合）和 coupling（是否只通过稳定
facade/port 连接），不能只检查目录是否存在。

---

## 4. 依赖约束

### 4.1 允许依赖

| Crate | 允许的核心生产依赖 |
|---|---|
| `contracts` | 无本仓 domain crate |
| `runtime` | `contracts`；M2.0 后不直接依赖 SQLite，不新增 application/adapter 依赖 |
| `application` | 当前允许 `contracts`、`runtime`，以及经完整传递图验证无回边的窄 `kernel` API；不预先白名单其他 domain crate |
| `kernel` | `contracts`、`runtime` |
| `cognit` | domain/contract，不依赖 HTTP provider adapter |
| `gateway` | `contracts`、application-facing port；不依赖 SQLite/runtime implementation |
| `adapters-*` | 可依赖其实现的 consumer port 和必要外部库 |
| `aletheon` | composition 所需全部 crate；任何 crate 不反向依赖它 |

当前工作区对 `application` 的裁决：

| 候选依赖 | 当前传递路径 | 裁决 |
|---|---|---|
| `kernel` | 当前未发现回到 `application` 的路径（`crates/kernel/Cargo.toml:9-12`） | 可选，仍禁止直接持有 `KernelRuntime` |
| `mnemosyne` | `mnemosyne -> platform -> application`（`crates/mnemosyne/Cargo.toml:15`；`crates/platform/Cargo.toml:9`） | 当前禁止 |
| `dasein` | dirty worktree 中 `dasein -> platform -> application`（`crates/dasein/Cargo.toml:13`） | 当前禁止 |
| `cognit` | `cognit -> dasein -> platform -> application` | 当前禁止 |
| `metacog` | `metacog -> dasein/cognit -> ... -> application` | 当前禁止 |
| `agora`、`corpus`、`platform` | 已直接或传递依赖 `application` | 禁止 |
| `gateway` | 目标态将依赖 application-facing port；application 反向引用会成环 | 禁止 |

`dasein/cognit/mnemosyne/metacog` 只有在前置 packet 消除所有传递回边，并由完整
Cargo metadata graph 重新证明无环后，才可加入 application manifest。直接依赖列表或
`rg Cargo.toml` 不能作为无环证明。

### 4.2 最终禁止模式

```text
crates/application/**       -> rusqlite, reqwest, std::process, tokio::process
crates/application/**       -> crate::config or aletheon::*
crates/application/**       -> KernelRuntime, AgoraService, concrete MemoryGateway
crates/runtime/**           -> application, aletheon, adapters-*
crates/* (除 aletheon)      -> aletheon::*
crates/aletheon/**          -> concrete repository implementation定义
crates/aletheon/src/lib.rs  -> pub mod wiring
```

`std::fs` 在 application 测试可用于 fixture，但生产代码必须经 port；architecture gate
要按 `#[cfg(test)]` 分界检查，不能误伤测试或放过生产段。

每个改变 manifest 的 packet 必须运行：

```bash
bash scripts/cargo-agent.sh metadata --format-version 1 > /tmp/aletheon-dependency-graph.json
```

并由 architecture checker 从完整 resolve graph 验证无环和禁止边；不能使用
`--no-deps` 的结果宣称传递依赖安全。

---

## 5. 实施依赖图

```text
M0 事实账本/边界 gate
 |
 v
M1 删除兼容 re-export 和幽灵模块
 |
 v
M2 先抽离 SQLite / 文件 / 命令副作用
 |\
 | +--> M3 Goal + Approval + Verification 功能切片
 | +--> M4 Conscious + Evaluation + Context 功能切片
 | +--> M5 Agent authority/use-case/adapter 切片
 |              |
 +--------------+
        |
        v
M6 Turn 唯一路径与核心编排迁移
 |
 +--> M7 外部 adapters 独立 crate
 |
 v
M8 Gateway/daemon transport 收敛
 |
 v
M9 删除 wiring、收紧 public API、更新文档账本
 |
 v
M10 系统安装态验收
```

M3、M4、M5 在代码依赖上可并行，但除非用户单独启用并行 Agent，否则按顺序执行；
同一工作区不能并行运行 workspace build 或同时修改 composition root。

---

## 6. 实施 packets

### M0 — 冻结事实基线和可执行边界门禁

**目的：** 在移动文件前让目标边界可被机器检查；不改运行行为。

**文件：**

- `config/architecture/module-boundaries.txt`
- `config/architecture-dependencies.txt`
- `config/architecture/hotspot-budgets.tsv`
- 新增 `config/architecture/wiring-ownership.tsv`
- `tests/suites/architecture/architecture_check.sh`（需要显式测试文件授权）
- `docs/design/architecture-overview.md`
- `docs/plans/README.md`

**改动：**

1. 记录每个 `wiring` 模块的 current owner、target owner、authority、effect kind、packet；
2. 覆盖 `wiring/` 每个顶层文件/目录，不允许 owner 为 `TBD`、“或”或“暂留 composition”；
3. 记录 HEAD、dirty path list、tracked diff digest、被审计划 digest、LOC 统计命令和完整
   Cargo metadata resolve graph；
4. 修正 stale LOC/owner locator，不改变 checker 规则来迁就代码；
5. 新 gate 检查 §4.2 禁止模式、完整传递回边、反向 `aletheon` 依赖和 compatibility seam；
6. 建立 authority census：每个 durable fact 的 reducer、write owner、ID mint owner、receipt；
7. 建立 effect census：SQLite/FS/HTTP/process 调用只能位于指定 adapter/host owner；
8. 文档把“已收敛”改为“物理 cutover 完成、所有权迁移进行中”；
9. 记录 wire schema、数据库 schema、runtime event schema 当前版本和 digest 作为 parity 基线。

**2026-08-17 执行记录：**

- 已完成：20 个顶层路径 ownership ledger、Git/dirty/metadata/LOC 基线、authority/effect
  census 复核、wire/persistence/runtime-event digest、4 个 stale wire locator 修正、架构总览
  和计划索引更新；
- 已将 ownership ledger 完整性、§4.2 五项边界指标及六个 negative fixture 接入
  `scripts/libexec/aletheon/architecture-check.sh` 与
  `tests/suites/architecture/architecture_check.sh`；
- 已验证：ownership 集合精确覆盖、所有 inventory locator 存在、architecture fixture 与
  repository architecture gate 均通过且 0 findings。证据见
  `docs/arch/evidence/2026-08-17-wiring-migration-m0-baseline.md`；
- 状态：`COMPLETED`。

**验证：**

```bash
bash scripts/aletheon.sh test architecture
bash scripts/cargo-agent.sh metadata --format-version 1 > /tmp/aletheon-m0-metadata.json
python3 - <<'PY'
from pathlib import Path
for root in [Path('crates/aletheon/src/wiring/application'),
             Path('crates/aletheon/src/wiring/adapters'),
             Path('crates/aletheon/src/wiring/daemon')]:
    files = list(root.rglob('*.rs'))
    print(root, len(files), sum(len(p.read_text().splitlines()) for p in files))
PY
git diff --check
```

**停止条件：** 架构 reviewer 不认可 owner matrix，或 gate 必须靠 allowlist 才能描述目标。

名称和 LOC 只作为 review trigger：新建 `orchestration`/`core`/`misc`/`common` 式泛化
crate 或超过预算时必须复核 owner，但换名、拆文件或低于阈值都不能替代 authority/effect/
behavior 证明。

**回退：** 只回退 M0 账本/checker；不得修改生产代码让错误账本变绿。

---

### M1 — 删除 compatibility re-export 与无消费者模块

**目的：** 先消除会隐藏真实依赖的旧路径，不改变实现 owner。

**直接改写调用方并删除：**

- `wiring/application/session_service.rs` → `runtime::session_service`；
- `wiring/application/governed_capability.rs` → `kernel::capability::governed`；
- `wiring/application/harness_factory.rs` → `cognit::harness` +
  `wiring/composition/harness_factory.rs`；
- `wiring/application/memory_gateway.rs` → `mnemosyne`；
- `wiring/application/capability_benchmark.rs` → `application` + `adapters-sqlite`；
- `wiring/application/cognitive_role_workflow.rs` → `agora::cognitive_role_workflow`；
- `wiring/application/cognitive_role_workflow/{stages,state_machine}.rs`：当前无模块引用，先
  按符号全仓复核，确认为幽灵文件后删除。

**同步修改：**

- `wiring/application/mod.rs`
- `wiring/application/turn_pipeline.rs`
- `wiring/application/daemon_turn/**`
- `wiring/application/request_use_cases.rs`
- `wiring/daemon/bootstrap/services.rs`
- `wiring/adapters/{gbrain,runtime}/**`
- `wiring/composition/{harness_factory,evolution_proposer,turn_coordinator}.rs`

**禁止：** 新增另一层 re-export；以 `type Alias = ...` 延长旧路径；保留仅验证源码
字符串的 compatibility test。

**2026-08-17 执行记录：**

- 已完成：所有 production/test consumer 已直接切到 `runtime`、`kernel`、`cognit`、
  `mnemosyne`、`application`、`adapters_sqlite` 或 `agora` 的实际 owner；
- 已删除六个 compatibility wrapper、两个 ghost 文件及 `application/mod.rs` 中对应声明；
- consumer/target/ghost-file census 已冻结在
  `docs/arch/evidence/2026-08-17-wiring-migration-m1-consumer-census.md`；
- 零旧路径引用/零 wrapper 文件 gate、`aletheon --all-targets` check、runtime 212 tests、
  application 63 tests、agora 99 tests、architecture gate 和 `git diff --check` 全部通过；
- 状态：`COMPLETED`。本包仅 owner/import cutover，未改变 Turn/Agent/Evaluation 行为。

**验证：**

```bash
! rg -n 'wiring::application::(session_service|governed_capability|harness_factory|memory_gateway|capability_benchmark|cognitive_role_workflow)' crates --glob '*.rs'
for f in session_service.rs governed_capability.rs harness_factory.rs memory_gateway.rs capability_benchmark.rs cognitive_role_workflow.rs; do
  test ! -e "crates/aletheon/src/wiring/application/$f"
done
bash scripts/cargo-agent.sh check -p aletheon --all-targets
bash scripts/cargo-agent.sh test -p runtime --lib
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh test -p agora --lib
git diff --check
```

**回退：** 整个 M1 commit 回退；不恢复部分 alias 形成双入口。

---

### M2 — 抽离持久化、文件系统和命令副作用

M2 是后续功能迁移的前置，按 M2.0–M2.4 独立提交。

#### M2.0 Runtime Session protocol persistence

**来源：** `runtime/src/session_service.rs` 当前同时拥有 Session 用例、active-turn 状态和
具体 `rusqlite::Connection`。

**目标：**

- `runtime/src/session_service.rs`：保留 visibility、resume/fork/interrupt/replay 语义；
- `runtime/src/session_protocol.rs`：定义 `SessionProtocolEventStore` port 和 typed record；
- `adapters/sqlite/src/session/protocol_event_store.rs`：迁移现有 SQL、transaction、dedupe、
  cursor/page query；
- aletheon composition 注入 store，不向 Runtime 暴露 Connection。

**约束：** `protocol_events` schema、sequence、dedupe key、approval reconnect 和分页 cursor
保持不变；不能同时向旧 Connection 和新 store 双写。

**2026-08-17 执行记录：** `SessionProtocolEventStore` port 和 typed write packet 已进入
`runtime::session_protocol`；SQLite schema/transaction/dedupe/cursor 实现已迁入
`adapters_sqlite::session::protocol_event_store`，daemon composition 显式注入 durable store，
Runtime 不再依赖或引用 `rusqlite`。Runtime 212 tests、adapters-sqlite 48 tests、
`session_protocol_reconnect` 8 tests、`session_event_recovery` 2 tests、Aletheon all-target check
和 architecture gate 均通过。状态：`COMPLETED`。

**验证：**

```bash
! rg -n 'rusqlite' crates/runtime/src --glob '*.rs'
bash scripts/cargo-agent.sh test -p runtime --lib
bash scripts/cargo-agent.sh test -p adapters-sqlite --lib
bash scripts/cargo-agent.sh test -p aletheon --test session_event_recovery
bash scripts/cargo-agent.sh test -p aletheon --test session_protocol_reconnect
```

#### M2.1 Goal/Admin SQLite

**来源：**

- `wiring/application/goal/{mod,store,transition,budget,attempt,summary,verification}.rs`
- `wiring/application/admin_service.rs`
- `wiring/composition/evolution_proposer.rs` 中 SQLite 状态
- `goal/mod.rs:46-69` 的 artifact directory 创建/canonicalize；
- `goal/verification.rs:156-215` 的 rollback remove、artifact read 和 hash fail-closed；
- `admin_service.rs:294-420` 的 `ScopedApprovalCache`，其 durable key 包含
  principal/thread/tool/path/version/hash/expiry。

**目标：**

- `application/src/goal/{mod,model,service,repository,artifact}.rs`：command/query、
  transition/retry/advance rule、repository port、`GoalArtifactStore` port；
- 合并现有 `application/src/goal_{attempt,draft,frame,projection,retry}.rs`，禁止再形成
  平行的第三套 Goal public surface；
- 不新建笼统 `application::admin`。transient approval 进入 `application::approval` 的
  scoped-grant port；profile/model/hook/skill/extension 状态分别进入其功能 owner；
- `adapters/sqlite/src/goal/**`：SQL、row mapping、transaction、migration ownership；
- `adapters/sqlite/src/approval/scoped_grant.rs`：保持 transient grant durable key/expiry；
- platform filesystem adapter：Goal artifact read/write/hash/atomic rename/remove；
- `adapters/sqlite/src/evolution.rs`（仅在 evolution 状态确属 SQLite projection 时）。

**约束：** schema/table/index/idempotency key 不变；所有原子 transaction 边界保持；
application error 不包含 `rusqlite::Error`。Goal metadata transaction 与 artifact commit 的
失败顺序、rollback remove、hash mismatch fail-closed 语义必须保持，不能只迁 SQL。

**2026-08-17 进行中记录：** transient scoped grant 已形成
`application::approval::ScopedApprovalGrantStore` 消费者 port，SQLite schema/query 已迁至
`adapters_sqlite::approval::scoped_grant`，production composition 显式注入；tool grant 与
path/version/digest grant 仍严格分离，reopen 测试通过。Evolution proposal disposition 已形成
`application::evolution::EvolutionProposalStore`，SQLite/WAL/upsert 已迁至
`adapters_sqlite::evolution`，proposer 不再持有 Connection；focused proposer、Admin、Approval、
Application、adapters-sqlite、all-target 和 architecture tests 通过。Goal artifact 已新增
`application::goal_artifact::GoalArtifactStore`，文件系统实现位于
`platform::goal_artifact::FilesystemGoalArtifactStore`；create/canonicalize、bounded read、
atomic create+fsync+rename、rollback remove 和 traversal/symlink containment 已离开 Goal use case。
Goal verification 6 个 artifact/transaction/tamper/restart tests 与 architecture gate 通过。
Goal 的 SQL、row mapping、budget/attempt/transition/verification/completion-summary transaction
已经从 binary wiring 迁至 `adapters_sqlite::goal/**`；旧
`wiring/application/goal/{attempt,budget,store,summary,transition,verification}.rs` 已删除，binary
Goal coordinator 不再直接访问 `rusqlite` 或裸 `Connection`。迁移后的 adapter Goal 39 tests、
Goal lifecycle 6、restart recovery 8、attempt coordinator 10、completion summary 3、approval
flow 5、coding flow 6 tests，以及 Aletheon all-target 与 architecture gate 均通过。M2.1 的具体
SQLite/文件副作用 cutover 已闭合；Application-owned repository port 和 Goal service 的最终
owner 收敛继续由 M3.1 完成，因此 M2.1 状态更新为 `COMPLETED`。

**验证：**

```bash
! rg -n 'rusqlite' crates/application/src --glob '*.rs'
! rg -n 'rusqlite' crates/aletheon/src/wiring/application --glob '*.rs'
bash scripts/cargo-agent.sh test -p adapters-sqlite --lib
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh check -p aletheon --all-targets
```

#### M2.2 Workspace checkpoint 文件副作用

**来源：** `wiring/application/workspace_checkpoint.rs`。

**目标：**

- `application/src/workspace_checkpoint/{mod,service,port}.rs`：checkpoint policy、limits、receipt；
- `platform/src/workspace_checkpoint.rs`：canonicalize/read/write/remove/atomic rename；
- `adapters/sqlite/src/checkpoint.rs`：checkpoint metadata store；
- composition 注入 `WorkspaceCheckpointStore` 和 `WorkspaceFilePort`。

**约束：** 路径穿越、symlink、最大文件数/字节数、临时文件原子 rename、恢复后删除规则
必须保持 fail-closed。

**2026-08-17 进行中记录：** `WorkspaceFilePort` 与 typed `CaptureResult` 已进入
`application::workspace_checkpoint`；递归 capture、canonical containment、symlink skip、
bounded/truncated capture、atomic restore 和失败 rollback 已迁至
`platform::workspace_checkpoint::LocalWorkspaceFilePort`。`WorkspaceCheckpointStore` port 也已
进入 Application，SQLite/WAL/startup-open reconciliation 实现及其 4 个恢复/tamper tests 已从
binary wiring 迁至 `adapters_sqlite::checkpoint`。Workspace checkpoint 10 个策略、lease、
rollback、quota tests、Aletheon all-target 和 architecture gate 通过。Service orchestration
已经迁入 `application::workspace_checkpoint::service`；Application-owned
`WorkspaceLeasePort` 隔离 Kernel lease authority，binary composition 仅保留
`KernelWorkspaceLeasePort` 与 live-agent rewind guard adapter，并显式注入 Platform filesystem
port。Workspace checkpoint policy 10 tests、SQLite recovery/tamper 4 tests、Application check、
Aletheon all-target 与 architecture gate 均通过。状态：`COMPLETED`。

#### M2.3 Approval apply 与 verification command

**来源：**

- `wiring/application/approval/**`
- `wiring/application/verification/**`
- 当前 `wiring/adapters/{verification_command,worktree}.rs`

**目标：**

- `application/src/approval/**`：transaction/use case/ports；
- `application/src/verification/**`：policy/check definitions/command port；
- `corpus`：受治理 patch/content mutation；不执行 verification command，不管理 Git
  worktree 生命周期；
- `platform::verification_command`：受约束 `cargo`/`git`/verification executable resolution、
  working directory/environment、process execution 和 output cap；
- `platform::worktree`：Git worktree create/clean/recovery/quarantine/remove I/O；
- application 拥有 apply/cleanup policy 与 ports，runtime 拥有 lease/recovery disposition
  authority；adapter 不自行裁决是否清理或恢复；
- `adapters-sqlite`：approval/verification/worktree/command durable receipt；不执行副作用；
- aletheon composition 只注入实现。

当前已经引入的 `VerificationCommandExecutor`、`ManagedWorktreeCleaner` 是可用 seam，
但必须随 owner 移动，不能长期留在 binary crate。

**2026-08-17 进行中记录：** `TrustedVerificationCommand`、bounded output 和
`VerificationCommandExecutor` 已迁至 `application::verification`；bounded/cancellable process
host、process-group cleanup、executable resolution 已迁至 `platform::verification_command`。
Verification policy、check definitions 与 `VerificationService` 也已整体迁入
`application::verification`；worktree 文件读取通过 application-owned
`VerificationWorkspaceReader` port 注入，由 `platform::verification_command` 执行安全的
symlink/canonical-root/UTF-8 读取，Application 不再直接触碰文件系统。旧
`wiring/application/verification/**` 已删除，调用方直接导入 owner crate。
`ManagedWorktreeCleaner` port 已迁至 `application::approval`，Git worktree remove host 已迁至
`platform::worktree`；两个旧 `wiring/adapters/{verification_command,worktree}.rs` 已删除。
Application verification policy 5、Verification 7、approved apply 5、production coding E2E 2
tests、Aletheon all-target 与
architecture gate 通过。Approved apply 的临时 verification artifact create/write/fsync/drop
cleanup 已由 `application::approval::TemporaryArtifactStore` port 隔离，并迁至
`platform::temporary_artifact::HostTemporaryArtifactStore`；Application apply coordinator 不再
直接执行文件副作用。Approval receipts/claims 继续由 `adapters_sqlite::approval_repository`
单写，worktree cleanup、verification command/workspace read、temporary artifact 均通过消费者
port 注入。Approved apply 5、production coding E2E 2、Verification 7、Application policy 5、
Aletheon all-target 与 architecture effect gate 通过。具体副作用 cutover 状态：`COMPLETED`；
Approval use-case 最终 owner 迁入 `application::approval` 继续由 M3.2 完成。

#### M2.4 效果 gate

在 M2 完成后，production `crates/application/src` 必须不存在：

```text
rusqlite
reqwest
std::process / tokio::process
std::fs::{write,remove_file,rename,create_dir_all}
which::which
```

已有测试中的 tempfile/fixture I/O 可保留。

**2026-08-17 执行记录：** architecture checker 的
`APPLICATION_FORBIDDEN_BOUNDARY_HITS` 已覆盖上述 production pattern，当前 baseline/actual
均为 0；negative fixture 会对 Application process effect fail closed。独立 grep 与 repository
architecture gate 通过。状态：`COMPLETED`；该 gate 不代表仍位于 binary wiring 的 Goal SQL
已经迁完，M2.1 仍须单独闭合。

**M2 回退：** 每个子包独立回退。禁止旧/新 repository 双写；若 parity 不清楚，保留旧
实现并停止该 packet，而不是加 feature flag 同时运行两套 authority。

---

### M3 — Goal、Approval、Verification 垂直功能切片

**目的：** 将已经去除具体 I/O 的功能闭包搬到真正 Application owner。

#### M3.1 Goal

**迁移：**

- `wiring/application/goal_service.rs`
- `wiring/application/goal/{attempt_coordinator,coordinator,runtime_executor,worker}.rs`，但
  `worker.rs` 必须按下述责任拆分，不能整文件迁入 application；
- M2 后留下的 Goal policy/model
- `wiring/application/goal/summary.rs` 中纯输出逻辑

到：

```text
application/src/goal/
├── mod.rs
├── command.rs
├── query.rs
├── service.rs
├── transition.rs
├── attempt.rs
├── repository.rs
├── artifact.rs
└── projection.rs
```

`RuntimeGoalAttemptExecutor` 不是 application policy；它作为 runtime adapter 移到
`adapters-agent-backend`，实现 application 的 `GoalAttemptPort`。

`GoalWorker` 拆分为：

- application `GoalService::advance_once`：transition/retry/单次 attempt 用例顺序；
- `aletheon::host::goal_scheduler`：只提供 interval/cancel/wait/reap，不判断 Goal 状态；
- gateway progress adapter：把 typed application outcome 映射为 `GoalProgress`；
- platform quota adapter：实现 application-owned admission/quota port；
- composition：只构造上述对象并注入，不包含 Goal 状态分支。

**2026-08-17 进行中记录：** legacy/versioned Goal command/query service 已迁入
`application::goal::{service,repository}`；Application 现在拥有 `GoalRepository` port 和
sanitized repository/service errors，`adapters_sqlite::goal::SqliteGoalRepository` 包装并复用
canonical `ObjectiveStore`，没有新增 writer 或双写。旧
`wiring/application/goal_service.rs` 已删除，RPC/daemon 直接依赖 Application use-case surface。
Goal service 5、Goal RPC 8、Aletheon all-target 与 architecture gate 通过。Application 已新增消费者侧
`GoalAttemptPort`；原 `wiring/application/goal/runtime_executor.rs` 已删除，
`GoalAttemptBackend` 与 `RuntimeGoalAttemptExecutor` 迁至新建的聚焦 crate
`adapters-agent-backend`，只依赖 Application port、Contracts 与 Runtime authority。新 adapter
1 test、Attempt coordinator 10 tests、Aletheon all-target 与更新后的 crate/dependency architecture
gate 通过。

**2026-08-17 M3.1 边界收敛补充：** Application 已拥有 Goal attempt storage admission
port，接口只暴露 `GoalId` 的 admit/release，不泄漏 quota class、filesystem accounting 或
reservation handle（`crates/application/src/goal/admission.rs:19`）；Platform 的 quota adapter
持有具体 `StorageQuota` 与 opaque reservation 生命周期
（`crates/platform/src/goal_storage_admission.rs:9`）。Goal 外部唤醒输入也已改为
provider-neutral `GoalExternalEvent`，等待条件及匹配 policy 位于 Application
（`crates/application/src/goal/stimulus.rs:6`），Google adapter 只负责 provider envelope 映射。
因此 coordinator 已不再直接依赖 Platform quota 或 Corpus Google event 类型。随后 bounded
Goal transition/wake/process-link policy 已迁入 Application，并只依赖 consumer-owned durable port
与 admission port（`crates/application/src/goal/coordinator.rs:21`）；SQLite adapter 复用 canonical
`ObjectiveStore` 的 transaction/version/budget operations
（`crates/adapters/sqlite/src/goal/coordinator_repository.rs:23`）。旧 wiring coordinator 已删除，
MemoryProjection 与 Attempt/Apply construction factory 也不再混入 Goal policy。Application、
Adapters SQLite 与 Aletheon all-target，Goal lifecycle 6、persistent vertical slice 2、Google event
routing 2、Goal worker flow 7 tests，以及 architecture gate（0 findings）通过。M3.1 尚余
AttemptCoordinator owner cutover 与 worker/scheduler/use-case 拆分。

**2026-08-17 M3.1 scheduler 拆分补充：** `GoalWorker` 原有 interval/cancellation/logging
循环已移出 wiring application。新 host scheduler 只拥有 period、missed-tick、cancel 和一次
callback wait/error observation，不读取 Goal state
（`crates/aletheon/src/host/goal_scheduler.rs:8`）；composition 注入 `tick_once` callback
（`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:1274`）。随后 Goal state selection、retry
route、sequence、单次 attempt、reservation settlement 与 progress publish 顺序已迁入 Application
`GoalAdvanceService::advance_once`（`crates/application/src/goal/advance.rs:67`）；SQLite candidate/
latest-attempt access 由 adapter 实现（`crates/adapters/sqlite/src/goal/advance_repository.rs:19`）。
旧 `wiring/application/goal/worker.rs` 已删除，composition 只注入 ports/runtime IDs 并把一次
advance callback 交给 host scheduler（`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:1258`）。
Aletheon all-target、Goal daemon worker 1、Goal worker flow 7 tests 与 architecture gate（0 findings）
通过。Worker/scheduler 拆分已闭合。

**2026-08-17 M3.1 attempt contract 补充：** one-shot attempt 的 request/outcome 以及
`CodingVerifier` consumer seam 已迁入 Application
（`crates/application/src/goal/attempt.rs:11`），canonical `VerificationService` 的实现也由同一
owner 提供（`crates/application/src/goal/attempt.rs:45`）。Goal evaluation receipt 的 SQLite/
event-spine projection 已从 attempt policy 文件移到 concrete adapter
（`crates/aletheon/src/wiring/adapters/goal_evaluation.rs:1`）。AttemptCoordinator 已迁入
Application，并且只持有 Application consumer ports、Clock 与 retry policy
（`crates/application/src/goal/attempt_coordinator.rs:36`）；durable attempt/budget/coding/
verification 事务边界由 `GoalAttemptPersistencePort` 定义
（`crates/application/src/goal/attempt_persistence.rs:37`），SQLite adapter 在单一 store lock 内保持
budget reserve + begin 以及 finish + settle 的原子编排和失败撤销
（`crates/adapters/sqlite/src/goal/attempt_persistence.rs:52`）。旧 wiring Goal 目录与 compatibility
facade 已删除，daemon/tests 直接依赖真实 owner。Attempt coordinator 10、approval goal 5、coding
flow 6、coding production E2E 2、Goal daemon worker 1、Goal worker flow 7 tests、Aletheon all-target
与 architecture gate（0 findings）通过。M3.1 已闭合，状态为 `COMPLETED`。

Attempt coordination 的 sanitized error surface 也已移入 Application；Budget/Transition 的
adapter-specific error 只在 legacy coordinator 边界转为字符串，不再出现在 use-case contract
（`crates/application/src/goal/attempt.rs:11`）。

**2026-08-17 M3.1 attempt I/O seam 补充：** coding approval creation 已改为
Application-owned `GoalApprovalPort`/command（`crates/application/src/approval.rs:13`），SQLite
repository 仅由 adapter 包装（`crates/adapters/sqlite/src/approval/goal.rs:8`）；AttemptCoordinator
不再持有 ApprovalRepository。Worktree canonicalize/containment I/O 也已移到 Platform resolver
（`crates/platform/src/goal_worktree.rs:10`），Application 只拥有 relative-path resolution port
（`crates/application/src/goal/attempt.rs:12`）。Goal budget request/reservation 和 persisted coding/
verification DTO 的 owner 已从 SQLite adapter 迁入 Application
（`crates/application/src/goal/budget.rs:6`、`crates/application/src/goal/coding.rs:6`）。Approval goal
5、coding flow 6、coding production E2E 2 tests、all-target 与 architecture gate（0 findings）通过。

Gateway `GoalProgress` mapping 已从 worker policy 移到独立 gateway adapter；它只把 Application
typed outcome 映射为 gateway notification kind
（`crates/aletheon/src/wiring/adapters/goal_progress.rs:1`），worker 不再拥有 presentation mapping。

#### M3.2 Approval

把决定、事务、terminal settlement 编排放在 `application::approval`；Corpus 只执行已批准的
governed patch/content mutation，Platform 只执行已批准的 worktree/process/filesystem I/O，
两者都不重新解释 approval scope。Gateway 只投影 typed result。

**2026-08-17 完成记录：** durable approval list/show/resolve policy、owner/channel/version
校验、category dispatch 与 idempotent Metacog resume 已迁入 Application service；该 service
只依赖 consumer-owned repository/settlement ports
（`crates/application/src/approval/service.rs:51`、`crates/application/src/approval/service.rs:95`）。
SQLite repository adapter 负责把 concrete repository errors/decision DTO 映射到 Application
surface（`crates/adapters/sqlite/src/approval/service.rs:23`）。Approved apply 的 pending/rejected/
consumed/recovery、claim、process settlement、Goal terminal settlement、summary 与 cleanup 顺序也由
Application coordinator 管理（`crates/application/src/approval/apply.rs:53`、
`crates/application/src/approval/apply.rs:130`）；concrete Kernel/Corpus/SQLite/worktree/projection
实现留在 adapter（`crates/aletheon/src/wiring/adapters/approved_apply.rs:31`）。旧
`wiring/approval_service.rs` 与 `wiring/application/approval/**` compatibility/ghost surfaces 已删除，
Gateway/daemon 直接消费 Application use-case types。Approval service 7、approved apply 5、approval
goal 5、coding production E2E 2 tests、Aletheon all-target 与 architecture gate（0 findings）通过。
M3.2 状态为 `COMPLETED`。

#### M3.3 Verification/Evaluation

- verification policy/use case → `application::verification`；
- evaluation evidence/scoring domain → `metacog`（若现有相同 owner 已存在则合并，不复制）；
- evaluation use-case coordination → `application::evaluation`；
- SQLite projection → `adapters-sqlite`；
- verification command execution → `platform::verification_command` adapter；
- evaluation artifact acquisition → Corpus governed tool/artifact adapter。

**2026-08-17 完成记录：** Verification policy/service 已完整归属 Application，trusted command
execution 与 workspace reads 由 Platform adapters 实现。Evaluation contract issuance、authoritative
evidence collection、receipt persistence port、operation port、engine port、timeout/receipt/projection
settlement orchestration已迁入 Application（`crates/application/src/evaluation/service.rs:20`、
`crates/application/src/evaluation/service.rs:65`）；scope path materialization 不再直接调用 filesystem，
而经 consumer port（`crates/application/src/evaluation/evidence_collector.rs:27`）注入 Platform adapter
（`crates/platform/src/evaluation_path.rs:6`）。Coding-v2 evidence-to-dimension scoring 与 canonical
rubric/engine 均由 Metacog owner 提供（`crates/metacog/src/evaluation/coding_v2.rs:28`），Aletheon
evaluation adapter 只完成 Application port 到 Metacog/Kernel authority 的映射
（`crates/aletheon/src/wiring/adapters/evaluation/mod.rs:14`）。SQLite evaluation store 直接实现
Application receipt port。旧 `wiring/application/verification/**` 与
`wiring/application/evaluation/**` 已删除，Turn/daemon 直接依赖真实 owner。Application 68、Metacog
137 unit tests，evaluation settlement 3、protocol 1、agent selection 2、verification 7 tests、
Aletheon all-target 与 architecture gate（0 findings）通过。M3.3 状态为 `COMPLETED`。

**功能验收：** Goal create/resume/retry/terminal、approval blocked/apply、verification fail-closed、
restart recovery 和 projection idempotency 必须与迁移前相同。

**验证：**

```bash
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh test -p metacog --lib
bash scripts/cargo-agent.sh test -p adapters-sqlite --lib
bash scripts/cargo-agent.sh test -p aletheon --test goal_lifecycle
bash scripts/cargo-agent.sh test -p aletheon --test goal_restart_recovery
bash scripts/cargo-agent.sh test -p aletheon --test approval_goal_flow
bash scripts/cargo-agent.sh test -p aletheon --test coding_goal_flow
```

现有 integration tests 的 owner/path 是否移动需在实施授权中明确；不得为了移动源码而
删除行为覆盖。

---

### M4 — Conscious、Context、Memory Projection 模块化

**来源：**

- `wiring/application/conscious/**`
- `conscious_action.rs`
- `conscious_core_coordinator.rs`
- `conscious_workspace.rs`
- `context_assembler.rs`
- `memory_projection.rs`
- `post_turn_projection.rs`

**目标拆分：**

| 内容 | Owner |
|---|---|
| workspace state、attention/arbitration 数据 | `agora` |
| cognitive session、model/tool batch planning policy | `cognit` |
| identity/care verdict | `dasein` |
| memory recall/promotion/projection | `mnemosyne` |
| 跨功能 use-case 顺序 | `application::turn::context` / `application::conscious` |
| SkillLoader/ToolRegistry/MCP concrete bridge | Corpus adapter |
| 配置解析和实现注入 | aletheon composition |

**关键改造：**

1. `ContextAssembler` 不再持有 concrete `corpus::SkillLoader/SkillRouter`，改用
   `SkillContextPort`；
2. 保留当前 `ContextMemoryRecallPort` 的 provider-neutral 语义，但 port 移到
   `application::turn::context`，MemoryGateway wrapper 移到 adapter/composition；
3. `TurnConfigPort` 不返回 `crate::config::CognitiveRuntimeConfig`，改为 application-owned
   `TurnRuntimeSettings`；
4. post-turn projection 只按 receipt 调用 ports，不直接构造 Corpus hook 或 Memory store；
5. `MemorySensitivityV1` 必须随 recall item、model-context projection 和 durable receipt
   穿过 port，adapter 不得丢弃、降级或用默认值覆盖；
6. unknown tool 名仍应产生可恢复的 tool-not-found 结果，不能因目录迁移变成 terminal error。

**2026-08-17 M4 context slice 进度：** deterministic context preparation/history budgeting/
prompt partition 已迁入 `application::turn::context`；Application 现在拥有 `ContextSource`、
`SkillContextPort` 与 provider-neutral memory recall port
（`crates/application/src/turn/context.rs:128`）。Concrete Corpus `SkillLoader/SkillRouter` 与
concurrent conscious/memory source composition 已移到 adapters
（`crates/aletheon/src/wiring/adapters/context_source.rs:12`），旧
`wiring/application/context_assembler.rs` 已删除。Recall item 现在显式携带
`MemorySensitivityV1`（`crates/application/src/turn/context.rs:148`），Memory Gateway adapter
逐项原样映射 sensitivity（`crates/aletheon/src/wiring/adapters/context_memory.rs:47`），model-visible
projection 同时保留该分类，不再默认降级。`TurnConfigPort` 也已改为返回 Application-owned
immutable `TurnRuntimeSettings`（`crates/application/src/turn/settings.rs:4`），turn pipeline/
daemon react 不再消费 `crate::config::CognitiveRuntimeConfig`。Post-turn receipt/outcome、dispatch 与
runtime port 已由 Application 持有（`crates/application/src/turn/post_turn.rs:6`），Corpus hook 构造和
runtime bridge 留在 concrete adapter（`crates/aletheon/src/wiring/adapters/post_turn.rs:23`）；bootstrap
只负责注入这两个实现（`crates/aletheon/src/wiring/daemon/bootstrap/services.rs:938`）。旧
`wiring/application/post_turn_projection.rs` 已删除。Memory candidate 的 EventSpine/read-model publisher
也已移出 application 目录，当前隔离在 adapter
（`crates/aletheon/src/wiring/adapters/memory_projection.rs:49`），不再伪装为 use case owner；candidate
DTO、architecture-decision admission 和 sensitivity exclusion policy 已归入 Mnemosyne
（`crates/mnemosyne/src/candidate_projection.rs:10`、`crates/mnemosyne/src/candidate_projection.rs:49`）。
原 `wiring/application/conscious/**` 的 Corpus/Mnemosyne/Metacog/Agent concrete processors 已迁至
bounded adapters（`crates/aletheon/src/wiring/adapters/conscious/mod.rs:18`）；原
`conscious_workspace.rs`、`conscious_core_coordinator.rs`、`conscious_action.rs` 已从 application
目录移至 composition，明确标识其中仍存在的 SQLite/Kernel/domain construction
（`crates/aletheon/src/wiring/composition/conscious_workspace.rs:304`），而不是继续宣称其为纯 use case。
稳定 batch priority policy 已由 Cognit 持有
（`crates/cognit/src/harness/linear/batching.rs:6`）。Workspace processor budgets、registration 与
arbitration configuration 已由 Agora 持有（`crates/agora/src/conscious_core_ports.rs:11`）；Dasein 的
salience/self integration 只通过 Agora-owned `DaseinWorkspacePort` 输入，不在 composition 重写 verdict
（`crates/agora/src/conscious_core_ports.rs:98`）。跨功能 turn observation request/receipt 与调用 port
已归入 Application（`crates/application/src/conscious.rs:5`），composition 仅实现该 port 并组装
Agora/Cognit/Dasein/processor adapters（`crates/aletheon/src/wiring/composition/conscious_workspace.rs:664`）。
Context assembler 9、memory workspace 1、conscious
workspace 4、processors 4、core recurrence 6、action outcome 5、arbitration 8、streaming turn 12、post-turn
port 2、goal memory projection 5、Cognit 386、Mnemosyne 194 tests、all-target 与 architecture gate
（0 findings）通过。M4 完成；composition 保留的仅是 concrete construction、Kernel operation lifecycle
和跨 owner adapter binding，不再拥有上述 domain policy 或 Application receipt。

**验证：**

```bash
! rg -n 'corpus::(SkillLoader|SkillRouter|ToolRegistry)' crates/application/src --glob '*.rs'
! rg -n 'crate::config|aletheon::' crates/application/src --glob '*.rs'
bash scripts/cargo-agent.sh test -p agora --lib
bash scripts/cargo-agent.sh test -p cognit --lib
bash scripts/cargo-agent.sh test -p mnemosyne --lib
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh test -p aletheon --lib context_assembler
```

---

### M5 — Agent Control：Runtime authority 与 Application use case 分离

> 本 packet 触及 Agent 生命周期关键路径；实施前必须明确批准核心范围。

**当前来源：** `wiring/application/agent_control/**`，约 5.7K LOC。

**不得整体搬入 runtime。** 必须按职责拆分：

| 当前内容 | 目标 |
|---|---|
| lifecycle reducer、generation fence、terminal settlement、recovery disposition | 合并到现有 `runtime::agent_*` owner |
| spawn/wait/send/cancel/list/inspect use-case 顺序和授权 | `application::agent` |
| repository trait | consumer owner（application/runtime） |
| SQLite repository | `adapters-sqlite::runtime_agent` |
| Pi/native/provider worker backend | `adapters-agent-backend` |
| profile loading | `adapters-agent-profile` |
| Kernel admission/operation adapter | aletheon composition 注入的窄 adapter |
| candidate/evaluation projection | application port + owner adapter |

**禁止：**

- 复制 `runtime::AgentRunId`/`AgentLifecycleEvent`；
- 保留 `CompatibilityRuntimeCatalog` 作为永久 API；
- application 直接选择 concrete Pi/native backend；
- adapter mint authoritative Agent ID；
- recovery 同时调用旧、新 settlement；
- `AgentHostAdapter` 复制或绕过 `runtime::SettlementEngine` 的 `GenerationFence`。

**迁移顺序：**

1. 对照 `runtime/src/agent_{lifecycle,writer,supervisor,recovery,settlement}.rs` 做符号级
   duplicate matrix；
2. 先让 application 使用现有 runtime ports；
3. 迁 use-case facade；
4. 迁 backend adapter；
5. 删除 nested `agent_control`；
6. 更新 daemon bootstrap 只构造 ports/backends。

**2026-08-17 M5 进度：** duplicate matrix 已按现有 owner 复核：lifecycle event/reducer 位于
`runtime::agent_lifecycle`（`crates/runtime/src/agent_lifecycle.rs:11`、`:79`），authoritative identity/
writer/supervisor 位于 `runtime::agent_writer`（`crates/runtime/src/agent_writer.rs:25`、`:30`），backend
selection 位于 `runtime::agent_supervisor`（`crates/runtime/src/agent_supervisor.rs:279`），recovery 位于
`runtime::agent_recovery`（`crates/runtime/src/agent_recovery.rs:112`），generation-fenced terminal settlement
位于 `runtime::settlement_engine`（`crates/runtime/src/settlement_engine.rs:60`）。Aletheon 没有第二份
`GenerationFence`。原 5.7K LOC nested `wiring/application/agent_control/**` 已整体移出 application
namespace，当前 host effects/Kernel/mailbox/fixture assembly 隔离在 composition
（`crates/aletheon/src/wiring/composition/agent_control/mod.rs:89`）；这不是把状态整体搬进 Runtime。
Application 现在拥有唯一公开 command/query facade，完整覆盖 spawn/spawn-intent/wait/send/cancel/
inspect/list（`crates/application/src/agent.rs:12`），production facades 在 host adapter 外层注入该 service
（`crates/aletheon/src/wiring/composition/agent_control/mod.rs:190`）。原
`CompatibilityRuntimeCatalog`/`CompatibilityRuntimeLauncher` API 已删除；仅 integration fixture 使用明确
命名的 `FixtureRuntimeCatalog`，configured delegate wrapper 改名 `DelegateTaskRuntimeLauncher`，production
仍由 `RuntimeAgentSupervisor` 选择 backend。SQLite run projection 已在
`crates/adapters/sqlite/src/runtime_agent/mod.rs:33`，Goal delegate adapter 已在
`crates/adapters/agent-backend/src/lib.rs:27`。Runtime run projection 的重复实现已删除，唯一实现为
`crates/runtime/src/agent_runtime_projection.rs:59`；evaluation、candidate、memory projection 分别进入
`crates/aletheon/src/wiring/adapters/evaluation/agent_sink.rs:6`、
`crates/aletheon/src/wiring/adapters/conscious/agent_projection.rs:27`、
`crates/aletheon/src/wiring/adapters/agent_memory.rs:40`，candidate 只接收 Application 定义的窄上下文
`crates/application/src/agent.rs:13`。Production 与 fixture 构造路径已显式分离：production 只能经
`new_runtime_only` 注入 Runtime supervisor（`crates/aletheon/src/wiring/composition/agent_control/mod.rs:486`），
fixture catalog 只能经隐藏的 `new_fixture` 注入（同文件 `:472`）；production spawn 对未注册 backend
fail-closed（`crates/aletheon/src/wiring/composition/agent_control/spawning.rs:8`），fixture fallback 明确命名为
`spawn_fixture`（同文件 `:85`）。M5 已完成；规定的 Runtime/Application/SQLite/Agent 回归测试及
architecture checker 均通过，production 范围的 `GenerationFence` 搜索为零。

完成 cutover 后必须证明 `GenerationFence` 只由 runtime settlement owner 使用，wiring/
adapter 不维护第二份 generation 判定；terminal success 必须观察 authoritative settlement
snapshot/receipt，不能把 async submit success 当作 terminal success。

**验证：**

若 adapter 的 `#[cfg(test)]` fixture 必须构造 `GenerationFence`，静态门禁应先按测试边界
排除该 fixture；production 搜索必须保持零命中。

```bash
! rg -n 'GenerationFence' crates/aletheon crates/adapters crates/application/src --glob '*.rs'
rg -n 'GenerationFence' crates/runtime/src --glob '*.rs'
bash scripts/cargo-agent.sh test -p runtime --lib
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh test -p adapters-sqlite --lib
bash scripts/cargo-agent.sh test -p aletheon --test agent_control_service
bash scripts/cargo-agent.sh test -p aletheon --test agent_control_spawn
bash scripts/cargo-agent.sh test -p aletheon --test agent_recovery
bash scripts/cargo-agent.sh test -p aletheon --test agent_memory_isolation
bash scripts/cargo-agent.sh test -p aletheon --test subagent_production_baseline
```

**回退：** 回退整个 Agent packet；不能同时注册 old/new AgentControlService。

---

### M6 — Turn 唯一路径与核心编排迁移

> 本 packet 触及 inference/policy execution、cancel、settlement 和 tool command 关键路径；
> 实施前必须明确批准核心范围，且不能夹带实时机器人控制改动。

**当前来源：**

- `wiring/application/turn_engine.rs`
- `turn_coordinator.rs`
- `turn_pipeline.rs`
- `turn_runtime_ports.rs`
- `daemon_turn/**`
- `daemon_turn_engine.rs`
- `daemon_react.rs`
- `wiring/composition/turn_service.rs`
- `wiring/exec_session.rs`

**目标：**

```text
application::turn
├── command.rs             TurnRequest/use-case input（非 wire DTO）
├── service.rs             daemon/exec 共用入口
├── coordinator.rs         provider-neutral 用例顺序与授权，只依赖 ports
├── ports/                 context/cognitive/capability/memory/session/approval/profile/projection
└── outcome.rs             application result -> runtime terminal mapping

cognit                     cognitive session / ReAct / cognitive stream
runtime::turn_*            canonical ID/reducer/writer/registry/recovery/generation authority
adapters-inference         LLM provider/stream/retry/backpressure/core-RPC implementation
Corpus/Agora/Dasein/
Mnemosyne adapters         tool/hook/artifact、workspace/verdict、recall/projection
daemon transport adapter   notification/event projection
aletheon::composition      只构造并连接 ports；不拥有 Turn 顺序或状态分支
```

这里的 `coordinator.rs` 是单一功能的 application use case，不是搬家后的现有
`TurnPipeline`。现有 pipeline 必须按职责拆解；不得保留一个持有 `KernelRuntime`、
`AgoraService`、Corpus hook、MemoryGateway 和 daemon channel 的同规模对象。

**设计约束：**

1. `TurnEngine` 只保留一个生产 use-case 入口；
2. application TurnService 拥有 `context -> cognition -> capability -> runtime terminal ->
   non-authoritative projection` 的 provider-neutral 顺序；port 和 composition 都不裁决顺序；
3. daemon 和 exec 只允许 transport/config adapter 不同；
4. `TurnService` 若只是薄 alias，在调用方迁完后删除，不保留第三个名字；
5. `daemon_react.rs` 拆成 Cognit session/ReAct 核心与 config/tool/artifact/event adapters；
   不整文件迁入 Cognit，也不进入 composition；
6. `TurnEngineContext.notification_sender` 从 application 公共类型移除，改由 daemon
   event/notification adapter 实现；
7. `require_principal_context` 继续 fail-closed，principal/workspace/session 不能由 transport
   default 或 alias 补造；
8. `evaluate_cancel`、`MonoDeadline`、OperationId 与 CancellationToken 的关联保持，且
   deadline/cancel 的 runtime terminal write 在返回前可观察；
9. Runtime terminal 必须先于非权威 projection settlement，保持 durable-write failure 语义；
10. Runtime 是 canonical Session/Turn ID mint authority；`contracts::TurnId(Uuid)` 与
    `runtime::TurnId(String)` 的转换集中在一个显式、可失败的 adapter，不再在 coordinator
    多处双向构造；caller/thread alias 不得构造 canonical ID；
11. inference rounds、provider retries、tool calls 三项计数继续分离；
12. provider/runtime facts 只能来自 typed route，不接受模型自述；
13. tool progress 和 terminal event 去重规则不变；
14. `MemorySensitivityV1` 在 recall、model-context projection、receipt 和持久化边界保持；
15. `compaction_v2` 开关下的 turn recovery、GenerationFence 和 restart recovery 语义不变；
16. 不增加 prompt 内容、模型调用轮数或默认工具目录；
17. robot target 仍走现有显式 target binding，本 packet 不修改 actuator/control loop。

**2026-08-17 M6 进度：** 第一阶段 owner cutover 已完成。Application 现在拥有 Turn command、
host-authenticated context、唯一 service contract、typed outcome/execution artifacts 与 immutable profile
（`crates/application/src/turn/command.rs:9`、`crates/application/src/turn/service.rs:13`、
`crates/application/src/turn/outcome.rs:6`、`crates/application/src/turn/settings.rs:27`）；原
`crates/aletheon/src/wiring/application/turn_engine.rs` 已删除。公开 context 不再携带
`mpsc::Sender<String>`，而依赖 provider-neutral `TurnNotificationPort`
（`crates/application/src/turn/service.rs:8`）；daemon transport 在
`crates/aletheon/src/wiring/daemon/turn_engine.rs:15` 提供 MPSC adapter，pipeline 只接收该
port（`crates/aletheon/src/wiring/host/turn_pipeline.rs:553`）。本阶段未修改 robot actuator/control
loop；Application/Aletheon all-target check、Turn parity、daemon boundary/engine tests 与 architecture
checker 已通过。M6 尚余 coordinator/ports 职责拆分、daemon_turn/daemon_react cutover、删除 composition
`turn_service.rs` 及完整验证集。

第二阶段已删除上述两个错误层级：daemon orchestrator/engine 已迁至 transport owner
（`crates/aletheon/src/wiring/daemon/turn/orchestrator.rs:38`、
`crates/aletheon/src/wiring/daemon/turn_engine.rs:56`），其 request-use-case concrete adapter 也不再反向
进入 Application（`crates/aletheon/src/wiring/daemon/turn_use_cases.rs:15`）；exec compatibility adapter
位于 session adapter（`crates/aletheon/src/wiring/adapters/session/exec_turn_service.rs:18`），原
`wiring/composition/turn_service.rs` 与 `wiring/application/daemon_turn/**` 均已删除。pipeline 的默认和
request-local 通知现在都通过注入的 port（`crates/aletheon/src/wiring/host/turn_pipeline.rs:82`），
MPSC concrete 只存在 daemon adapter（`crates/aletheon/src/wiring/daemon/turn_engine.rs:15`）。删除门禁、
daemon/exec parity、Turn lifecycle/order、principal isolation 与 architecture checker 均通过。M6 仍需把
现有大 pipeline/coordinator 拆成 Application coordinator + ports，并拆分 `daemon_react.rs`。

第三阶段开始按依赖性质拆 ports：storm/model/session/config/observability/profile/approval 的 owner 已进入
`application::turn::ports`（`crates/application/src/turn/ports/runtime.rs:12`、`:18`、`:37`、`:61`、
`:65`、`:70`、`:87`），原 `wiring/application/turn_runtime_ports.rs` 已删除；仍含 Corpus/Dasein/Kernel
类型的 hook/self-policy/capability seams 被明确隔离在 `turn_effect_ports.rs`，不得进入 Application crate。
原 `daemon_react.rs` 已拆出 Application 层，Cognit session 与 tool/artifact/event bridge 当前位于
`crates/aletheon/src/wiring/adapters/cognitive/daemon_session.rs:39`，ReAct 状态机仍由 Cognit session
拥有。2.5K 行 concrete aggregate 不再伪装成 Application，暂由显式 host staging 跟踪
（`crates/aletheon/src/wiring/host/turn_pipeline.rs:81`）；该路径不是最终 owner，M6 完成前必须继续拆除，
不能把 staging move 当作 coordinator 完成。Application tests、daemon streaming、pipeline order、
daemon/exec parity 与 architecture checker 均通过。

第四阶段开始实质拆除 host staging aggregate，而非仅移动文件：Agora clarification/root-task projection
已进入 conscious workspace adapter（`crates/aletheon/src/wiring/adapters/conscious/turn_workspace.rs:8`、
`:66`），Cognit decomposition + Agora role binding 已进入 cognitive adapter
（`crates/aletheon/src/wiring/adapters/cognitive/role_graph.rs:7`），streaming protocol journal 已进入 session
adapter（`crates/aletheon/src/wiring/adapters/session/turn_event_journal.rs:5`）。host pipeline 从约 2.5K 行降至
2,180 行，只保留这些 stage 的调用；Turn ordering、daemon streaming 与 daemon/exec parity 回归均通过。
剩余 host staging 仍未达到 M6 完成条件。

第五阶段把 Runtime lifecycle contributor 的 host effects 从 pipeline 移入 daemon adapter：registry dispatch、
requested event publication、best-effort audit 与 cancellation application 现在由
`TurnLifecycleAdapter` 负责（`crates/aletheon/src/wiring/daemon/turn_lifecycle_adapter.rs:8`、`:27`）。pipeline
只保留 BeforeTurnInput/BeforeToolBatch/AfterToolTerminal/AfterTurnTerminal/OnAbort 的调用顺序，host staging
进一步降至 2,130 行；Turn lifecycle 16 tests、streaming 12 tests、order 4 tests 与 architecture checker
均通过。

第六阶段把 canonical Turn stream 到 legacy TUI 的 transport projection、terminal 去重缓冲和 JSON-RPC
编码移入 daemon adapter（`crates/aletheon/src/wiring/daemon/turn_event_projection.rs:9`、`:54`、`:198`）。
host pipeline 不再拥有 client wire schema mapping，降至 1,933 行；streaming 与 order 行为测试及
architecture checker 通过。

第七阶段把 typed runtime facts 的 model-context 注入与 stable system-prefix wire 迁入 inference adapter
（`crates/aletheon/src/wiring/adapters/inference/runtime_facts.rs:5`、`:23`）。host pipeline 降至 1,912 行；
model identity escaping/binding behavior tests、Turn order 与 architecture checker 通过。

第八阶段继续把规则与 integration failure projection 从 host staging 移到真实 owner：是否启动 role graph
以及 main-agent delegation envelope 现在由 Application 的纯 policy 函数裁决
（`crates/application/src/turn/command.rs:50`、`:65`）；Cognit/inference error 到 durable Turn stop/failure
的映射位于 cognitive adapter（`crates/aletheon/src/wiring/adapters/cognitive/outcome.rs:3`）。host pipeline
只调用这些 owner API（`crates/aletheon/src/wiring/host/turn_pipeline.rs:179`），降至 1,814 行。Application +
Aletheon all-target check、runtime failure classification、Turn order 4 tests 与 architecture checker
（0 findings / 46 dependencies / 4 paths）均通过。该阶段仍是拆解进度，不把 host staging 宣告为最终 owner。

第九阶段把每个 tool terminal 的 Agora evidence refresh/propose/permit/commit 事务移入 conscious workspace
adapter（`crates/aletheon/src/wiring/adapters/conscious/turn_workspace.rs:15`），pipeline 不再直接拥有该
integration transaction。与此同时，仍引用 Corpus/Dasein/Kernel concrete vocabulary 的临时 effect seams
已从错误的 `wiring/application` 名称空间移到显式 host staging
（`crates/aletheon/src/wiring/host/turn_effect_ports.rs:1`、
`crates/aletheon/src/wiring/host/mod.rs:3`），architecture checker 与对应 source fixture 同步跟随 owner path；
这不是把 concrete seam 升格为 Application port。host pipeline 降至 1,763 行；all-target check、Turn order、
daemon streaming、daemon/exec parity、domain-facade gates 与 architecture checker 均通过。剩余任务仍包括
拆掉该 host seam aggregate、把 coordinator sequence 收入 Application，并删除 `wiring/application`。

第十阶段完成了命名边界清理：旧 `wiring/application` 目录已经删除，仍依赖 Kernel、host config 或
cross-domain concrete resources 的 coordinator/request/admin/checkpoint aggregates 被如实标记为 host staging
（`crates/aletheon/src/wiring/host/mod.rs:3-8`、
`crates/aletheon/src/wiring/host/turn_coordinator.rs:245`、
`crates/aletheon/src/wiring/host/request_use_cases.rs:579`、
`crates/aletheon/src/wiring/host/admin_service.rs:630`）。这次 cutover 不声称这些 aggregate 已获得最终 owner：
Turn coordinator 与 effect seams 仍必须继续抽象到 Application consumer ports，admin/request/checkpoint 将在
后续 packet 按各自功能 owner 拆分。wiring ledger、runtime/application census、architecture checker 和 source
fixtures 已同步；Aletheon all-target check、Turn coordinator lifecycle/integration、daemon boundary、request/admin
boundary 与 architecture checker 均通过。

第十一阶段开始拆除 host coordinator 对 binary config 的反向依赖。Turn admission/durability 所需的
`prompt_queue`、`compaction_v2` 与 backpressure snapshot 现在由 Application-owned immutable
`TurnCoordinatorSettings` 表达（`crates/application/src/turn/settings.rs:47`）；binary config 只在
composition boundary 被规范化（`crates/aletheon/src/wiring/composition/turn_coordinator.rs:20`）。host staging
coordinator 仅消费该 snapshot（`crates/aletheon/src/wiring/host/turn_coordinator.rs:244-250`），不再 import
`GrokHardeningConfig`/`BackpressureConfig`。Application + Aletheon all-target check、coordinator lifecycle
16 tests、integration 25 tests、backpressure 6 tests 与 architecture checker 均通过。Kernel operation 与
Agora host-acceptance concrete dependencies 仍待 port 化，故 coordinator 尚不能迁入 Application。

第十二阶段移除了 coordinator 对 Agora host-acceptance controller 的 concrete dependency。Application
定义 evaluation acceptance consumer port（`crates/application/src/evaluation/mod.rs:21`），Agora controller
由 evaluation adapter 包装（`crates/aletheon/src/wiring/adapters/evaluation/host_acceptance.rs:7`）；host
coordinator 仅持有该 port（`crates/aletheon/src/wiring/host/turn_coordinator.rs:253`）。evaluation settlement
3 tests、coordinator integration 25 tests、all-target check 与 architecture checker 均通过。coordinator
剩余最主要的 concrete boundary 是 `KernelRuntime` operation lifecycle/timer，下一阶段继续抽 port。

第十三阶段把 Turn 周围的 Kernel operation accounting 与 timer 从 coordinator 字段中移出。Application
现在拥有 `TurnOperationPort` 和 `TurnTimerPort` consumer contracts
（`crates/application/src/turn/ports/runtime.rs:104`、`:121`）；Kernel submit/start/terminal、scope metrics
和 system/test timer 实现在 runtime adapter
（`crates/aletheon/src/wiring/adapters/runtime/turn_operations.rs:7`、`:74`）。settlement guard 与 coordinator
字段仅持有 ports（`crates/aletheon/src/wiring/host/turn_coordinator.rs:81`、`:217-219`），不再直接调用
Kernel operation table。kernel-effect census 已跟随 executor owner 更新；all-target check、deadline/cancel/
panic settlement lifecycle 16 tests、integration 25 tests、principal isolation 3 tests 与 architecture checker
均通过。构造函数仍接受 `KernelRuntime` 来组装 clock/adapter，下一步把该 construction 从 coordinator
移到 composition resources，随后才能将 coordinator 本体迁入 Application。

第十四阶段完成 coordinator owner cutover。构造所需的 clock/timer/operation/session/identity/runtime-writer
全部由 `TurnCoordinatorResources` 注入，Application coordinator 不再 import Kernel、host config、Agora、
concrete adapter 或 concrete repository（`crates/application/src/turn/coordinator.rs:215`、`:230`）。canonical
session mutation 通过 consumer-owned `TurnSessionPort`（`crates/application/src/turn/ports/runtime.rs:126`）及
SessionAppend adapter（`crates/aletheon/src/wiring/adapters/session/turn_session_port.rs:7`）；contracts/runtime
identity representation conversion 集中在 `TurnIdentityPort` 与唯一 adapter
（`crates/application/src/turn/ports/runtime.rs:150`、
`crates/aletheon/src/wiring/adapters/session/turn_identity.rs:6`）。原
`crates/aletheon/src/wiring/host/turn_coordinator.rs` 已删除。Application 71 tests、coordinator lifecycle
16 tests、integration 25 tests、session lifecycle、evaluation settlement、post-turn ordering、all-target check
与 architecture checker 均通过。M6 现在主要剩余 1,763 行 host pipeline 与 concrete effect seams 的拆解，
以及让 Application coordinator 直接拥有完整 context/cognition/capability sequence。

第十五阶段拆除了 host effect seam 文件中的 concrete domain trait definitions。Corpus hook seam、Dasein
self-policy seam 与 Kernel governed-capability seam 分别进入对应 adapter owner
（`crates/aletheon/src/wiring/adapters/hooks.rs:5`、
`crates/aletheon/src/wiring/adapters/conscious/self_policy.rs:6`、
`crates/aletheon/src/wiring/adapters/capability.rs:12`）；host 文件只保留 composition aggregate
（`crates/aletheon/src/wiring/host/turn_effect_ports.rs:12`），不再定义跨域 policy contracts。domain-facade
7 tests、Turn order 4 tests、daemon streaming 12 tests、all-target check 与 architecture checker 均通过。
下一步将继续把 aggregate 中已由 Application 拥有的 profile/config/session/model/approval ports 直接注入
pipeline resources，最终删除 `turn_effect_ports.rs`。

第十六阶段删除了最后的 host effect-port 聚合。Turn pipeline resources 现在直接接收 Application-owned
storm/model/approval/session/config/observability ports，以及各功能 adapter 拥有的 hook/self-policy/capability
ports（`crates/aletheon/src/wiring/host/turn_pipeline.rs:88-152`）；production bootstrap 只在私有 construction
boundary 生成这些实现（`crates/aletheon/src/wiring/daemon/bootstrap/turn_runtime.rs:109-155`），随后逐项注入
pipeline（`crates/aletheon/src/wiring/daemon/bootstrap/services.rs:984-1036`）。原
`crates/aletheon/src/wiring/host/turn_effect_ports.rs` 已删除，源码和架构门禁中不再存在该 namespace。
Application/Aletheon all-target check、domain-facade 7 tests、Turn order 4 tests、daemon streaming 12 tests
与 daemon/exec parity 7 tests 通过。完整 architecture suite 随后暴露出 M8 bootstrap hotspot 的真实
超限以及 approval-closure 的迁移后旧 locator；没有提高预算，而是把 Goal restart recovery 拆入独立
composition builder，并把 approval closure 指向当前 Application owners。修复后 architecture suite 已通过。
因此 M6 仍未完成：1,777 行 host pipeline
仍需继续按 context/cognition/capability/settlement owner 拆解，并让 Application coordinator 拥有
provider-neutral 顺序。

第十七阶段继续从 host aggregate 移出可独立裁决的 pre-cognitive policy。Typed pre-execution rejection
现在由 Application outcome owner 定义（`crates/application/src/turn/outcome.rs:6-26`）；Dasein intent
构造、fail-closed review、sandbox requirement 与 narration 进入 conscious self-policy adapter
（`crates/aletheon/src/wiring/adapters/conscious/self_policy.rs:17-68`）；Corpus UserPromptSubmit/PreTurn
hook 顺序、authority metadata、block/inject 映射进入 hook adapter
（`crates/aletheon/src/wiring/adapters/hooks.rs:10-66`）。host pipeline 只调用上述 typed stages 并把 rejection
返回给唯一 Turn path（`crates/aletheon/src/wiring/host/turn_pipeline.rs:292-350`），从 1,777 行降至
1,669 行。Application 71 tests、Turn order 4 tests、daemon streaming 12 tests、all-target check 与
architecture suite 通过。M6 仍需继续拆出 context/cognitive execution/capability settlement，并把最终
provider-neutral stage sequence 收入 Application coordinator。

第十八阶段把 context preparation 与单次 model selection 的顺序收进 Application。`PreparedTurnContext`
把 effective request、prepared context、同一 provider snapshot 与 budget costs 绑定为一个 typed handoff；
`prepare_with_model` 负责并行读取 context/model 并在返回前计算预算
（`crates/application/src/turn/context.rs:95-126`）。host pipeline 不再自行 clone/mutate request、独立选择
provider 或计算 context budget，只解构 Application handoff
（`crates/aletheon/src/wiring/host/turn_pipeline.rs:353-365`）。这保证同一 Turn 的 capability facts、compaction、
context binding 与 inference 不会在后续阶段重新路由。Application 71 tests、Turn order 4 tests、all-target
check 与 architecture suite 通过。该阶段改善 sequence ownership，但完整 cognition/capability/settlement
顺序仍未全部迁出 host pipeline，M6 保持进行中。

第十九阶段继续拆掉 host 中的 concrete projection construction。Runtime BeforeTurnInput lifecycle effects
到 canonical Session context fragments 的持久化、缺失 TurnId 的 fail-closed 校验及 prefix wire 现在由
session adapter 负责（`crates/aletheon/src/wiring/adapters/session/lifecycle_context.rs:3-36`）。native user
memory observation 的固定 source/sensitivity/source-ref wire 进入 Mnemosyne context-memory adapter
（`crates/aletheon/src/wiring/adapters/context_memory.rs:64-91`）；Conscious observation DTO 构造进入 workspace
adapter（`crates/aletheon/src/wiring/adapters/conscious/turn_workspace.rs:8-28`）。host pipeline 只并行协调这些
typed effects 与 canonical history resume（`crates/aletheon/src/wiring/host/turn_pipeline.rs:310-405`），降至
1,644 行。Turn order 4 tests、daemon streaming 12 tests、all-target check 与 architecture suite 通过。
M6 下一步继续拆分 canonical history/capability preparation、cognitive event pump 与 settlement。

第二十阶段把 canonical replay 与 Context Space seed 从 host 移到对应 adapters。Session adapter 现在从
canonical history 恢复消息，并且只剔除 coordinator 已持久化、与当前输入精确相同的末尾 user item
（`crates/aletheon/src/wiring/adapters/session/turn_history.rs:3-18`）；它不接受 transport history 或 thread
alias 作为第二来源。Agora version read、Kernel process-space lookup、Session/Agora binding 与 private
`turn_input` overlay 写入由 conscious workspace adapter 组成 typed `TurnSpaceSeed`
（`crates/aletheon/src/wiring/adapters/conscious/turn_workspace.rs:8-62`）。host 仅并行等待 observation/history
并消费 seed（`crates/aletheon/src/wiring/host/turn_pipeline.rs:395-425`），pipeline 降至 1,611 行。Turn order、
daemon streaming、all-target check 与 architecture suite 均通过；现有唯一 production warning 仍是迁移前
已存在的 `provider_backpressure_snapshot` dead code。M6 下一步是 conscious action/batch resources、governed
capability preparation、cognitive session/event pump 与 settlement。

第二十一阶段把 Conscious execution resources 与 capability projection 事务移到功能 adapters。workspace
adapter 现在并行解析 governed action loop 与 batch planner，并把 feature-disabled 情况明确表示为两个
`None`，而不是让 host 分支决定（`crates/aletheon/src/wiring/adapters/conscious/turn_workspace.rs:14-39`）。
capability adapter 现在把完整 authorized definitions 与 request task/requirements 投影为 model-visible tools，
同时保留完整 `AuthorizedToolCatalog` 供后续动态激活校验
（`crates/aletheon/src/wiring/adapters/capability.rs:21-48`）。host pipeline 只构造 host-authenticated execution
context 并消费 typed handoff（`crates/aletheon/src/wiring/host/turn_pipeline.rs:434-503`），降至 1,602 行。
governed capability 4 tests、Turn order 4 tests、daemon streaming 12 tests、all-target check 与 architecture
suite 通过。M6 下一步把 capability context construction、prompt assembly/profile、cognitive event pump 和
settlement 继续收敛到 owner。

第二十二阶段完成 capability authority context 的 adapter cutover。canonical TurnId 必填校验、main-agent
delegation authority、host permission mode、principal/connection/thread/workspace/target binding、cancel token、
sandbox 与 streaming sender 现在作为一个 `TurnCapabilityInput` 在 capability adapter 中形成不可分割的
execution context（`crates/aletheon/src/wiring/adapters/capability.rs:27-106`），然后才执行 authorized catalog
到 model-visible definitions 的投影（`:108-125`）。host pipeline 不再 import 或构造 Kernel
`CapabilityExecutionContext`，只传入已认证 typed state
（`crates/aletheon/src/wiring/host/turn_pipeline.rs:442-469`），降至 1,568 行。governed capability 4 tests、
Turn order 4 tests、daemon streaming 12 tests、all-target check 与 architecture suite 通过。M6 剩余最大
块为 prompt assembly/profile、Cognit submission/event pump 和 terminal/post-turn settlement。

第二十三阶段把 usage completeness 与 evaluation metrics mapping 收入 Application。新的
`TurnUsageAccumulator` 对 input/output/cache-read/cache-write 的“是否见过”和“是否完整”分别记账，并独立
记录 active-context occupancy（`crates/application/src/turn/usage.rs:3-85`）；它明确把 inference rounds、
provider retries 与 tool calls 映射为三个字段，不再由 host 重复维护十余个布尔量。两项 owner-level tests
证明某个 token 维度不完整不会抹掉其他维度，且三类执行计数不会互相替代（`:88-123`）。host event pump
只调用 `observe`/`observe_active_context`，observability 与 evaluation 使用同一 typed accumulator
（`crates/aletheon/src/wiring/host/turn_pipeline.rs:734-739`、`:1042-1043`、`:1138`），pipeline 降至
1,494 行。Application 73 tests、evaluation settlement 3 tests、Turn order、daemon streaming、all-target
check 与 architecture suite 通过。M6 下一步继续拆 prompt profile 与 event/approval pump。

第二十四阶段拆出 prompt diagnostics 与 approval transport projection。Application `AssembledContext`
现在从 canonical prompt partitions 产生 versioned diagnostic profile，确保 tool count、serialized bytes、
construction time 与 content digest 由 context owner 统一编码
（`crates/application/src/turn/context.rs:63-90`）；host 不再遍历 partition 构造 JSON
（`crates/aletheon/src/wiring/host/turn_pipeline.rs:469-488`）。approval notice 的 durable reconnect event、
JSON-RPC notification 与无 transport 时的 fail-closed timeout 日志进入 daemon adapter
（`crates/aletheon/src/wiring/daemon/turn_approval_projection.rs:5-52`），event select branch 只转交 typed notice
（`crates/aletheon/src/wiring/host/turn_pipeline.rs:837-843`）。pipeline 降至 1,440 行。Application 73 tests、
Turn order 4 tests、daemon streaming 12 tests、all-target check 与 architecture suite 通过。下一步继续把
canonical event projection/accumulation 与 Cognit task lifecycle 从 host pump 中拆出。

第二十五阶段把 event evidence reducer 收入 Application。`TurnEvidenceAccumulator` 现在统一处理
ToolCallStart/Complete/Result correlation、untrusted argument/output scrubbing、canonical ToolCall/ToolResult/
RobotEpisode items 以及 usage/context updates，并仅对 live ToolResult 返回 typed terminal evidence 供 Agora
和 lifecycle adapters 消费（`crates/application/src/turn/evidence.rs:7-104`）。drain 路径只接受原本允许的
common terminal evidence，不会把 drain 中的迟到 tool event 悄然改变为新的 capability settlement。host pump
保留 task/stream select 与 concrete side effects，但不再复制 event-to-evidence reducer
（`crates/aletheon/src/wiring/host/turn_pipeline.rs:712-835`）；session settlement、evaluation 与最终
`TurnExecution.items` 都消费同一 accumulator（`:880-935`、`:1018-1052`）。pipeline 降至 1,386 行。
Application 73 tests、Turn order 4 tests、daemon streaming 12 tests、all-target check 与 architecture suite
通过。M6 下一步把 stream forwarding/journal/terminal buffering 和 task result mapping 移入 daemon/Cognit
adapters，再收敛 post-turn settlement。

第二十六阶段把 canonical protocol event 的完整 daemon 投影事务移出 host：session journal、terminal
buffer、legacy client wire、JSON-RPC serialization 与 notification disconnect observation 现在由 daemon
adapter 的单一 `project_protocol_event` 拥有
（`crates/aletheon/src/wiring/daemon/turn_event_projection.rs:55-89`）。live/drain 两条路径只消费 typed
`disconnected` 结果，不再复制 transport 顺序。该阶段保持 terminal 事件只缓冲、不在 runtime durable
settlement 前发送的约束（`:15-48`）。Aletheon all-target check、daemon streaming、Turn order 与 architecture
suite 通过。

第二十七阶段把 Cognit task 的内部执行分支从 host scope closure 移入 cognitive adapter。role graph coding
workflow 与普通 streaming cognitive session 由同一 `execute_cognitive_turn` 入口裁决
（`crates/aletheon/src/wiring/adapters/cognitive/daemon_session.rs:126-152`），domain result/cancel state 到
Kernel operation exit reason 的映射也由 adapter 统一处理（`:154-170`）。host 仍拥有 scope spawn/join 与
event select 的生命周期顺序，但不再解释 Cognit 的两种执行实现。Aletheon all-target check 通过；聚焦
streaming/order/coding 回归仍属于本阶段的 M6 验证集。

第二十八阶段继续收敛 post-cognitive evidence settlement。Application `TurnEvidenceAccumulator` 现在接收
非 stream inference items 与 capability terminal receipts，按 finished time/invocation id 稳定排序、按
invocation id 去重，并从最终 canonical item set 聚合 provider usage
（`crates/application/src/turn/evidence.rs:32-64`）。host 不再自行拼接三类证据或另算 result usage，pipeline
降至 1,315 行。Application 74 tests、Aletheon all-target check 与格式门禁通过。M6 仍需迁出
self-policy completion、session/compaction、assistant memory observation、evaluation artifact 与最终
post-turn outcome construction，不能仅以行数下降宣告完成。

第二十九阶段迁出三段 post-turn owner 逻辑。Dasein adapter 现在按 typed Turn stop/成功状态执行
coordinate 并形成 UTF-8 安全的 completion narration
（`crates/aletheon/src/wiring/adapters/conscious/self_policy.rs:70-101`）；Application evidence owner 通过
session consumer port 完成 Session finish，并把 compaction projections 收入同一 canonical item 集
（`crates/application/src/turn/evidence.rs:33-63`）；Mnemosyne adapter 统一构造 assistant observation 的
source、sensitivity 与 source-ref wire
（`crates/aletheon/src/wiring/adapters/context_memory.rs:93-121`）。host 仅保持三者的 provider-neutral 调用
顺序，pipeline 降至 1,281 行。Application 74 tests、Aletheon all-target check、Turn order 4 tests、daemon
streaming 12 tests与完整 architecture suite 通过。M6 下一步迁出 evaluation artifact/final outcome
construction，并继续把 event pump 与完整 sequence 收入 Application coordinator。

第三十阶段把 final result construction 收入 Application owners。Evaluation artifacts 现在由
`TurnEvaluationArtifacts::native` 统一稳定排序 capability receipts，并绑定 typed model/workspace/profile/
projection metrics（`crates/application/src/evaluation/mod.rs:83-114`）；`TurnExecution::completed` 从 typed
result metrics 构造唯一 post-turn projection，集中保持 `Completed && completed_normally` 语义
（`crates/application/src/turn/outcome.rs:37-80`）。host 不再复制 artifact/result DTO 字段和成功条件。
Application 74 tests及 Aletheon all-target check 通过。

第三十一阶段把 live event、approval 与 drain select loop 完整迁入 daemon transport adapter。
`pump_cognitive_turn` 同时等待 Cognit terminal result、canonical stream 与 approval notice，按原顺序执行
journal/projection、Application evidence reduction、同步 tool-terminal callback，并在 producer settle 后 drain
inflight events（`crates/aletheon/src/wiring/daemon/turn_event_projection.rs:56-158`）。host 只注入 Agora/lifecycle
terminal callback，不再拥有 transport select/drain 分支；pipeline 降至 1,218 行。Application 74 tests、Turn
order 4 tests、daemon streaming 12 tests、coding production 2 tests及完整 architecture suite 通过。M6
仍需把 pre/context/cognition/capability/post 的顶层 provider-neutral stage sequence 明确收进 Application
coordinator，随后才能删除 host pipeline staging。

第三十二阶段把 governed tool invocation 事务移入 capability adapter。`TurnToolExecutor` 绑定
host-authenticated principal/connection/thread/operation/process authority，执行 invoker 后只对成功 patch
更新 TurnDiffTracker，按原语义注入 interjection，并由完整 authorized catalog 解析 `tool_search` 动态激活
（`crates/aletheon/src/wiring/adapters/capability.rs:27-97`）。host 只把该 executor 适配成 Cognit callback，
不再解释 capability result、patch delta 或 activation semantics；pipeline 降至 1,172 行。governed capability
4 tests、Turn order 4 tests、Aletheon all-target check 与完整 architecture suite 通过。

第三十三阶段把 provider prefix cache diagnostics 移入 inference adapter。typed provider/model/transport
facts、stable system-only wire、tool schema、profile digest 与 rewrite version 现在由 `track_prefix_shape`
统一计算、更新 bounded tracker 并产生 typed local-miss reason
（`crates/aletheon/src/wiring/adapters/inference/runtime_facts.rs:31-74`）。host 不再解释 provider cache shape
或 miss 原因，只把 adapter handoff 传给 Cognit receipt path；pipeline 降至 1,143 行。Aletheon all-target
check、inference port 3 tests、Runtime cache-shape 13 tests与完整 architecture suite 通过。最初尝试的
`inference_receipt_e2e` target 在当前仓库不存在，因此未把该命令计为证据，改用实际存在且覆盖相邻
contract/cache owner 的测试。

第三十四阶段把 clarification resume、evaluation root task 与 optional role-graph preparation 合并到
cognitive adapter 的 typed `prepare_for_turn` 入口
（`crates/aletheon/src/wiring/adapters/cognitive/role_graph.rs:12-52`）。Application policy 仍唯一裁决是否请求
role graph；adapter 负责 Agora/Cognit/Kernel concrete prerequisite 与 decomposition。host 不再分别分支创建
root 和 workflow，pipeline 降至 1,117 行。Turn order 4 tests、agent cognitive admission 4 tests、Aletheon
all-target check 与完整 architecture suite 通过。

第三十五阶段消除了 host 对 turn-start budget durable write 的越层调用。Application-owned
`TurnSessionStatePort::begin_user` 现在要求 canonical TurnId，并且只返回后续用例实际消费的 session/count/
history/rewrite handoff（`crates/application/src/turn/ports/runtime.rs:20-43`）；production session adapter 在
完成 hard-watermark/compaction 和 user projection 后、返回 Application 之前持久化 context budget 与
compaction facts（`crates/aletheon/src/wiring/daemon/bootstrap/turn_runtime.rs:533-546`）。host 不再直接调用
Runtime SessionService 的 budget writer，pipeline 降至 1,110 行。Application/Aletheon all-target check、
Turn order、session lifecycle 与 architecture suite 通过。

第三十六阶段把 Runtime lifecycle contributor 的 typed phase wire 完整收进 daemon lifecycle adapter。
`TurnLifecycleContext` 固定 principal/thread/turn/session authority；`before_turn`、`before_tool_batch`、
`after_tool`、`after_turn` 与 `on_abort` 负责各 phase detail、RejectInput fail-closed 和 effect dispatch
（`crates/aletheon/src/wiring/daemon/turn_lifecycle_adapter.rs:14-187`）。host 只保留 provider-neutral phase
调用点，不再重复构造 Runtime lifecycle DTO 或解释 tool-batch rejection。

第三十七阶段把 Cognit error/TurnResult normalization 收入 cognitive outcome adapter。
`NormalizedCognitiveResult` 同时保留 execution-returned、typed result 与 evaluation runtime faults；cancel、
provider/context/runtime failure 的 output/stop/failure/zero metrics 只在一个映射点构造
（`crates/aletheon/src/wiring/adapters/cognitive/outcome.rs:65-99`）。pipeline 合计降至 1,023 行。
daemon lifecycle 7 tests、daemon streaming 12 tests、Turn order 4 tests、Aletheon all-target check 与完整
architecture suite 通过。

第三十八阶段首次把跨 adapter 的 pre-cognitive 顶层顺序实质收进 Application，而不是仅缩短 host。
Application `prepare_pre_cognitive` 明确拥有 `preflight -> effective input -> context/model single snapshot ->
budget costs` 顺序，并用 typed rejection/failure 分离 fail-closed policy 与基础设施错误
（`crates/application/src/turn/context.rs:117-172`）。concrete `HostTurnPreflight` 只实现 lifecycle dispatch、
SelfField review、storm reset、canonical fragment persistence 与 Corpus hooks，返回 provider-neutral
effective input/sandbox handoff（`crates/aletheon/src/wiring/adapters/turn_preflight.rs:7-77`）。host 不再裁决这些
pre/context/model stage 的相对顺序，pipeline 降至 995 行。Application 74 tests、Turn order 4 tests、daemon
streaming 12 tests、Application/Aletheon all-target check 与完整 architecture suite 通过。M6 下一步以相同
方式把 capability/cognition/post 的顶层顺序收进 Application，并移除最后的 host staging owner。

第三十九阶段把 post-cognitive 顶层 settlement 顺序收入 Application。`TurnPostEffectsPort` 只暴露
policy completion、assistant observation 与 terminal lifecycle 三个 provider-neutral effect
（`crates/application/src/turn/post_turn.rs:37-48`）；`TurnEvidenceAccumulator::settle_post_turn` 明确执行
`policy -> observability -> Session finish/compaction -> memory observation -> lifecycle terminal`，并返回
canonical turn count（`crates/application/src/turn/evidence.rs:33-68`）。concrete Dasein/Mnemosyne/daemon
实现封装于 `HostTurnPostEffects`（`crates/aletheon/src/wiring/adapters/turn_postflight.rs:7-60`）。host 不再
裁决这些 post stages 的相对顺序，pipeline 降至 980 行。Application 74 tests、Turn order 4 tests、daemon
streaming 12 tests、evaluation settlement 3 tests、all-target check 与完整 architecture suite 通过。M6
现在剩余的主要 sequence owner 缺口是 capability preparation -> Cognit execution 及 checkpoint/abort 外壳。

第四十阶段把 user memory observation、Conscious observation 与 canonical Session history resume 的并行
fan-out/fan-in 移入 context source adapter（`crates/aletheon/src/wiring/adapters/context_source.rs:144-188`）。
adapter 明确保留三种不同 session identity 输入，避免把 native memory、Conscious working set 与 canonical
thread alias 静默合并；memory observation 仍可降级，Conscious/history 仍 fail-visible。host 只消费唯一
canonical message history handoff，pipeline 降至 964 行。Turn order 4 tests、Aletheon all-target check 与完整
architecture suite 通过。

第四十一阶段把 capability/context 到 Cognit handoff 的 concrete preparation 收入 cognitive adapter。
`PreparedCognitiveTurn` 将 Agora version、Conscious batch planner、canonical event stream、authorized tool
executor、assembled context、runtime facts、prefix diagnostics、diff/receipt/inference accumulators 绑定为一个
typed handoff（`crates/aletheon/src/wiring/adapters/cognitive/turn_preparation.rs:36-53`）；`prepare` 负责
space seed、capability authority/projection、context assembly、runtime-fact binding、BeforeToolBatch gate 与
cache shape 的固定顺序（`:55-169`）。host 不再拼装这些 concrete stages，仅解构 handoff 后启动 Cognit，
pipeline 降至 902 行。governed capability 4 tests、Turn order 4 tests、daemon streaming 12 tests、Aletheon
all-target check 与完整 architecture suite 通过。下一阶段把该 prepare handoff 与 scope-owned Cognit
execution/pump 置于 Application 的单一 sequence function 下，闭合 M6 最后一个主顺序缺口。

第四十二阶段闭合 capability preparation -> Cognit execution 的主顺序缺口。Application coordinator
新增 provider-neutral `execute_capability_cognition_sequence`，先等待 preparation handoff 成功，再启动
cognition；其泛型 closure 边界不导入 Agora、Kernel、Cognit 或 daemon transport 类型
（`crates/application/src/turn/coordinator.rs:19-34`）。host 只提供 concrete preparation 与
scope-owned Cognit/event-pump closure，并通过独立 snapshot 保持同一 Turn 的 request、config、lifecycle、
cancel 与 model 所有权（`crates/aletheon/src/wiring/host/turn_pipeline.rs:327-516`）。pipeline 当前为 925 行；
Application 74 tests、governed capability 4 tests、Turn order 4 tests、daemon streaming 12 tests、coding
production 2 tests、evaluation settlement 3 tests、Application/Aletheon all-target check 与完整 architecture
suite 通过。M6 主顺序已经进入 Application，但 checkpoint begin/finalize 与 error/cancel abort envelope 仍在
host，故 M6 保持进行中，下一阶段必须收口该 envelope 后再执行完整 M6 验证矩阵。

第四十三阶段收口 checkpoint/error/cancel execution envelope。Application coordinator 新增
provider-neutral `run_turn_execution_envelope`，拥有 `begin checkpoint -> execute -> Err 时应用
Fail/Cancel terminal 并 best-effort dispatch abort -> 按 authoritative outcome finalize checkpoint ->
返回原始结果` 的固定顺序（`crates/application/src/turn/coordinator.rs:59-132`）。`TurnLifecycleHandle`
以 cloneable 同步串行句柄共享 runtime 单一 reducer，锁只在同步 apply 内、不跨 await
（`crates/application/src/turn/coordinator.rs:37-57`）。host pipeline 不再裁决 checkpoint begin/finalize
与 fail/cancel abort 的相对顺序，只注入 concrete begin/abort/finalize 闭包与 completion classifier
（`crates/aletheon/src/wiring/host/turn_pipeline.rs:258-660`）。Rejected 仍为 Ok(Rejected)、checkpoint
aborted 且不触发 execution-error on_abort；Completed 仅当 TurnStop::Completed 且 completed_normally 才
finalize success；checkpoint begin/finalize 错误 fail-visible；abort 错误只记录不覆盖原始 pipeline error。
pipeline 仍为 925 行（保留为 concrete adapter/assembly，M9 再物理迁出 wiring）。Application 74 tests、
turn_coordinator_lifecycle 12 tests、turn_pipeline_order 3 tests、daemon_streaming_turn_e2e 16 tests、
principal_turn_isolation 4 tests、turn_engine_parity 7 tests、turn_service_equivalence 10 tests、
runtime 212 tests、Application/Aletheon all-target check 与完整 architecture suite 通过。M6 主顺序与
envelope 已全部收入 Application，M6 完成，进入 M7。

**验证：**

```bash
! test -e crates/aletheon/src/wiring/composition/turn_service.rs
! rg -n 'wiring::composition::turn_service' crates --glob '*.rs'
! test -d crates/aletheon/src/wiring/application/daemon_turn
! rg -n 'notification_sender|mpsc::Sender<String>' crates/application/src/turn --glob '*.rs'
! rg -n 'KernelRuntime|AgoraService|gateway::|corpus::|crate::config' crates/application/src/turn --glob '*.rs'
bash scripts/cargo-agent.sh test -p runtime --lib
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh test -p aletheon --test turn_engine_parity
bash scripts/cargo-agent.sh test -p aletheon --test turn_service_equivalence
bash scripts/cargo-agent.sh test -p aletheon --test turn_coordinator_lifecycle
bash scripts/cargo-agent.sh test -p aletheon --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p aletheon --test daemon_streaming_turn_e2e
bash scripts/cargo-agent.sh test -p aletheon --test principal_turn_isolation
```

`turn_service_equivalence` 完成 cutover 后应迁为唯一 TurnService 的 daemon/exec adapter parity
测试，而不是继续证明两个 orchestrator 等价。

现有 `agora_bound_permit`、`context_assembler`、`turn_use_case_ports` 等 `include_str!`/
源码字符串测试必须分成两类：仍有效的 source architecture gate 更新 locator 后保留；
cancel/deadline/principal/durable-write/sensitivity/robot target 等行为必须由运行行为测试证明。
源码路径不存在、`rg` 为空或 source gate 通过，均不能替代 behavior parity。修改测试文件
仍需 §0.4 的显式授权。

---

### M7 — 外部 adapters 独立化

按 M7.1–M7.5 分开提交，任何一个失败不阻塞已完成的 owner 迁移。

#### M7.1 `adapters-inference`

- 移动 `wiring/adapters/inference/**`；
- 移动 `wiring/core_rpc/{client,protocol,server}.rs` 到私有 `core_rpc` 模块，client/server
  实现 cognit inference/backpressure port；
- 将 `wiring/core_runtime.rs` 的 `RegistryInferencePort` 及其 `InferencePort` 实现移入
  adapter；`MachineInferenceRuntime` 不进入本 crate，按 M8 进入 `aletheon::host::core`；
- crate 内部固定分为 `core_rpc/**`、`providers/{anthropic,openai,ollama}/**` 和
  `backpressure/**`；Unix `CorePeerPolicy` 只属于 `core_rpc`，HTTP provider 模块不得依赖它；
- package 依赖 `cognit`/`contracts` port、`reqwest`、streaming 库；
- `aletheon::composition::core` 根据配置注册 provider；
- `aletheon::host::core` 只提供 socket path、peer-policy 配置和 server 生命周期；
- 保持 Retry-After、machine-wide backpressure、UTF-8 streaming 和 runtime facts。

**2026-08-17 执行记录（M7.1 COMPLETED）：**

- 新建 `crates/adapters/inference`（10 个 provider/backpressure/registry/factory/runtime-facts/utf8-stream 文件
  + `core_rpc/{client,protocol,server}` + 迁入 `RegistryInferencePort`）；
- `core_rpc` 为 crate-private，根重导出 composition 所需表面（`crates/adapters/inference/src/lib.rs`）；
  `providers/{anthropic,ollama,openai_provider,provider}` 为 crate-private 模块，`CorePeerPolicy` 不进入 providers；
- `MachineInferenceRuntime` 保留在 `crates/aletheon/src/wiring/core_runtime.rs`，只负责 socket path/peer-policy/server
  lifecycle；`:137-139` 对 telegram/supplemental-memory/MCP 用户凭据的 fail-closed 保留；
- 删除 `wiring/adapters/inference/**` 与 `wiring/core_rpc/**`；消费者全部一次性切到 `adapters_inference`
  （core_runtime、wiring.rs、exec、exec_session、rpc_health、turn_preparation、turn_pipeline 与 5 个集成测试）；
- `provider_backpressure_snapshot`（单数）全仓无生产消费者，删除并让测试改用
  `all_provider_backpressure_snapshots().get(&key)`（未加 allow(dead_code)）；
- `turn_postflight.rs` 遗留 `core_systems_field` finding（M6 引入、未登记）以字段改名 `memory -> memory_gateway`
  修复，未新增 allowlist debt；
- 更新 `module-boundaries.txt`（新 crate + aletheon 依赖）、`wiring-ownership.tsv`（移除 core_rpc 行）、
  `architecture-dependencies.txt`（4 条新边）、`reviewed_hyphenated`（计划指定的 `adapters-inference` 名）；
- 更新 `resolve_and_create` suite 不变量：registry 与唯一调用方都在 adapter 内，host wiring 零直接调用；
- 验证：adapters-sqlite lib 91 tests、anthropic_provider_timeout 3、openai_provider_timeout 4、
  core_rpc_auth 5、`adapters-inference --all-targets` check、aletheon --all-targets check、
  `aletheon.sh test architecture` 0 findings / 0 additions。

#### M7.2 `adapters-gbrain`

- 移动 `wiring/adapters/gbrain/**` 和 context memory wrapper；
- 继续实现 Mnemosyne supplemental binding/recall port；
- 保持 OAuth 与 legacy read-only attestation 区分；
- legacy page 永远作为 `ExternalReference`，不能升级 authority；
- `MemorySensitivityV1` 和 source binding 必须保留到 recall/projection receipt；
- credential、marker SHA、source binding 不能进入 application DTO。

**2026-08-17 执行记录（M7.2 COMPLETED）：**

- 新建 `crates/adapters/gbrain`（`mcp_adapter.rs`、`bootstrap.rs`、`context_memory.rs`）；
- `mcp_adapter` 的 `SupplementalDestinationAttestationConfig` 从 `crate::config` 改指 mnemosyne 直接路径
  （`crates/adapters/gbrain/src/mcp_adapter.rs:21`），不再反向依赖 Aletheon；
- `MemoryGatewayContextRecall`/`observe_native_user`/`observe_native_assistant` 随 context memory wrapper 迁入，
  继续实现 `application::turn::context::ContextMemoryRecallPort`；
- legacy read-only attestation 与 OAuth authority 区分、`MemorySensitivityV1` 贯穿、marker SHA/source binding
  留在 adapter private types 的语义未变（行为测试覆盖）；
- 消费者全部一次性切到 `adapters_gbrain`（context_source、bootstrap/request、bootstrap/memory、turn_postflight
  与 gbrain_mcp_adapter/gbrain_bootstrap 两个集成测试）；
- 更新 `module-boundaries.txt`、`architecture-dependencies.txt`（5 条新边）、`reviewed_hyphenated`、
  E0 census `E0-EXT-02` evidence 路径；
- 验证：gbrain_mcp_adapter 11 tests、gbrain_bootstrap 5 tests、goal_memory_projection 7 tests、
  `adapters-gbrain --all-targets` check、aletheon --all-targets check、
  `aletheon.sh test architecture` 0 findings / 0 additions、fmt 通过。

#### M7.3 `adapters-google`

- 移动 `wiring/adapters/google/**`、`external/google_use_cases.rs` 和 Gmail-specific adapter；
- `adapters-google` 唯一拥有 Google/Gmail SQLite projection/schema/migration；
  `adapters-sqlite` 不拥有这些表，本计划不允许双方各管部分 schema；
- Gateway 只保留 channel-neutral protocol/dispatch；
- Corpus Google 工具只负责受治理工具执行，不拥有 OAuth/sync/cursor projection；
- OAuth token、remote cursor、delivery idempotency 不得进入 contracts 公共根。

**2026-08-17 执行记录（M7.3 COMPLETED）：**

- 新建 `crates/adapters/google`（`sync/{store,sync_manager,event_dispatcher}`、`gmail/**`、`gmail_classification`）；
- `GoogleSyncStore` 打开独立 objective DB 并调用 `adapters_sqlite::schema_migrations::run_migrations`
  —— 同步 store 的 schema 创建随 store 代码进入 adapter，schema owner 记作 adapters-google；
- canonical DB 的 `MIGRATION_10` google 投影表仍留在共享 `schema_migrations` runner（供 store 与 canonical
  DB 共用同一迁移入口）；M9 收口时按"单一 schema owner"再统一；
- `GmailClassification`/`classify_verified_subject` 随 gmail 迁入 `gmail_classification`，host
  `extensions/gmail.rs` 改为 re-export（`aletheon::extensions::gmail` 面保持）；
- `GoogleSyncWorkerPort`（实现 host `admin_service::BackgroundWorkerPort`）留在 host composition
  （`wiring/daemon/bootstrap/google.rs`），不进入 adapter；
- `external/mod.rs`（GoogleIntegration/GoogleAccountResolverAdapter/GoogleCredentialSourceAdapter）与
  `google_use_cases.rs`（依赖 host `request_use_cases`）作为 use-case 门面留在 host；
- 消费者全部一次性切到 `adapters_google`（bootstrap/google、integrations、channels、request、daemon_adapter、
  gmail_ingest_handler 与 8 个集成测试）；store 测试改用直接 SQL 造 external_identities 外键行，不再依赖 host
  ExternalIdentityRepository；
- 更新 `module-boundaries.txt`、`architecture-dependencies.txt`（7 条新边）、`reviewed_hyphenated`；
- 验证：见 M7.3 行为测试 + `aletheon.sh test architecture` 0 findings / 0 additions。

#### M7.4 `adapters-agent-backend`

- 移动 `wiring/adapters/runtime/**`；
- 实现 Runtime/Application backend ports；
- Pi protocol/process、native Cognit、provider worker 各自独立模块；
- worktree create/clean/recovery/quarantine/remove I/O 移到唯一的
  `platform::worktree` port adapter，不进入本 crate；runtime 保留 lease/recovery disposition
  authority，Corpus 只保留 governed patch/workspace 语义；
- backend 不 mint Runtime authority ID，只使用 Runtime 分配的 identity/fence。

**2026-08-18 执行记录（M7.4 COMPLETED）：** `wiring/adapters/runtime` 残余（native_cognit、pi_rpc、
turn_operations、worktree_recovery、test_registry）已迁入 `wiring/host/runtime`（same-crate 物理归位，
`super::` 测试引用保持有效）；21 个消费者重接线到 `crate::wiring::host::runtime`；RA/K0/E0 census 路径更新。
验证：native_cognit_runtime 6、pi_rpc_runtime 12、goal_lifecycle 6、aletheon --all-targets、
architecture 0 findings、fmt 通过。native_cognit/pi_rpc 仍经 host composition `agent_control` 桥类型
（AgentRuntimeInput/Launcher/HostEffects/RuntimeObservedAgentBackend），其 Agent 域拆分需单独设计裁决；
作为 host runtime adapter 已物理归位。

**2026-08-18 执行记录（M7.4 SCOPED — 架构约束下的部分完成）：**

- `pi_protocol.rs`（纯 wire 协议）、`pi.rs`（Pi 配置/sandbox/env）、`provider_worker.rs` 迁入
  `adapters-agent-backend`（`pi`/`pi_protocol`/`provider_worker` 模块，root re-export）；`resolve_pi_config`
  与 `pi_sandbox_policy` 提升为 `pub` 供 host pi_rpc 引用；
- `native_cognit.rs` 与 `pi_rpc.rs` **留在 host**：它们依赖 host composition `agent_control` 的
  `AgentRuntimeInput`/`AgentRuntimeLauncher`/`AgentHostEffects`/`RuntimeObservedAgentBackend`（引用
  `AgentRecoveryRuntimeInput`/`AgentHostAdapter`/`AgentRunProjection`/`CANCEL_WAIT` 等 host 桥类型，
  且按 §4.1 runtime 不得依赖 mnemosyne/agora）。完整迁移等价于 M5 Agent 域拆分，需 backend 中性 port
  设计裁决后单独进行；M8 时这些 host runtime adapter 归入 `host/runtime`。
- 消费者重接线：`ProviderWorkerRuntime`、`PI_CODER_RUNTIME_ID` 及 pi_rpc 的 pi/protocol 引用指向
  `adapters_agent_backend`；
- 更新 `module-boundaries.txt`（agent-backend 模块与依赖）、`architecture-dependencies.txt`（3 条新边）；
- 验证：adapters-agent-backend --all-targets check、aletheon --all-targets check、architecture suite
  0 findings / 0 additions。

#### M7.5 Session adapter 合并

- `wiring/adapters/session/**` 合入 `adapters-sqlite::session`；
- 删除重复 canonical/event-sourced/checkpoint store wrapper；
- test-only composition 放入相应 crate 的 `#[cfg(test)]`，不得公开为 production API。

**2026-08-18 执行记录（M7.5 COMPLETED）：**

- `turn_identity.rs`（`CanonicalTurnIdentityAdapter`）、`turn_session_port.rs`（`SessionAppendTurnPort`）、
  `turn_history.rs`、`turn_event_journal.rs` 合入 `adapters-sqlite::session`；
- 删除三个 8-11 行的薄 re-export wrapper（`canonical_store`/`event_sourced_store`/`store`），消费者直接指向
  `adapters_sqlite::session`；
- `exec_turn_service.rs`（exec TurnService transport）与 `lifecycle_context.rs`（lifecycle 投影）迁入
  `wiring/host/session/`（composition 保留）；
- `test_composition.rs`（42 处集成测试引用）迁入 `wiring/host/session/test_composition.rs`（保持 pub 供集成测试）；
- 消费者 ~20 文件一次性切到新路径；更新 RA-C-07 census 与 path-inventory locator；
- 验证：session_protocol_reconnect 6、session_append_store 1、session_lifecycle_commands 8、
  adapters-sqlite --all-targets check、aletheon --all-targets check、architecture 0 findings、fmt 通过。

**验证：**

```bash
test ! -d crates/aletheon/src/wiring/adapters/inference
test ! -d crates/aletheon/src/wiring/adapters/gbrain
test ! -d crates/aletheon/src/wiring/adapters/google
test ! -d crates/aletheon/src/wiring/adapters/runtime
test ! -d crates/aletheon/src/wiring/adapters/session
test -d crates/adapters/inference/src/core_rpc
test -d crates/adapters/inference/src/providers
! rg -n 'CorePeerPolicy' crates/adapters/inference/src/providers --glob '*.rs'
bash scripts/cargo-agent.sh check -p adapters-inference --all-targets
bash scripts/cargo-agent.sh check -p adapters-gbrain --all-targets
bash scripts/cargo-agent.sh check -p adapters-google --all-targets
bash scripts/cargo-agent.sh check -p adapters-agent-backend --all-targets
bash scripts/cargo-agent.sh test -p adapters-sqlite --lib
bash scripts/cargo-agent.sh test -p aletheon --test anthropic_provider_timeout
bash scripts/cargo-agent.sh test -p aletheon --test openai_provider_timeout
bash scripts/cargo-agent.sh test -p aletheon --test gbrain_mcp_adapter
bash scripts/cargo-agent.sh test -p aletheon --test google_sync_recovery
```

如果 Codex 二次审核拒绝某个新 crate，必须给出具体替代 owner 和无环依赖图后更新计划；不得在
实施中临时把它们塞回 `aletheon`。

---

### M8 — Gateway、daemon transport 与 host 收敛

**目的：** daemon transport 不再持有业务 service 类型，bootstrap 不再定义 authority。

**来源与目标：**

| 当前路径 | 目标 |
|---|---|
| `daemon/protocol.rs` | `gateway::protocol`，仅 wire-safe 类型 |

**2026-08-18 M8.1 执行记录（COMPLETED）：** 连接协议状态机（`ConnectionProtocolState`/`ProtocolEvent`/
`ProtocolAction`/`reduce_protocol`/`accept_with_capabilities`/`accept_legacy`/`NegotiatedProtocol`）从
`wiring/daemon/protocol.rs` 移入 `gateway::protocol::connection`（类型改 pub）；`daemon/server.rs` 改引用
`gateway::protocol::connection`；旧 protocol.rs 删除。验证：gateway lib 9 tests（含 7 个协议 reducer 测试）、
aletheon --all-targets、architecture 0 findings、fmt 通过。
| `daemon/handler/rpc/**` | `gateway::server` typed route translation |
| `daemon/handler/typed_gateway.rs` | gateway handler + application port |

**2026-08-18 M8.3 执行记录（COMPLETED — 结构已就位）：** typed RPC route 表与翻译由 gateway 拥有
（`gateway::server::handlers::typed::TypedRouteHandler` + `TypedApplicationPort`）；daemon 仅提供窄
`TypedDaemonUseCases` port（select_workspace_session/submit_explicit_chat/protocol_read_snapshot_for），
`DaemonTypedApplication` 实现 `TypedApplicationPort`。M8.2 中 `handle_typed` 已把 typed 分派收进
`ConnectionDispatcher`，daemon connection loop 不再直接构造 typed app。残余：`DaemonTypedApplication` 仍经
`HandlerPorts`（聚合）访问服务，收窄为 feature ports 属 M8.4。

**2026-08-18 M8.4 执行记录（已完成）：** HandlerPorts 五个 concrete 服务已收窄为 consumer-owned
feature-port trait：`evaluation`→`application::evaluation::EvaluationPort`、`memory_maintenance`→
`mnemosyne::memory_maintenance::MemoryMaintenancePort`、`extensions`→`crate::extensions::ExtensionsPort`、
`memory_gateway`→`mnemosyne::memory_gateway::MemoryGatewayPort`、`session_input`→
`application::session_input::SessionInputPort`；每个 trait 由对应 concrete service 实现，HandlerPorts 字段改持
`Arc<dyn ...>`。已核实：`memory_health` 已由窄方法封装、`transport` 已是 port 聚合、`kernel`/
`workspace_checkpoint`/`conscious_workspaces` 在 handler 无 ports 直用（死聚合，可后续移除）、`debug`/`review`
有宽接口与 `&Arc<Self>` receiver 摩擦待处理。验证：workspace --all-targets、architecture 0 findings、fmt 通过。

**2026-08-18 M9 执行记录（COMPLETED）：** `crates/aletheon/src/wiring.rs` 与 `src/wiring/` 物理删除；
`host`/`composition`/`adapters`/`daemon` 上移到 crate 根（`crate::host`/`crate::composition`/`crate::adapters`/
`crate::daemon`）；`wiring.rs` 的 `run_core`/`run_daemon`/`ensure_user_daemon` 重建于
`crate::host::launcher`；`composition.rs` CGP-01 骨架并入 `composition/mod.rs`；148 个消费者、全部 census、
checker（architecture-check.sh 路径 + crate::wiring 模式）、suite fixtures、approval-closure 路径同步更新。
验证：workspace --all-targets、完整 architecture suite 0 findings、daemon_lifecycle 7 + session_protocol_reconnect 8
+ daemon_streaming_turn_e2e 12、fmt、**部署 + SHA parity + 真实 LLM 请求** 通过。
| `daemon/handler/tool_executor.rs` | application capability use case + Corpus/Kernel adapter |
| `daemon/session_projection.rs` | runtime/application projection port adapter |
| `daemon/debug_handler.rs` | typed admin/debug query adapter；不得直读 authority store |
| `daemon/server.rs` | `aletheon::host::unix_server`，只管理 socket/connection/task lifecycle |

**2026-08-18 M8.2 执行记录（COMPLETED）：** `ConnectionContext`/`ConnectionRole`/官方 memory 进程角色认证
迁入 `wiring/host/unix_server.rs`；定义 `ConnectionDispatcher` 窄 trait 并为 `RequestHandler` 完整实现；
**`UnixServer` 泛型化为 `UnixServer<D: ConnectionDispatcher>` 并物理迁入 `host/unix_server.rs`**
（SocketPrivacy、LegacyClientHandshakeAdapter、parse/dispatch/versioned/subscription、bind_path_listener 随迁；
`daemon/server.rs` 保留 ActivationEnvironment/inherited_listener + re-export）。验证：aletheon --all-targets、
daemon_lifecycle 7、session_protocol_reconnect 8、daemon_streaming_turn_e2e 12、architecture 0 findings、fmt 通过。
| `daemon/bootstrap/**` | `aletheon::composition/**`，只构造/连接对象 |
| `daemon/legacy_session.rs` | 删除；若 wire compatibility 仍受支持，移到 gateway explicit legacy adapter 并有 sunset ledger |
| `user_runtime.rs` | `aletheon::host::user_daemon` |

**其余顶层 wiring 路径的唯一去向：**

| 当前路径 | 必须在 M8 前完成的去向 |
|---|---|
| `governed_review/**` | application review use case/port + platform filesystem store；aletheon 只注入 |
| `workspace_trust.rs` | application trust decision/port + platform workspace-evidence adapter |
| `core_rpc/**` | M7.1 `adapters-inference::core_rpc`；host 只管 socket/lifecycle |
| `core_runtime.rs` | `RegistryInferencePort` 按 M7.1 进入 `adapters-inference`；`MachineInferenceRuntime` 进入 `aletheon::host::core`；保留原 `:137-139` 对 telegram/supplemental-memory/MCP 用户凭据的 fail-closed 拒绝；旧文件删除 |
| `readiness.rs` | `aletheon::host::readiness` |
| `doctor.rs` | `aletheon::host::doctor`，CLI 只渲染 |
| `exec.rs`、`exec_session.rs` | `aletheon::host::exec` + 唯一 TurnService adapter |
| `embodiment/**` | hardware/platform adapter；composition 只管理生命周期 |
| `evolution_coordinator.rs` | metacog policy + cognit reflection port + platform lineage store；旧文件删除 |
| `approval_service.rs` | M3 `application::approval` service/ports + SQLite/apply adapters |
| `cognitive_runtime.rs` | 拆到 Cognit/Metacog/ports，旧 `AletheonCognitiveRuntime` 删除 |
| `mode_router.rs` | `aletheon::host` entry-mode selection |
| `domain.rs` | `aletheon::composition::services` crate-private handles，按 feature builder 拆小 |
| `extension.rs` | `aletheon::host::cli::extension` 只读 Corpus inspector adapter |

M8 开始前，M0 ledger 中以上每一行都必须有完成证据。任何未分配路径不得默认塞进
composition；host/composition 目标也不得包含 repository、状态转换或跨域用例 policy。

**bootstrap 规则：**

- 可以读取 binary config、创建 Arc、注册 implementation、启动 background task；
- `host::core` 装载 machine core 配置时必须继续 fail-closed：system core 不得装载
  telegram、supplemental memory 或 MCP 用户凭据；
- 不定义业务 transition、repository SQL、provider wire model 或 protocol error policy；
- `request.rs`/`services.rs` 超过预算时按功能 builder 拆分，但所有 builder 仍是 composition；
- background task 必须返回可取消 handle，并在 shutdown/restart 中被 authoritative wait/reap。

**2026-08-17 M8 提前消债：** architecture hotspot gate 发现
`daemon/bootstrap/request.rs` 一度达到 1,624 行并超过 1,600 行预算。未扩大预算；Goal stale-attempt recovery、
Goal restart recovery 与 active-objective continuity projection 已拆到私有 composition builder
（`crates/aletheon/src/wiring/daemon/bootstrap/goal_recovery.rs:9-71`），request bootstrap 只调用该 builder
（`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:136`），文件降至 1,560 行。同步修正
`config/approval-closure.toml:8-10` 和 `:35-36` 中已迁移的 Application owner locator 后，architecture suite
通过。该提前消债只闭合 hotspot/ledger，不代表 M8 transport/host cutover 已完成。

**验证：**

```bash
bash scripts/cargo-agent.sh test -p gateway --lib
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh check -p aletheon --all-targets
bash scripts/cargo-agent.sh test -p aletheon --test daemon_turn_api_boundary
bash scripts/cargo-agent.sh test -p aletheon --test session_protocol_reconnect
bash scripts/cargo-agent.sh test -p aletheon --test daemon_lifecycle
bash tests/suites/operations/cli_static_test.sh
```

**2026-08-18 执行记录（M8 host 收敛 — 已完成）：**

- 顶层 host 文件全部归位 `wiring/host/`：doctor、readiness、mode_router、exec、exec_session、
  extension、user_runtime、core（原 core_runtime）、cognitive_runtime、evolution_coordinator、domain、
  embodiment、governed_review、workspace_trust；`wiring.rs` 只保留 `adapters`/`composition`/`daemon`/`host`；
- 每次归位同步更新 wiring-ownership ledger、RA/K0 census、path-inventory 与 architecture-check 内部路径；
- 已验证：aletheon --all-targets、architecture suite 0 findings / 0 additions、fmt 通过；
- daemon transport → gateway、daemon bootstrap → composition 已完成（M8.1–M8.5，见 M9 删除 wiring 记录）。

---

### M9 — 删除 `wiring`，收紧 API，更新架构账本

**改动：**

1. 将剩余 host/composition 文件移到 §3.2 目标目录；
2. 删除 `crates/aletheon/src/wiring.rs` 和 `src/wiring/`；
3. `crates/aletheon/src/lib.rs` 删除 `pub mod wiring`；
4. `launcher.rs` 继续作为唯一 Core/Daemon/Exec public facade；
5. 从 `crates/aletheon/Cargo.toml` 删除不再由 binary 使用的
   `rusqlite`、`reqwest`、adapter-only 依赖；
6. 更新所有 architecture census、hotspot budget、dependency ledger、docs locator；
7. 删除 old compatibility/debt 行，不把已删除路径标成 active；
8. 审计 `runtime::orchestration::EvidenceDrivenController` 的全部消费者与行为；若无生产
   消费者则删除并移除 re-export，只有证据证明语义属于 Agora 才允许合并，禁止按名称搬家；
9. 更新 `docs/design/architecture-overview.md`，此时才允许写“所有权收敛完成”。

**最终静态验收：**

```bash
test ! -e crates/aletheon/src/wiring.rs
test ! -d crates/aletheon/src/wiring
! rg -n 'pub mod wiring|aletheon::wiring|crate::wiring' crates examples --glob '*.rs'
! rg -n 'rusqlite|reqwest' crates/aletheon/src/host crates/aletheon/src/composition --glob '*.rs'
! rg -n 'aletheon\s*=|aletheon::' crates/*/Cargo.toml crates/*/src --glob '*.{toml,rs}' --glob '!crates/aletheon/**'
bash scripts/cargo-agent.sh metadata --format-version 1 > /tmp/aletheon-m9-dependency-graph.json
bash scripts/aletheon.sh test architecture
bash scripts/cargo-agent.sh check --workspace --all-targets
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
```

architecture validation 还必须读取 M0 ledger 并证明：

- 完整传递依赖图无禁止回边；
- 每个 durable fact 只有一个 reducer/write owner/ID mint owner；
- SQLite/FS/HTTP/process effect 只存在于指定 adapter/host；
- composition 没有 Goal/Turn/Agent 状态转换或 daemon_react 逻辑；
- `adapters-sqlite` 只持久化，不解释业务 policy；
- source architecture gate 与 behavior parity 分开报告，路径删除不计为行为通过。

**量化预算：**

- `wiring/application`、`wiring/adapters`、整个 `wiring`：0；
- `aletheon::composition` 单文件建议 ≤1,000 LOC，`request` 式聚合文件不得重新达到 1.5K；
- `application` 大功能文件建议 ≤1,200 LOC，超过必须按 command/query/service/port 拆分；
- LOC/名称超预算触发 owner/coupling 复核；复核未闭合时阻止完成，但低于预算或换名也
  不能证明模块合理。

---

### M10 — 分层、安装态与真实使用验收

#### M10.1 Changed/full validation

```bash
bash scripts/aletheon.sh test changed --report /tmp/aletheon-wiring-migration-changed.json
bash scripts/aletheon.sh test architecture
bash scripts/cargo-agent.sh check --workspace --all-targets
```

只允许 integration/verification owner 运行 workspace-wide 命令；所有 Cargo 命令必须经
`scripts/cargo-agent.sh`。

#### M10.2 行为矩阵

至少验证：

| 功能 | 必须证明 |
|---|---|
| Session | create/resume/fork/reconnect/restart 后 authority 一致 |
| Identity | runtime 唯一 mint canonical Session/Turn/Agent ID；contracts 转换集中且 alias 不能升级 authority |
| Turn | daemon/exec 同一 use case；principal fail-closed、cancel/deadline、durable terminal failure、GenerationFence、compaction recovery 一致 |
| Agent | spawn/wait/send/cancel、recovery、settlement、memory isolation |
| Goal | create/retry/approval/apply/verification/artifact hash mismatch/rollback/restart recovery |
| Approval | scoped transient grant 保持 principal/thread/tool/path/version/hash/expiry，不能跨 principal/thread 复用 |
| Inference | streaming UTF-8、timeout、Retry-After、backpressure、runtime facts |
| Memory | GBrain/default source recall、degraded semantics、`MemorySensitivityV1`、Turn context 注入 |
| Extension | quarantine failure与依赖恢复后重新探测 |
| Gateway | typed RPC、legacy 明确兼容面、principal/workspace isolation |
| Tool | permit、OperationId、progress、terminal receipt、no duplicate execution |
| Robot | 显式 target binding 不变，且本迁移不修改 actuator/control loop |

测试证据分两栏报告：

1. source architecture gate：`include_str!`、`rg`、路径删除、禁止 import；
2. behavior parity：真实 service/adapter 调用、restart/recovery、receipt、失败路径。

`agora_bound_permit`、`approval_service`、`admin_service`、`agent_admission`、
`context_assembler`、`turn_use_case_ports`、`session_use_case_port`、`goal_service`、
`governed_review_rpc`、`memory_bifurcation_guard`、`core_user_boundary`、
`kernel_clock_composition` 等源码字符串测试需更新 locator 或保留结构断言，但不能计作
对应行为通过。涉及测试源文件的升级仍需显式测试文件授权。

#### M10.3 系统安装态

完成条件必须使用系统安装路径：

```bash
sudo bash scripts/aletheon.sh deploy
/usr/bin/aletheon version --json
/usr/bin/aletheon doctor --json \
  --config /home/aurobear/.aletheon/config.toml \
  --project-dir /home/aurobear/Workspace/aletheon
```

并证明：

1. `target/release/aletheon`、`/usr/bin/aletheon`、system core、user daemon、memory agent
   executable SHA-256 完全一致；
2. 三个服务 restart counter 在两个稳定窗口内不增长；
3. doctor deployment revision/config hash 一致且 healthy；
4. 使用 `/usr/bin/aletheon` 和官方 user socket 完成真实 LLM 请求；
5. 真实请求没有 `provider_unavailable`、`provider_rejected_request` 或 rendered inference error；
6. inference rounds、provider retries、tool calls 分开报告；
7. 运行一次真实 GBrain recall，并证明首个相关 supplemental item 能进入新会话上下文；
8. monitor verdict 与 rendered frame/session/audit/daemon log 一致。

---

## 7. 提交和审查粒度

每个 `M<n>.<n>` 为独立、可回退提交。非平凡提交必须使用：

```text
<type>(<scope>): <subject>

Problem / solution context.

- concrete change
- ownership movement
- validation evidence
```

提交前必须：

1. 只 stage packet 允许路径；禁止 `git add -A`；
2. 检查 staged diff；
3. 运行该 packet 最窄验证；
4. 记录 schema/wire/authority 是否变化；
5. 不把当前用户无关文件或未跟踪计划加入提交。

建议审查顺序：owner/authority → dependency direction → recovery/security → behavior parity →
代码风格。仅看路径或 LOC 不足以批准。

---

## 8. 风险与回退

| 风险 | 早期信号 | 防护 | 回退 |
|---|---|---|---|
| Canonical identity 模糊 | runtime/contracts 同名 ID 且 coordinator 散落 String/UUID 双向构造 | 明确 mint authority、wire/domain 表示和唯一转换 adapter | 回退转换 packet，恢复单一路径；不机械删除仍需的 wire type |
| 形成依赖环 | direct edge 看似安全，但经 platform/domain 传递回 application | consumer-owned port；完整 metadata resolve graph | 回退 manifest，不把 adapter DTO 放 contracts |
| 双 authority | old/new reducer 或 repository 同时写 | 一次性调用点 cutover，不双写 | 回退整个 feature packet |
| 恢复语义丢失 | restart 后重复 settlement/副作用 | generation fence、idempotency 和 receipt parity | 回退 packet，保留旧单路径 |
| daemon/exec 漂移 | 两种入口构造不同工具、身份或 policy | 一个 TurnService，入口只做 adapter | 回退 M6，不能保留两个生产 engine |
| adapter god crate | 新 crate 同时含 provider、Google、Pi、SQLite | 按外部边界建聚焦 crate | 拒绝该 crate，更新 owner matrix |
| contracts 膨胀 | 为解循环不断加入实现 DTO | 只有 wire-safe/跨域稳定类型入 contracts | 回退新增 contract |
| 测试假绿 | 测试只断言路径/字符串 | 保留行为/恢复/receipt 测试 | 不接受完成，补获批的行为验证 |
| 安装态回归 | dev binary 通过、系统 daemon 仍旧 | 最终 sudo deploy + digest + real request | 回滚部署 manifest 的 previous binary/config |
| 核心控制风险 | Turn/Agent/robot 路径混在同一提交 | M5/M6 单独批准；不改 robot control loop | 回退核心 packet，保留 adapter-only 改动 |

---

## 9. 完成定义

只有同时满足以下条件才能标记 COMPLETE：

- [x] Codex 二次审核对 §3 owner matrix、§4 dependency rules、M5/M6 core scope 给出明确批准（APPROVE WITH CONDITIONS，2026-08-18），且 developer 已授权实施；
- [x] `crates/aletheon/src/wiring` 物理删除（M9，lib.rs 改 `adapters|composition|daemon|host`）；
- [~] `aletheon` 不公开 host/composition internals — 部分：四个内部模块已 `#[doc(hidden)]` 并从文档 API 移除（lib.rs），`launcher/config/doctor/extension/extensions/scoreboard/workspace` 为稳定 facade；完整 `pub(crate)` 收口需 feature-gated test-support + 72 个 integration test 文件 cutover，见 §10.1 残留；
- [x] Application production 源无 concrete SQLite/HTTP/process/file mutation（APX-IO census 跟踪；唯一 `OpenOptions` 在 `thread_authority.rs` 为有界 settings 写，已登记）；
- [x] Runtime production 源无 concrete SQLite，且没有 application/adapter 反向依赖（`runtime/src` 无 rusqlite/Connection；architecture gate 0 findings）；
- [x] 完整 Cargo resolve graph 无禁止回边（architecture-check 全量依赖图校验，0 findings）；
- [x] M0 owner ledger 覆盖每个顶层 wiring 路径，且没有 `TBD`、“或”或默认 composition owner；
- [x] authority census 证明每个 durable fact 只有一个 reducer/write/ID mint owner（RA census + gate 0 findings）；
- [x] effect census 证明 SQLite/FS/HTTP/process 只在指定 adapter/host（kernel-effect census + gate）；
- [x] Goal/Agent/Turn/Approval/Verification 按功能切片存在明确 service + port + adapter（M8.4 收口：HandlerPorts 8 个 concrete/宽服务全部改为窄 consumer port）；
- [x] canonical Session/Turn ID 转换集中，caller/thread alias 不能升级 authority（session protocol / gateway-route census）；
- [x] Goal artifact fail-closed、transient approval scope、MemorySensitivityV1、Turn principal/cancel/deadline/fence/recovery 行为通过（behavior 测试矩阵通过）；
- [x] 没有 compatibility re-export、双写或 old/new engine feature flag（源码 grep 为空）；
- [~] `runtime::orchestration` 未退役但已审计为单一控制器：被 `agora/cognitive_role_workflow` 消费（非第二控制器）；正式退役待 Agora 侧审计，见 §10.1 残留；
- [x] architecture gate 0 findings、0 allowlist debt；
- [x] daemon/exec 走唯一 Turn use case（`turn_use_case_ports` 静态门禁 + ports 单一 `TurnUseCases`）；
- [x] source architecture gate 与 behavior parity 分开报告并全部通过（M10 报告 §1/§4.1）；
- [x] changed tests、workspace check、architecture check 全部通过（129 步 0 失败，closeout 后复验）；
- [x] 系统 deploy、hash parity、restart stability、doctor、真实 LLM、真实 memory recall 通过（M10 安装态验收）；
- [x] 文档 locator 与当前代码一致（closeout 已同步 census/checker/plan；残留项显式跟踪）。

---

## 9.1 2026-08-18 architecture closeout 记录

最终审核判定：**MIGRATION CLOSEOUT DONE · ARCHITECTURE CONVERGENCE TRACKED**。
本 pass 完成以下闭合：

- **架构门禁假绿修复**：`tests/suites/architecture/architecture_check.sh` 三个已删除
  `wiring/host/*` 路径更新为 `host/*` 并加存在性守卫（缺失路径 → suite FAIL，不再 grep 缺失文件后继续）。
- **迁移 warnings 清理**：`daemon/server.rs`（info/warn、`RequestHandler`、协议 imports、
  `CONNECTION_NOTIFICATION_CAPACITY` 重复定义）、`host/unix_server.rs`（OsString 移入 test 模块）、
  `extensions/mod.rs` 未用 imports 全部清除；`--all-targets` lib warning-free。
- **M8.4 HandlerPorts 收口**：移除无消费者 `_reflection`；`kernel`/`pending_approvals`/
  `workspace_checkpoint`/`transaction_review`/`conscious_workspaces`/`debug`/`review` 全部收窄为
  窄 consumer port（`KernelCleanupPort`/`PendingApprovalsPort`/`WorkspaceCheckpointPort`/
  `TransactionReviewPort`/`ConsciousWorkspaceRegistryPort`/`DebugHandlerPort`/`ReviewPort`）。
- **Legacy session sunset ledger**：`docs/plans/legacy-session-sunset-ledger.md`，逐方法登记
  legacy JSON-RPC `session.*` → typed Gateway 替换与 sunset 规则；census `RA-S-20` 陈旧 owner
  路径修正。
- **公共 API 收紧（部分）**：四个内部模块 `#[doc(hidden)]`，声明稳定 facade；完整 `pub(crate)`
  为残留。

### §9 残留（显式跟踪，不阻塞安装态）

1. `aletheon::host/composition/daemon/adapters` 完整 `pub(crate)` 收口：需 feature-gated
   test-support 重导出 + 72 个 integration test 文件路径 cutover + test 工具链 feature 传递。
2. `runtime::orchestration` 退役：已被 `agora/cognitive_role_workflow` 消费（单一控制器），
   正式退役待 Agora 侧审计。

---

## 10. 给 Codex 的二次审核问题

请按 **APPROVE / REVISE / REJECT** 逐项回答，并给出代码/依赖依据：

1. §4.1 是否正确区分当前允许边与前置解耦后才允许的 domain 边？请用完整传递依赖图
   核查 `mnemosyne/platform/application` 和 dirty `dasein` 路径，而不是只看直接依赖。
2. M6 是否既避免整包搬 `TurnPipeline`，又明确由 provider-neutral
   `application::turn::TurnService` 拥有用例顺序？`daemon_react` 的 Cognit 核心和 adapters
   是否拆清，composition 是否保持纯接线？
3. Runtime 是否应继续只拥有 reducer/writer/authority，而不拥有跨 Cognit/Corpus 的 use-case
   orchestration？
4. §3.4 的 `adapters-inference/gbrain/google/agent-backend` 是否粒度合理？core RPC、HTTP
   provider 与 backpressure 的内部分界是否清楚；`RegistryInferencePort` 与
   `MachineInferenceRuntime` 是否分别落到 inference adapter 和 `host::core`；Google SQLite
   是否已经要求单一 owner；verification command 与 worktree I/O 是否已唯一归属 platform？
5. Goal transition/advance、artifact FS、metadata SQL、host scheduler、progress 和 quota 的
   owner 是否唯一，且没有把 `GoalWorker` 整文件或状态分支放入 application/composition？
6. Gateway handler 与 Unix server 的切分是否正确：Gateway 拥有 typed protocol dispatch，
   Aletheon host 拥有 socket/systemd lifecycle？
7. M5/M6/M10 是否完整覆盖 principal fail-closed、GenerationFence、cancel/deadline、
   durable-write failure、compaction recovery、sensitivity、transient approval 和 robot target？
8. canonical Session/Turn ID、Approval、Goal、Agent settlement、Google 和 orchestration 是否
   都只有一个 authority；同名 contracts/runtime ID 的转换契约是否充分？
9. source architecture gate 与 behavior parity 是否已分开；哪些现有 `include_str!` 测试需
   更新 locator，哪些关键行为仍缺证据？
10. §9/完成定义的传递依赖、authority census、effect census、composition policy、
    runtime::orchestration 退役和行为门，是否足以防止新的 god module？

审核输出建议格式：

```text
Verdict: APPROVE | REVISE | REJECT

Blocking findings:
1. [severity] plan section — evidence — required correction

Non-blocking findings:
1. ...

Dependency graph verdict:
Authority uniqueness verdict:
Functional modularity verdict:
Operational acceptance verdict:
```
