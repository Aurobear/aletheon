# 生产就绪缺口与路线图协调分析（E′/A/B/C/D 伴随文档）

**Date:** 2026-07-30
**Last reviewed:** 2026-07-31
**Status:** 实现、部署与部分安装态复核记录；真实 TUI provider 验收仍未通过，
因此不得标称整体 production-ready
**Scope:** 区分已经实现、已设计但未实现、部分覆盖和完全未覆盖的生产能力；
给出启用门槛、依赖顺序和文件所有权约束。F1/F2 是本分析最初识别的新增
workstream；当前分支已实现其代码闭环，但仍按下述安装态证据独立判定。
本文件不定义具体接口或实现步骤。

## 0. 2026-07-31 实现后复核

以下结论取代 §3 中实现前的“当前事实”描述，但保留原表作为缺口来源：

| 项 | 当前实现证据 | 当前验收状态 |
|---|---|---|
| F1 | provider-keyed permit、共享 cooldown 及独立计数位于 `crates/cognit/src/adapters/inference/backpressure.rs:13-181`；health RPC 导出全部 snapshot（`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:137-171`） | 代码闭环；安装态真实请求观察到 provider 重试，尚未通过零错误压力验收 |
| F2 | closure matrix 将已暴露操作绑定到 producer/resolver/receipt/replay，并将其余操作显式 deny（`config/approval-closure.toml:1-78`）；架构门禁执行 verifier（`tests/suites/architecture/architecture_check.sh` 末尾） | 代码闭环，待真实人工 resolve/restart 验收 |
| #3 | Fabric 提供统一 trust/classification/scrub 契约（`crates/fabric/src/types/data_governance.rs:1-140`）；Memory 与 Turn 持久投影消费该契约（`crates/mnemosyne/src/consolidation/extractor.rs`、`crates/executive/src/application/turn_pipeline.rs`） | 代码闭环，focused 泄漏 fixture 已通过；待真实外部通道验收 |
| #4 | health 导出 provider 与 Turn watchdog 指标和默认 SLO alerts（`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:137-171`）；Turn 诊断来自 active index 与 monotonic age（`crates/executive/src/application/turn_coordinator.rs:45-62,151-170`） | 代码闭环，待运行态故障注入 |
| #5 | backup 使用 SQLite online backup 并逐库检查（`scripts/libexec/aletheon/backup.sh:20-43`），restore 在复制前后校验（`scripts/libexec/aletheon/restore.sh:38-72`）；migration inventory 覆盖 12 个持久组件（`config/release/migration-matrix.toml`、`scripts/libexec/aletheon/verify/migration-matrix.sh:16-65`） | 代码闭环，待安装态 backup/restore drill |
| #6 | 非本地 MCP 对 restricted-data egress fail closed，并将返回内容标为 untrusted 后 scrub（`crates/corpus/src/tools/mcp/wrapper.rs:91-127`） | 主要外部工具路径闭环，待真实 channel/MCP 验收 |
| #7 | F1 关闭 machine provider 协调；Hardware 已由 production embodiment composition 调用（`crates/executive/src/host/daemon/bootstrap/request.rs:823-850`）；role runtime 先 prepare 后注册（`crates/executive/src/host/daemon/bootstrap/runtime.rs:322-364`） | 路由代码闭环，待逐 runtime 安装态矩阵 |
| #8 | 版本化 coding harness 已存在（`tests/coding/README.md:1-31`），CI 验证 replay/契约（`.github/workflows/ci.yml:81-85`），release gate 执行 acceptance 与 installed-host drill（`scripts/libexec/aletheon/release-acceptance.sh:296-339`） | 确定性门禁闭环，真实模型 workflow 待本次运行 |
| #9 | responsibility 位于 `config/architecture/module-boundaries.txt:21-25`；hotspot owner/line budget 位于 `config/architecture/hotspot-budgets.tsv:1-7` 并由 architecture fitness test 执行 | 持续治理项，不作一次性“完成”声明 |

### 0.1 2026-07-31 安装态复核结果

- `sudo bash scripts/aletheon.sh deploy` 已通过；release、`/usr/bin` 及两个运行中
  daemon 的统一摘要为
  `f870a8058e9fbdc0e0385d913139fb185976faceb0d9eff3986458e2615ecad5`，
  deploy gate 同时证明 restart counter 在两个 7 秒窗口保持稳定，并通过 official
  user socket 的真实请求。
- 首次真实 TUI 多轮复核暴露了一个终态顺序缺陷：Cognit 的 pipeline-local
  `TurnDone` 曾先于 coordinator active-index 清理到达客户端，紧接的同 thread
  follow-up 因此被拒绝。现在 pipeline 只缓冲该事件
  （`crates/executive/src/application/turn_pipeline.rs:1279-1290`），daemon 在
  `submit_with` 返回后才发送权威终态
  （`crates/executive/src/application/daemon_turn/execute.rs:255-293`）。对应 focused
  lifecycle test 与最终 workspace suite 均通过。
- 修复后，一个未重启 TUI session 的三轮请求全部获得权威 `turn_done`、返回输入框，
  且 rendered frame 没有 provider error；但同一轮 daemon journal 出现
  `provider_unavailable` retry。因此按本仓库验收口径仍判定失败。
- 随后的另外两次 fresh-session 相同 repository-analysis 任务也都产生实质答案、
  返回输入框且 frame 无错误，但 journal 再次记录 `provider_unavailable` retry。
  三次均失败，不能用 monitor/frame 表面 PASS 覆盖 provider 证据；需要 provider
  稳定后重新取得三次连续零错误结果，才可关闭安装态门槛。

### 0.2 2026-07-31 定位与 always-on 承载复核

本节进一步取代把“存在模块”“默认启用”和“生产接线”混为一谈的旧表述：

| 项 | 当前代码事实 | 准确缺口 / 本轮处理 |
|---|---|---|
| 离线混合推理 | production runtime 构造并使用 `LlmScheduler`（`crates/executive/src/core/runtime_core.rs:153-177`），scheduler 会在 provider 失败后尝试下一候选（`crates/cognit/src/adapters/inference/scheduler.rs:270-353`）；但默认配置只有云 provider，本地 Ollama 示例被注释（`config/default.toml:35-57`），`IntentClassifier`/`InferenceRouter` 未进入安装运行时（`crates/cognit/README.md:23-25`） | 不能写成“turn 没有 failover”，也不能宣称默认 offline-first。README 改为 cloud-default、Ollama 可配置、llama.cpp planned；真正默认本地兜底仍是开放项 |
| Linux sandbox | Bubblewrap 提供真实 namespace 隔离；此前 capabilities 声称 seccomp，但没有生成/传入 `--seccomp` BPF FD。`Auto` 还会沿 Bubblewrap → Process → Noop 退化（`crates/corpus/src/security/sandbox/executor.rs:39-46`） | 本轮停止虚假 seccomp 声明，并将 shipped safe/dev 默认改为 `require`；`full` 仍由用户显式选择无沙箱。真实 seccomp filter 是后续独立 workstream |
| daemon 连接与 turn | accept loop 原先在 peer credential 后无条件 spawn connection（`crates/executive/src/host/daemon/server.rs:639-667`）；Turn backpressure 已存在，但默认无限（`crates/executive/src/composition/config/backpressure.rs`） | 本轮引入默认 64 个连接、8 个并发 turn 的 host-owned 上限，并以原子 admission 防止并发越界 |
| 顶层 turn 累计工作量 | Provider 单请求、tool result 和 child Agent 各有局部上限，但 shipped `agent.max_iterations=0` 允许顶层循环无限 | 本轮 shipped 默认改为 50 iterations；累计 provider tokens/成本/事件字节的统一硬结算仍是开放项 |
| Event spine | SQLite append 有 1 秒写 admission timeout（`crates/executive/src/adapters/events/sqlite_event_spine.rs:175-183`），但没有统一容量水位、retention/vacuum 调度 | 仍为 P0 开放项；不能用 append backpressure 代替磁盘生命周期治理 |
| OS 集成 | FUSE 是 design-only（`README.md:286-289`）；eBPF/io_uring 是 feature-gated/experimental（`README.md:274-282`） | 必须分别报告 `planned`、`experimental`、`installed`，不得统称“已生产接入”或简单统称“mock” |
| Mnemosyne migration | supplemental store 已有 `PRAGMA user_version` runner；FactStore 有幂等列迁移，但其他 backend 多为各自 `CREATE TABLE IF NOT EXISTS` | 缺口是主存 backend 的统一 schema/version/migration policy，不是“完全没有 migration runner” |
| CI | 默认-feature workspace suite 之外，PR 现在分别编译 io_uring、Linux integration 和 Mnemosyne all-features contract，并执行每 target 5 秒的 bounded fuzz；依赖系统 Leptonica/Tesseract 的 `ocr-tesseract` 不伪装成通用 runner 可编译 | OS feature contract 与 bounded fuzz 已进入 PR gate；原生 OCR 依赖必须由专用 runner/image gate，FUSE/eBPF 仍无可 gate 的生产 feature |

> **验收口径。** 评分内核基础部分已有 2026-07-30 的历史安装态生产验收；
> Goal retry/replan 与 AgentControl capability selection 的 L2 闭环已于
> 2026-07-31 实现并通过 focused tests，但仍须按实现计划重复安装态验收
>（`docs/plans/2026-07-30-coding-capability-evaluation-kernel-implementation.md:11-13,1108-1145`）。
> E′/A/B/C/D、F1/F2 以及本文列出的横向加固，在通过各自安装态验收前都不得
> 标称 `production-wired`。

## 1. 证据规则

- 正面事实使用当前分支的 `path:line` 锚点。
- “未找到实现”只表示在 2026-07-31 对当前工作树进行的仓库级符号/调用点检索
  未找到，不等价于未来版本的永久性断言。
- 行数属于易变快照，必须附生成命令和日期，不能当架构契约。
- 设计文档描述的是待实现目标；只有代码、持久化记录、安装态运行证据才能证明
  已接入生产。

## 2. 五份 workstream 的真实覆盖边界

这五块以纵向能力为主，但不是彼此完全独立，也并非完全没有触及横向能力。

| Workstream | 已锁定覆盖 | 仍在边界外或仅部分覆盖 |
|---|---|---|
| **E′** | 在唯一 spawn choke point 上收窄 tools、writable roots、protected paths 和全部预算维度（`docs/plans/2026-07-30-per-child-capability-attenuation-design.md:55-64`） | 明确不处理兄弟间聚合预留和 per-agent MCP registry（同文档 `:66-72`） |
| **A** | 真 embedding、持久向量索引、RRF、异步 backfill；并要求 remote embedding 获取 machine/provider-scoped permit（`docs/plans/2026-07-30-semantic-memory-embeddings-design.md:185-191`） | A 是 provider backpressure 的消费者，不应独占机器级协调器；大规模 backfill 尚未压测 |
| **B** | Planner → Explorer → Executor → Tester → Reviewer，以及有界 Fixer 修复环（`docs/plans/2026-07-30-multi-agent-planning-loop-design.md:6-12`） | Wave 1 只选择串行图（同文档 `:270-278`）；工作流级重启恢复尚未定义 |
| **C** | genome-only、候选感知 sandbox、人工 `DaseinModification` 审批和重启安全 genome store（`docs/plans/2026-07-30-metacognition-evolution-wiring-design.md:254-270,481-507`） | Apply 仍被 A/B evidence、sandbox 和 durable store 阻塞；验证语料的所有权/版本流程尚未指定 |
| **D** | 不只是 TUI：还包含 Fabric 协议、Corpus 执行约束、Executive 持久授权/RPC，并通过 capability bits 对旧 peer fail closed（`docs/plans/2026-07-30-tui-diff-multipane-approval-design.md:431-441,598-607,653-660`） | 多客户端协同、远程 TUI、无障碍和完整协议版本生命周期不在本稿范围 |

因此，本文讨论的是这些设计的**剩余横向缺口和交叉接线风险**，而不是声称五块
“完全没有触及”运营、安全或 backpressure。

## 3. 生产能力缺口（按承重程度排序）

| # | 能力 | 当前事实与准确缺口 | 严重度 |
|---|---|---|---|
| 1 | **F1：机器级 provider 并发、限流与冷却协调** | 已有跨连接 turn 数限制，但默认无限且粒度是 turn，不是 provider（`crates/executive/src/composition/config/backpressure.rs:9-29`）。`LlmScheduler` 有路由、failover、health 和逐请求 retry（`crates/cognit/src/adapters/inference/scheduler.rs:68-80,308-339`），没有共享的 provider-keyed permit/cooldown 状态。A 已正确提出一个消费者需求，但机器级所有权、所有 LLM/embedding caller 的接入、共享 `Retry-After`、公平性和可观测性仍未设计/验收。架构账本也明确列为未完成（`docs/design/architecture-overview.md:128-134`）。 | 🔴 极高 |
| 2 | **F2：审批平面清单与破坏性动作闭环** | 系统存在三条不同审批平面，不能合并描述：transient tool gate、durable `ApprovalCategory`、Metacog governance，详见 §4。`SendMail` 已有高风险 approval、执行前 binding 校验，以及幂等 outbox/reconciliation（`crates/executive/src/adapters/channel/gmail/report.rs:144-327,421-445`），不是“完全未接线”的类别；2026-07-31 的仓库级检索未找到 `DeleteFile`/`GitPush` 的生产 producer。缺口是逐类别证明 producer → human resolve → resume → terminal receipt/replay，而不是再造一个审批枚举。 | 🔴 高（安全） |
| 3 | **统一密钥/PII scrub 与最小化投影** | Memory consolidation 已对 API key/token/password/private key 做局部 redaction（`crates/mnemosyne/src/consolidation/extractor.rs:37-94`）；Agora trace 明确将内容视为敏感，除非另有脱敏投影（`crates/agora/src/trace/mod.rs:8-14`）。缺的是记忆、工具输出、事件、审计/API 投影共用的分类与 scrub 契约，以及防止原始敏感载荷进入非必要持久层的验证。 | 🟠 高 |
| 4 | **运营遥测、SLO 与卡死检测** | daemon 已有 `status`/`health` RPC（`crates/executive/src/host/daemon/handler/rpc.rs:46-49`），health 返回 liveness/readiness/components（`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:125-155`）；Turn 和 native child 也已有可选 deadline/budget timeout（`crates/executive/src/composition/turn_service.rs:121-138`、`crates/executive/src/adapters/runtime/native_cognit.rs:305-333`）。剩余缺口是经验证的 metrics 导出、队列/permit/cooldown 指标、默认 operator SLO/告警，以及区分“有 deadline”与“能发现并诊断卡死 Turn”的 watchdog 策略。 | 🟠 高 |
| 5 | **跨存储统一生命周期** | 不能再表述为“没有保留策略”：Mnemosyne 已有带 backup、lease、age 和 batch 的 retention compactor（`crates/mnemosyne/src/retention/compactor.rs:5-12,29-44,57-103`），MemoryService 执行 forget（`crates/mnemosyne/src/service.rs:1013-1037`），Corpus 也清理过期 overflow（`crates/corpus/src/tools/tools/output/persistence.rs:102-125`）。真实缺口是跨库 inventory、schema/version 规则、备份/恢复次序、容量水位、vacuum/compaction 调度和统一 retention 证明。 | 🟠 中高 |
| 6 | **工具输出与外部通道的注入/egress 治理** | Recall 已渲染为 `untrusted="true"` 并明确禁止把历史内容当指令（`crates/mnemosyne/src/projection.rs:234-250`）；child context 也标为 untrusted reference data（`crates/executive/src/adapters/runtime/native_cognit.rs:769-785`）。尚未证明所有工具结果、邮件/MCP/外部 channel 摄入都采用同等 typed trust 标记，也未形成跨工具 egress/data-classification 策略。 | 🟠 中高 |
| 7 | **provider/runtime 完整性** | 架构仍明确列出 Runtime selector 未统一、Pi RPC diff/artifact receipt 未完成、Hardware 无生产 caller，以及 machine-wide provider coordination 未完成（`docs/design/architecture-overview.md:123-134`）。需要按实际 provider/runtime 路由逐条验证，而不是用单个 mock 或 fallback 代表完整支持。 | 🟡 中 |
| 8 | **真实行为回归 fixture/harness/receipt 门禁** | 评分内核基础已生产验收，且存在通用 `CapabilityBenchmarkRuntime`（`crates/executive/src/application/capability_benchmark.rs:373-425`）；但架构仍将“真实 coding fixture/harness/receipt”列为未完成（`docs/design/architecture-overview.md:134`）。缺的是版本化真实仓库 fixture、相同 task packet/contract、权威终态 receipt、可复现失败分类以及进入 CI/发布门禁的策略。 | 🟡 中 |
| 9 | **可维护性与所有权集中度** | 2026-07-31 使用 `wc -l` 的快照为：`crates/corpus/src/security/runner.rs` 1998、`crates/executive/src/application/agent_control/mod.rs` 1779、`crates/executive/src/application/agent_control/settlement.rs` 1556、`crates/cognit/src/harness/linear/mod.rs` 2177、`crates/mnemosyne/src/service.rs` 1469。它们是变更冲突和评审负担信号，不单独证明质量差或 bus factor=1。`SECURITY.md:13-30` 已提供私密报告流程；人员风险必须由维护者/所有权数据单独评估，本文不再作“单人项目”断言。 | 🟡 中 |

> **已确认不是缺口：AgentControl 结算级 crash recovery。** settlement store 有
> `agent_settlement_receipts`、`idempotency_key`、`IdempotentReplay` 和
> `INSERT OR IGNORE`（`crates/executive/src/application/agent_control/settlement.rs:65,128-243`），
> Executive 也会调用 `recover_settlement_resources`
>（`crates/executive/src/application/agent_control/mod.rs:338-402`）。这不等于 B 的
> 整个角色图可从中途恢复，也不等于 C 的 genome rollback 已重启安全。

## 4. F2 必须区分的三条审批平面

| 审批平面 | 当前 authority / 状态 | 本轮准确边界 |
|---|---|---|
| **Transient tool approval** | `PolicyVerdict::RequireApproval` 进入 Corpus runner；当前只有静态 L2+ 才进入后续 gate（`crates/corpus/src/security/runner.rs:328-340`） | D 负责修复 typed decision、host-policy-at-every-level、per-tool/per-path session grant 和重启安全 enforcement；D 不改变 durable `ApprovalCategory`（`docs/plans/2026-07-30-tui-diff-multipane-approval-design.md:649-651`） |
| **Durable action approval** | `ApprovalCategory` 定义九类动作（`crates/fabric/src/types/approval.rs:31-43`）；`SendMail` 已有生产创建和消费校验 | F2 对每个类别建立 producer/resolver/resume/receipt/replay 矩阵。没有 producer 的类别保持不可达或 deny-by-default，不能因 enum 存在就宣称闭环 |
| **Metacog governance** | `DefaultMetacogService` 已校验有效 `DaseinModification` snapshot 与 subject binding（`crates/metacog/src/governance/service.rs:328-354`） | C 负责 pending proposal、human resolve、permit mint、durable apply/read-back/rollback；在 C Wave 3 验收前保持 `evolution_permitted=false` |

`cognit::Critic::check_risk` 只是按 action 名称发现 delete/rm/destroy 且要求
`rollback_action` 的计划检查（`crates/cognit/src/core/critic.rs:69-88`）；它既不签发
approval，也不执行或恢复动作，不能作为审批闭环原语。

## 5. 各文档内部仍需守住的边界

- **E′：** 兄弟间聚合预留和 per-agent MCP registry 仍是明确非目标；但 E′ 并非
  只改 `agent_control`。它还触及 Fabric tool contract、Corpus agent-control tool、
  `turn_pipeline.rs` 和 `native_cognit.rs`
  （`docs/plans/2026-07-30-per-child-capability-attenuation-design.md:278-297`）。
- **B：** Wave 1 串行执行是锁定取舍，不是缺陷误报；取消必须调用 child-level
  `AgentControlPort::cancel` 并观察权威终态，不能再写成“只在 wait 边界”
  （`docs/plans/2026-07-30-multi-agent-planning-loop-design.md:345-349,463-464`）。
  真正未定义的是角色图中途崩溃后的重放/恢复策略。
- **C：** 旧 sandbox 忽略 candidate、rollback snapshot 仅内存、migration 不足以
  证明 durable apply；修订稿已把 candidate-aware sandbox 与 restart-safe store
  纳入 Wave 3 必需项，而不是后续优化。
- **A：** remote embedding 已要求共享 backpressure contract；A 不负责证明所有
  LLM caller 都接入 F1。大规模 backfill 性能和容量边界仍需真实数据验收。
- **D：** 已覆盖 capability negotiation 和旧 peer fail-closed；尚未覆盖的是完整
  protocol version lifecycle，而不是“完全没有协议兼容设计”。

## 6. 跨文档文件所有权与排序风险

E′/A/B/C/D 不是五条可以无条件并行的独立支线。动手前必须按文件和责任切分：

| 共享面 | 相关 workstream | 协调要求 |
|---|---|---|
| `crates/fabric/src/types/tool.rs` | E′、D | E′ 只增加 host-only delegation authority；D 增加 diff/scoped-approval neutral DTO。先锁定字段和 serde compatibility，避免双方各自改同一构造器 |
| `crates/executive/src/application/turn_pipeline.rs` | E′、B、D | E′ mint authority，B 驱动 role graph，D 发布 approval scope/artifact。必须指定段落所有者并串行合入 |
| `crates/executive/src/adapters/runtime/native_cognit.rs` | E′、B | E′ 提供唯一 authority mint/forward 路径；B 只能消费，不能复制 attenuation |
| Executive approval/service/bootstrap | C、D、F2 | D 管 transient session grant；C 管 `DaseinModification` durable resolve；F2 维护类别矩阵，不能引入第三套 repository |
| provider/bootstrap/backpressure | A、B、F1 | F1 拥有 machine/provider coordinator；A 的 embedding 和 B 的 role children 都是消费者 |

**文件级顺序：**

1. E′ 的 authority contract 先于 B 的 child spawn 接线；但 E′ 与 D 在 Fabric/
   Turn composition 上仍需显式所有权切分，不能称为“完全无冲突”。
2. F1 在 B **生产启用**前必须可用。若只先合入 B 的 disabled/default-off plumbing，
   不得将其描述为 production-wired，也不得启用多 Agent 扇出。
3. D 与 C plumbing 可以在 Wave 1 落地，但审批文件必须串行修改；C apply 继续保持
   Wave 3 和 default-off。
4. C 的 versioned evaluation corpus 由评分/评测域维护 canonical fixture contract，
   Metacog 只消费版本化 corpus 与 digest，并在 `CandidateSandboxReceipt` 中记录版本；
   C 不得私建一套不可比较的 fixture 语义。

## 7. 新增 workstream 的准确定位

### F1 — Machine Provider Backpressure

- **所有者：** machine/provider core，而不是 Mnemosyne 或某个 session。
- **消费者：** 主 Agent、B role children、A embedding worker，以及其他 provider caller。
- **B 的关系：** B 的代码 plumbing 可 default-off 先落；B 的生产启用以 F1 安装态
  验收为硬门槛。
- **不归 E′：** E′ 只证明单个 child authority 是 parent subset，不协调兄弟请求或
  provider quota（`docs/plans/2026-07-30-per-child-capability-attenuation-design.md:66-72`）。

### F2 — Approval Closure Matrix

- **所有者：** 现有 Executive durable approval authority + Corpus transient gate；不
  新建审批系统。
- **已有生产路径：** `SendMail` 不作为“零实现”缺口，但仍进入 F2 验收矩阵，证明
  human resolve、幂等 outbox、ambiguous reconciliation 和终态记录一致。
- **待证类别：** 至少审计 `DeleteFile`、`GitPush` 以及其余 enum variant 的生产
  producer/consumer；未证明的类别保持 deny-by-default。
- **与 B 的关系：** B role profile 在 F2 未闭环前不得获得相应破坏性工具；这允许
  非破坏性、受 E′ 收窄的 B plumbing 先落，而不会把 F2 错写成所有 B 代码的前置。

## 8. 一致的 Wave 与启用门槛

```text
Wave 0  基础安全/承载
        E′ implementation + installed acceptance
        F1 design/implementation + machine/provider acceptance
        F2 category inventory + deny-by-default matrix

Wave 1  生产接线（共享文件按 §6 串行）
        B plumbing/default-off; production enablement requires E′ + F1
        D end-to-end diff/scoped transient approval
        C verify/parking/governance plumbing only; evolution_permitted=false
        F2 closes the destructive categories actually exposed in this wave

Wave 2  证据与召回
        A semantic memory/backfill
        evaluation-kernel Goal + AgentControl L2 closure
        real coding fixture/harness/receipt regression gate

Wave 3  受治理演化
        C apply only after A/B evidence + candidate-aware sandbox
        + durable genome store + DaseinModification approval closure
```

这与主稿中的标记保持一致：E′=Wave 0、B=Wave 1、A=Wave 2、C apply=Wave 3。
#3–#7（scrub、运营遥测、跨存储生命周期、外部内容治理、runtime 完整性）可以与
上述 workstream 并行，但在项目整体宣称“适合日常生产开发”前必须分别关闭；#9
属于持续性治理，不能用一次拆文件替代长期 owner/变更热点度量。

## 9. 整体生产就绪判定

以下条件同时满足前，只能报告单项 capability 的实现/测试状态，不能报告整个
Agent OS 已适合日常生产开发：

1. 评分内核 Goal/AgentControl L2 闭环重新执行安装态验收；
2. E′、F1 和 B 的实际 child fan-out 在官方 socket 上被观测，且 provider retries、
   inference rounds、tool calls 和 active context 分开计量；
3. 暴露的每个破坏性 capability 都有 F2 矩阵中的 producer、human resolve、resume、
   terminal receipt 和 restart/replay 证据；
4. A/B/C/D 各自的 default-off、降级和 fail-closed 语义在真实 TUI 中与持久化记录、
   daemon 日志一致；
5. scrub、health/readiness、metrics/SLO、存储恢复/retention、外部内容 trust/egress
   均有可运行的验收，而不只是设计文本；
6. 最终执行 `sudo bash scripts/aletheon.sh deploy`，证明
   `target/release/aletheon`、`/usr/bin/aletheon`、machine daemon 与 user daemon
   执行文件 SHA-256 相同，systemd restart counters 稳定，并通过 `/usr/bin/aletheon`
   + official user socket 完成真实 LLM 多轮开发请求。

任何 monitor PASS 与 rendered frame、Session/evaluation/approval receipt 或 daemon
日志不一致，都按失败处理。
