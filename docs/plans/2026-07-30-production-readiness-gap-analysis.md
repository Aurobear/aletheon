# 生产就绪缺口与路线图协调分析（E′/A/B/C/D 伴随文档）

**Date:** 2026-07-30
**Last reviewed:** 2026-08-01
**Status:** 实现、部署与部分安装态复核记录；最新系统部署、project-workspace、
subagent restart/recovery、reconnect/resume、连续三次真实 TUI，以及同一未重启
TUI 的三轮 scoped 开发请求均已有通过证据。但 Gmail 真实账号、人工审批矩阵、
跨会话 secret、child/embedding fan-out、加密 backup/restore drill 等 §9 门槛尚未
全部关闭，因此不得标称整体 production-ready
**Scope:** 区分已经实现、已设计但未实现、部分覆盖和完全未覆盖的生产能力；
给出启用门槛、依赖顺序和文件所有权约束。F1/F2 是本分析最初识别的新增
workstream；当前分支已实现其代码闭环，但仍按下述安装态证据独立判定。
本文件不定义具体接口或实现步骤。

## 0. 2026-07-31 实现后复核

以下结论与 §3 均已按当前代码重写；不再保留与实现相冲突的旧“当前事实”：

| 项 | 当前实现证据 | 当前验收状态 |
|---|---|---|
| F1 | machine core 持有 endpoint/model-keyed permit、主动 request-start pacing 与 cooldown；LLM canonical factory 直接消费，user-daemon embedding 经 core RPC 获取 socket-backed lease（`crates/cognit/src/adapters/inference/backpressure.rs:50-186`、`crates/cognit/src/composition/inference_factory.rs:117-185`、`crates/executive/src/host/core_rpc/server.rs:200-280`）；health 分开导出 core/user snapshot（`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:138-179`） | 跨进程代码与 focused test 闭环；双客户端 pacing、最新系统部署的 official-client smoke 与连续真实 TUI 质量门禁已通过，仍待 child/embedding fan-out |
| F2 | closure matrix 将已暴露操作绑定到 producer/resolver/receipt/replay，并将其余操作显式 deny（`config/approval-closure.toml:1-78`）；架构门禁执行 verifier（`tests/suites/architecture/architecture_check.sh` 末尾） | 代码闭环，待真实人工 resolve/restart 验收 |
| #3 | Fabric 提供统一 trust/classification/scrub 契约（`crates/fabric/src/types/data_governance.rs:1-153`）；Memory projection、最终 context assembly 与 Dasein injection 均在模型可见边界再次 scrub（`crates/mnemosyne/src/projection.rs:260-267`、`crates/executive/src/application/context_assembler.rs`、`crates/cognit/src/harness/session.rs`） | 代码、focused 泄漏 fixture 和一次系统部署已完成；仍待安装态跨会话 secret fixture 与真实外部通道验收 |
| #4 | health 导出 machine-core/user provider 与 Turn watchdog 指标和默认 SLO alerts（`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:138-187`）；monitor 只调用 process-global health，并对 readiness/SLO alert fail closed；现又从 durable TUI events 检查 tool error、权威 `text_snapshot`、终态文本结构和 `turn_done`（`tools/aletheon-monitor/src/tools/health.py:31-132`、`tools/aletheon-monitor/src/tools/diagnose.py:72-198`） | 代码、focused monitor tests 和最新系统部署完成；仍待可控故障注入矩阵 |
| #5 | backup 使用 SQLite online backup 并逐库检查（`scripts/libexec/aletheon/backup.sh:20-43`），restore 在复制前后校验（`scripts/libexec/aletheon/restore.sh:38-72`）；migration inventory 覆盖 12 个持久组件（`config/release/migration-matrix.toml`、`scripts/libexec/aletheon/verify/migration-matrix.sh:16-65`）。安装器现在创建 unit 声明的 cache root，并只在 Restic 与非空受保护凭证均存在时启用 timer（`scripts/libexec/aletheon/install-systemd.sh:20-24,91-102`） | staging consistency smoke 已通过；未配置主机的 backup timer 已正确 default-off，待真实加密 backup/restore drill |
| #6 | 非本地 MCP 对 restricted-data egress fail closed，并将返回内容标为 untrusted 后 scrub（`crates/corpus/src/tools/mcp/wrapper.rs:91-127`） | 主要外部工具路径闭环，待真实 channel/MCP 验收 |
| #7 | F1 关闭 machine provider 协调；Hardware 已由 production embodiment composition 调用（`crates/executive/src/host/daemon/bootstrap/request.rs:823-850`）；role runtime 先 prepare 后注册（`crates/executive/src/host/daemon/bootstrap/runtime.rs:322-364`） | 路由代码闭环，待逐 runtime 安装态矩阵 |
| #8 | 版本化 coding harness 已存在（`tests/coding/README.md:1-31`），CI 验证 replay/契约（`.github/workflows/ci.yml:81-85`），release gate 执行 acceptance 与 installed-host drill（`scripts/libexec/aletheon/release-acceptance.sh:296-339`） | 确定性门禁闭环，真实模型 workflow 待本次运行 |
| #9 | responsibility 位于 `config/architecture/module-boundaries.txt:21-25`；hotspot owner/line budget 位于 `config/architecture/hotspot-budgets.tsv:1-7` 并由 architecture fitness test 执行 | 持续治理项，不作一次性“完成”声明 |

### 0.0 2026-08-01 当前安装态覆盖结论

本节覆盖下方同日较早的失败历史，但不删除失败证据：

- 2026-08-01 最终 `sudo bash scripts/aletheon.sh deploy` 完整通过；release、
  `/usr/bin/aletheon`、运行中 `aletheon-core.service` 与 user `aletheon.service`
  可执行文件的 SHA-256 均为
  `29e06dcac8c8150817c08d6154818ed5c782055c2ca986d19e1e9cd5f56a02db`。
  两个 unit 在随后 8 秒窗口均保持 `NRestarts=0`、`active/running`，official
  user socket 的安装态真实 LLM boundary 请求也再次 PASS。
- 安装态 `project_workspace` 聚合场景在 commit
  `4c7200843ae16dfa3bd73c31ce1c6e7255cb83d1` 上 PASS：Git root/head/status
  前后相同，artifact delivery、repository analysis 与 authoritative outside-write
  denial 全部通过，证据汇总为 `/tmp/aletheon-project-workspace-pass.json`。最终部署
  只新增 default-off backup 安装修复，release binary 摘要未变化；部署后又单独重跑
  authoritative boundary，证据为 `/tmp/aletheon-post-deploy-real-llm.json`。
- workspace boundary 不再接受模型自述：monitor 要求 exact `file_write` 参数、非空
  transaction、权威 terminal error receipt、host 上文件确实不存在，并验证相同 receipt
  已呈现在 frame（`tools/aletheon-monitor/src/scenarios.py:162-231`）。
- subagent restart/recovery 场景 24 项全部 PASS（session
  `e2a1eed8-f6aa-4bee-9efc-aa9d37f8201a`，事件
  `.scenario-runs/4a0ac02bbb96432dbc801ca07ef53bb8/{initial-events,post-restart-events}.jsonl`）；
  reconnect/resume 场景 12 项全部 PASS（session
  `bf802faa-cf67-4ed8-a704-baa06246eb90`，汇总
  `/tmp/aletheon-reconnect-resume-fixed.json`）。这些证据关闭本轮 child lifecycle 与
  session resume/restart 的具体回归，不自动替代 §9 其他专项矩阵。
- 未配置 Restic 的当前主机现在呈现 `backup=disabled`：timer 为
  `disabled/inactive`、service 为 `inactive/dead` 且 `Result=success`，cache root 为
  `aletheon:aletheon 0700`；本地 staging consistency smoke 通过。真实 Restic、加密
  repository 与 restore drill 仍未配置，因此 #5 不能关闭。
- `ALETHEON_PRODUCTION_GMAIL_ACCOUNT` 当前为空，真实 Gmail 场景只能返回
  `BLOCKED:gmail_test_account_not_configured`。这是明确外部验收前置条件，不得用 fixture
  或其他已通过场景覆盖。
- 三个连续 fresh TUI repository-overview run 经当前安装 monitor 对 durable events
  重放均为 PASS：`0031a58282ed4069a91e1b6c31c4b010.jsonl`、
  `a058448f61594b2aae20a478866b393c.jsonl`、
  `ded20f9d695641dfa95d65c41078b8e9.jsonl`。每轮均以 `repo_inspect` 开始，
  无递归/wildcard inventory、无 tool/provider error、终态 snapshot 完整，并通过
  presence contradiction 与 unsupported staffing inference 检查。
- 同一未重启 TUI session 的三轮 scoped 请求也全部通过，权威事件文件为
  `bd304371fad24055b7b401a6336d294a.jsonl`；三轮分别记录 4/2/2 个 inference rounds
  和 3/1/1 个 tool calls，均有稳定 prompt、权威 `turn_done` 和完整 terminal snapshot。
- workspace integration 首次运行暴露 `execute_script_hook_inject` 一次非确定性失败；
  该 exact test 随即通过，随后完整 `corpus --lib` 561 项通过。该 flake 保留为验证
  风险，不能把首次 workspace run 表述为全绿。
- 上述证据关闭 §9.6 的最终 deploy/真实多轮请求部分和 repository-overview 的三次
  模型参数质量门禁；它不关闭 §9.1–§9.5 的各专项安装态矩阵，整体仍未
  production-ready。
- `project_workspace` 的最终效果门禁虽通过，但其中一轮 artifact delivery 曾先后
  触发 premature `change_accept` 与 `validation_run`，随后通过 `git_diff` →
  `change_accept` 自恢复。该 PASS 只证明最终 artifact/transaction 正确，不证明最优
  provider 使用量或零错误工作流；性能/效率评分必须继续把 inference rounds、retries
  与 tool errors 分开记录。

### 0.1 2026-07-31 安装态复核结果

- 2026-07-31 15:19 的 `sudo bash scripts/aletheon.sh deploy` 已通过；当次
  release、`/usr/bin`、`aletheon-core.service` 与 user `aletheon.service` 的运行中
  可执行文件统一摘要为
  `954b85a64e1bf6031886c3f6d51094c211ac670320ce5a6052fa307d4cd003ba`。
  deploy gate 同时证明 restart counter 在两个 7 秒窗口保持稳定，并通过 official
  user socket 的真实请求。当前实际 machine unit 是 active 的
  `aletheon-core.service`；`aletheon.service` 这个 system-scope 兼容 unit inactive
  不能据此否认 machine core。
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
- 本轮安装态复核（session `66e107c7-1e81-4f42-85c8-9b0a39614d8b`）再次在
  2026-07-31 14:16:14–14:17:00 记录 4 次 `provider_unavailable` retry，且 monitor
  在权威 `turn_done` 前超时。因此它是明确 FAIL，不纳入三次连续通过计数。
- 补入主动 pacing 后，两个并发、独立的
  `/usr/bin/aletheon --full` official-socket client 都在 18.721 秒内获得非空终态；
  machine-core snapshot 为 `admitted=4, paced=1, queued=1, rejected=0`，且对应
  journal 无 provider error。这是 F1 的安装态跨 session pacing 正证据。
- 随后的三个 fresh TUI repository-analysis run 都没有 provider retry/error，但按
  frame + durable event 重算只有第一个真正通过：run 2 的 provider 输出以未闭合
  Markdown/未完成中文句子结束；run 3 的 8 个 tool 中有 1 个
  `exec_command` 因缺少 change transaction 失败。原 monitor 将三者都报 PASS 是
  monitor defect；`tools/aletheon-monitor/src/tools/diagnose.py:72-174` 已改为
  fail closed，72 个 monitor tests 通过。三份 bundle 为
  `.scenario-runs/acceptance/real-tui-repo-review-{1,2,3}.json`，对应 private event
  receipts 为 `98ef...f37`、`ba08...23`、`9f83...bd1`。因此连续三次计数仍为 0。
- 15:33 再次运行 deploy 时，安装与服务重启已完成，但 official-client smoke 连续
  收到上游 HTTP 503（无 `Retry-After`），最终 status 124；machine snapshot
  记录 `cooldown_updates=5`，服务 restart counter 仍为 0。该次 deploy **整体失败**，
  不能被前一次成功覆盖，也不能归咎为 backpressure authority 缺失。
- 之后的修复部署已于 16:40 完整通过：release、`/usr/bin/aletheon` 与运行中的 user
  daemon 摘要均为
  `38959ba3921f69b3e171935ff307e56ba7c2144522efe94b80245111c1f4b0c9`，
  official user socket 真实请求、两段 restart 稳定窗口均通过；当前 user unit
  `NRestarts=0`。因此 15:33 的 503 是一次真实失败证据，但不再是“latest deploy”
  状态。
- 终态质量链路又关闭了两个真实缺陷。第一，bounded live event channel 丢失增量时，
  daemon 在 coordinator settlement 后发送完整 `TextSnapshot` 再发送 `TurnDone`
  （`crates/fabric/src/events/ui_event.rs:227-234`；
  `crates/executive/src/application/daemon_turn/execute.rs:274-308`），TUI 以 snapshot
  替换草稿（`crates/interact/src/tui/response.rs:103-112`）。第二，三个 provider
  adapter 不再对任意网络 chunk 调用 lossy UTF-8，而是在完整 SSE/NDJSON frame
  边界严格解码（`crates/cognit/src/adapters/inference/utf8_stream.rs:1-49`；
  `openai_provider.rs:572-597`、`anthropic.rs:394-420`、`ollama.rs:346-370`）。
  monitor 同时要求恰好一个 snapshot 位于权威 `turn_done` 前，并拒绝 U+FFFD
  （`tools/aletheon-monitor/src/tools/diagnose.py:124-197`）。
- `aletheon -m` 的默认会话冲突也已在代码中关闭：每个 one-shot 默认使用新的
  `message-<uuid>` session；只有显式设置 `ALETHEON_BENCHMARK_SESSION_ID` 才共享
  session（`crates/interact/src/tui/cli.rs:724-771`）。显式共享请求若只收到
  `{queued:true}`，现在返回错误而非空输出 exit 0（同文件 `:676-687`）。
- 最新部署后的真实 TUI run 11 在 provider、tool、终态结构、snapshot 与编码断言上
  全部通过（`.scenario-runs/acceptance/real-tui-repo-review-11.json`），但答案引用了
  本文修订前的陈旧 one-shot 缺口。依照“架构/成熟度结论必须符合实际代码”的口径，
  该次不计入最终连续三次 streak；须在本文纠偏后重新起算。
- 后续系统部署完整通过，release、`/usr/bin/aletheon` 与运行中 user daemon
  摘要一致，两段 restart 稳定窗口和 official user-socket 请求通过。具体当前摘要
  以本轮 typed runtime/deploy provenance 输出为准，本文不维护可漂移的“最新 SHA”。随后 run 36 的 provider/tool/snapshot/UTF-8
  与 repository-overview 顺序均通过，但答案仍把历史 secret 事件写成当前已证漏洞、引用
  旧部署摘要，并从计划推导 single-runtime 缺口；按事实准确性门禁仍判 FAIL。该结果证明
  monitor 机械 PASS 不能代替人工/评分内核的事实时态校验。

### 0.2 2026-07-31 定位与 always-on 承载复核

本节进一步取代把“存在模块”“默认启用”和“生产接线”混为一谈的旧表述：

| 项 | 当前代码事实 | 准确缺口 / 本轮处理 |
|---|---|---|
| 离线混合推理 | production runtime 构造并使用 `LlmScheduler`（`crates/executive/src/core/runtime_core.rs:153-177`），scheduler 会在 provider 失败后尝试下一候选（`crates/cognit/src/adapters/inference/scheduler.rs:270-353`）；但默认配置只有云 provider，本地 Ollama 示例被注释（`config/default.toml:35-57`），`IntentClassifier`/`InferenceRouter` 未进入安装运行时（`crates/cognit/README.md:23-25`） | 不能写成“turn 没有 failover”，也不能宣称默认 offline-first。README 改为 cloud-default、Ollama 可配置、llama.cpp planned；真正默认本地兜底仍是开放项 |
| Linux sandbox | Bubblewrap 提供真实 namespace 隔离；此前 capabilities 声称 seccomp，但没有生成/传入 `--seccomp` BPF FD。`Auto` 还会沿 Bubblewrap → Process → Noop 退化（`crates/corpus/src/security/sandbox/executor.rs:39-46`） | 本轮停止虚假 seccomp 声明，并将 shipped safe/dev 默认改为 `require`；`full` 仍由用户显式选择无沙箱。真实 seccomp filter 是后续独立 workstream |
| daemon 连接与 turn | accept loop 原先在 peer credential 后无条件 spawn connection（`crates/executive/src/host/daemon/server.rs:639-667`）；Turn backpressure 已存在，但默认无限（`crates/executive/src/composition/config/backpressure.rs`） | 本轮引入默认 64 个连接、8 个并发 turn 的 host-owned 上限，并以原子 admission 防止并发越界 |
| 顶层 turn 累计工作量 | Provider 单请求、tool result 和 child Agent 各有局部上限，但 shipped `agent.max_iterations=0` 允许顶层循环无限 | 本轮 shipped 默认改为 50 iterations；累计 provider tokens/成本/事件字节的统一硬结算仍是开放项 |
| Event spine | SQLite append 有 1 秒写 admission timeout，并在 production bootstrap 消费 shipped 1 GiB `max_event_spine_bytes` 硬上限（`crates/executive/src/adapters/events/sqlite_event_spine.rs:181-216`、`crates/executive/src/host/daemon/bootstrap/services.rs:66-69`、`config/default.toml:19`） | 无界增长已关闭；达到 hard cap 后 fail-closed，但 retention/vacuum 调度与容量预警仍开放，不能把拒写上限等同于完整磁盘生命周期治理 |
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
| 1 | **F1：机器级 provider 并发、主动节拍与冷却协调** | 双进程 authority 已统一到 machine core：endpoint/model key、socket-backed embedding lease、core health snapshot 均已接线（`crates/fabric/src/include/memory.rs:155-175`、`crates/executive/src/application/inference_port.rs:77-109`、`rpc_health.rs:138-179`）。当前又增加 machine-wide request-start pacing（`crates/cognit/src/adapters/inference/backpressure.rs:78-153`）及无建议 transient 的 30 秒 fallback。双 official-client installed pacing 与最新 deploy smoke 已通过；剩余缺口是 child/embedding fan-out及连续三次全工具成功、终态完整且事实准确的真实 TUI。 | 🔴 极高（验收） |
| 2 | **F2：审批平面清单与破坏性动作闭环** | 三条 authority 仍须区分（见 §4）；closure matrix 已将当前暴露的 `apply_code`、`activate_goal`、`send_mail`、`apply_genome` 绑定到 producer/resolver/receipt/replay，并显式 deny 其余动作（`config/approval-closure.toml:1-68`）。剩余缺口是真实 human resolve → resume → terminal receipt → restart/replay，不是再造审批枚举。 | 🔴 高（验收） |
| 3 | **统一密钥/PII scrub 与最小化投影** | 共享 typed scrub 位于 `crates/fabric/src/types/data_governance.rs:1-153`。consolidation、legacy recall projection、最终 host fragment assembly、Dasein durable injection 与工具/MCP 投影都消费它；本轮 focused fixtures 证明 raw `sk-`/`key-`/中文密钥标签不会从旧状态进入模型上下文。剩余缺口是重新部署后的跨会话与真实 channel/MCP 复核。 | 🟠 高（验收） |
| 4 | **运营遥测、SLO 与卡死检测** | health 已导出 machine-core/user provider snapshot、active Turn age 与默认 SLO alerts（`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:138-187`）；monitor 不再无 session 调用 session-scoped `status`，且 readiness/SLO alert 会使 verdict 失败（`tools/aletheon-monitor/src/tools/health.py:31-132`）。durable TUI event 校验现可拒绝隐藏 tool error、不完整增量、缺失权威 snapshot 与 UTF-8 replacement（`tools/aletheon-monitor/src/tools/diagnose.py:72-198`）。Turn watchdog 使用 active index 与 monotonic age（`crates/executive/src/application/turn_coordinator.rs:45-62,151-170`）。普通 one-shot 已隔离 session，显式共享 queued receipt 则 fail closed；剩余缺口是通用异步 queued receipt/terminal correlation 与安装态故障注入矩阵。 | 🟠 高（验收） |
| 5 | **跨存储统一生命周期** | migration inventory、SQLite online backup/restore 校验、Mnemosyne retention compactor 与 Event spine shipped hard cap 已形成代码闭环（`config/release/migration-matrix.toml`、`scripts/libexec/aletheon/backup.sh:20-43`、`scripts/libexec/aletheon/restore.sh:38-72`、`crates/mnemosyne/src/retention/compactor.rs:5-103`、`crates/executive/src/adapters/events/sqlite_event_spine.rs:204-216`）。剩余缺口是 Event spine retention/vacuum/预警与整套安装态 backup/restore drill。 | 🟠 中高 |
| 6 | **工具输出与外部通道的注入/egress 治理** | Recall 和 child context 已标 untrusted；非本地 MCP 对 restricted egress fail closed 并 scrub 返回内容（`crates/mnemosyne/src/projection.rs:234-267`、`crates/executive/src/adapters/runtime/native_cognit.rs:769-785`、`crates/corpus/src/tools/mcp/wrapper.rs:91-127`）。剩余缺口是逐 channel/MCP 安装态矩阵，而不是主 MCP 路径完全缺失。 | 🟠 中高（验收） |
| 7 | **provider/runtime 完整性** | selector 当前可在 manifested `native-cognit` 与 `pi-rpc` 间按 capability/priority 选择（`crates/executive/src/host/daemon/bootstrap/request.rs:996-1012,1098-1104`）；`pi-coder`、Goal provider worker 与 executable extension 仍经 compatibility/普通注册，未统一进 manifest selection（同文件 `:1026-1097`、`crates/executive/src/host/daemon/bootstrap/extensions.rs:300-323`）。Hardware 已有 production caller；resident Pi RPC 的 artifact list 仍为空（`crates/executive/src/adapters/runtime/pi_rpc.rs:420-434`）。缺口是统一真实外部路由、补 RPC artifact receipt，并做逐 runtime 安装态矩阵。 | 🟡 中 |
| 8 | **真实行为回归 fixture/harness/receipt 门禁** | 版本化 coding harness、deterministic replay/contract CI 和 release installed-host drill 已落地（`tests/coding/README.md:1-31`、`.github/workflows/ci.yml:81-85`、`scripts/libexec/aletheon/release-acceptance.sh:296-339`）。真实验收现同时要求 provider clean、全 tool success、权威 snapshot、终态文本完整且架构/成熟度结论符合实际代码。最新 deploy 已通过；run 11 的机械断言通过但引用陈旧计划事实，故 streak 仍未重新成立。 | 🟡 中（验收） |
| 9 | **可维护性与所有权集中度** | 2026-07-31 使用 `wc -l` 的当前快照为：`crates/corpus/src/security/runner.rs` 2038、`crates/executive/src/application/agent_control/mod.rs` 1843、`crates/executive/src/application/agent_control/settlement.rs` 1557、`crates/cognit/src/harness/linear/mod.rs` 2177、`crates/mnemosyne/src/service.rs` 1469。它们是变更冲突和评审负担信号，不单独证明质量差或 bus factor=1。`SECURITY.md:13-30` 已提供私密报告流程；人员风险必须由维护者/所有权数据单独评估，本文不再作“单人项目”断言。 | 🟡 中 |

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
- **C：** candidate-aware sandbox、durable genome store 与治理 apply 代码已落地但
  default-off；没有真实安装态 human approval/apply/read-back/rollback 证据前，不能因
  focused tests 通过就宣称受治理自演化已生产启用。
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
