# 能力补全评审稿（提交 Codex 审核）

**Date:** 2026-07-31
**Status:** 已按当前代码和 2026-07-31 安装态证据复核；session picker、安全边界、
machine-core provider 协调、权威终态 snapshot 与严格 UTF-8 stream framing 均已进入
系统安装二进制。最新 deploy 已通过，但真实 TUI 连续 streak 和其余整体门槛尚未关闭。
所有"当前事实"取当前工作树
`path:line` 快照；行号易变，不作架构契约。
**Branch when reviewed:** `auro/test/20260731-plan-acceptance-cleanup`（未提交、暂不创建 PR）
**目的:** 汇总"对比 Claude/Codex/Pi 后仍可补全的生产能力 + TUI 现状 + 最新分支/session
+ 遗留问题"，供 Codex 逐条审核与取舍。
**验收口径继承:** 本稿不推翻 `docs/plans/2026-07-30-production-readiness-gap-analysis.md`
的验收门槛；overall production sign-off 仍以该文为准，当前**未通过**（见 §5.1）。

> **当前状态覆盖表（后文历史记录不得覆盖本表）**
>
> | 项 | 2026-07-31 当前状态 |
> |---|---|
> | Secret scrub | 历史泄漏已定位；当前四层 scrub 代码已部署。当前开放项仅是跨会话与 channel/MCP 安装态验收，不能表述为“当前现网仍已证泄漏” |
> | Provider | machine-core backpressure 已部署，近期真实 TUI 无 provider error；开放项是 child/embedding fan-out 与连续三次事实准确的质量 streak |
> | Claude Code/Codex sibling runtime | §2.1 是扩展建议，不是已证明的当前 provider 单点故障；不得据此声称 scheduler 无 failover |
> | Production command sandbox | governed command 路径由 Corpus `SandboxExecutor` 持有，安装态 Bubblewrap 可用并在 required backend 缺失时 fail closed；Platform `LinuxSandboxHost::apply` 不是该生产路径 |
> | 最新安装摘要 | 由本轮 `system_status` 与 deploy provenance gate 动态给出；本文不维护可漂移的“当前 SHA” |

---

## 0. 评审决策

1. 安全泄漏、provider 稳定和专项验收先于新增 claude-code/codex runtime。
2. 新 runtime 初期各自维护协议适配器；协议稳定后再根据重复事实提取公共层。
3. `provider_unavailable` 仍使该次验收失败；也不能只因 F1 类型存在就反推跨进程
   authority 已闭环。诊断必须区分 provider 上游失败、provider backpressure timeout
   和 scheduler 路由耗尽。
4. TUI 快赢可独立于 Workstream D 落地，不等待整个协议/多面板 wave。
5. 会话管理直接实现交互式 overlay：`↑/↓` 选择、Enter 恢复、Esc 关闭。
6. 跨会话密钥投影与 F2 同为最高安全优先级，但保持不同 authority/模块边界；
   recall/projection 必须对历史记录再次 scrub，不能假设写入时已净化。

---

## 1. 架构前提（供审核者对齐）

两条"成为 agent"的执行路径，决定"对比 Claude/Codex/Pi"的准确含义：

| 路径 | 机制 | 现状证据 |
|---|---|---|
| **A. 自研 ReActLoop → 原始 LLM provider** | transport 分派 `{OpenAi, Anthropic, Ollama}`，实际请求经 machine core | `crates/cognit/src/composition/inference_factory.rs:59-123`；core RPC `crates/executive/src/host/core_rpc/client.rs:181-331` |
| **B. Agent runtime 委派** | `AgentRuntimeRegistry` + manifested capability selector；既含内建 `native-cognit`，也可含外部 runtime | `crates/executive/src/application/agent_control/execution.rs:386-464`；能力枚举 `crates/runtime/src/manifest.rs:31-44` |

- **Anthropic / OpenAI 当前只作为 A 的 provider transport**；仓库没有 Claude Code/Codex
  CLI runtime adapter。provider kind 为 `{OpenAi, Anthropic, Ollama}`。
- **B 的 manifested selectable runtime 至少有两个：** always-on 的 `native-cognit`
  （`bootstrap/request.rs:996-1012`）与配置/探测成功后注册的外部编码 runtime `pi-rpc`
  （`bootstrap/request.rs:1098-1104`）。因此 `Auto` 不是“只有 Pi 一个候选”；Pi 以更低
  priority 优先匹配其支持的能力，Native 仍是内建候选。
- **"Pi"** 非 Inflection 聊天 Pi，
  它内部再调 `ANTHROPIC/OPENAI/GEMINI` key：`crates/executive/src/adapters/runtime/pi.rs:33-52`。
  已于 2026-07-26 通过 `docs/testing/generic-subagent-runtime-acceptance.md`
  （三次真机 TUI PASS，全选中 `pi-rpc`，`override_used=false`）。
- 架构自陈未完成："Runtime selector 尚未统一所有真实外部执行路由"
  `docs/design/architecture-overview.md:127`。

---

## 2. 生产能力缺口（按 承重 × 顺架构度 排序）

### 2.1 【新增建议·高】把 Claude Code / Codex 也接成兄弟 `SubAgentRuntime`

- **当前事实:** manifested selector 已在 `native-cognit` / `pi-rpc` 间选择；但仓库没有
  `claude-code` / `codex` runtime。Pi 注册样板 `register_pi_runtime`
  （`crates/executive/src/adapters/runtime/pi.rs:54-67`），
  协议解析样板 `crates/executive/src/adapters/runtime/pi_protocol.rs`
  （LF-delimited JSON，严格拒绝未知事件）。config 挂点
  `crates/executive/src/composition/config/mod.rs:107`（`pi_runtime: CodingRuntimeConfig`）。
- **缺口:** 无 `claude-code` / `codex` runtime。manifested external runtime 的现成参考是
  `pi-rpc`；package executable runtime 虽能 probe/sandbox/rollback，却只做普通
  `register`，不会参与 manifested auto-selection
  （`crates/executive/src/host/daemon/bootstrap/extensions.rs:300-323`）。
- **建议方向:** 新增 `claude_protocol.rs` / `codex_protocol.rs`（对照 `pi_protocol.rs`），
  `register_claude_runtime` / `register_codex_runtime`，在当前真实 composition
  `host/daemon/bootstrap/request.rs` 注册，
  config 增 `claude_runtime` / `codex_runtime`。能力位必须由适配器声明与 probe/验收事实
  决定，不能预设某品牌“更强于”某类任务。
- **要动的文件:** `crates/executive/src/adapters/runtime/{claude_protocol,codex_protocol}.rs`(新)、
  `bootstrap/runtime.rs`、`composition/config/mod.rs`、`config/default.toml`。
- **风险/边界:** 不得让 runtime 自行授权（`architecture-overview.md:139` 纪律）；
  env key 注入需沿用 Pi 的白名单最小化（`pi.rs:33-52`），不得把 key 写进 AgentResult。

### 2.2 【F1·极高·安全/承载】机器级 provider 并发/限流/冷却协调

- **当前事实:** 复核双进程拓扑后，合并基线并未真正闭环：LLM backpressure state 位于
  machine core，而 embedding 与 health 曾读取 user daemon 的进程内 registry。当前工作树
  已把稳定 key 统一为 endpoint/model（`crates/fabric/src/include/memory.rs:157-163`），
  embedding 经 authenticated core RPC 获取 socket-backed permit 并上报 cooldown
  （`crates/executive/src/application/inference_port.rs:77-109`、
  `crates/executive/src/host/core_rpc/server.rs:200-280`），health 从 core 拉 snapshot
  （`crates/executive/src/host/daemon/handler/rpc/rpc_health.rs:138-179`）。
- **新增闭环:** coordinator 现同时持有 proactive request-start pacing
  （`crates/cognit/src/adapters/inference/backpressure.rs:78-153`）；checked-in Leju
  system/default 配置为 15 秒，其他 provider 的兼容默认仍为 0
  （`crates/cognit/src/config/mod.rs:631-652`、`config/default.toml:42-54`）。
  两个独立 official client 的安装态并发运行均返回非空终态，snapshot 为
  `paced=1, queued=1, rejected=0`。
- **剩余缺口:** 仍需安装态 child fan-out/embedding 并发证据及连续三次 provider clean、
  全 tool success、终态结构完整的真实 TUI 验收。历史 `provider_unavailable` 只能证明该次 provider 路径失败，不能证明
  F1 缺失；必须用 backpressure snapshot、scheduler 日志和 provider 错误分类定位。
- **依赖:** A(embedding)、B(role children) 生产启用都以 F1 安装态验收为硬门槛
  （`2026-07-30-production-readiness-gap-analysis.md` §7/§8）。

### 2.3 【F2·高·安全】破坏性动作审批闭环矩阵

- **当前事实:** F2 代码清单已闭环：`apply_code`、`activate_goal`、`send_mail`、
  `apply_genome` 绑定 producer/resolver/receipt/replay，其余破坏性操作显式 deny；
  全局默认也是 deny（`config/approval-closure.toml:1-68`）。
- **剩余缺口:** 在系统安装态完成真实 human resolve → resume → terminal receipt →
  restart/replay 验收；不能用静态 closure verifier 代替副作用闭环证据。

### 2.4 【高】离线/本地推理转正

- **当前事实:** Ollama transport 已在 factory 路径
  （`crates/cognit/src/adapters/inference/ollama.rs`，`inference_factory.rs:102-112`），但
  README 标 experimental，无 local-first 路由、无本地 intent-classifier/llama.cpp 分支。
- **缺口:** local-first 路由策略 + 安装态验收，兑现 README §设计目标"Offline-first"。
- **要动的文件:** `crates/cognit/src/adapters/inference/scheduler.rs`（local-first）。

### 2.5 【中高】语义/向量记忆

- **当前事实:** A 的代码已存在：SQLite vector backend
  （`crates/mnemosyne/src/backends/vector_sqlite.rs`）、durable embedding worker
  （`crates/mnemosyne/src/consolidation/embedding_worker.rs`）、RRF
  （`crates/mnemosyne/src/recall/pipeline.rs:418,480-501`）及 production bootstrap
  （`crates/executive/src/host/daemon/bootstrap/request.rs:668-745`）。
- **缺口:** shipped 配置仍 default-off；安装态需要验证真实 embedding、降级、重启恢复、
  stale-index/rotation 和 backfill 容量边界。

### 2.6 【中】Plugin runtime 能力分裂，部分类型仍是 stub

- **当前事实:** 旧 PluginManager 的 command subprocess 已可执行，但 native `.so` / WASM /
  agent entry 仍显式 `Err("... not yet implemented")`
  （`crates/executive/src/adapters/plugin/runtime.rs:20-94`）。另一条 package executable
  runtime 已有 bubblewrap/probe/activation/rollback 路径
  （`crates/executive/src/host/daemon/bootstrap/extensions.rs:19-160,183-324`），不能把整个
  plugin/extension runtime 统称为 stub。
- **缺口:** 两条扩展体系未统一；native/WASM/agent entry 尚未实现，package executable
  runtime 也未进入 manifested auto-selection。

---

## 3. TUI 现状与快赢补全

栈 `ratatui 0.29 + crossterm 0.28 + pulldown-cmark + syntect`；默认进 TUI
（`crates/aletheon/src/main.rs:505-533`），脏标记按需重绘（`crates/interact/src/tui/app/lifecycle.rs:88,104`）。
核心（流式/markdown/工具卡/审批 modal）已接近 Claude Code/Codex，`StreamController` 对标 Codex
（`crates/interact/src/tui/render/streaming.rs:2`）。

**"写了没接线"的死代码 + 浅实现（建议作为低风险快赢，独立于 Workstream D 先落 —— 见 §0 问题4）：**

| 项 | 证据 | 差距 |
|---|---|---|
| 权限/模式指示器未进常驻状态栏 | `render_widget_from_state`（`crates/interact/src/tui/status.rs:61`）、`format_status_line`（`crates/interact/src/tui/state.rs:173`）**从未被调用** | 用户看不到当前 mode，仅切换时瞬时 toast |
| Help overlay 未绑键 | `crates/interact/src/tui/help_overlay.rs` 仅 `pub mod` 声明（`tui/mod.rs:19`），无 `?`/Ctrl+H | 无可发现帮助 |
| 完整 header 死代码 | `crates/interact/src/tui/render/header.rs:10` 未用，实渲染仅 `"aletheon"`（`render/renderable.rs:113`） | |
| 会话 picker | 当前工作树已将 `/sessions` 权威 response 接入 overlay，支持导航与 typed resume（`session_picker.rs:23-143`、`response.rs:498-508`） | 上一安装二进制已通过交互复核；本地又补 tiny-terminal 边界，待随本轮重部署；尚无搜索/分页 |
| diff 视图基础 | `crates/interact/src/tui/diff_view.rs:30` 无 word-level intra-line、无语法高亮，`width<100` 隐藏（`render/draw.rs:73-83`） | 窄屏静默消失 |
| 鼠标仅滚轮 | `handle_mouse`（`crates/interact/src/tui/app/key_handler.rs:20-41`），`EnableMouseCapture` 抑制原生选区无替代 | 无法框选复制 |
| syntect 主题硬编码 | `base16-ocean.dark`（`crates/interact/src/tui/markdown.rs:37`） | 不随明暗终端 |

Workstream D 的主要代码已经落地：Diff artifact RPC、持久 scoped approval 与
`DiffView` 均存在；剩余的是专项安装态验收及表中仍未接线/较浅的交互项，不能再写成
“实现待落地”。

**结构性耦合（已关闭的终态缺陷）:** TUI 仍用 `TextDelta` 做实时草稿，但不再把 bounded
delta channel 当成权威终态。daemon 在 settlement 后发送完整 `TextSnapshot` 再发送
`TurnDone`（`crates/executive/src/application/daemon_turn/execute.rs:274-308`），TUI
以 snapshot 替换草稿（`crates/interact/src/tui/response.rs:103-112`）。因此中间 delta
丢失或非流式终态不再永久留下空白/畸形答案。

---

## 4. 最新分支与 Session（现状记录，非缺口）

### 4.1 已合并基线与当前验收分支

常驻 daemon 硬化已通过 PR #143 合并到 `dev`（merge commit `a5ae802`）。当前工作位于
`auro/test/20260731-plan-acceptance-cleanup`，暂不创建 PR：
1. 沙箱强制：`[sandbox] preference = "auto" → "require"`（`config/default.toml`）；`Require`
   落到 noop 硬报错（`crates/fabric/src/types/sandbox.rs:229-233`）。
2. 诚实 seccomp：`seccomp_filter: true → false`（`crates/corpus/src/security/sandbox/bubblewrap.rs:254-260`）
   —— bwrap `--seccomp` 需已打开的 BPF FD，当前未传，之前为虚标（修正假安全声明，非倒退）。
3. 有界准入：backpressure 默认有界 + 新增 `max_connections`；连接改原子
   `try_increment_connections()`（`crates/executive/src/host/daemon/handler/connection.rs:9-20`），
   超限 `warn!`+丢弃（`crates/executive/src/host/daemon/server.rs:661-667`）。

### 4.2 Session 两层（审核者勿混）

- **Canonical 持久:** SQLite `sessions-v1.db`（`crates/executive/src/adapters/session/canonical_store.rs:31`），
  schema v4，事务迁移 + fail-closed 拒绝更新/部分 schema（`:77-158`），append-only；prompt queue
  DB 层按 `(principal,thread)` 幂等（`adapters/session/prompt_queue_sqlite.rs`）。
- **进程内工作层:** `SessionManager` 无 journal 投影缓存，canonical replay 水合
  （`crates/executive/src/host/daemon/session_manager.rs:10-47`）；多会话 =
  `HashMap<String, Arc<Mutex<SessionManager>>>`（`bootstrap/sessions.rs:20-58`）。
- **连接↔会话解耦:** `SO_PEERCRED` 认证（`server.rs:645-651`），共享 registry，`principal_id`
  scoping；每连接独立 notify channel（`server.rs:657-659`）。
- ⚠️ **待修文档漂移:** `crates/executive/src/core/session.rs:4` "persist to JSONL" 错误，
  持久实为 SQLite。

---

## 5. 遗留问题与风险

### 5.1 整体生产就绪：仓库 ledger 判为未通过

以下 PASS/FAIL/FAIL 是**历史验收记录，不是当前 provider 健康状态或当前三次 streak**。
旧三次 fresh-session 因 daemon journal 的 `provider_unavailable` 全部失败。主动 pacing
部署后当时的新三次 run 已无 provider error，但重新核对 durable event 后是 PASS/FAIL/FAIL：
第二次输出结构不完整，第三次含一个失败 `exec_command`。monitor 曾把三次都判 PASS，
因此已补 event-level fail-closed 断言；随后又要求恰好一个权威 `TextSnapshot` 位于
`TurnDone` 前并拒绝 U+FFFD
（`tools/aletheon-monitor/src/tools/diagnose.py:72-198`，focused monitor tests passed）。
15:33 的一次 deploy 因上游 HTTP 503 失败；后续部署已恢复通过。后续部署已通过。当前安装摘要必须读取本轮 typed runtime/deploy 输出，本文不把某次摘要标成永久“最新”。
run 11 的 provider/tool/snapshot/encoding 断言通过，但引用了本稿修订前的陈旧 one-shot
缺口，按“结论须符合实际代码”的口径不计入最终 streak。完整证据见
`docs/plans/2026-07-30-production-readiness-gap-analysis.md` §0.1。
**结论: 现阶段不得标称 production-ready。**

### 5.2 运行时会硬报错/静默 no-op 的 stub

- plugin runtime 三类 `not yet implemented`（`crates/executive/src/adapters/plugin/runtime.rs:46-48`）。
- eBPF ring buffer "not yet implemented" 静默 no-op（`crates/dasein/src/impl/perception/sources/ebpf_source.rs:228`）。

### 5.3 feature-flag 掉的能力

`io_uring/ebpf/fuse/vector-*` 为 placeholder cfg（`Cargo.toml:66-69`）；自演化
`enabled=false, evolution_permitted=false`（`crates/executive/src/core/evolution_coordinator.rs:44-53`），
Apply 阻塞于 Wave-3。

### 5.4 panic / unsafe 点

- boot-time `.expect()`：`bootstrap/channels.rs:42,49`（Telegram）、`bootstrap/tools.rs:35`
  —— misconfig 即崩（热路径 `handler/rpc/*` 无生产 unwrap/panic，已确认）。
- unsafe 集中区：socket FD dup / `sockaddr_storage`（`crates/executive/src/host/daemon/server.rs:117-192`）、
  subagent `libc::kill`（`crates/corpus/src/tools/subagent/command.rs:237-368`）—— 最大 blast radius。

### 5.5 可维护性

超大文件：`crates/corpus/src/security/runner.rs`(2038)、`crates/cognit/src/harness/linear/mod.rs`(2177)、
`crates/executive/src/application/agent_control/mod.rs`(1843) —— 变更冲突/评审负担信号
（2026-07-31 `wc -l` 快照）。

---

## 6. 真机 TUI 运行时问题（2026-07-31 历史复现与当前状态）

本节保留历史复现证据，但历史复现不等于当前安装态仍存在同一缺陷。每项必须分别写明
“历史缺陷”“当前代码”和“尚待安装态验收”，不得把已修复代码重新列为当前漏洞。

### 6.1 会话 resume/picker

**当前代码行为:**

| 操作 | 走的路径 | 结果 |
|---|---|---|
| `/resume`（无 id） | `SessionLoadPrevious`（`crates/interact/src/tui/app/submit.rs:186-207`，打印"恢复最近的上一会话..."） | 只回到**紧邻上一个**，不能挑 |
| `/resume <id>` | `resume(id)`（`crates/executive/src/host/daemon/handler/rpc/rpc_session.rs:64`） | 能跳，但**须手打完整 id** |
| 空 id 到 daemon | `-32021 "Missing session_id parameter"`（`rpc_session.rs:60-62`） | 硬错误 |
| `/sessions` | `submit.rs:175-184` 记录 `OpenSessionPicker`；`response.rs:498-508` 解析权威列表 | 打开可导航 picker |

**当前边界:**
- `crates/interact/src/tui/app/key_handler.rs:44-62` 已优先路由 picker 的导航/恢复键。
- 后端原语中 `list_available`、`load_previous` 与 `resume(id)` 已接 UI；
  `load_recent` 仍未发现命令绑定。四个原语位于
  `rpc_session.rs:44,173,194,55`；协议
  `crates/fabric/src/protocol/client.rs:53-54`（`SessionLoadRecent`/`SessionLoadPrevious`）。

**当前修复（已随当前系统二进制部署）:** 已新增
`crates/interact/src/tui/session_picker.rs`，`/sessions` 的权威 response 进入 picker，
支持 `↑/↓`/`j/k`、Enter、Esc，并由现有 typed `resume(id)` 发起恢复；
`app/key_handler.rs`、`app/submit.rs`、`response.rs` 和 `render/draw.rs` 已完成接线。
2026-07-31 使用 `/usr/bin/aletheon` + official user socket 打开 `Sessions (100)`，Down
移动选中项后 Enter 成功恢复第二个 session 并返回输入框；monitor 的通用
`wait_stable` 因 overlay 隐藏 prompt 超时，但连续三帧相同且 daemon 的 typed
`Session resumed` 与最终 frame 一致，因此 picker 本身通过；通用 overlay 稳定条件
仍可独立改进。

### 6.2 真机三类报错

**(a) `Searched **/* ✗`「too broad」红叉** —— `crates/corpus/src/tools/tools/glob.rs` 的
**故意护栏**。当前本地修复保持 `is_error=true`（调用确实未执行，不能伪装成功），同时将
tool description/schema 改为明确禁止 `**`/`**/*`，返回 `Policy guidance:`；TUI 对该类失败
显示黄色 `⚠`，普通执行错误仍显示红色 `✗`。待真实 TUI 验收模型不再开局发起全仓扫描。

**(b) `provider_unavailable` 重试** —— 是 §5.1 判“整体验收未通过”的硬门槛。早期
machine-core journal 为 HTTP 429 且无 `Retry-After`；现在已增加 15 秒主动 pacing 和
30 秒 bounded shared fallback cooldown
（`crates/cognit/src/adapters/inference/backpressure.rs:78-168`、
`crates/fabric/src/include/memory.rs:165-168`）。双客户端 pacing 证据通过后，新 TUI
三次未再产生 provider error；15:33 deploy 随后遭遇上游 HTTP 503，而 16:40 的最新
deploy 已恢复通过。503 与 429、跨进程 authority、tool/output 质量必须分别分类，不能
用“F1 模块存在”直接判完成。

**(c) ⚠️ 跨会话密钥明文泄漏（历史安全缺陷，当前代码已修）** —— 旧部署态 `conscious-context`/`dasein-state` 曾把**别的
会话存的 secret 明文带出**（如 `sk-...`、`API 密钥 key-...`）。合并基线的 redaction
当时只在 memory consolidation 局部做
（`crates/mnemosyne/src/consolidation/extractor.rs:37-94`），recall projection 和另一条
direct context/Dasein injection 边界都可能重放旧状态。
当前本地修复扩展统一 scrub policy 对裸
`sk-...`/`key-...`/中文密钥标签的识别，并在
`crates/mnemosyne/src/projection.rs` 对历史 record 再次 scrub 后才进入模型可见 workspace；
同时在 `crates/executive/src/application/context_assembler.rs` 的最终 host-fragment 边界及
`crates/cognit/src/harness/session.rs` 的 durable Dasein injection 边界再次 scrub。focused 泄漏 fixtures 已通过并已进入系统二进制；当前准确风险是“跨会话 secret fixture
与真实 channel/MCP 的安装态验收尚未关闭”，而不是“当前代码仍已证泄漏”。

### 6.3 one-shot session 与异步 queued receipt

**原缺陷:** 两个默认 `aletheon -m` 曾共享 session；第二个请求收到
`{"queued":true,"prompt_id":...}` 后会无输出、exit 0，违反“异步 receipt 不等于
terminal success”。

**当前代码:** 每个 one-shot 默认创建新的 `message-<uuid>` session；只有显式设置
`ALETHEON_BENCHMARK_SESSION_ID` 才复用 session
（`crates/interact/src/tui/cli.rs:724-771`）。若显式共享的 one-shot 只收到 queued
receipt，CLI 现在返回错误而不是把它当成成功终态（同文件 `:676-687`）。focused
request-contract test 已通过并随最新 digest 部署。普通并发 one-shot 的 installed
复核仍应证明两个客户端都得到非空终态；通用 prompt queue 的终态订阅/重连语义仍是
独立协议能力，不能由“默认分配不同 session”代替。

---

## 7. 已裁定落地顺序

1. **密钥 scrub 安装态复核**（§6.2c）+ **F2 破坏性动作**（§2.3）—— 安全类，最高优先；
   secret 明文跨会话是历史复现缺陷，当前代码修复已部署，但真实跨会话/channel/MCP
   验收尚未关闭。
2. **provider 稳定与 F1 安装态验收**（§2.2 / §6.2b）—— 当前工作树已补
   machine-core 跨进程 authority，不再另建协调器；下一步取得零错误及并发/冷却分类证据。
3. **claude-code/codex 兄弟 runtime**（§2.1）—— 最顺架构、复用已验收模式，破除委派单点。
4. **TUI 快赢**（§3 + §6.1）—— 会话 picker（↑↓ 选择/Enter 恢复）、接线 mode 指示器、绑 `?`
   help、diff 语法高亮不隐藏、`**/*` 护栏降级。
5. 离线推理转正（§2.4）/ 语义记忆（§2.5）/ plugin runtime 分期（§2.6）。

> 本顺序以当前代码事实为准；实现存在不等于专项安装态验收已经通过。
