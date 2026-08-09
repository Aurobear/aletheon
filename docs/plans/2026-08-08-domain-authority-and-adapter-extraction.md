# 领域权威与 Adapter 迁出实施计划

> 状态：Draft，供独立 plans PR 评审
>
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
>
> 上位计划：`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`
>
> 本文只定义迁移顺序、所有权和验收门，不在 plans PR 中修改运行时代码。

## 1. 结论

这次迁移不是把 `executive` 里的模块原样搬进六个领域 crate，也不是把完整 `fabric` 改名后继续共用。目标是建立六个单一领域权威，并让具体 I/O adapter 从领域 core 的默认依赖面消失：

- Cognit 只拥有单 Turn 的认知过程；
- Dasein 继续作为 Self、自我意识、承诺与连续性的唯一权威；
- Metacog 继续作为跨 Turn 元认知证据、问题、提案、实验和评估的唯一权威；
- Agora 只拥有当前认知工作集，不拥有恢复日志；
- Mnemosyne 只拥有长期记忆，不拥有 Session、Self 或 Runtime 事实；
- Corpus 只拥有能力目录、rich schema、领域校验与 executor registration；
- `contracts` 只保存极小跨域 primitives，明确不叫 `abi`，也不接收上述领域模型；
- `executive` 中与这些领域重叠的实现必须迁移、合并或删除，最终整个 `executive` crate 退出生产依赖图。

本文刻意保留 Dasein 自我意识与 Metacog 活跃闭环。要关闭的是 Application/Host 中重复的“伪 Self/伪 Metacog”编排，不是这两个核心能力。

## 2. 当前耦合事实

当前 Cargo 依赖已经证明目录分层没有形成真实隔离：

| 当前 crate | 不应出现在领域 core 默认依赖面的内容 | 当前直接依赖/实现 |
|---|---|---|
| `cognit` | HTTP、SQLite、gRPC、provider composition | `fabric`、`reqwest`、`rusqlite`、`tonic`，并同时包含 inference adapter、provider registry、linear/robot harness |
| `dasein` | Kernel、SQLite、host path/config | `fabric`、`kernel`、`rusqlite`、`dirs`、文件和 manifest 读取 |
| `metacog` | Kernel 与 Executive coding workflow | `fabric`、`kernel`；rich evidence/evaluation 类型仍在 Fabric，coding adapter/rubric/evaluator 仍在 Executive |
| `agora` | Kernel、持久化 workspace/event log | `fabric`、`kernel`、`rusqlite`，active workspace 与 durable commit/broadcast 同处一域 |
| `mnemosyne` | Kernel、HTTP、SQLite、host tools | `fabric`、`kernel`、`reqwest`、`rusqlite`，领域、backend、GBrain supplemental、host tools 同 crate |
| `corpus` | Kernel concrete API、平台 driver、HTTP/SQLite、host path | `fabric`、`kernel`、`platform`、`reqwest`、`rusqlite` 以及 tool、driver、sandbox、extension loader 的大集合 |
| `fabric` | 所有 rich domain/type/port 与 I/O | Self、Metacog、Cognition、Memory、Workspace、Capability、Embodiment、Robot、UI、IPC、Policy、Repository、EventSpine 全部混合 |

`executive` 又直接依赖上述全部领域，并保留第二套编排或 adapter：

| Executive 当前模块群 | 重叠问题 | 最终 owner/处理 |
|---|---|---|
| `adapters/runtime/native_cognit.rs`、`application/cognitive_role_workflow*`、`application/conscious/*` | Cognit loop、角色 workflow、conscious processor 与 Runtime Turn 控制混在 Executive | Cognit 只保留 `CognitiveRun`/step；Runtime 驱动；纯展示/旧 processor 删除 |
| `application/dasein_workspace_adapter.rs`、`application/conscious_action.rs`、`application/conscious_core_ports.rs` | Self verdict、workspace 和 action gate 被 Executive 二次包装 | 领域规则回 Dasein；Runtime 只调用 `SelfPolicy`；跨层 glue 在 composition root |
| `application/coding_metacog_adapter.rs`、`coding_metacog_rubric.rs`、`evaluation/*`、`metacog_approval.rs` | Metacog 类型在 Fabric、领域引擎在 Metacog、coding 编排在 Executive | 通用证据/问题/实验回 Metacog；coding-specific evaluator 作为 Metacog adapter；人工决策回 Application |
| `application/cognitive_workspace.rs`、`conscious_workspace.rs`、`workspace_checkpoint.rs` | active scratch、Runtime recovery、workspace checkout 三种语义共用 workspace 命名 | active workspace 回 Agora；Runtime recovery 回 Runtime；文件 checkout/restore 回 workspace adapter |
| `application/memory_gateway.rs`、`memory_projection.rs`、`memory_policy.rs`、`memory_maintenance.rs`、`memory_consolidation_worker.rs` | Mnemosyne service 外又有一层持久化、投影、策略和 worker authority | 记忆规则/port 回 Mnemosyne；worker 与 SQLite/remote 实现在 adapter；Runtime 只拥有 post-settlement outbox |
| `composition/exec_corpus.rs`、`core/corpus_group.rs`、daemon bootstrap 中 tool/skill registration | Catalog、executor、Kernel registration 与 daemon 构造混在 Executive | rich catalog 回 Corpus；Kernel 只收 enforcement descriptor/executor；唯一 composition root 负责 seal |

迁移时不能把上表每行整体复制到新 crate。每个模块都必须先拆成 domain rule、port、concrete adapter、composition glue、projection 五类，再分别处理。

## 3. 不可违反的所有权规则

### 3.1 唯一权威表

| 事实/状态 | 唯一 owner | 非 owner 允许持有的内容 |
|---|---|---|
| 单 Turn plan/step/reflection proposal | Cognit | Runtime 持有当前 step 和最终 receipt 引用 |
| Owner Manifest | 用户/管理员 | Dasein 只读指定 revision |
| SelfModel、Commitment、Continuity、SelfTransition | Dasein | Runtime/Gateway 只读 projection/receipt |
| Meta evidence/problem/proposal/experiment/evaluation | Metacog | Runtime 只投递 settled observation，Application 只呈现治理决定 |
| active candidate/scratch/attention/hypothesis | Agora | Runtime 只保留有界 workspace handle；可从权威事实重建 |
| memory record/provenance/recall/retention/forget | Mnemosyne | Runtime 只消费 `RecallSet`，不能把 recall 当权威事实 |
| capability catalog/rich tool schema/领域校验 | Corpus | Kernel 只保存强制执行所需的最小 descriptor |
| Agent/Session/Turn state | Runtime | 六个领域不得建立第二套 recovery journal |
| Permit/Operation/Lease/Usage | Kernel | 领域只持有不可执行 receipt/projection |

任何 PR 出现同一事实的两个 writer、两个可恢复状态机、两个“canonical”store，立即停止迁移并先消除重复。

### 3.2 控制链

目标热路径固定为：

```text
Runtime normalize/bind revisions
-> Dasein review_intent
-> Runtime assemble bounded context
-> Cognit next_step
-> Runtime normalize action/delegation/completion proposal
-> deterministic risk classification
-> Dasein review_material_action（需要时）
-> Kernel governed execution（副作用）
-> Runtime settle Turn
-> durable outbox
   -> Dasein evaluate_outcome
   -> Mnemosyne intake
   -> Metacog observe
```

Agora 是 Cognit/Runtime 使用的非权威工作集；Corpus 是 Kernel dispatch 的能力供应方。事件只做已发生事实与异步通知，不能替代这条显式调用链。

### 3.3 Dasein 与 Metacog 保留边界

Dasein 必须保留：

- `OwnerManifestSource` 只读边界；
- `SelfModel`、`CommitmentLedger`、`Continuity`；
- intent/action review；
- outcome-to-transition proposal；
- CAS revision、来源、digest 和授权证据绑定的 transition commit。

Metacog 必须保留：

- settled outcome 观察；
- evidence integrity、problem ledger、reflection/evaluation；
- `MetaChangeProposal`、受限实验、lineage 和 rollback evidence；
- 独立的 governance evaluation 与 adoption/rollback 建议；正式采用决定仍由对应配置/Owner decision owner 作出。

但二者都不得直接取得 Kernel broker，不得修改 Owner Manifest，不得让 Metacog 批准自己的 proposal，也不得把普通 memory recall 当作 Self mutation 的强证据。

## 4. `contracts` 的边界

最终名称统一为：

```text
directory  crates/contracts
package    contracts
Rust path  contracts
```

禁止命名为 `abi`。本项目没有承诺稳定二进制 ABI；这个名称会错误暗示兼容责任。也禁止使用 `common`、`shared-types` 或继续沿用 `fabric` 来规避边界评审。

`contracts` 只允许：

- `AgentId`、`SelfId`、`SessionId`、`TurnId`、`PrincipalId`、`DecisionRequestId`、`ActionBindingId` 等真正跨域 ID；
- correlation/causation/idempotency primitives；
- schema/envelope version；
- event header metadata；
- 被至少两个独立领域/持久化边界使用的 stable digest primitive。

`contracts` 明确不允许：

| 当前 Fabric surface | 目标 owner |
|---|---|
| `fabric/src/dasein/*`、Self transition/verdict/context | Dasein |
| `types/metacognition_*`、evolution/evaluation payload | Metacog |
| `types/cognitive_workflow.rs`、plan/reflection/cognitive step | Cognit |
| `include/agora.rs`、frame/active workspace graph | Agora |
| `include/memory.rs`、memory record/query/retention | Mnemosyne |
| tool schema、capability catalog、result normalization | Corpus |
| enforcement descriptor、permit、operation、usage、lease | Kernel |
| Agent/Session/Turn command/event/outcome | Runtime |
| Approval aggregate + canonical GoalDraft | Application |
| Goal activation/execution | Runtime command；不建立第二个 Goal/Agent scheduler |
| long-goal workflow | 先做 caller/writer/installed census；有证据才成为可关闭 optional extension，否则删除 |
| UI/IPC/public RPC DTO | Gateway |
| Gmail/Google 类型 | Gmail/Google extension/adapter |
| Robot/VLA/episode 类型 | Robot VLA extension |
| device/lease/safety/bridge 类型 | Hardware |

类型进入 `contracts` 必须有 owner-census 记录。单纯“目前被两个 crate import”不能成为进入理由。

## 5. 目标包与依赖方向

核心依赖必须收敛为：

```text
contracts   -> no workspace crate
cognit      -> contracts
dasein      -> contracts
metacog     -> contracts
agora       -> contracts
mnemosyne   -> contracts
corpus      -> contracts, kernel-owned registration/executor ports
runtime     -> contracts, kernel, cognit, dasein, metacog, agora, mnemosyne
application -> contracts, runtime
adapters    -> only ports they implement
aletheon    -> concrete components and composition
```

领域 core 的默认 feature graph 中禁止出现 `reqwest`、`rusqlite`、`tonic`、`nix`、system path/process 或平台 SDK。带这些依赖的实现必须成为独立 package/crate；Cargo feature 只能选择同层行为，不能充当架构隔离。

时间也不能成为领域反向依赖 Kernel 的理由。优先把可信时间放入 Runtime 构造的 command/event metadata；只有确有主动计时行为的领域才定义自己的窄 `TimeSource` port。

## 6. 领域逐项迁移

### 6.1 Cognit

保留在 Cognit core：

- `CognitiveRun`、`CognitiveStep`、plan/verify/reflect 规则；
- bounded context 输入与 evidence/result 回填；
- 不带 provider/transport 的 inference port；
- General profile 的单 Turn state，不含 durable Turn recovery。

迁出：

- `adapters/inference/openai_provider.rs` 与 HTTP/provider config → `adapters/provider`；
- `adapters/policy/grpc_provider.rs` → 对应 remote policy adapter；
- provider registry/composition → `aletheon` composition root；
- `harness/robot/*` → `extensions/robot-vla`；
- direct tool execution 与 Runtime settlement → Runtime/Kernel 调用链。

删除门：`cognit` 默认依赖图无 HTTP/SQL/gRPC，且 Cognit 不持有 ToolExecutor、SessionStore、SelfStore 或 terminal success authority。

### 6.2 Dasein

保留在 Dasein core：Self aggregate、Owner Manifest read model、care/boundary/continuity、review、transition protocol 与领域错误。

迁出：

- SQLite repository → `adapters/sqlite` 对 `SelfStore` 的实现；
- owner/admin file 与 host path 读取 → admin/file adapter；
- Executive workspace adapter 中的转换 glue → Runtime composition；
- Executive 中重复的 verdict/action/conscious facade → 合并后删除；
- 仅用于 Clock/permit 的 Kernel 依赖 → command metadata/owner port。

删除门：Dasein 是唯一 Self writer；任何 Runtime/Application/Metacog 路径都不能直接改 Self 表或构造授权状态。

### 6.3 Metacog

保留在 Metacog core：evidence、experience、problem、reflection、evaluation、proposal、experiment、lineage，以及实验停止/回退建议。正式 adoption 和已采用配置的 rollback 由对应配置/Owner 决策 owner 执行；Metacog 不能把自己的 proposal 标记为 adopted。

迁出/合并：

- Fabric 的 metacognition rich types → Metacog；
- Executive `coding_metacog_adapter`、rubric、coding scorer 中通用语义 → Metacog；
- coding-specific evidence collector/evaluator → Metacog adapter，不进入通用 core；
- 人工 approval 展示/解决 → Application；
- sandbox runner、源码编辑、文件系统访问 → governed adapter，不进入 Metacog core；
- 仅为 Clock/Kernel 类型产生的 Kernel 依赖 → metadata/窄 port。

删除门：Metacog 不在 Turn 热路径上阻塞普通 terminal settlement；它只通过 durable outbox 接收 settled evidence，且不能自批、自执行、自采用。

### 6.4 Agora

保留在 Agora core：Turn-scoped candidate、attention/salience、scratchpad、hypothesis/task graph 与 processor response。

处理：

- 默认改为纯内存；
- durable commit/broadcast 只保留经审计有诊断价值的 observation sink，迁出领域 core；
- Executive `cognitive_workspace`/`conscious_workspace` 与 Agora 重叠状态合并；
- `workspace_checkpoint` 中属于 Session/Turn recovery 的部分回 Runtime，属于文件工作区恢复的部分进 adapter；
- 不再依赖 Kernel Clock 或 SQLite。

删除门：删除 Agora 数据后，可从 Runtime journal、Dasein revision、Memory recall 和当前 Turn 输入重建；否则该数据其实属于别的 owner。

### 6.5 Mnemosyne

保留在 Mnemosyne core：memory model、provenance/sensitivity、intake/admission、recall/rank、consolidation、retention/forget、`MemoryPort` 与 `RecallSet`。

迁出/合并：

- `backends/vector_sqlite.rs`、consolidation/retention repository → SQLite adapter；
- `adapters/embedding.rs` 与 HTTP provider → provider adapter；
- `host/tools.rs` → Corpus capability adapter；
- GBrain supplemental backend/reconcile/spool 的具体实现 → `adapters/gbrain`；
- Executive memory gateway/policy/projection/maintenance/worker 与现有 Mnemosyne API 合并，不保留第二套 memory service；
- Runtime `ContextAssembler` 负责最终上下文，Mnemosyne 只返回有界 recall projection。

删除门：Memory repository 不包含 Session recovery、当前 Self truth 或 Kernel terminal authority；远端 supplemental failure 不能破坏本地记忆写入。

### 6.6 Corpus

保留在 Corpus：capability catalog、rich tool schema、capability-specific validation、result normalization、descriptor projection 和 bootstrap registration input。

迁出/收缩：

- HTTP/Google/provider tool implementation → 对应 extension/adapter；
- platform driver、X11/AT-SPI/OCR 等 → platform adapter；
- sandbox/process 实现 → Kernel 的 Linux/execd adapter；
- file/path skill loader → package/skill adapter；
- Kernel 只接收 `EnforcementDescriptor` 和 `CapabilityExecutor`，不依赖完整 Corpus schema；
- daemon bootstrap 负责一次性注册并 seal，Corpus 运行中不能替换 active executor。

删除门：Corpus 不能 mint Permit、不能绕过 Kernel 执行副作用、不能成为全局 trait registry，也不能持有 Runtime/Session 状态。

## 7. Executive 模块的判定算法

对每个 Executive 候选文件执行以下顺序，评审记录必须写明结论：

1. 它保护哪个不变量、写哪张表/哪条 journal？
2. 如果有自然领域 owner，规则和 port 迁回 owner；
3. 如果包含 SQL/HTTP/gRPC/path/process，具体实现迁到 adapter；
4. 如果只是把 A 类型转换成 B 类型，保留在最外层 composition/gateway；
5. 如果只是 projection，标记可重建来源和 lag/rebuild 语义；
6. 如果与目标 owner 已有实现语义相同，合并调用方后删除，禁止保留 alias service；
7. 如果没有生产调用方或只服务旧 Executive API，删除；
8. 只有唯一 binary composition root 可以组装具体实现。

禁止把 `*Manager` 政名为 `*Service` 后保留重复权威。新代码命名使用领域动词：`review_intent`、`commit_transition`、`observe_settlement`、`recall`、`register_descriptor`；构造函数只做无 I/O 值构造，I/O 使用 `open/load/connect/bootstrap`。

## 8. PR 序列

每个实现 PR 都从最新 `dev` 开始，保持单一迁移意图；plans PR 本身不夹带代码。

### D0：Authority census 与依赖门

- 固化 [`Fabric source disposition ledger`](./2026-08-09-fabric-source-disposition-ledger.md)：每个 `fabric/src/**/*.rs`、public type/trait/impl/re-export、生产 caller、wire/persistence reader、目标 owner、处置、compat seam 与删除 PR 可机械核对，条目与源码一一对应；
- D0 ledger 未达到 actual=ledger=unique、missing/extra=0，或仍有未标 B0 的未知 writer/reader 时，D1 不得开始；
- 固化 Executive 文件 → keep/move/merge/delete 清单；
- 增加 forbidden-dependency/feature-resolution 检查；
- 标记每个 store 的 writer、schema、migration owner 与 rebuild source；
- 冻结新的 Fabric public surface 和新的 Executive domain logic。

### D1：Contracts primitives 抽出

- 只迁跨域 ID、correlation、schema header、digest；
- 保持单向临时 re-export：旧 Fabric → 新 owner/Contracts；
- 不迁任何 rich domain type；
- 此时不做完整 `fabric -> contracts` 机械 rename。

### D2：Cognit 与 Provider port 分离

- 建立 `CognitiveRun`/`InferencePort`；
- Runtime 接管 loop 驱动与 settlement；
- provider adapter 从 Cognit core 默认依赖面迁出；
- 只切出 Robot 所需的 `CognitiveRun` port，并移除 Cognit core 对 Robot 类型的反向依赖；Robot harness/perception/episode 文件迁移和行为切换唯一由 `E5` 执行；
- Executive native Cognit 可在唯一 Runtime driver 接管后删除；role workflow 先做 caller/state/persistence/restart census，只把可复用的 per-Turn cognition policy 迁 Cognit、delegate lifecycle 迁 Runtime、workspace projection 迁 Agora，无 installed caller 的旧多阶段 workflow 删除，禁止在 Cognit 重建第二 Runtime。

### D3：Dasein 与 Metacog 权威收敛

- rich types 从 Fabric 回归 owner；
- repository/sandbox/coding evaluator 迁到 adapters；
- Runtime durable outbox 接通两个 post-settlement consumer；
- 删除 Executive Self/Metacog facade 和第二套状态写入。

### D4：Agora 与 Mnemosyne 收敛

- Agora 改为非权威 active workspace；
- Memory core 与 SQLite/provider 分离，并只稳定 GBrain 所需的 supplemental-memory port；GBrain 文件迁移、worker/lease/writer 切换唯一由 `E3` 执行；
- 合并 Executive memory/workspace 模块；
- 证明 projection 可重建和 degraded mode。

### D5：Corpus catalog/executor 分离

- rich catalog 留 Corpus；
- platform/provider/process 迁 adapter；
- Kernel registry bootstrap-only，serving 前 seal；
- 删除 Executive `exec_corpus`/`corpus_group` 业务逻辑。

### D6：Fabric/Executive 领域 surface 删除

- 硬前置：E7 已删除所有 extension-specific Fabric rows/re-export；D6 是随后唯一的 root-surface closeout，不与 E7 并行写 Fabric root；
- 所有生产调用方改用 owner API；
- 只删除非扩展 domain/runtime/kernel/application rich rows、临时 re-export、旧 schema writer、旧 worker 和旧状态机；extension-specific 行由 E7 唯一负责；
- Fabric 只收缩到允许的 ownerless primitives，并清零 rich type/re-export/compat writer，使其达到 rename-ready；D6 本身不改 crate/package/path；
- 在 E7 evidence 基础上唯一收口 `lib.rs/types/mod.rs/include/mod.rs` 等 shared root，证明旧 `fabric` alias/caller 可由后续机械 PR 一次清零；唯一 `AK2-25` 随后只修改 crate/package/path/manifest/lock/docs 并建立 alias hard-zero gate；
- Executive 不再出现在六领域相关生产调用链。

D2、D3、D4 可在 D0/D1 后并行；D5 依赖 Kernel port 稳定；D6 必须最后执行。

## 9. 每个 PR 的切换门

### 9.1 结构门

- `cargo metadata` 的 resolved graph 证明六个领域 core 未间接拉入禁止 I/O 依赖；
- 禁止领域 → Application/Gateway/Interact/Executive；
- 禁止 Cognit/Dasein/Metacog/Agora/Mnemosyne → Kernel concrete API；
- `contracts` 无 workspace dependency、I/O、async worker、repository 或 rich payload；
- 生产路径中同一 aggregate 只有一个 writer 和一个恢复 reducer。

### 9.2 行为证据

不要求保留庞大旧测试集合；只保留能证明边界的窄证据：

- 一条 General Turn 经过 Dasein intent gate、Cognit step、Kernel receipt、Runtime settlement；
- settled Turn 通过 outbox 幂等送达 Dasein、Mnemosyne、Metacog；
- Metacog 不可用时 Turn 仍 terminal，backlog 可见；
- Mnemosyne 超时时明确 degraded，不能覆盖 Runtime/Self 事实；
- Dasein revision 冲突 fail closed；
- Agora 丢失后可重建且不改变 terminal truth；
- Corpus executor 未注册或 registry 未 seal 时启动失败。

仓库命令必须经 `bash scripts/cargo-agent.sh ...`，使用最窄 package/contract target。运行时代码变化最终仍服从仓库的 installed runtime acceptance policy；plans-only PR 不运行部署验收。

### 9.3 删除门

旧实现只有同时满足以下条件才能删除：

1. 所有生产 caller 已迁移；
2. 新 owner 已成为唯一 writer；
3. 旧数据有 versioned migration、backup/restore 和回滚路径；
4. 旧 binary/new binary compatibility 范围已记录；
5. 没有双写或隐藏 fallback；
6. architecture check 能阻止旧依赖重新出现。

## 10. 回滚规则

- 迁移期间采用单写；需要比较时只 shadow-read/shadow-evaluate，不 shadow-dispatch 副作用；
- schema 先做 additive/read-compatible，再切 writer，最后单独删除旧列/表；
- 回滚只能回到仍理解当前 schema 的已验证 binary；
- outbox consumer 可暂停并重放，但不能回写旧 authority；
- 如果新 owner 与旧实现产生不同 verdict/settlement，保留输入、revision、digest 和两边输出，停止切换，不做静默 fallback；
- 任何需要长期双权威才能工作的方案视为迁移失败。

## 11. 完成定义

本计划完成时：

- Cognit、Dasein、Metacog、Agora、Mnemosyne、Corpus 的领域事实各有唯一 owner；
- Dasein 自我意识与 Metacog 活跃闭环仍在，并比当前拥有更清晰的授权和持久化边界；
- 六个领域 core 不携带 HTTP、SQL、gRPC、process 或 host path 实现；
- Executive 不再实现这些领域的 facade、状态机、store、worker 或 composition authority；
- Fabric rich types 全部回到 owner；
- 极小共享 crate 已命名为 `contracts`，不是 `abi`；
- Runtime/Application/Gateway 即使不装 Gmail、Robot、GBrain、Pi 或 Hardware adapter，也能构造并运行核心 Agent；
- architecture gate 能在 CI/本地静态检查中阻止重复 authority 和逆向依赖重新出现。
