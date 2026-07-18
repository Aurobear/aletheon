# Channel 子系统解耦重构 + 应用层定位

> **Status:** Done（核心+归位）— Phase 0–4② + 部分 Phase 5 完成并提交（分支 `auro/refactor/channel-gateway`）；config 泛化标为可选后续
> **Author:** 架构调研 2026-07-18

## 实施进度（2026-07-18）

| Phase | Commit | 内容 | 状态 |
|---|---|---|---|
| 0 | `ef7d010e` | 抽 `intent.rs`（`Intent`/`classify_intent`）+ `notify.rs`，纯逻辑零行为变化 | ✅ |
| 1 | `f4b9bf5a` | `CapabilityRegistry`/`CapabilityHandler`/`OutboundEffect`/`ApprovalResolver` 取代 god-object；`ChannelRouter`→`ChannelDispatcher`；5 个 `Option<Arc<dyn>>` 字段清零；`ActivateGoal` fork→resolver 注册表 | ✅ 41/41 |
| 2 | `c35c1d95` | 中立层 google-free（dispatcher/telegram 仅剩文档注释）；`handlers/google_read.rs` + `ChatPreprocessor` 钩子；删 `enqueue_google_notification` | ✅ |
| 3 | `00ab6e1a` | Gmail 成为注册的 `EventCapabilityHandler`（`IntentKind::GmailIngest` + `EventCapabilityRegistry`）；Gmail 领域内部 7 文件零改动，安全/幂等/reconciliation 全保 | ✅ |
| 5(部分) | `9eb30b16` | 删除 Phase 3 后变死的 `GoogleMailIngressSink` | ✅ |
| 4① | `82898d64` | `ChannelApprovalPort`（fabric 原生词汇）+ `ApprovalRepositoryPort` 实现；dispatcher/approval handler 脱离具体 `ApprovalRepository`；**核心审批契约零改动** | ✅ |
| 4② | `8131b9de` | **建 `crates/gateway`**，中立引擎物理搬出 executive；方向 `executive→gateway→fabric` 单向无环 | ✅ gateway 38 + executive 9 套件绿 |

**两大诉求均已兑现**：
1. **「耦合太严重」已解决**——中立 dispatcher/transport 无任何 provider 领域分支；god-object → 注册表；Gmail 与 Telegram 统一在「能力皆注册 handler」模型（Gmail 走非双工事件路径，语义未变）。
2. **「应用层归位」已完成**——渠道引擎物理落到 `crates/gateway`（只依赖 `fabric`），`interact` 保留纯人机 UI。

### crate 归属最终形态（实际采用）
`executive → gateway → fabric` 单向依赖：
- **`crates/gateway`**（新，只依赖 fabric）：`dispatcher/effect/intent/notify/ports/registry/store` + `telegram/` transport + `handlers/{chat,goal,greeting,approval,google_read}` + transport/executor 端口 trait。
- **`executive`**（依赖 gateway，实现其 trait）：`daemon_adapter`（`DaemonChannel*Executor`/`ApprovalRepositoryPort`）、`gmail/` 子树、`handlers/gmail_ingest`（`GmailIngestHandler`）、bootstrap 组合。7 处原 `r#impl::channel` 引用改为 `gateway::` 导入（同向，无需反转）。
- `GoalProgress::from_outcome` 泄漏解法：转换移到 `executive::impl::goal::goal_progress_from_outcome`，中立 `GoalProgress` 结构留 gateway。

> 说明：曾担心的「7 处依赖反转 + 组合根上移 bin」在 `executive→gateway` 方向下**不需要**（只有 `gateway→executive` 边缘方向才需要）。当前方向使 gateway 成为被 executive 依赖的渠道引擎 crate；若日后要让 gateway 成为 executive 之上的纯边缘层，再做那次反转即可，非必需。

### 剩余（可选后续，非解耦必需）
- **Phase 4 config 泛化**（`ChannelsConfig` 包 `TelegramConfig`）：**推测性多渠道扩展**，当前仅一个双工 transport，无第二 entry 可放；且触碰在跑任务正改的 `cognit/config/mod.rs`。为零当前收益不做，待真正加第二渠道时再上。
- **Phase 5 剩余**：`docs/arch/agent-google/03`、`deepseek/07` 架构描述更新（本次已更新本 fix 文档）。
- `channel/exec_server_client.rs` **保留**（非死代码——`host/mod.rs:154,167` 调用 `ExecServerClient::spawn`；原「删死桩」作废）。

---

> **Scope:** `crates/executive/src/impl/channel/` 的分层重构 + channel/edge 代码的 crate 归属
> **决策前提（已与需求方确认）:** (a) Gmail 一并统一到新模型；(b) 清爽重命名并同步改测试，不留旧名兼容层；(c) 应用层落到**新建 `crates/gateway`**（变体 B），`interact` 保留纯人机 UI（TUI/CLI/ACP）。

---

## 1. 问题诊断（为什么改）

当前 `crates/executive/src/impl/channel/` 把三个正交关注点熔在一起，且整块「应用层集成代码」错放进了核心运行时 crate。

### 1.1 分层反转：中立 router 反向依赖具体 provider
`ChannelRouter::process()` 直接调 `super::telegram::is_google_read_query(...)`——
- `router.rs:660-676`（live path）
- `router.rs:825-841`（`recover_pending_inbox`，同一段逻辑**重复第二遍**）

一个号称 provider 无关的路由器，编译期硬依赖 `telegram` 子模块，依赖箭头指反了。

### 1.2 传输层被灌满应用/领域语义
`telegram/mod.rs:27-58` 装着 Google 意图检测 `is_google_read_query`、账户选择 prompt `account_choice_prompt`、`<trusted-google-account>` 提示词工程——本该只会 `getUpdates`/`sendMessage`。

### 1.3 God-object router
`ChannelRouter`（`router.rs:257`）挂 5 个 `Option<Arc<dyn>>` executor（turn/goal/approval/gmail_draft/google_accounts）+ 5 个 `with_*` builder；`process()` ~300 行内联 approval/google/goal/outbox 全部分支；`fabric::GoogleEvent` 在 `enqueue_google_notification`（`router.rs:292-321`）里被直接 match（与 `google/event_dispatcher.rs:337` 的 `bounded_notification_text` **重复**）。
后果：加一个 provider 要改 god-object + `init_telegram_channel`（10 参数，`bootstrap/channels.rs:26`）+ `request.rs`（17 处 telegram 触点）。

### 1.4 Gmail 是另一条并行管线
Gmail 不是 `ChannelTransport`，而是绕过 `ChannelStore`/turn-executor 的 event-ingress→classifier→draft-goal 管线（入口 `gmail/event_ingress.rs:136`），只在 **ApprovalRepository / ObjectiveStore / Telegram outbox** 三点与 Telegram 收敛。两套 DTO、两套 store、两套幂等。

### 1.5 位置错层（本文新增的核心判断）
以上全部代码住在 `executive`——**核心 agent 运行时/daemon crate**。而 Telegram/Gmail 属于「外部世界如何触达 agent」的**边缘/应用层**。核心 crate 里塞满 provider 集成代码，是 1.1–1.4 之外的、更宏观的一层耦合。详见 §4。

---

## 2. 复用的既有范式（不要另造轮子）

- **`LifecycleRegistry` / `LifecycleContributor`**（`crates/executive/src/service/lifecycle_contributors.rs:68,106`）：typed contributor 按 phase `register()`（:112），`contribute()` 返回**有界 `LifecycleEffect`**（:44），dispatcher 零领域知识。→ Capability 注册表照抄此形状。
- **`interact/src/acp/` 模块拆分**（`transport.rs` 纯 IO / `event_map.rs` 纯翻译 / `gateway.rs` backend trait + dispatch loop / `mod.rs` 协议类型 + 关联，声明「Executive 仍是权威」）。→ channel 模块按同样风格切文件；同时 **acp 已经证明 edge 适配器就该住 `interact`**（见 §4）。
- **保留的真中立资产（行为不改，仅可能改名）**：`ChannelStore` 全部 SQLite 幂等契约（`store.rs`，schema user_version=1）、`InboundMessage/OutboundMessage/MessageContent/UserAction`（`fabric/types/channel.rs`）、`route_content` 纯分类逻辑、turn-before-send 原子性（`complete_inbound` `store.rs:214`）、at-least-once（`flush_pending_outbox`）。

---

## 3. 目标逻辑架构：Transport / Intent / Capability 三层

```
Layer 1  Transport (port)      纯 provider duplex I/O，零 domain
  MessageTransport trait       transport_id / receive(cursor) / send(&Outbound)
  transport/telegram.rs        纯 HTTP getUpdates/sendMessage（去掉所有 google 逻辑）
  transport/gmail.rs           Gmail 作为 InboundSource(mail→Inbound) + ReportSink

Layer 2  Intent (pure)         provider 无关
  intent.rs  classify_intent   InboundMessage → Intent
             { Greeting, Chat, GoalCommand, ApprovalCallback,
               GoogleReadQuery, GmailIngest, Unsupported }

Layer 3  Capability registry   仿 LifecycleRegistry
  registry.rs CapabilityRegistry / CapabilityHandler
              handle(ctx, inbound, intent) -> Vec<OutboundEffect>
  handlers/chat.rs      turn executor
  handlers/goal.rs      goal 命令
  handlers/approval.rs  approval 回调 + 按 ApprovalCategory 的 resolver 子注册表
  handlers/greeting.rs
  handlers/google_read.rs   is_google_read_query + account picker（仅 google 存在时注册）
  handlers/gmail_ingest.rs  收编 classifier/sender_policy/ingest/goal_draft/report

Dispatcher (thin, 中立)   dispatcher.rs
  insert(dedup) → resolve_principal → classify_intent → registry.dispatch
  → persist outbound(atomic) → send      —— 无任何 Google/Gmail 分支
```

**命名映射（清爽重命名）**：`ChannelRouter`→`ChannelDispatcher`；`ChannelTransport`→`MessageTransport`；`ChannelTurnExecutor` 语义并入 `ChatHandler`；`GmailDraftApprovalExecutor` + `ActivateGoal` fork → `ApprovalResolver`（按 `ApprovalCategory` 注册，Gmail 注册 `ActivateGoal` resolver）；`GoogleChannelAccountDirectory` 迁入 `handlers/google_read.rs`。

---

## 4. 应用层该放哪（crate 归属 / 六边形架构）

### 4.1 依赖事实（已核实，无环）
- `interact` **只依赖 `fabric`**（`crates/interact/Cargo.toml:10`）——它就是既有的「用户/外部交互」edge crate：TUI、CLI、ACIX、ACP 适配器（`crates/interact/src/{tui,acix,acp}`）。
- `executive` **不依赖 `interact`** → channel 代码移出 executive 不会产生循环依赖。
- `bin/aletheon` 同时依赖 `executive` 与 `interact`（`crates/bin/Cargo.toml:17-18`）——是**组合根 composition root**。
- DTO 已经在正确的地方：`fabric::types::channel`（`InboundMessage/OutboundMessage/...`）。

结论：**channel 子系统当前错放在 `executive`。** ACP 已经证明「edge 适配器住 `interact`、经 trait/socket 与核心解耦、由 `bin` 组装」是本仓库既定模式。channel 应遵循同一模式。

### 4.2 目标 crate 布局（ports-and-adapters）

| 层 | 内容 | Crate | 依赖 |
|---|---|---|---|
| **Ports / DTO（契约）** | `InboundMessage/OutboundMessage`（已有）+ 新增端口 trait `MessageTransport` / `CapabilityHandler` / `ApprovalResolver` | `fabric` | — |
| **Adapters（应用/边缘层）** | Telegram/Gmail transport + `ChannelDispatcher` + `CapabilityRegistry` + `ChannelStore`（中立引擎） | **新建 `crates/gateway`（已选定，变体 B）** | `fabric` only |
| **Capability 实现（核心）** | `ChatHandler`（经 `DaemonTurnOrchestrator`）、`GoalHandler`、`ApprovalResolver` 实现——需要核心内部 | `executive` | `fabric` |
| **Composition root** | 构造 transport(adapters) + capability 实现(core)，注册进 registry，spawn 循环 | `bin`（或 executive daemon bootstrap 的一个瘦 spawner） | both |

关键接缝**已经存在**：`ChannelTurnExecutor` trait（现 `router.rs:62`，由 `daemon_adapter.rs` 实现）正是「dispatcher 在应用层、turn 实现在核心」的分界。重构只是把这个 trait 上提到 `fabric`，dispatcher/transport 下沉到 `interact`。

### 4.3 已选定：变体 B（新建 `crates/gateway`）

**决定**：新建 `crates/gateway`，专司 bot/机器渠道（Telegram/Gmail/未来 WhatsApp/Slack）；`interact` 保留纯人机 UI（TUI/CLI/ACP）。人机 UI 与机器渠道彻底分家，语义最清晰。

新 crate 设定：
- `crates/gateway/Cargo.toml`：`name = "gateway"`（或 `aletheon-gateway`）；依赖仅 `fabric`（+ `tokio`/`reqwest`/`rusqlite`/`async-trait` 等实现依赖），**不依赖 `executive`**。
- 加入 workspace `members`（`Cargo.toml`）。
- `bin/aletheon` 增加 `gateway = { path = "../gateway" }` 依赖，在组合根装配。
- 内部模块布局即 §3 的 `transport/ intent.rs registry.rs dispatcher.rs handlers/ effect.rs notify.rs store.rs loop.rs`。

（曾评估的变体 A「channel 进 interact」已放弃：会把 bot 渠道与人机 UI 混在同一 crate。）

---

## 5. 分阶段迁移（strangler-fig，每阶段可编译 + 测试绿）

> 前 3 阶段先在 `executive` 内部完成逻辑解耦（§3），Phase 4/6 再做 crate 搬迁（§4）。这样「模式纠正」与「物理搬家」解耦，任一步都可独立评审。

### Phase 0 — 骨架 + 纯逻辑抽取（零行为变化）
- 建新模块：`transport/`、`intent.rs`、`registry.rs`、`dispatcher.rs`、`handlers/`、`effect.rs`、`notify.rs`、`loop.rs`。
- `store.rs` 原样迁入；`route_content`→`intent.rs::classify_intent`（保留全部 `RoutedInput` 语义）；`render_approval_notification` / GoalProgress 文本渲染→`notify.rs`。
- 测试：`route_content` 7 个模块内测试（`router.rs:1081-1153`）迁 `intent.rs`；`store.rs` 6 个测试原样。

### Phase 1 — Capability 注册表 + 拆 god-object
- 新增 `CapabilityRegistry`/`CapabilityHandler`/`OutboundEffect`（仿 `lifecycle_contributors.rs:44,68,106`；`OutboundEffect` 有界枚举 `Reply/Enqueue/None`）。
- `ChannelDispatcher::dispatch` 重写为「classify → registry.dispatch → 收 effect → 原子 persist → send」，**逐字保留** `process()` 的 11 步副作用（dedup skip / 未知发件人 reject 且推进 cursor / chat 失败 `fail_inbound` 不推进 cursor 并返回 Err / goal 命令吞错 / greeting 文案 / build_outbound 各分支文案）。
- Chat/Goal/Greeting/Approval 入 `handlers/`；`ActivateGoal` 特判→`ApprovalResolver` 注册表。删 `ChannelRouter` 5 个 `Option<Arc<dyn>>` 字段与 builder。
- 测试：`channel_router.rs`(6)→`channel_dispatcher.rs`；`telegram_goal_commands.rs`(2)、`telegram_restart_recovery.rs`(5) 改引用，断言不变。

### Phase 2 — 净化 transport + 移出 Google
- `is_google_read_query`/account prompts（`telegram/mod.rs:27-58`）→ `handlers/google_read.rs::GoogleReadHandler`，仅 `google` 存在时注册；`GoogleChannelAccountDirectory` 迁入。`transport/telegram.rs` 瘦回纯 HTTP。
- dispatcher 删 `enqueue_google_notification` + `fabric::GoogleEvent` 依赖；Google 通知文本统一由 `google/event_dispatcher.rs::bounded_notification_text`（:337）产出，走 `ChannelStore::enqueue_outbound`（`DurableGoogleNotificationSink` `event_dispatcher.rs:197` 已是此形态，保留它、删经 router 的重复路径）。
- 测试：`google_telegram_query.rs`(6) 改为针对 `GoogleReadHandler` 的 seam，断言逐条保留。

### Phase 3 — Gmail 统一到新模型（体量最大、安全面最广，建议独立 PR）
- Gmail 建模为 `transport/gmail.rs`：`InboundSource` 从 `GoogleEvent::MailReceived` 产出 `InboundMessage{ intent=GmailIngest }`（承接 `gmail/event_ingress.rs:136`），report 发送建模为 `ReportSink`/`OutboundEffect`。
- `GmailIngestHandler` 包住 `classifier.rs`/`sender_policy.rs`/`ingest.rs`/`goal_draft.rs`/`report.rs`（领域逻辑不动，仅改接入方式）。Gmail 自有 dedup `(account_id,message_id)` 保留为 handler 内部状态。
- `GmailGoalDraftCoordinator` 的收敛改由 `ApprovalResolver(ActivateGoal)` 驱动。
- **必须保留**：独立幂等/重放/reconciliation（`report.rs:197-327`）、sender deny-by-default（SPF/DKIM/authserv，`sender_policy.rs`）。

### Phase 4 — 配置与 bootstrap 泛化 + crate 搬迁（§4）
- 新增中立 `ChannelsConfig`（provider 列表），`TelegramConfig`（`cognit/src/config/mod.rs:1023`）纳入其下；重生成 `config/schema/aletheon-config.schema.json`（schemars），serde 向后兼容。
- `init_telegram_channel`(10 参数)→ `ChannelSubsystem::build(deps, config)`；`telegram_poll_loop`→泛型 `channel_poll_loop`（`loop.rs`），每 transport 一个。
- **crate 搬迁（变体 B）**：新建 `crates/gateway`（§4.3）；`MessageTransport`/`CapabilityHandler`/`ApprovalResolver` trait + DTO 上提到 `fabric`；transport+dispatcher+registry+store 下沉到 `crates/gateway`；capability 实现留 `executive`（实现 `fabric` 的端口 trait）；组装点移到 `bin`（构造 gateway 的 transport + executive 的 capability 实现，注册后 spawn `channel_poll_loop`）。
- `telegram_task` 健康句柄（`request.rs:1319/1351` → `request_use_cases.rs:138/225-231`）泛化为 per-channel handle map，保留 `"telegram"` 健康键兼容。

### Phase 5 — 收尾
- 删 `exec_server_client.rs` 死桩（全 `todo!()`，:12-54）或明确注释保留。更新 `docs/arch/agent-google/03_CHANNEL_AND_MOBILE_COMMUNICATION.md`、`docs/plans/deepseek/07-external-integration-maturity.md`。
- 全量 `cargo test --workspace` + `cargo clippy --workspace -- -D warnings`。

---

## 6. 关键约束（不可回归）
- **中立层零领域依赖**：dispatcher/transport 不得再出现 `google`/`gmail`/`GoogleEvent`/`ActivateGoal`（grep 作为验收门）。
- **ChannelStore 幂等契约不变**：inbound `(channel,message_id)` dedup、outbound `correlation_id UNIQUE`、turn-before-send 原子提交、cursor 仅在 complete/reject 推进、chat 失败不推进 cursor。
- **安全属性保留**：owner-only 门禁、Gmail sender deny-by-default、OAuth token 不入 Debug/日志、Telegram token sanitize。
- **无环**：应用层 crate 只依赖 `fabric`；核心提供 capability 实现；`bin` 组装。
- **每阶段绿**：strangler-fig，任一阶段结束 `cargo test -p executive` 全绿再进下一阶段。

---

## 7. 验证
- 测试：`cargo test -p executive`（dispatcher/intent/store/transport/handler）；`cargo test --workspace`。
- 静态门：`cargo clippy --workspace -- -D warnings`；`rg -n "google|gmail|GoogleEvent|ActivateGoal" <中立层文件>` 应为空。
- 配置：schema 重生成后审 `git diff config/schema/aletheon-config.schema.json`；`TelegramConfig::validate` 自测（`cognit/src/config/mod.rs:1120`）通过。
- 端到端冒烟：`telegram.enabled=true`+`bot_token_env`+`owner_user_id`，daemon 起，手机发 `/start`、chat、`/goal <intent>`、点 approval 按钮；Gmail 冒烟：投一封 `[GOAL]` 邮件确认 draft-goal + Telegram 审批推送。

---

## 8b. 工作区隔离（已定）

正在跑的 grok/exec 任务（tool-exec/MCP/multi-user/sandbox/checkpoint/acp）与 `aletheon-bak` 的 MCP WIP 会与本重构的 Phase 4 接线点撞车。已确认隔离方案：

- **不在 `aletheon-bak` 重构**：它非干净备份，有 20 个未提交改动，其中 4 个（`cognit/config/mod.rs`、`channel/exec_server_client.rs`、`bootstrap/request.rs`、`service/request_use_cases.rs`）直接压在本重构目标上。
- **独立 worktree**：`/home/aurobear/Bear-ws/.worktrees/aletheon-channel-gateway`，分支 `auro/refactor/channel-gateway`，基于主仓 `aletheon`（8d79823b）。主仓内 `channel/*` 与 `fabric/types/channel.rs` 均干净、未被在跑任务触碰。
- **冲突面**：Phase 0–3（新建 `crates/gateway` + 重组 `channel/`）与在跑任务近零重叠，可并行；**Phase 4 排最后**，等 MCP/multi-user 那摊落地后 rebase。
- **待办**：本计划文档当前在主仓为未跟踪文件；开工时应将其提交到 `auro/refactor/channel-gateway` 分支，让计划随重构分支走。

## 8. 建议 / 待定
1. **Phase 0–2 先合入**即可解决「耦合太严重」的核心诉求，相互独立、风险可控。
2. **Phase 3（Gmail 统一）拆独立 PR**——安全面最广，延后不影响前序价值。
3. **crate 归属已定：变体 B（新建 `crates/gateway`，§4.3）**——人机 UI（interact）与 bot/机器渠道（gateway）彻底分家。搬迁集中在 Phase 4。
4. 未做需求外扩张（多用户、富媒体、webhook）——本计划只做「模式纠正」与「归位」，不加功能。
