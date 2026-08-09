# Interact authority census：TUI、CLI、ACP 只做 Presentation

> 状态：Draft；只读代码 census 与迁移约束，不包含生产代码修改
>
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
>
> 范围：`crates/interact/src/**/*.rs` 的 56 个当前文件
>
> 依赖计划：`2026-08-08-runtime-authority-consolidation.md`、`2026-08-08-composition-gateway-presentation-extraction.md`

## 1. 结论与不可变边界

`interact` 目前不是 Agent Core 的主要 composition root：生产代码没有构造 canonical `Agent`，也没有直接打开 Session/Turn/Agent repository。但是它仍然越界承担了 core ID、可信身份、effective permission/workspace、runtime/task route、socket/framing、兼容协议和 terminal 判断的一部分。因此不能把现有 `tui::App` 原样改名成“客户端”，而必须按 model/controller/renderer/typed client 拆开。

目标边界固定为：

```text
TuiRenderer -- user gesture --> TuiController -- typed preference/command --> GatewayClient
     ^                              |                                  |
     |                              v                                  v
     +-------------------------- TuiModel <--- typed projection/cursor/events
                                                                    |
                                                                    v
                                                Application -> canonical Runtime
```

- `TuiModel` 只是 daemon-owned facts 的可重建 projection，加上 input、selection、scroll、modal 等 connection-local 状态；不得写 Session/Turn/Agent truth。
- `TuiController` 只把明确的用户手势翻译成 typed command/preference；不得 mint core ID、计算 effective policy、推导 terminal 或直接持有 repository。
- `TuiRenderer` 是纯渲染：零 socket、零 env、零 filesystem、零 command、零 Runtime/Kernel/domain import。
- `gateway-client/protocol` 唯一拥有内层 Gateway request correlation、Gateway framing、subscription、cursor recovery、reconnect/backoff 和 protocol error；TUI、CLI 不再自写 Gateway JSON-line client。ACP adapter 只可保留外层 ACP framing、ACP connection correlation 与 DTO translation，不得手写内层 Gateway JSON-RPC/business method。
- Presentation 临时对象只能使用 `UiOverlayId`。兼容流没有 canonical `TurnId` 时也不得调用 `TurnId::new()`；overlay 与 durable ID 必须类型不可互换。
- ACP 只保留 ACP wire translation 与 connection-local correlation；`create/prompt/cancel/recover` 必须通过同一个 typed Gateway client，不能持有 `ExecutiveAcpBackend`、`RequestHandler` 或 Session repository。
- `fabric` 收缩并重命名为 `contracts` 后，Interact 仍只依赖 `gateway-client/gateway-protocol`；typed command/query/event/error 与 projection DTO 归 Gateway，`UiOverlayId` 归 Interact 本地模型。只有 ownerless canonical ID/version/correlation primitive 可由 Gateway protocol 引用 `contracts`，不得把旧 Fabric 数据模型重新导入 Presentation。
- `Executive` 必须完全退出 `interact` 的 normal dependency graph。测试不得成为保留 Executive dependency 的借口。

## 2. 计数口径与 authority-point census

以下只数基线生产路径中的定义/构造点；`#[cfg(test)]` fixture ID 不计入 production mint。把服务端返回的字符串包装为 `SessionId` 与 mint 分开记录，因为前者有些是兼容 decode，但两者都必须由 typed protocol 收口。

| 类别 | 当前计数 | file/symbol 证据 | 目标 |
|---|---:|---|---|
| canonical `SessionId` mint | 1 | `single_message.rs::prompt_intent` 用 `Uuid::new_v4()` 生成 `message-*` | `RA-03` 后由 Runtime `CreateSession` 返回 ID |
| client-side `SessionId(String)` reconstruction | 17 | `app/lifecycle.rs` 4、`app/key_handler.rs` 3、`app/submit.rs` 8、`reducer.rs` 1、`single_message.rs` explicit-session branch 1 | typed DTO decode；不接受普通 string 伪装 core ID |
| canonical `TurnId` mint | 2 | `reducer.rs::begin_live_turn`、`ensure_live_turn_id` | `UiOverlayId`；canonical Turn ID 只能来自 Runtime event/receipt |
| canonical `AgentId` mint | 0 | 全目录无 `AgentId` 构造 | 保持为零；ACP/TUI 只消费 Agent projection |
| core `Agent`/`Session` constructor | 0 | `App::new`、`AcpCorrelation` 都是 UI/connection state，不是 core aggregate | 保持为零；`SessionNew` 变 typed command，不在 client 构造 aggregate |
| local principal derivation | 1 | `intent.rs::local_principal` 从 effective UID 构造 `PrincipalId` | peer credential 在 Gateway host 建立，client 不提交 effective principal |
| effective permission derivation | 1 definition / 7 call sites | `host.rs::permission_mode_from_environment`；host/single-message/TUI/lifecycle/submit 注入 | client 只提交 requested mode；Kernel/host 计算 effective permission |
| effective workspace derivation/authorization | 3 | `host.rs::resolve_workspace`、`tui/mod.rs::run_with_config`、`acp/gateway.rs::canonical_authorized_workspace` | CLI path 是 request；canonicalize/trust/effective roots 在 authenticated Gateway/Kernel |
| runtime requirement construction | 2 | `host.rs::agent_runtime_requirements`、`app/submit.rs::send_to_daemon` | typed optional preference；Runtime 验证 capability 与 route |
| local `TaskKind` inference | 1 | `app/submit.rs` 把任意 `next_agent_runtime` 推成 `TaskKind::Coding` | 删除；TaskKind 必须由显式 CLI/user request 或 Application classifier 给出 |
| local execution-target selection | 1 state machine | `state.rs::{select_execution_target_for_next_turn,reconcile_execution_target}` | 仅作为 requested preference；effective target 必须由 Runtime receipt 回投 |
| raw socket/framing implementations | 10 files，8 reachable + 2 stale | `acp/transport.rs`、`memory_client.rs`、`single_message.rs`、`tui/{json_lines,mod,response}.rs`、`tui/app/{lifecycle,submit}.rs`；stale `tui/{debug,rpc_client}.rs` | 统一 typed GatewayClient/ACP transport；Presentation 不见 `UnixStream` |
| local terminal/error inference | 4 | `response.rs::{handle_event,apply_legacy_v0_response}`、`single_message.rs::run`、`app/lifecycle.rs::simple_line_mode` | 只接受 typed `TurnSettlement`/command error；未知 schema fail closed |
| direct Executive dependency | 2 files | `host.rs` production launcher/lifecycle；`response.rs` test-only tool bridge | 分别由 `aletheon` host facade、contract fixture 取代；Cargo dependency 删除 |
| direct repository dependency | 0 | 无 `*Repository/*Store::open` authority import；`InputStateStore` 是 local draft/history | 保持为零；local UI persistence 不得升级成 Session store |

### 2.1 17 个 Session wrapper 的处置不是“一刀切删除”

- `SessionId` 来自 typed `SessionCreated/SessionReadSnapshot` 时，迁成 gateway-client 内部 decode，Presentation 持 opaque reference。
- `SessionId` 来自 `/resume <string>`、ACP request、benchmark env 或其他未验证字符串时，只能是 `LegacySessionAlias`/requested reference；Gateway/Runtime 原子解析并验证 principal binding。
- `SessionNew`、fork、rewind 等 UI action 可以保留，但命令不得携带 caller-minted new Session ID；创建成功后只消费 Runtime receipt。

### 2.2 Terminal 与 error 的当前危险点

- `response.rs::handle_event(ClientEvent::TurnDone)` 依据本地 `turn_cancel_requested` 和先前 UI status 推出 Completed/Interrupted/Failed；这仍是第二 terminal state machine。
- `apply_legacy_v0_response` 按 `result.response/status/sessions/models/...` 字段猜业务类型；compat deadline 不能变成永久协议。
- `single_message.rs` 对 connection lost、timeout 和 raw `error.message` 多数只打印后返回 `Ok(())`，存在 false success 风险。
- `simple_line_mode` 自己读行、匹配 JSON shape、交互 approval、轮询并判断结束；与 TUI 和 one-shot 重复实现 client lifecycle。

## 3. Blocker 词典

| Blocker | 必须先得到的证据 |
|---|---|
| `I0-CALLERS` | production/test/re-export caller 已区分；零 production caller 才可提 DELETE |
| `I1-CONTRACT` | `gateway-protocol` 拥有 typed command/query/event/error、opaque client references 与 projection DTO；Interact 拥有不可序列化的 local `UiOverlayId`；`contracts` 只提供 ownerless canonical ID/version/correlation primitive |
| `I2-SESSION` | `RA-03` canonical Session create/resume/fork 与 alias import 可用，client 不 mint Session ID |
| `I3-TURN` | `RA-04` canonical Turn terminal/event/cursor 可用，未知或断流不会 false success |
| `I4-AGENT` | `RA-05` Agent projection、delegate receipt、runtime preference contract 已稳定 |
| `I5-CLIENT` | `CGP-02` GatewayClient 统一内层 Gateway framing/correlation/subscription/reconnect/backoff；外层 ACP wire/correlation 仍局限于 ACP adapter |
| `I6-ACP` | `CGP-05` ACP create/prompt/cancel/recover 全走 typed client，connection isolation 等价 |
| `I7-TUI` | `CGP-06/07` command-only cutover与 App 拆分；compat state 有删除 deadline |
| `I8-HOST` | `CGP-04/08` official daemon/composition 已切换，Interact 不再启动 Executive |
| `I9-OPTIONAL` | stale debug/goal/workflow/test helper 的支持承诺、installed caller 与替代路径已判定 |

## 4. 56-file disposition ledger

`KEEP` 表示责任本身可以留，但仍要遵守 import/纯度 gate；`MOVE` 表示责任整体迁 owner；`SPLIT` 表示同一文件中的责任分到多个目标；`DELETE` 只用于已具备 caller 证据的壳；`INVESTIGATE` 在 `CGP-00` 结束前不得偷换成删除。

| # | 当前文件 | file/symbol census 与越界点 | Target | Disposition | 阶段 / RA 依赖 | Blocker |
|---:|---|---|---|---|---|---|
| 1 | `crates/interact/src/acix/mod.rs` | 仅重导出 `GroundingProvider/Result/Mock`；全仓无 `interact::acix` caller，领域类型不应由 Presentation facade 暴露 | DELETE | DELETE | CGP-00 | I0-CALLERS |
| 2 | `crates/interact/src/acp/event_map.rs` | `map_client_event_to_acp` 与 `is_turn_terminal`；生产只映 typed event，测试内 mint Turn/Operation ID | interact ACP wire adapter | MOVE | CGP-05 / RA-04 | I1-CONTRACT, I3-TURN, I6-ACP |
| 3 | `crates/interact/src/acp/gateway.rs` | `AcpBackend`、dispatch、workspace canonicalization、recover loop、ACP frames；业务 backend 抽象仍以 Executive 为语义，认证/effective workspace 与 protocol 混合 | interact ACP wire adapter + gateway-client backend + Gateway/host authenticated workspace | SPLIT | CGP-05 / RA-03, RA-04 | I1-CONTRACT, I2-SESSION, I3-TURN, I6-ACP |
| 4 | `crates/interact/src/acp/mod.rs` | ACP DTO、connection correlation、metrics、`establish_principal`；protocol-local binding 可留，但 host principal/policy 构造必须移出 | interact ACP DTO/correlation + gateway-client backend + Gateway/host identity | SPLIT | CGP-02, CGP-05 / RA-03 | I1-CONTRACT, I2-SESSION, I6-ACP |
| 5 | `crates/interact/src/acp/transport.rs` | 有界 newline JSON framing；属于 ACP wire，不属于 TUI model/controller | interact ACP wire adapter | KEEP | CGP-05 | I1-CONTRACT, I6-ACP |
| 6 | `crates/interact/src/host.rs` | `ExecutiveDaemonEnsurer`、daemon lifecycle concrete types、socket/workspace/effective permission/runtime requirement 和 cwd mutation混合；唯一 production `executive::` 源 | aletheon lifecycle/daemon client + gateway-client/protocol + presentation helper | SPLIT | CGP-04, CGP-06, CGP-08 / RA-03, RA-05 | I1-CONTRACT, I2-SESSION, I4-AGENT, I8-HOST |
| 7 | `crates/interact/src/intent.rs` | 从 UID 构造 principal，并把 workspace、permission、TaskKind、runtime/target 作为 effective authority 填进 wire | gateway-client/protocol | SPLIT | CGP-02, CGP-06 / RA-01, RA-03 | I1-CONTRACT, I2-SESSION, I5-CLIENT |
| 8 | `crates/interact/src/lib.rs` | crate facade 暴露 ACIX、ACP、host、memory、TUI；全局 allow 掩盖 oversized/错误边界 | presentation helper | SPLIT | CGP-00, CGP-07 | I0-CALLERS, I7-TUI |
| 9 | `crates/interact/src/memory_client.rs` | 自有 `UnixStream`、request ID、initialize、JSON-line、最多 32 unrelated messages；协议是 typed，但 transport 重复实现 | gateway-client/protocol | MOVE | CGP-02, CGP-06 | I1-CONTRACT, I5-CLIENT |
| 10 | `crates/interact/src/single_message.rs` | 自有 socket/read loop/approval/timeout/JSON shape；mint SessionId，读 benchmark env，terminal/error 可打印后成功 | TuiController + gateway-client/protocol | SPLIT | CGP-02, CGP-06 / RA-03, RA-04 | I2-SESSION, I3-TURN, I5-CLIENT |
| 11 | `crates/interact/src/tui/activity_detail.rs` | `ActivityDetail` 从 projection entry 纯渲染 | TuiRenderer | MOVE | CGP-07 | I7-TUI |
| 12 | `crates/interact/src/tui/agent_inspector.rs` | `AgentInspector` 同时 JSON decode、selection key handling、render；只读 Agent projection，无 Agent mint | TuiModel + TuiController + TuiRenderer | SPLIT | CGP-06, CGP-07 / RA-05 | I1-CONTRACT, I4-AGENT, I7-TUI |
| 13 | `crates/interact/src/tui/app/key_handler.rs` | 巨型 gesture controller；直接组 SessionId、写 approval response、调用 write_request，混 modal 和 transport | TuiController | SPLIT | CGP-06, CGP-07 / RA-03, RA-04 | I2-SESSION, I3-TURN, I5-CLIENT, I7-TUI |
| 14 | `crates/interact/src/tui/app/lifecycle.rs` | terminal loop、line-mode parser、manual socket/read/timeout/approval、projection polling、raw JSON/error format 混合；4 个 Session wrapper | TuiController + gateway-client/protocol | SPLIT | CGP-02, CGP-06, CGP-07 / RA-03, RA-04 | I2-SESSION, I3-TURN, I5-CLIENT, I7-TUI |
| 15 | `crates/interact/src/tui/app/mod.rs` | 3 行 namespace shell；可在拆分后仅做 controller module 声明，不承载 state | TuiController | KEEP | CGP-07 | I7-TUI |
| 16 | `crates/interact/src/tui/app/submit.rs` | command dispatch、policy提示、typed/raw write、8 个 Session wrapper、runtime requirement、TaskKind::Coding 推导与 execution target 混合 | TuiController + gateway-client/protocol | SPLIT | CGP-06, CGP-07 / RA-03, RA-04, RA-05 | I2-SESSION, I3-TURN, I4-AGENT, I5-CLIENT |
| 17 | `crates/interact/src/tui/approval_dialog.rs` | approval modal state、key decision、render 在一类；decision 只是 user gesture，不得成为 validated approval | TuiModel + TuiController + TuiRenderer | SPLIT | CGP-07 | I1-CONTRACT, I7-TUI |
| 18 | `crates/interact/src/tui/awareness.rs` | 从 `AwarenessState` 渲染，不推进 Self/Metacog 状态 | TuiRenderer | MOVE | CGP-07 | I7-TUI |
| 19 | `crates/interact/src/tui/chat.rs` | transcript entries、tool-arg JSON format、layout cache、word wrap、renderer 混合；compat transcript 还是第二 conversation mirror | TuiModel + TuiRenderer | SPLIT | CGP-07 / RA-04 | I3-TURN, I7-TUI |
| 20 | `crates/interact/src/tui/checkpoint_picker.rs` | checkpoint projection、selection/key action、modal render 三责合一 | TuiModel + TuiController + TuiRenderer | SPLIT | CGP-07 / RA-03 | I2-SESSION, I7-TUI |
| 21 | `crates/interact/src/tui/command.rs` | 纯 command syntax enum/parse facade；当前每次 parse 构造 registry | presentation helper | KEEP | CGP-06, CGP-07 | I1-CONTRACT, I7-TUI |
| 22 | `crates/interact/src/tui/completion.rs` | completion candidates、selection state、render，并直接触发 filesystem attachment discovery | TuiModel + TuiController + TuiRenderer | SPLIT | CGP-07 | I7-TUI |
| 23 | `crates/interact/src/tui/conscious_core.rs` | 只把 `ConsciousCoreSnapshot` 转为展示行；Self/Dasein authority 不在此 | TuiRenderer | MOVE | CGP-07 | I1-CONTRACT, I7-TUI |
| 24 | `crates/interact/src/tui/debug.rs` | 未被 `tui/mod.rs` 声明，当前不可达；含三套 direct socket/read loop、raw JSON、timer polling 与 CLI action | INVESTIGATE | INVESTIGATE | CGP-00 | I0-CALLERS, I5-CLIENT, I9-OPTIONAL |
| 25 | `crates/interact/src/tui/diff_view.rs` | mutation projection、local selection/scroll 和 render 混合；注释已承认 terminal/recovery 由 Host 决定 | TuiModel + TuiRenderer | SPLIT | CGP-07 | I1-CONTRACT, I7-TUI |
| 26 | `crates/interact/src/tui/file_picker.rs` | 有界、nofollow 的 workspace file discovery；直接 filesystem I/O 仅服务 palette，不是 Runtime authority | presentation helper | KEEP | CGP-07 | I7-TUI |
| 27 | `crates/interact/src/tui/goal.rs` | 未被 module tree 声明且引用不存在的 `super::cli::GoalAction`；旧 raw RPC/JSON formatter | INVESTIGATE | INVESTIGATE | CGP-00 | I0-CALLERS, I9-OPTIONAL |
| 28 | `crates/interact/src/tui/help_overlay.rs` | help overlay 纯 render | TuiRenderer | MOVE | CGP-07 | I7-TUI |
| 29 | `crates/interact/src/tui/history_search.rs` | search model、key controller 与 render 合一；只处理 local input history | TuiModel + TuiController + TuiRenderer | SPLIT | CGP-07 | I7-TUI |
| 30 | `crates/interact/src/tui/host_time.rs` | client clock/timer adapter；可供 controller/client 注入，不能进入 reducer/renderer | presentation helper | KEEP | CGP-07 | I5-CLIENT, I7-TUI |
| 31 | `crates/interact/src/tui/input.rs` | history model与 local draft/history filesystem store 混合；本地持久化是允许的 UI state，但不得存 durable conversation | TuiModel + presentation helper | SPLIT | CGP-07 | I7-TUI |
| 32 | `crates/interact/src/tui/input_safety.rs` | paste sanitize 与 attachment preflight；可做 fail-closed client check，但不能冒充 Kernel/effective workspace enforcement | presentation helper | KEEP | CGP-06, CGP-07 | I1-CONTRACT, I7-TUI |
| 33 | `crates/interact/src/tui/json_lines.rs` | TUI 私有 UTF-8 newline buffer，和 ACP/CLI/Mem client framing 重复 | gateway-client/protocol | MOVE | CGP-02, CGP-06 | I1-CONTRACT, I5-CLIENT |
| 34 | `crates/interact/src/tui/markdown.rs` | markdown/highlight/table render；纯 presentation | TuiRenderer | MOVE | CGP-07 | I7-TUI |
| 35 | `crates/interact/src/tui/mod.rs` | 659 行 facade + terminal setup + direct UnixStream + workspace/env/model + monolithic `App` + compat IDs；第二 workspace derivation点 | TuiController + gateway-client/protocol | SPLIT | CGP-06, CGP-07 / RA-03, RA-04 | I2-SESSION, I3-TURN, I5-CLIENT, I7-TUI |
| 36 | `crates/interact/src/tui/pager.rs` | pager selection/scroll state与 render 合一 | TuiModel + TuiController + TuiRenderer | SPLIT | CGP-07 | I7-TUI |
| 37 | `crates/interact/src/tui/plan_view.rs` | Plan/Critique projection、version selection和 render 合一；不得调用 Cognit | TuiModel + TuiRenderer | SPLIT | CGP-07 | I1-CONTRACT, I7-TUI |
| 38 | `crates/interact/src/tui/reducer.rs` | canonical projection reducer，但兼容 path 在两处 mint `TurnId`，生成 live activity/item IDs 并维护 terminal overlay | TuiModel | SPLIT | CGP-06, CGP-07 / RA-01, RA-04 | I1-CONTRACT, I3-TURN, I7-TUI |
| 39 | `crates/interact/src/tui/registry.rs` | command catalog、availability、skill raw JSON decode、fuzzy search与 legacy route mapping混合 | TuiModel + TuiController | SPLIT | CGP-06, CGP-07 | I1-CONTRACT, I5-CLIENT, I7-TUI |
| 40 | `crates/interact/src/tui/render/draw.rs` | terminal draw、layout composition与 frame-recorder test side effect 混合 | TuiRenderer | SPLIT | CGP-07 | I7-TUI, I9-OPTIONAL |
| 41 | `crates/interact/src/tui/render/header.rs` | header 纯 render | TuiRenderer | MOVE | CGP-07 | I7-TUI |
| 42 | `crates/interact/src/tui/render/input_line.rs` | input line 纯 render | TuiRenderer | MOVE | CGP-07 | I7-TUI |
| 43 | `crates/interact/src/tui/render/mod.rs` | 4 行 renderer namespace | TuiRenderer | KEEP | CGP-07 | I7-TUI |
| 44 | `crates/interact/src/tui/render/renderable.rs` | layout/renderable tree；持只读 AppState/workspace，但 renderer不得获得 filesystem或 command能力 | TuiRenderer | MOVE | CGP-07 | I7-TUI |
| 45 | `crates/interact/src/tui/response.rs` | 2246 行 raw socket decode、typed+V0 protocol、controller effects、formatters和第二 terminal machine；测试直接 import Executive bridge | TuiModel + TuiController + gateway-client/protocol | SPLIT | CGP-02, CGP-06, CGP-07 / RA-04 | I1-CONTRACT, I3-TURN, I5-CLIENT, I7-TUI |
| 46 | `crates/interact/src/tui/rpc_client.rs` | 未被 module tree 声明；one-line raw JSON-RPC socket helper，只被同样 stale 的 goal/workflow 源引用 | INVESTIGATE | INVESTIGATE | CGP-00 | I0-CALLERS, I5-CLIENT, I9-OPTIONAL |
| 47 | `crates/interact/src/tui/session_picker.rs` | Session list selection、key action与 render 合一；string ID 仅是 projection reference | TuiModel + TuiController + TuiRenderer | SPLIT | CGP-06, CGP-07 / RA-03 | I1-CONTRACT, I2-SESSION, I7-TUI |
| 48 | `crates/interact/src/tui/session_protocol.rs` | 第二套 Session RPC wrapper/JSON conversion 与 compatibility aliases；应由 GatewayClient contracts 统一 | gateway-client/protocol | MOVE | CGP-02, CGP-06 / RA-03 | I1-CONTRACT, I2-SESSION, I5-CLIENT |
| 49 | `crates/interact/src/tui/state.rs` | projection + UI selection 状态；`live_turn_id: TurnId` 类型错误，execution target 必须明确为 requested preference | TuiModel | SPLIT | CGP-06, CGP-07 / RA-01, RA-04 | I1-CONTRACT, I3-TURN, I7-TUI |
| 50 | `crates/interact/src/tui/status.rs` | status bar state与 widgets混合；只显示 typed runtime/accounting facts，不从 prose/model self-report推导 | TuiModel + TuiRenderer | SPLIT | CGP-07 / RA-04 | I1-CONTRACT, I3-TURN, I7-TUI |
| 51 | `crates/interact/src/tui/streaming.rs` | bounded text/table holdback 的 local display model，terminal text replacement应由 authoritative snapshot触发 | TuiModel | KEEP | CGP-07 / RA-04 | I3-TURN, I7-TUI |
| 52 | `crates/interact/src/tui/subagent_view.rs` | child Agent projection纯 render | TuiRenderer | MOVE | CGP-07 / RA-05 | I4-AGENT, I7-TUI |
| 53 | `crates/interact/src/tui/task_console.rs` | 大型 Task/Activity renderer；读取动态 JSON budget/progress 并含旧 Executive 架构 fixture text，必须改 typed display DTO | TuiRenderer | SPLIT | CGP-06, CGP-07 / RA-04, RA-05 | I1-CONTRACT, I3-TURN, I4-AGENT, I7-TUI |
| 54 | `crates/interact/src/tui/term_compat.rs` | terminal capability/env detection与 theme；属于 presentation helper，结果注入 renderer，renderer本身不读 env | presentation helper | KEEP | CGP-07 | I7-TUI |
| 55 | `crates/interact/src/tui/test_infra.rs` | frame/event recorder与 scripted input；直接 filesystem I/O，当前编入 production module但只应服务最小 presentation smoke | INVESTIGATE | INVESTIGATE | CGP-00, CGP-07 | I0-CALLERS, I9-OPTIONAL |
| 56 | `crates/interact/src/tui/workflow.rs` | 未被 module tree 声明；旧 workflow JSON 文件与 raw RPC CLI；是否保留取决于 optional Application workflow 决策 | INVESTIGATE | INVESTIGATE | CGP-00 | I0-CALLERS, I5-CLIENT, I9-OPTIONAL |

## 5. 按目标聚合后的拆分边界

### 5.1 `TuiModel`

只接收 typed `ProjectionDelta`、`ProjectionSnapshot` 和 local `UiAction`。建议最小状态是：

- daemon projection：opaque Session/Turn/Agent references、items、tasks、activities、approval views、cursor、typed terminal settlement；
- local view：scroll、selection、input draft、modal、completion、streaming overlay；
- `UiOverlayId`：由 controller/model 的专用 source 生成，仅用于 local overlay；禁止实现 `Into<TurnId>` 或序列化成 core ID 字段；
- projection reconnect 后 local overlay 可以丢弃，不能覆盖 snapshot，也不能据 overlay 回写 Runtime。

### 5.2 `TuiController`

允许读取 input/model 并调用一个 `GatewayClient` trait。它可以产生 `SubmitPromptRequest`、`RequestSessionCreation`、`ResumeSessionReference`、`CancelActiveTurn`、`SubmitApprovalChoice`，但这些都是请求，不是 authority facts。以下全部禁止：

- 读取 `ALETHEON_PERMISSION_MODE` 并声称 effective；
- 把 runtime selection 自动变成 `TaskKind::Coding`；
- 从 UID 构造可信 principal；
- 从任意字符串/UUID 构造 canonical Session/Turn/Agent ID；
- 从 `turn_done` 文本、error string、socket EOF、spinner state 推导成功。

### 5.3 `TuiRenderer`

Renderer 输入必须是不可变 `TuiView<'_>`，不直接取得 `WorkspacePolicy`、clock、env、filesystem handle、GatewayClient 或 command sender。`task_console.rs` 中 `serde_json::Value` budget/progress 必须由 versioned display DTO 取代；缺字段显示 `unknown`，不能猜测。

### 5.4 Gateway client / protocol

收口当前至少五套 transport lifecycle：ACP transport、MemoryClient、single-message、interactive TUI、simple-line/raw RPC。一个 client 必须统一：

- typed request/response/event envelope 和 negotiated version；
- request ID correlation；
- bounded framing 和 strict UTF-8；
- authenticated connection metadata（由 host填充，不由 presentation声明）；
- snapshot + cursor subscription/recovery；
- machine/provider backpressure advice 与 bounded reconnect/backoff；
- EOF、timeout、unknown schema、provider rejection、cancel 的 typed failure；
- terminal 只来自 authoritative terminal event/receipt。

ACP 额外保留自身 wire frame 和 connection-local ACP session correlation，但其 backend 应是 `GatewayClient` typed ports，不再定义“Executive boundary”。

## 6. 分阶段迁移与删除顺序

1. **CGP-00 / RA-00 census hardening**：冻结本 ledger；逐 wrapper 判定 canonical receipt、legacy alias或 fixture；确认 stale 文件 caller；增加“Interact core ID mint 只能下降”静态 gate。无行为变更。
2. **RA-01 + CGP-02 contracts/client**：RA-01 提供 owner-assigned canonical ID primitive；CGP-02 在 `gateway-protocol` 引入 opaque references、typed errors/events 与 GatewayClient；Interact 独立引入 local `UiOverlayId`。先让 old transport 单向适配新 client contract，不切 writer。
3. **RA-03 Session cutover**：Runtime 创建 Session；迁 new/resume/fork/rewind；删除 single-message mint 和未验证 string-to-SessionId constructor。
4. **RA-04 Turn cutover**：所有 terminal、cursor、live overlay 改从 typed Turn events投影；删除 reducer local TurnId mint和 UI terminal machine。
5. **RA-05 Agent contract**：TUI runtime selection只成为 explicit preference；Agent inspector/subagent/task console消费 canonical Agent/delegate projections。
6. **CGP-05 ACP**：ACP backend换 typed client，保持 cancel/recover并发和 connection isolation；删除 Executive语义与 direct store recovery。
7. **CGP-06 TUI/CLI**：删除 raw UnixStream/manual framing/local principal/effective policy/TaskKind inference；one-shot、memory、line mode复用同一 client。
8. **CGP-07 App split**：拆 Model/Controller/Renderer；删除 compat transcript、V0 field-shape parser和重复 Session protocol；只保留 local input/history/preferences persistence。
9. **CGP-08**：`host.rs` 不再 import Executive；`Cargo.toml` 删除 `executive` dependency；stale optional files按 I9 结论删除或重写成 typed client caller。

每一步回滚只能切换 client adapter或 UI composition，不能恢复 client-side authority或第二 Runtime writer。

## 7. CGP-00 hard gate

`CGP-00` 在下列条件全部满足前不得进入 `CGP-02` 的实际 client cutover，CGP-01 最多建立空 skeleton：

- 本基线 56 个文件全部有唯一 ledger 行；新增/删除文件必须同步更新 actual/ledger 计数。
- 1 个 Session mint、17 个 Session wrapper、2 个 Turn mint全部有 source、输入来源、target owner和删除/translation阶段；`AgentId` mint保持 0。
- `interact` 新增 `SessionId::new`、`TurnId::new`、`AgentId::new` 或 unchecked string constructor 静态失败；fixture只能位于明确 test module。
- Interact-local `UiOverlayId` 已落为不可序列化类型，不能转换为 canonical `TurnId`，也不进入 `contracts` 或 Gateway wire。
- 10 个 raw transport/framing 文件有统一 GatewayClient映射；两个 stale source先完成 caller census，不以“看似没用”直接删除。
- 4 个 terminal/error inference点均映射到 typed terminal/error，timeout/EOF/unknown/provider rejection不得返回成功。
- `host.rs` 所有 Executive symbol有 `aletheon`/Gateway client替代；`response.rs` test-only Executive import有 contract fixture替代。
- direct repository dependency保持 0；ACP和TUI不得新增 Session/Turn/Agent store或Runtime/Kernel concrete dependency。
- `TuiModel/TuiController/TuiRenderer/GatewayClient` 的 import rule已能静态执行，且旧 `App` 字段只减不增。
- stale `debug.rs/goal.rs/rpc_client.rs/workflow.rs` 和 `test_infra.rs` 已分别得出 KEEP/MOVE/DELETE 结论及 production caller证据；`INVESTIGATE` 在 writer cutover前归零。

## 8. 覆盖与可复算结果

复算命令只读：

```text
actual = rg --files crates/interact/src -g '*.rs' | sort
ledger = 从本文件第 4 节第一列 path 提取并 sort
```

基线结果：

- `actual=56`
- `ledger=56`
- `unique=56`
- `missing=0`
- `extra=0`
- `duplicate=0`
- disposition：`KEEP=9 / MOVE=13 / SPLIT=28 / DELETE=1 / INVESTIGATE=5`；五项调查必须在对应 cutover 前归零，而不是默认删除

Authority-point 复算摘要：canonical mint `Session=1 / Turn=2 / Agent=0`；Session string reconstruction `17`；core Agent/Session constructor `0`；effective permission `1 definition / 7 call sites`；workspace/effective authorization `3`；runtime requirement construction `2`；TaskKind local inference `1`；raw transport/framing `10 files`；terminal/error inference `4`；Executive imports `2 files（1 production + 1 test-only）`；direct repository imports `0`。
