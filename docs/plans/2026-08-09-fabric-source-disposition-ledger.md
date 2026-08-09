# Fabric source disposition ledger

> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`；审计对象：`crates/fabric/src/**/*.rs`。

## 1. 判定规则

`fabric` 最终收缩并重命名为 `contracts`，但 **contracts 不是 ABI**，也不是新的共享模型仓库。它只容纳无 owner 的值语义 primitive（稳定 ID 包装、时间值、版本号、跨边界相关键）以及确实需要被两个以上 owner 共同引用的极小关联 DTO。Session/Turn/Agent 状态机、领域快照、repository、live permit、policy、wire codec、持久化 schema、UI model、IPC backend 都不能因为“到处在用”而留在 `contracts`。

Disposition 含义：

- `CONTRACTS`：可在 D1 证明为 ownerless primitive 后进入最小 `contracts`；
- `MOVE`：语义与实现整体回到唯一 owner；
- `SPLIT`：同一文件混有 primitive、rich model、port 或 adapter，按 owner 拆开；
- `COMPAT`：只作为有截止日期的旧 wire/re-export 兼容层；
- `DELETE`：caller 清零后删除，不用新名字复刻；
- `INVESTIGATE`：同名不等于同义，或 wire/persistence/caller 事实不足，D0/B0 结案前不得移动或合并。

Blocker：`B0` caller/语义/格式 census；`B1` 新 owner authority 与单 writer 就绪；`B2` wire/schema 双读或版本协商；`B3` owner-defined port 就绪；`B4` 数据迁移与回滚；`B5` adapter/composition 切换；`B6` 保留扩展行为 smoke；`B7` 旧入口与 re-export 清零。

## 2. 逐文件去向

### 2.1 adapters、contract、Dasein 与 events

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/adapters/mod.rs` | skill schema adapter 私有模块入口 | Corpus adapter | MOVE | D5 | B3+B7 |
| `crates/fabric/src/adapters/skill_schema.rs` | JSON Schema/instance 校验实现 | Corpus skill adapter | MOVE | D5 | B0+B3 |
| `crates/fabric/src/contract/command.rs` | client intent / command-output v1 wire envelopes | Gateway protocol | COMPAT | CGP-01/CGP-02 | B0+B2+B7 |
| `crates/fabric/src/contract/mod.rs` | command 与 envelope-v2 兼容 re-export | Gateway protocol | COMPAT | CGP-02 | B2+B7 |
| `crates/fabric/src/dasein/context.rs` | temporal/self/care/assertion rich snapshots | Dasein | MOVE | D3 | B1+B3 |
| `crates/fabric/src/dasein/event.rs` | `DaseinEvent`、temporal event kind | Dasein | MOVE | D3 | B1 |
| `crates/fabric/src/dasein/mod.rs` | Dasein surface 与通配 re-export | Dasein | MOVE | D3 | B7 |
| `crates/fabric/src/dasein/ops.rs` | `DaseinOps` authority port | Dasein | MOVE | D3 | B1+B3 |
| `crates/fabric/src/dasein/transition.rs` | self version/event、experience、transition rich model | Dasein | MOVE | D3 | B1+B4 |
| `crates/fabric/src/dasein/types.rs` | Stimmung/Angst/affect/readiness 领域值 | Dasein | MOVE | D3 | B1 |
| `crates/fabric/src/events/evolution.rs` | reflection/mutation/evolution 与 agent lifecycle payload 混合 | Metacog + Runtime | SPLIT | D3+RA-02 | B0+B1+B2 |
| `crates/fabric/src/events/mod.rs` | event 模块聚合入口 | 各 event owner | DELETE | D6 | B7 |
| `crates/fabric/src/events/routing_policy.rs` | event priority routing policy | Runtime event routing | MOVE | RA-02 | B1+B3 |
| `crates/fabric/src/events/spine.rs` | `EventIdentity`、tree sequence、payload、spine append/read | Runtime Journal/EventSpine | SPLIT | RA-02 | B1+B3+B4 |
| `crates/fabric/src/events/subscription.rs` | in-process schema subscription registry | Runtime event projection | MOVE | RA-02 | B0+B1 |
| `crates/fabric/src/events/types.rs` | EventType/Priority/subscription timeout | Runtime event routing | MOVE | RA-02 | B0+B1 |
| `crates/fabric/src/events/ui_event.rs` | collaboration/awareness/sub-agent/TUI event models，混有 delegate lifecycle、认知/自我策略与演化证据 | Gateway projection + Runtime delegate + Cognit/Dasein policy + Metacog | SPLIT | CGP-03+RA-05+D2+D3 | B0+B1+B2+B5 |

### 2.2 include surfaces

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/include/admission.rs` | admission/budget/lease enforcement traits | Kernel | MOVE | K2/K3 | B1+B3 |
| `crates/fabric/src/include/agora.rs` | Agora proposal/commit/view/version conflict models | Agora | MOVE | D4 | B1+B4 |
| `crates/fabric/src/include/body.rs` | embodied action/result/side effects 与 `BodyRuntime` | Hardware + Robot adapter | SPLIT | E4/E5(prep)+E4-K6b/E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B3+B6 |
| `crates/fabric/src/include/capability_invoker.rs` | capability invocation port | Kernel | MOVE | K2/K4 | B1+B3 |
| `crates/fabric/src/include/chronos.rs` | `Clock`/`Timer` callers 跨 Agora/Cognit/Corpus/Dasein/Runtime/Interact/Kernel | owner-local clock/deadline ports + host time adapter；Kernel 只拥有 enforcement clock port | SPLIT | D0+D2+D3+D4+D5+RA-02+CGP-04+K1 | B0+B1+B3+B5 |
| `crates/fabric/src/include/cognit.rs` | Plan/step/cost/execution/reflection rich model | Cognit | MOVE | D2 | B1+B3 |
| `crates/fabric/src/include/compaction.rs` | message compaction algorithms/strategy/outcome | Cognit context + Mnemosyne summary | SPLIT | D2+D4 | B0+B1 |
| `crates/fabric/src/include/extension_provider.rs` | tool/hook/agent-runtime/connector provider traits | Corpus + Application extension package | INVESTIGATE | D0+D5+APX-05 | B0+B3 |
| `crates/fabric/src/include/memory.rs` | memory models/query/compaction/embedding/provider throttling | Mnemosyne + provider adapter | SPLIT | D4 | B0+B1+B3+B4 |
| `crates/fabric/src/include/meta.rs` | candidate/test/evaluation/recommendation/migration runtime ops | Metacog + Application config | SPLIT | D3+APX-04 | B0+B1+B3 |
| `crates/fabric/src/include/mod.rs` | legacy include-module aggregation | 各 owner | DELETE | D6 | B7 |
| `crates/fabric/src/include/plugin.rs` | generic plugin context/lifecycle interface | Corpus/Application extension package | INVESTIGATE | D0+D5 | B0+B3 |
| `crates/fabric/src/include/process.rs` | process/operation handle 与 manager traits | Kernel | MOVE | K1/K5 | B1+B3 |
| `crates/fabric/src/include/runtime.rs` | agent status/info 与 scheduler models | Runtime | MOVE | RA-03 | B1+B4 |
| `crates/fabric/src/include/self_field.rs` | intent/identity/care/conflict/self-awareness rich model | Dasein | MOVE | D3 | B1 |
| `crates/fabric/src/include/space.rs` | `SpaceManager` generic authority trait；现仅 Kernel 静态引用，不能据名称归给 Agora | owner 未决，候选 Kernel operation / Runtime session / Agora space | INVESTIGATE | K0+D0 | B0+B1+B3 |
| `crates/fabric/src/include/subsystem.rs` | generic subsystem lifecycle/context/health abstraction | Composition root / owner-specific health ports | INVESTIGATE | D0+CGP-04 | B0+B3+B7 |
| `crates/fabric/src/include/turn.rs` | recall/self/agora/capability/turn requirement mixed DTO | Runtime + Cognit + Dasein + Agora + Kernel | SPLIT | RA-01+D2+D3+D4+K2 | B0+B1+B3 |

### 2.3 IPC backends 与 bus

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/ipc/backends/io_uring.rs` | io_uring IPC backend；基线外部/内部构造 caller 均为零 | installed/config 证据后 DELETE，非默认 Linux 迁移项 | INVESTIGATE | D0+CGP-04 | B0+B5+B7 |
| `crates/fabric/src/ipc/backends/io_uring_transport.rs` | io_uring `Transport` wrapper；基线外部/内部构造 caller 均为零 | installed/config 证据后 DELETE，非默认 Linux 迁移项 | INVESTIGATE | D0+CGP-04 | B0+B5+B7 |
| `crates/fabric/src/ipc/backends/json_rpc.rs` | legacy JSON-RPC adapter/handler bridge；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE；观测到 live route 才拆入 Gateway adapter | INVESTIGATE | D0+CGP-02 | B0+B2+B5+B7 |
| `crates/fabric/src/ipc/backends/json_rpc_transport.rs` | legacy JSON-RPC Unix transport；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE；观测到 live route 才拆入 Gateway adapter | INVESTIGATE | D0+CGP-02 | B0+B2+B5+B7 |
| `crates/fabric/src/ipc/backends/manager.rs` | environment probe/backend selection/queue；基线无构造 caller | installed/config 证据后 DELETE；只有观测到 live backend selection 才拆入 Composition | INVESTIGATE | D0+CGP-04 | B0+B5+B7 |
| `crates/fabric/src/ipc/backends/mod.rs` | 旧 backend re-export shell；子模块大多仍待 installed/config census | —；D0 若发现 live adapter，只迁具体子项并在目标重建窄 facade | DELETE | D6 | B0+B5+B7 |
| `crates/fabric/src/ipc/backends/priority_queue.rs` | `PriorityChannel`/`AgentMessage` queue；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE；有 live delegate route 才重判 owner | INVESTIGATE | D0+RA-02 | B0+B1+B5+B7 |
| `crates/fabric/src/ipc/backends/shared_mem.rs` | shared-memory region/backend；基线外部/内部构造 caller 均为零 | installed/config 证据后 DELETE，非默认 Linux 迁移项 | INVESTIGATE | D0+CGP-04 | B0+B5+B7 |
| `crates/fabric/src/ipc/backends/shared_mem_transport.rs` | shared-memory `Transport` wrapper；基线外部/内部构造 caller 均为零 | installed/config 证据后 DELETE，非默认 Linux 迁移项 | INVESTIGATE | D0+CGP-04 | B0+B5+B7 |
| `crates/fabric/src/ipc/backends/transport_adapter.rs` | legacy `IpcBackend` to `Transport` bridge；`IpcBackendAdapter` 基线 caller 为零 | DELETE（不是兼容 seam） | DELETE | D6 | B7 |
| `crates/fabric/src/ipc/backends/unix_socket.rs` | legacy Unix socket IPC backend；daemon 使用独立 host socket，基线无此类型 caller | installed/config 证据后 DELETE，勿与 daemon socket 混同 | INVESTIGATE | D0+CGP-04 | B0+B5+B7 |
| `crates/fabric/src/ipc/bus/communication_bus.rs` | multi-transport request/pubsub/event coordinator；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE；禁止复制成第二 Runtime/Gateway bus | INVESTIGATE | D0+RA-02+CGP-02 | B0+B1+B5+B7 |
| `crates/fabric/src/ipc/bus/in_process.rs` | `InProcessTransport`/metrics；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE | INVESTIGATE | D0+RA-02 | B0+B1+B5+B7 |
| `crates/fabric/src/ipc/bus/kernel_bus.rs` | `CanonicalEventBus` 实为 non-durable broadcast notification，不是 journal | Runtime notification/projection adapter（rename、可替换、非权威）或 RA-02 caller cutover 后 DELETE | INVESTIGATE | D0+RA-02 | B0+B1+B7 |
| `crates/fabric/src/ipc/bus/mod.rs` | bus module exports | Runtime/Gateway adapters | DELETE | D6 | B7 |
| `crates/fabric/src/ipc/bus/pubsub.rs` | legacy pub/sub protocol；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE | INVESTIGATE | D0+CGP-02 | B0+B2+B5+B7 |
| `crates/fabric/src/ipc/bus/request_response.rs` | legacy request/response correlation；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE | INVESTIGATE | D0+CGP-02 | B0+B2+B5+B7 |
| `crates/fabric/src/ipc/bus_handle.rs` | bus send/request/subscribe trait；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE；不得默认成为 Gateway port | INVESTIGATE | D0+CGP-01 | B0+B3+B5+B7 |

### 2.4 IPC envelopes、mailbox、stream 与 transport

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/ipc/envelope.rs` | legacy module endpoint/pattern/payload envelope | Gateway wire compatibility | INVESTIGATE | D0+CGP-02 | B0+B2+B7 |
| `crates/fabric/src/ipc/envelope_v2.rs` | schema/message IDs、routing、correlation、codec envelope v2 | Gateway wire + owner correlation primitives | INVESTIGATE | D0+D1+CGP-02 | B0+B2 |
| `crates/fabric/src/ipc/ipc_msg.rs` | `IpcMessage`/`ForkDirective`/`ForkResult` legacy wire；基线无 Fabric 外 type caller | installed/config/route/migration census 后 DELETE；有 live fork route 才拆 Runtime delegate/Kernel process | INVESTIGATE | D0+RA-05+K5 | B0+B1+B2+B5+B7 |
| `crates/fabric/src/ipc/ipc_types.rs` | legacy numeric AgentId/message/backend traits | Runtime/Gateway transport adapter | INVESTIGATE | D0+RA-01+CGP-02 | B0+B2+B7 |
| `crates/fabric/src/ipc/mailbox.rs` | mailbox/service traits and in-process implementation | Runtime delegation | SPLIT | RA-05 | B0+B1+B3 |
| `crates/fabric/src/ipc/mod.rs` | IPC module/re-exports | Runtime/Gateway/adapter owners | DELETE | D6 | B7 |
| `crates/fabric/src/ipc/protocol.rs` | generic transport protocol trait；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE | INVESTIGATE | D0+CGP-02 | B0+B3+B5+B7 |
| `crates/fabric/src/ipc/stream.rs` | bounded stream, overflow/end policy, schemas | Runtime turn event stream + Gateway projection | SPLIT | RA-02+CGP-02 | B0+B1+B2 |
| `crates/fabric/src/ipc/transport/mod.rs` | legacy transport kind/health/trait；基线无 Fabric 外 production caller | installed/config/route census 后 DELETE；有 live route 才 owner-define 窄 port | INVESTIGATE | D0+CGP-01/CGP-04 | B0+B3+B5+B7 |
| `crates/fabric/src/ipc/transport/unix_socket_transport.rs` | legacy Unix socket `Transport`；基线无构造 caller | installed/config 证据后 DELETE，勿与 daemon socket 混同 | INVESTIGATE | D0+CGP-04 | B0+B2+B5+B7 |

### 2.5 legacy kernel、policy、primitives、protocol 与 security

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/kernel/debug.rs` | debug levels/events/sink port | Observability adapter; Kernel emits facts | SPLIT | K4+CGP-04 | B0+B3 |
| `crates/fabric/src/kernel/debug_bus.rs` | recorder/subscriber/perf implementation | Observability/file adapter | MOVE | CGP-04 | B0+B5 |
| `crates/fabric/src/kernel/error.rs` | cross-domain mega error taxonomy | Owner-specific errors; minimal correlation code only | SPLIT | D1+D6 | B0+B3+B7 |
| `crates/fabric/src/kernel/mod.rs` | misleading legacy kernel module aggregation | Kernel + adapters | DELETE | D6 | B7 |
| `crates/fabric/src/kernel/observable.rs` | generic subsystem status/observable trait | owner health query + Gateway projection | INVESTIGATE | D0+CGP-03 | B0+B3 |
| `crates/fabric/src/kernel/registry.rs` | generic registration ID/registry implementation | Composition-local registry or DELETE | INVESTIGATE | D0+CGP-04 | B0+B7 |
| `crates/fabric/src/lib.rs` | Fabric mega public surface/re-exports | minimal `contracts` root | SPLIT | D1+D6+AK2-25 | B0+B7 |
| `crates/fabric/src/policy/execpolicy.rs` | TOML/program/network rich rules、matching heuristic 与 minimal enforcement facts 混合 | Corpus/config adapter + Kernel minimal verifier facts | SPLIT | D5+APX-04+K2/K3 | B0+B1+B3+B5 |
| `crates/fabric/src/policy/mod.rs` | multi-owner policy aggregation shell | 各 owner module 拆出后 DELETE | DELETE | D6 | B7 |
| `crates/fabric/src/policy/permission_authority.rs` | authority trait 吃 Dasein context/verdict/care score | Dasein policy port；跨 Kernel 时只输出 owner-neutral authorization evidence | SPLIT | D3+K3 | B0+B1+B3 |
| `crates/fabric/src/policy/verifier.rs` | final answer/messages 的 result verifier 与 no-op implementation | Cognit result verifier；Noop caller census 后 DELETE | SPLIT | D0+D2+K7 | B0+B1+B7 |
| `crates/fabric/src/primitives/cognitive.rs` | hypothesis/narrative/commitment domain models | Cognit + Dasein | SPLIT | D2+D3 | B0+B1 |
| `crates/fabric/src/primitives/comm.rs` | zero-behavior Command/Query/Event/Stream/Mailbox markers | DELETE; owner-defined ports replace | DELETE | D6 | B0+B3+B7 |
| `crates/fabric/src/primitives/mod.rs` | rich primitive re-export shell | DELETE；`contracts` modules 按 ownerless proof 重建，不沿用此 shell | DELETE | D1+D6 | B7 |
| `crates/fabric/src/protocol/client.rs` | legacy line JSON-RPC compat 与新 `ClientMessage` protocol 混合 | Gateway protocol + dated legacy compat seam | SPLIT | CGP-01/CGP-02 | B0+B2+B5+B7 |
| `crates/fabric/src/protocol/conscious_core.rs` | conscious-core processor wire ack/snapshot | Gateway projection + Metacog/Dasein owners | SPLIT | D3+CGP-02 | B0+B2+B3 |
| `crates/fabric/src/protocol/extension.rs` | extension/package/connector wire v1 | Gateway protocol + Application extension use case | SPLIT | APX-01+CGP-02+E1 | B2+B3+B6 |
| `crates/fabric/src/protocol/memory.rs` | memory gateway request/response/feedback wire | Gateway protocol; Mnemosyne commands | SPLIT | D4+CGP-02 | B2+B3+B4 |
| `crates/fabric/src/protocol/memory_maintenance.rs` | maintenance status/run/result wire | Gateway protocol; Mnemosyne maintenance | SPLIT | D4+CGP-02 | B2+B3+B4 |
| `crates/fabric/src/protocol/mod.rs` | legacy protocol aggregation shell | DELETE；Gateway protocol surface 重新建立 | DELETE | CGP-02+D6 | B7 |
| `crates/fabric/src/security/audit.rs` | audit record/logger with persistence behavior | Kernel audit port + durable adapter | SPLIT | K1/K4 | B3+B4 |
| `crates/fabric/src/security/circuit_breaker.rs` | `LoopCircuitBreaker`/`LoopVerdict` per-Turn cognition guard | Cognit guard + Runtime turn boundary；caller census 后确定 state owner | INVESTIGATE | D0+D2+RA-02 | B0+B1+B3 |
| `crates/fabric/src/security/loop_detector.rs` | per-Turn loop history/config/metrics，同时直接耦合 circuit breaker、tool-name risk classifier 与 `ToolResult` | Cognit local loop policy + Runtime turn lifecycle + Corpus normalized tool-outcome/risk projection | SPLIT | D0+D2+RA-02+D5 | B0+B1+B3 |
| `crates/fabric/src/security/mod.rs` | security module exports | Kernel/Cognit owners | DELETE | D6 | B7 |
| `crates/fabric/src/security/output_guardrail.rs` | `ToolResult` output validators/normalization；基线 production caller 仅 Corpus runner | Corpus result normalization | MOVE | D5 | B0+B1+B3 |
| `crates/fabric/src/security/policy.rs` | generic `PolicyEngine` 与 execpolicy/Corpus rule engine 语义重叠 | INVESTIGATE merge/delete；不得在 Kernel 复制 generic engine | INVESTIGATE | D0+D5+K3 | B0+B1+B7 |
| `crates/fabric/src/security/risk_classifier.rs` | generic risk class、tool-name classifier rules 与 enforcement 混合 | Kernel generic risk descriptor/enforcement + Corpus capability/tool classifier | SPLIT | K3+D5 | B0+B1+B3 |

### 2.6 types：admission 到 conscious field

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/types/admission.rs` | permit/principal/capability IDs、scope/risk/sandbox/live grants | Contracts IDs + Kernel admission | SPLIT | D1+K2/K3 | B0+B1+B3 |
| `crates/fabric/src/types/agent.rs` | legacy numeric `Pid` | Runtime/Kernel process identity | INVESTIGATE | D0+RA-01+K5 | B0+B7 |
| `crates/fabric/src/types/agent_control.rs` | agent command/query/result/wait DTO 与 size limits | Runtime command/query | MOVE | RA-01/RA-03 | B0+B1+B2 |
| `crates/fabric/src/types/agent_profile_event.rs` | agent profile switch event v1 | Runtime | MOVE | RA-03 | B1+B2 |
| `crates/fabric/src/types/agent_settlement.rs` | agent/resource/background settlement state machine | Runtime + Kernel resource/process | SPLIT | RA-03+K1 | B0+B1+B4 |
| `crates/fabric/src/types/approval.rs` | approval aggregate/status/decision/request IDs | Application approval + Kernel authorization evidence | SPLIT | APX-02+K3 | B0+B1+B3+B4 |
| `crates/fabric/src/types/attempt.rs` | cognitive attempt/runtime IDs, roles, usage/status | Runtime delegation + Cognit run | SPLIT | RA-05+D2 | B0+B1+B4 |
| `crates/fabric/src/types/capability.rs` | capability level/set rich model | Corpus catalog + Kernel descriptor | SPLIT | D5+K2 | B0+B1+B3 |
| `crates/fabric/src/types/change_transaction.rs` | workspace mutation transaction/version/state | Application review + Kernel operation | SPLIT | APX-04+K1 | B0+B1+B4 |
| `crates/fabric/src/types/channel.rs` | external channel/message/action wire model 与 provider translation 混合 | Gateway channel protocol + E2 Gmail/external translation adapter | SPLIT | CGP-02+E2(prep)+E2-K6a(cutover)+XRET-03(drain)+E7(delete) | B0+B2+B3+B6 |
| `crates/fabric/src/types/coding_job.rs` | optional coding use-case DTO、Linux/fs workspace 行为与 runtime/kernel execution state 混合 | Application coding DTO + Linux/fs adapter + Runtime command + Kernel operation | SPLIT | APX-03+RA-01+K1 | B0+B1+B3+B4+B5 |
| `crates/fabric/src/types/cognitive_workflow.rs` | role workflow graph/budget/validation/execution/state；caller、persistence 与 restart ownership 未结案 | INVESTIGATE：有证据的 per-Turn policy 才归 Cognit、delegate lifecycle 才归 Runtime；installed caller 为零则 DELETE | INVESTIGATE | D0+RA-05+D2 | B0+B1+B3+B4+B7 |
| `crates/fabric/src/types/conscious_arbitration.rs` | field arbitration/readout/capability batch authority port | Dasein + Runtime + Kernel | SPLIT | D3+RA-02+K2 | B0+B1+B3 |
| `crates/fabric/src/types/conscious_core.rs` | processor/field/self-view/candidate rich state | Dasein + Metacog | SPLIT | D3 | B0+B1 |
| `crates/fabric/src/types/conscious_core_trace.rs` | conscious trace/acceptance evidence schema | Metacog evidence + Gateway projection | SPLIT | D3+CGP-03 | B0+B2+B4 |
| `crates/fabric/src/types/conscious_field_metrics.rs` | field metric algorithms/history/indicators | Metacog | MOVE | D3 | B1+B4 |

### 2.7 types：context 到 extension

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/types/context.rs` | generic context/trace state | Runtime correlation + owner-specific context | SPLIT | D1+RA-01 | B0+B1 |
| `crates/fabric/src/types/context_budget.rs` | context budget sources/missing reasons/calculation | Cognit | MOVE | D2 | B1 |
| `crates/fabric/src/types/data_governance.rs` | trust/classification/governed content | Application policy + Mnemosyne/Corpus metadata | SPLIT | APX-01+D4+D5 | B0+B3+B4 |
| `crates/fabric/src/types/embodied_episode.rs` | embodied episode/attempt rich model | Robot VLA adapter | MOVE | E5(prep)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B6 |
| `crates/fabric/src/types/embodiment.rs` | device/skill/environment/safety/observation/action DTO | Hardware + Robot VLA adapter | SPLIT | E4/E5(prep)+E4-K6b/E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B3+B6 |
| `crates/fabric/src/types/emergency_stop.rs` | E-stop state/event | Hardware safety owner | MOVE | E4(prep)+E4-K6b(cutover)+XRET-03(drain)+E7(delete) | B1+B6 |
| `crates/fabric/src/types/episode_report.rs` | robot settlement/safe-stop/artifact report | Robot VLA adapter + Hardware receipts | SPLIT | E4/E5(prep)+E4-K6b/E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B3+B6 |
| `crates/fabric/src/types/evaluation/contract.rs` | evaluation subject/threshold/evidence contract | Metacog | MOVE | D3 | B1+B3 |
| `crates/fabric/src/types/evaluation/evidence.rs` | evaluation evidence snapshot | Metacog evidence store | MOVE | D3 | B1+B4 |
| `crates/fabric/src/types/evaluation/mod.rs` | evaluation re-exports/schema/error | Metacog | MOVE | D3 | B2+B7 |
| `crates/fabric/src/types/evaluation/receipt.rs` | evaluation decision/context/receipt/ref | Metacog | MOVE | D3 | B1+B4 |
| `crates/fabric/src/types/evidence.rs` | generic tool-result evidence model | Kernel receipt + Metacog evidence | INVESTIGATE | D0+K4+D3 | B0+B3 |
| `crates/fabric/src/types/exec.rs` | exec terminal/event envelope schema | Kernel operation + Gateway projection | SPLIT | K1+CGP-02 | B0+B1+B2 |
| `crates/fabric/src/types/execution_target.rs` | general/robot execution target selection | Runtime command + Robot adapter | SPLIT | RA-01+E5(prep)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B1+B6 |
| `crates/fabric/src/types/expected_outcome.rs` | Robot expected-outcome contract 与 independent deterministic validator；现有 Cognit provider JSON、Executive/Mnemosyne SQLite reader/writer 必须显式迁移 | Robot VLA hot-path；Cognit/Mnemosyne 与 persistence 只保留边界 translation | MOVE | E5(prep)+D4+APX-04(reader/writer cutover)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B1+B2+B3+B4+B6 |
| `crates/fabric/src/types/extension.rs` | extension identity/origin/constraints/descriptor | Corpus catalog + Application activation | SPLIT | D5+APX-01 | B0+B1+B3 |
| `crates/fabric/src/types/extension_asset.rs` | extension asset/runtime/capability descriptors | Corpus catalog | MOVE | D5 | B1+B3 |
| `crates/fabric/src/types/extension_package.rs` | package manifest/version/permissions/assets | Application extension package + Corpus catalog | SPLIT | APX-01+D5 | B0+B3+B4 |
| `crates/fabric/src/types/extension_state.rs` | activation/health transition state | Application extension activation | MOVE | APX-01 | B1+B4 |

### 2.8 types：external 到 lifecycle

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/types/external_event.rs` | external event v1/v2、mail/file refs 与 dedup | Application external-stimulus + Gmail adapter | SPLIT | APX-03+E2(prep)+E2-K6a(cutover)+XRET-03(drain)+E7(delete) | B2+B3+B4+B6 |
| `crates/fabric/src/types/external_identity.rs` | provider identity/capability/grant state 与 provider translation 混合 | Application identity/grant + external adapter translation | SPLIT | APX-02+E2(prep)+E2-K6a(cutover)+XRET-03(drain)+E7(delete) | B0+B3+B4+B6 |
| `crates/fabric/src/types/external_source.rs` | mail/calendar/file query/result/source DTO | Gmail/Google/external adapters | MOVE | E2(prep)+E2-K6a(cutover)+XRET-03(drain)+E7(delete) | B2+B6 |
| `crates/fabric/src/types/frame.rs` | `FrameRef`/expiry 混合 perception、reasoning 与 embodied observation 语义 | Robot perception + Cognit frame context + Hardware observation；Dasein 只消费 projection | SPLIT | E4/E5(prep)+D2+D3+E4-K6b/E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B1+B3+B4+B6 |
| `crates/fabric/src/types/genome.rs` | topology/identity/boundary/care/memory/mutation rich spec | Dasein + Metacog config proposal | SPLIT | D3 | B0+B1+B4 |
| `crates/fabric/src/types/goal.rs` | goal aggregate/state/budget/snapshot/repository semantics | Application optional Goal Draft; execution via Runtime commands | SPLIT | APX-03+RA-01 | B0+B1+B4 |
| `crates/fabric/src/types/governed_review.rs` | governed review schemas/requests/receipts/evidence | Application review/approval | MOVE | APX-02 | B2+B4 |
| `crates/fabric/src/types/grounding.rs` | vision grounding result/provider/mock | Robot VLA perception adapter | MOVE | E5(prep)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B3+B6 |
| `crates/fabric/src/types/hil_evidence.rs` | human-in-loop evidence/result | Application approval/evidence | MOVE | APX-02 | B0+B4 |
| `crates/fabric/src/types/hook.rs` | hook point/context/result/mode | Corpus hook catalog + Kernel invocation | SPLIT | D5+K2 | B0+B1+B3 |
| `crates/fabric/src/types/hook_ext.rs` | command hook config/result legacy extension | Corpus hook adapter | INVESTIGATE | D0+D5 | B0+B2+B7 |
| `crates/fabric/src/types/inference_receipt.rs` | provider terminal status/receipt schema | Cognit provider port | MOVE | D2 | B1+B2+B4 |
| `crates/fabric/src/types/lifecycle.rs` | generic lifecycle input/effect/rejection state machine | Runtime turn lifecycle + Kernel effects | SPLIT | RA-02+K2 | B0+B1+B3 |

### 2.9 types：LLM 到 process

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/types/llm_types.rs` | messages/tools/stream/model/provider traits and receipts | Cognit provider port + host adapter | SPLIT | D2+CGP-04 | B0+B3+B5 |
| `crates/fabric/src/types/local_authority.rs` | connection/thread/principal/profile/workspace/protected-path policy 与 host path facts 混合 | Gateway/host identity input + Application owner policy + Kernel verifier + Linux path adapter | SPLIT | CGP-01+APX-02+K3+CGP-04 | B0+B1+B3+B5 |
| `crates/fabric/src/types/message.rs` | LLM content/message/role model | Cognit | MOVE | D2 | B1 |
| `crates/fabric/src/types/metacognition_evaluation.rs` | rubric/dimension/gate/evaluation report | Metacog | MOVE | D3 | B1+B4 |
| `crates/fabric/src/types/metacognition_evidence.rs` | evidence IDs/kind/trust/items | Metacog | MOVE | D3 | B1+B4 |
| `crates/fabric/src/types/metacognition_experience.rs` | domain/experience/subject/outcome envelope schema | Metacog | MOVE | D3 | B1+B2+B4 |
| `crates/fabric/src/types/mod.rs` | all rich types mega module/re-export | 各 owner；只重建极小 contracts modules | SPLIT | D1+D6 | B0+B7 |
| `crates/fabric/src/types/model_projection.rs` | model-context projection/fragment receipts/classification | Cognit context projection | MOVE | D2 | B1+B4 |
| `crates/fabric/src/types/network_policy.rs` | concrete URL/host/protocol/port/DNS parse/eval 与 minimal scope bounds 混合；prod callers 在 Corpus web_fetch/search/registry | Corpus/CapabilityExecutor network adapter + Kernel minimal network-scope descriptor | SPLIT | D5+K3 | B0+B1+B3+B5 |
| `crates/fabric/src/types/objective.rs` | objective draft、runtime terminal state 与 storage codec 边界尚未分清 | Application optional Goal Draft + Runtime terminal projection + persistence adapter | INVESTIGATE | D0+APX-03+RA-02 | B0+B1+B2+B4 |
| `crates/fabric/src/types/operation.rs` | operation/process IDs/deadline plus Kernel state/cancel/exit | Contracts IDs/time + Kernel OperationSupervisor | SPLIT | D1+K1 | B0+B1+B4 |
| `crates/fabric/src/types/outcome_verification.rs` | 仅定义 serde `VerificationDecision/VerificationReport`，不含 validator/provider；现有 Cognit/Metacog/Executive/Mnemosyne 只是 producer/consumer 或持久化 reader/writer | Robot VLA report contract；codec/persistence 留在边界 adapter | MOVE | E5(prep; includes consumer translation)+D4+APX-04(reader/writer cutover)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B2+B4+B6 |
| `crates/fabric/src/types/paths.rs` | Linux production roots/runtime environment/path resolver | host/Linux config adapter | MOVE | APX-04+CGP-04 | B0+B5 |
| `crates/fabric/src/types/perception_observation.rs` | perception observation payload | Robot VLA perception adapter | MOVE | E5(prep)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B6 |
| `crates/fabric/src/types/permission.rs` | requested profile、decision、verified effect、prompt config 与 Corpus `PermissionContext`/rules 混合 | Gateway requested profile + Application decision + Kernel verified effect + Cognit prompt config + Corpus permission context/rules | SPLIT | CGP-01+APX-02+K3+D2+D5 | B0+B1+B2+B3 |
| `crates/fabric/src/types/process.rs` | Agent/process/profile/space/mailbox identity models | Contracts IDs + Runtime + Kernel process | SPLIT | D1+RA-01+K1 | B0+B1+B7 |

### 2.10 types：prompt 到 world state

| Current path | 主要 public/type/re-export 语义 | Target owner | Disposition | Slice | Blocker |
|---|---|---|---|---|---|
| `crates/fabric/src/types/prompt_queue.rs` | prompt envelope/state/queue snapshot/limits | Runtime turn queue | MOVE | RA-02 | B1+B4 |
| `crates/fabric/src/types/repository.rs` | repository context/evidence/instructions/VCS/validation/deploy policy | Application coding/review + repository adapters | SPLIT | APX-03/APX-04 | B0+B3+B4 |
| `crates/fabric/src/types/resource.rs` | generic mutable managed-resource lifecycle container | Composition root or owner-local implementation | INVESTIGATE | D0+CGP-04 | B0+B7 |
| `crates/fabric/src/types/robot_audit.rs` | robot audit record；Kernel 只消费 adapter 归一化 operation fact | Robot VLA adapter | MOVE | E5(prep)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B3+B6 |
| `crates/fabric/src/types/robot_failure.rs` | robot failure class/detail | Robot VLA adapter | MOVE | E5(prep)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B6 |
| `crates/fabric/src/types/sandbox.rs` | sandbox policy, command, backend and executor implementation mixed | Kernel descriptor/port + Linux sandbox adapter | SPLIT | K2+K5 | B0+B1+B3+B5 |
| `crates/fabric/src/types/sandbox_glob.rs` | deny-glob filesystem expansion implementation | Linux sandbox adapter | MOVE | K5 | B0+B5 |
| `crates/fabric/src/types/session.rs` | Turn/Item IDs plus Session aggregate/events/store ports | Contracts IDs + Runtime session authority | SPLIT | D1+RA-01/RA-04 | B0+B1+B4 |
| `crates/fabric/src/types/skill_proposal.rs` | Robot skill proposal contract 与 deterministic proposal validation；catalog/evidence 仅 projection | Robot VLA hot-path + Corpus catalog projection + Metacog post-settlement evidence | SPLIT | E5(prep)+D3+D5+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B1+B3+B6 |
| `crates/fabric/src/types/space.rs` | Session/Agora/Space/Memory/Artifact/World projection IDs | Contracts correlation IDs + Agora/Mnemosyne/Runtime owners | SPLIT | D1+D4+RA-01 | B0+B1+B7 |
| `crates/fabric/src/types/time.rs` | `WallTime`/`MonoTime`/`MonoDeadline` value primitives及 host/chrono conversion | Contracts primitives + host time conversion adapter | SPLIT | D1+K1 | B0+B3+B5 |
| `crates/fabric/src/types/tool.rs` | tool context/approval authority/grants/result/trait mixed | Corpus descriptor + Kernel capability + Application approval | SPLIT | D5+K2/K3+APX-02 | B0+B1+B3 |
| `crates/fabric/src/types/tool_stream.rs` | tool progress/event/error channel implementation | Kernel receipt stream + Gateway projection | SPLIT | K4+CGP-02 | B0+B1+B2 |
| `crates/fabric/src/types/turn.rs` | Turn request/state/result/event/metrics/error rich model | Runtime turn authority; Cognit result as owner DTO | SPLIT | RA-01/RA-02+D2 | B0+B1+B4 |
| `crates/fabric/src/types/vision.rs` | image/bounds plus PNG encoding | Robot perception value + media adapter | SPLIT | E5(prep)+E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B3+B6 |
| `crates/fabric/src/types/workspace.rs` | workspace observation/prediction/goal/broadcast rich state | Agora workspace + Cognit projection | SPLIT | D4+D2 | B0+B1+B4 |
| `crates/fabric/src/types/workspace_checkpoint.rs` | checkpoint aggregate/files/restore/integrity | Application workspace adapter + Runtime checkpoint command | SPLIT | APX-04+RA-04 | B0+B1+B4 |
| `crates/fabric/src/types/workspace_identity.rs` | workspace identity/path/device/inode value | Application workspace adapter; optional minimal correlation DTO | INVESTIGATE | D0+D1+APX-04 | B0+B4 |
| `crates/fabric/src/types/workspace_trust.rs` | executable config discovery/trust receipt/decision | Application workspace trust policy | MOVE | APX-02/APX-04 | B1+B3+B4 |
| `crates/fabric/src/types/world_state.rs` | world snapshot 与 async state authority port 混合 | Hardware observation + Robot world model + Metacog evidence/projection | SPLIT | E4/E5(prep)+D3+E4-K6b/E5-K6c(cutover)+XRET-03(drain)+E7(delete) | B0+B1+B3+B4+B6 |

## 3. D0 静态 evidence overlay

本节是逐文件实施证据，不是凭文件名作出的 owner 推测。它由基线 tree 与 `cargo metadata --no-deps` 生成，并与第 2 节按 path 一一对应。当前 overlay 可以启动 D0；它不表示 D1 已放行。

- `P`：可静态归因的 production source caller package 与不同源文件数。扫描 `crates/*/src/**/*.rs`，截断每个文件首个 `#[cfg(test)]` 之后的内容；只接受直接 `fabric::module` 路径，或唯一 public identifier 出现在 `use fabric...`/完整路径中。`∅` 仅表示没有可归因 lexical reference，**不是**“运行时无人使用”。
- package code：`AG=agora`、`CLI=aletheon`、`BA=basic_agent`、`COG=cognit`、`COR=corpus`、`DAS=dasein`、`EVO=evolution_loop`、`EX=executive`、`GW=gateway`、`HW=hardware`、`UI=interact`、`KER=kernel`、`META=metacog`、`MEM=mnemosyne`。
- `E/U/A`：本文件 regex 可见的 public item 总数 / 在 Fabric 内名字唯一的 item 数 / 与其他 Fabric 文件重名的 item 数。它是 caller 归因指纹，不替代 rustdoc/public-surface inventory；`pub use`、宏生成和类型推导必须在 D0 symbol artifact 补齐。
- `IO`：production 段内的边界语法，`S=Serialize/Deserialize surface`、`C=serde_json/bincode/toml codec call`、`W=socket/read_exact/write_all`、`P=filesystem/database syntax`。仅有 `S` 不是 live wire/persistence 的证明。
- `typed R/W`：只记录 caller 中能由显式 generic type 静态归因的 reader/writer；只对带 `B2`/`B4` 的行显示。`R:∅` 或 `W:∅` 是 `D0-IO-UNKNOWN`，必须补 type-inferred call、generic wrapper、schema/version、last reader/writer 后才能进入 D1 或 owner cutover。

`cargo metadata --no-deps` 的 direct manifest dependents 为 `agora, aletheon, basic_agent, cognit, corpus, dasein, evolution_loop, executive, gateway, hardware, interact, kernel, metacog, mnemosyne`。Manifest dependency 只证明 package 可见性，不算 symbol caller；因此 `BA`/`EVO` 未出现在 `P` 中仍需在 D0 确认是 unused dependency 还是 build/cfg 路径。

| Fabric path | P | E/U/A | IO | typed R/W |
|---|---:|---:|---:|---|
| `crates/fabric/src/adapters/mod.rs` | `∅` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/adapters/skill_schema.rs` | `∅` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/contract/command.rs` | `CLI1,EX5,GW3,UI7` | `39/34/5` | `S` | `R:EX1,UI3/W:∅` |
| `crates/fabric/src/contract/mod.rs` | `CLI1,EX5,GW3,UI7` | `0/0/0` | `-` | `R:∅/W:∅` |
| `crates/fabric/src/dasein/context.rs` | `DAS4,META1` | `14/14/0` | `S` | `-` |
| `crates/fabric/src/dasein/event.rs` | `DAS4,META1` | `2/2/0` | `S` | `-` |
| `crates/fabric/src/dasein/mod.rs` | `AG2,COG2,DAS13,EX11,META1` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/dasein/ops.rs` | `EX3` | `1/1/0` | `-` | `-` |
| `crates/fabric/src/dasein/transition.rs` | `AG2,DAS4,EX8` | `20/18/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/dasein/types.rs` | `COG2,DAS6,EX2,META1` | `6/6/0` | `S` | `-` |
| `crates/fabric/src/events/evolution.rs` | `COG2,DAS1,EX1` | `14/13/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/events/mod.rs` | `EX4` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/events/routing_policy.rs` | `∅` | `3/3/0` | `-` | `-` |
| `crates/fabric/src/events/spine.rs` | `COR1,EX24` | `12/10/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/events/subscription.rs` | `∅` | `9/5/4` | `-` | `-` |
| `crates/fabric/src/events/types.rs` | `∅` | `4/3/1` | `S` | `-` |
| `crates/fabric/src/events/ui_event.rs` | `COG3,EX9,UI7` | `15/13/2` | `S,C` | `R:UI1/W:∅` |
| `crates/fabric/src/include/admission.rs` | `EX9,KER6` | `3/3/0` | `-` | `-` |
| `crates/fabric/src/include/agora.rs` | `AG2,COR2,EX9` | `17/14/3` | `S,C` | `R:∅/W:∅` |
| `crates/fabric/src/include/body.rs` | `COG3,COR2` | `5/5/0` | `S` | `-` |
| `crates/fabric/src/include/capability_invoker.rs` | `EX3,KER1` | `1/1/0` | `-` | `-` |
| `crates/fabric/src/include/chronos.rs` | `AG2,COG14,COR25,DAS19,EX83,UI6,KER8,META9,MEM14` | `3/3/0` | `-` | `-` |
| `crates/fabric/src/include/cognit.rs` | `COG9,DAS1,EX4,UI2,META1,MEM7` | `21/18/3` | `S,C` | `-` |
| `crates/fabric/src/include/compaction.rs` | `COG2,COR1,EX3,MEM1` | `9/9/0` | `S` | `-` |
| `crates/fabric/src/include/extension_provider.rs` | `EX3` | `4/4/0` | `-` | `-` |
| `crates/fabric/src/include/memory.rs` | `COG3,EX4,MEM13` | `14/14/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/include/meta.rs` | `META8` | `6/5/1` | `S` | `-` |
| `crates/fabric/src/include/mod.rs` | `AG2,EX5,KER1` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/include/plugin.rs` | `EX1` | `2/2/0` | `-` | `-` |
| `crates/fabric/src/include/process.rs` | `EX2,KER4` | `4/4/0` | `-` | `-` |
| `crates/fabric/src/include/runtime.rs` | `EX1` | `4/4/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/include/self_field.rs` | `COG7,DAS9,EX10,META8,MEM2` | `24/21/3` | `S,C` | `-` |
| `crates/fabric/src/include/space.rs` | `KER2` | `1/1/0` | `-` | `-` |
| `crates/fabric/src/include/subsystem.rs` | `COR1,DAS1,EX6,META2,MEM7` | `7/6/1` | `S` | `-` |
| `crates/fabric/src/include/turn.rs` | `COG5,COR3,EX29,UI6,KER1` | `21/20/1` | `S` | `-` |
| `crates/fabric/src/ipc/backends/io_uring.rs` | `∅` | `4/1/3` | `P` | `-` |
| `crates/fabric/src/ipc/backends/io_uring_transport.rs` | `∅` | `2/1/1` | `-` | `-` |
| `crates/fabric/src/ipc/backends/json_rpc.rs` | `∅` | `3/1/2` | `C,W,P` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/backends/json_rpc_transport.rs` | `∅` | `3/1/2` | `S,C,W` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/backends/manager.rs` | `∅` | `18/14/4` | `P` | `-` |
| `crates/fabric/src/ipc/backends/mod.rs` | `∅` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/ipc/backends/priority_queue.rs` | `∅` | `9/5/4` | `-` | `-` |
| `crates/fabric/src/ipc/backends/shared_mem.rs` | `∅` | `8/6/2` | `-` | `-` |
| `crates/fabric/src/ipc/backends/shared_mem_transport.rs` | `∅` | `2/1/1` | `-` | `-` |
| `crates/fabric/src/ipc/backends/transport_adapter.rs` | `∅` | `3/2/1` | `S,C` | `-` |
| `crates/fabric/src/ipc/backends/unix_socket.rs` | `∅` | `6/1/5` | `S,C,W,P` | `-` |
| `crates/fabric/src/ipc/bus/communication_bus.rs` | `∅` | `19/10/9` | `-` | `-` |
| `crates/fabric/src/ipc/bus/in_process.rs` | `∅` | `10/4/6` | `-` | `-` |
| `crates/fabric/src/ipc/bus/kernel_bus.rs` | `COG1,COR3,DAS1,EX10` | `10/5/5` | `-` | `-` |
| `crates/fabric/src/ipc/bus/mod.rs` | `EX1` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/ipc/bus/pubsub.rs` | `∅` | `2/1/1` | `-` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/bus/request_response.rs` | `∅` | `5/2/3` | `-` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/bus_handle.rs` | `∅` | `1/1/0` | `-` | `-` |
| `crates/fabric/src/ipc/envelope.rs` | `DAS1,EX3,KER1` | `26/19/7` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/envelope_v2.rs` | `COG1,COR3,DAS1,EX26,KER1` | `81/75/6` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/ipc_msg.rs` | `∅` | `10/9/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/ipc_types.rs` | `∅` | `11/9/2` | `S,C` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/mailbox.rs` | `EX2,KER1` | `10/7/3` | `-` | `-` |
| `crates/fabric/src/ipc/mod.rs` | `COG1,COR1,DAS1,EX7,KER1` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/ipc/protocol.rs` | `∅` | `1/1/0` | `-` | `-` |
| `crates/fabric/src/ipc/stream.rs` | `COG2,COR1,EX3` | `21/17/4` | `S,C` | `R:∅/W:∅` |
| `crates/fabric/src/ipc/transport/mod.rs` | `∅` | `4/4/0` | `-` | `-` |
| `crates/fabric/src/ipc/transport/unix_socket_transport.rs` | `∅` | `7/1/6` | `S,C,W,P` | `R:∅/W:∅` |
| `crates/fabric/src/kernel/debug.rs` | `EX7` | `4/4/0` | `S` | `-` |
| `crates/fabric/src/kernel/debug_bus.rs` | `EX6` | `33/30/3` | `S,C,W,P` | `-` |
| `crates/fabric/src/kernel/error.rs` | `COR1` | `34/33/1` | `S` | `-` |
| `crates/fabric/src/kernel/mod.rs` | `EX6` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/kernel/observable.rs` | `UI1` | `2/2/0` | `S` | `-` |
| `crates/fabric/src/kernel/registry.rs` | `COR4,EX3` | `2/2/0` | `-` | `-` |
| `crates/fabric/src/lib.rs` | `∅` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/policy/execpolicy.rs` | `COR2` | `18/14/4` | `S,C` | `-` |
| `crates/fabric/src/policy/mod.rs` | `COG4,DAS1,EX2` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/policy/permission_authority.rs` | `DAS1,EX1` | `1/1/0` | `-` | `-` |
| `crates/fabric/src/policy/verifier.rs` | `COG4,EX1` | `3/2/1` | `-` | `-` |
| `crates/fabric/src/primitives/cognitive.rs` | `EX1` | `4/4/0` | `S` | `-` |
| `crates/fabric/src/primitives/comm.rs` | `∅` | `7/5/2` | `-` | `-` |
| `crates/fabric/src/primitives/mod.rs` | `∅` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/protocol/client.rs` | `CLI4,COR2,EX17,UI22` | `142/137/5` | `S,C` | `R:EX2,UI3/W:∅` |
| `crates/fabric/src/protocol/conscious_core.rs` | `EX1,UI1` | `4/3/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/protocol/extension.rs` | `CLI1,EX4` | `10/9/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/protocol/memory.rs` | `CLI1,EX6,UI1,MEM1` | `44/43/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/protocol/memory_maintenance.rs` | `EX2,UI1` | `14/12/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/protocol/mod.rs` | `CLI3,COR2,EX18,UI19,MEM1` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/security/audit.rs` | `COR1` | `6/5/1` | `S,C,P` | `R:∅/W:∅` |
| `crates/fabric/src/security/circuit_breaker.rs` | `COR1` | `6/2/4` | `-` | `-` |
| `crates/fabric/src/security/loop_detector.rs` | `COR1,DAS1` | `9/6/3` | `S` | `-` |
| `crates/fabric/src/security/mod.rs` | `COR2,DAS2` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/security/output_guardrail.rs` | `COR1` | `7/5/2` | `-` | `-` |
| `crates/fabric/src/security/policy.rs` | `COR1,DAS1` | `6/4/2` | `S` | `-` |
| `crates/fabric/src/security/risk_classifier.rs` | `COR2` | `7/6/1` | `S` | `-` |
| `crates/fabric/src/types/admission.rs` | `CLI1,COR16,EX69,GW3,HW1,UI2,KER6,META1,MEM2` | `28/26/2` | `S` | `-` |
| `crates/fabric/src/types/agent.rs` | `EX1` | `3/2/1` | `S` | `-` |
| `crates/fabric/src/types/agent_control.rs` | `COG1,COR2,EX34,UI1,MEM2` | `55/52/3` | `S,C` | `R:EX1/W:∅` |
| `crates/fabric/src/types/agent_profile_event.rs` | `EX1` | `3/3/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/agent_settlement.rs` | `EX4` | `13/13/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/approval.rs` | `COR1,EX14,GW6,UI1,META1` | `20/15/5` | `S,C` | `R:∅/W:∅` |
| `crates/fabric/src/types/attempt.rs` | `COR1,EX29,GW1` | `12/10/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/capability.rs` | `COR2,EX1` | `12/9/3` | `S` | `-` |
| `crates/fabric/src/types/change_transaction.rs` | `COR5,EX4,UI2` | `18/17/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/channel.rs` | `EX2,GW10` | `10/9/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/coding_job.rs` | `COR1,EX12` | `18/15/3` | `S` | `R:EX1/W:∅` |
| `crates/fabric/src/types/cognitive_workflow.rs` | `AG4,COG3,COR1,EX8` | `52/45/7` | `S,C` | `R:EX1/W:∅` |
| `crates/fabric/src/types/conscious_arbitration.rs` | `COG2,DAS1,EX10` | `12/12/0` | `S` | `-` |
| `crates/fabric/src/types/conscious_core.rs` | `AG1,EX13,UI1` | `15/14/1` | `S` | `-` |
| `crates/fabric/src/types/conscious_core_trace.rs` | `AG1,EX3` | `5/5/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/conscious_field_metrics.rs` | `EX2` | `21/18/3` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/context.rs` | `COG2,COR2,DAS1,EX6` | `6/5/1` | `S` | `-` |
| `crates/fabric/src/types/context_budget.rs` | `EX3,UI2` | `10/8/2` | `S` | `-` |
| `crates/fabric/src/types/data_governance.rs` | `COG1,COR1,EX4,MEM2` | `6/6/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/embodied_episode.rs` | `MEM1` | `2/2/0` | `S` | `-` |
| `crates/fabric/src/types/embodiment.rs` | `COG6,COR2,DAS1,EX14,HW8,UI1,MEM1` | `22/20/2` | `S,C` | `-` |
| `crates/fabric/src/types/emergency_stop.rs` | `HW1` | `2/2/0` | `S` | `-` |
| `crates/fabric/src/types/episode_report.rs` | `COG2,EX5` | `24/20/4` | `S,C` | `-` |
| `crates/fabric/src/types/evaluation/contract.rs` | `COG2,EX16,UI6,META1` | `13/10/3` | `S` | `-` |
| `crates/fabric/src/types/evaluation/evidence.rs` | `EX5` | `5/3/2` | `S,C` | `R:∅/W:∅` |
| `crates/fabric/src/types/evaluation/mod.rs` | `EX4` | `2/2/0` | `-` | `R:∅/W:∅` |
| `crates/fabric/src/types/evaluation/receipt.rs` | `EX12,UI4` | `8/6/2` | `S` | `R:UI1/W:∅` |
| `crates/fabric/src/types/evidence.rs` | `EX2` | `2/2/0` | `S` | `-` |
| `crates/fabric/src/types/exec.rs` | `CLI1,EX2` | `6/4/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/execution_target.rs` | `EX15,GW1,UI7` | `7/6/1` | `S` | `-` |
| `crates/fabric/src/types/expected_outcome.rs` | `COG4,EX3,META1,MEM1` | `10/9/1` | `S` | `R:COG1,EX1,MEM1/W:EX1,MEM1` |
| `crates/fabric/src/types/extension.rs` | `COR7,EX4` | `16/13/3` | `S` | `-` |
| `crates/fabric/src/types/extension_asset.rs` | `COR3,EX1` | `8/8/0` | `S` | `-` |
| `crates/fabric/src/types/extension_package.rs` | `COR4,EX1` | `7/7/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/extension_state.rs` | `∅` | `6/5/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/external_event.rs` | `COR3,EX6,GW1` | `16/14/2` | `S,C` | `R:∅/W:∅` |
| `crates/fabric/src/types/external_identity.rs` | `COR10,EX17` | `12/9/3` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/external_source.rs` | `COR6` | `19/16/3` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/frame.rs` | `COG2,DAS2,HW1` | `4/2/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/genome.rs` | `META9` | `12/12/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/goal.rs` | `EX19,GW3` | `11/7/4` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/governed_review.rs` | `EX4` | `28/25/3` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/grounding.rs` | `COR1,UI1` | `5/5/0` | `-` | `-` |
| `crates/fabric/src/types/hil_evidence.rs` | `META1` | `2/2/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/hook.rs` | `COR6,EX8` | `6/5/1` | `S` | `-` |
| `crates/fabric/src/types/hook_ext.rs` | `∅` | `3/3/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/inference_receipt.rs` | `COG1,EX3` | `4/3/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/lifecycle.rs` | `EX4` | `8/8/0` | `S` | `-` |
| `crates/fabric/src/types/llm_types.rs` | `COG9,COR3,EX37,UI1,MEM1` | `19/18/1` | `S,C` | `-` |
| `crates/fabric/src/types/local_authority.rs` | `CLI1,COR10,EX42,GW2,UI13,KER2` | `25/23/2` | `S,P` | `-` |
| `crates/fabric/src/types/message.rs` | `COG14,COR1,DAS1,EX32,MEM3` | `12/11/1` | `S` | `-` |
| `crates/fabric/src/types/metacognition_evaluation.rs` | `EX4,META5` | `5/5/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/metacognition_evidence.rs` | `COG1,EX6,META2` | `4/4/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/metacognition_experience.rs` | `EX2,META4` | `9/7/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/mod.rs` | `AG1,CLI1,COG7,COR11,DAS2,EX56,HW8,UI1,KER2,META9,MEM4` | `0/0/0` | `-` | `-` |
| `crates/fabric/src/types/model_projection.rs` | `COG1,EX3` | `3/3/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/network_policy.rs` | `COR3,EX4` | `3/3/0` | `S` | `-` |
| `crates/fabric/src/types/objective.rs` | `EX2` | `6/4/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/operation.rs` | `AG4,CLI1,COG6,COR4,EX55,HW1,KER8,MEM4` | `13/10/3` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/outcome_verification.rs` | `COG1,EX4,META1,MEM1` | `2/1/1` | `S` | `R:EX1,MEM1/W:EX1,MEM1` |
| `crates/fabric/src/types/paths.rs` | `CLI1,COG1,COR3,EX14,UI1` | `31/28/3` | `P` | `-` |
| `crates/fabric/src/types/perception_observation.rs` | `COG3,DAS1,EX1` | `2/1/1` | `S` | `-` |
| `crates/fabric/src/types/permission.rs` | `COR3,EX9,GW1,UI3` | `11/9/2` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/process.rs` | `COR2,EX29,KER4` | `19/15/4` | `S` | `-` |
| `crates/fabric/src/types/prompt_queue.rs` | `EX6` | `13/12/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/repository.rs` | `COR3` | `7/7/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/resource.rs` | `∅` | `9/7/2` | `-` | `-` |
| `crates/fabric/src/types/robot_audit.rs` | `∅` | `1/1/0` | `S` | `-` |
| `crates/fabric/src/types/robot_failure.rs` | `COG4,EX1` | `4/2/2` | `S` | `-` |
| `crates/fabric/src/types/sandbox.rs` | `COR10,EX10` | `26/23/3` | `S` | `-` |
| `crates/fabric/src/types/sandbox_glob.rs` | `COR1` | `1/1/0` | `P` | `-` |
| `crates/fabric/src/types/session.rs` | `CLI2,COR2,EX35,UI5` | `23/22/1` | `S` | `R:EX1,UI1/W:∅` |
| `crates/fabric/src/types/skill_proposal.rs` | `COG4` | `4/3/1` | `S` | `-` |
| `crates/fabric/src/types/space.rs` | `AG4,CLI2,COR1,EX39,GW1,UI7,KER2,MEM1` | `15/14/1` | `S` | `-` |
| `crates/fabric/src/types/time.rs` | `AG3,COG12,COR14,DAS10,EX30,HW4,UI6,KER3,META3,MEM11` | `6/5/1` | `S` | `-` |
| `crates/fabric/src/types/tool.rs` | `COG2,COR24,DAS1,EX13,UI3,KER1,MEM1` | `30/29/1` | `S,P` | `-` |
| `crates/fabric/src/types/tool_stream.rs` | `COR9,EX4,KER1` | `21/20/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/turn.rs` | `CLI1,COG7,EX20,UI4` | `8/8/0` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/vision.rs` | `COR1` | `3/3/0` | `S` | `-` |
| `crates/fabric/src/types/workspace.rs` | `AG3,EX25,MEM3` | `35/32/3` | `S,C` | `R:∅/W:∅` |
| `crates/fabric/src/types/workspace_checkpoint.rs` | `EX6` | `12/10/2` | `S,C` | `R:∅/W:∅` |
| `crates/fabric/src/types/workspace_identity.rs` | `EX5,MEM1` | `2/1/1` | `S` | `R:∅/W:∅` |
| `crates/fabric/src/types/workspace_trust.rs` | `EX3` | `10/9/1` | `S` | `R:EX1/W:∅` |
| `crates/fabric/src/types/world_state.rs` | `COG5,EX4,META1,MEM1` | `3/3/0` | `S` | `R:EX1/W:∅` |

### 3.1 COMPAT、SPLIT 内 dated seam 与零 caller adapter 的确定性处理

| Path / seam kind | exact current writer / reader | one-way seam | deadline | exact deletion PR |
|---|---|---|---|---|
| `crates/fabric/src/contract/command.rs`（整文件 `COMPAT`） | request writer：`fabric::protocol::client::ClientRpcRequest::to_json_rpc`，socket writer：`interact::tui::app::submit::write_request`；request reader：`executive::host::daemon::handler::RequestHandler::handle_client_intent`；output writer：`RequestHandler::dispatch_client_intent` 与 legacy cancel arm；output readers：`interact::single_message`、TUI lifecycle loop、`apply_typed_command_output` | legacy Fabric facade `contract::command -> Gateway protocol::command`；`contracts` 不依赖两者，禁止新 `fabric::contract::command` import | CGP-02 完成后、CGP-03 前，且不晚于 2026-10-31 | `CGP-03` |
| `crates/fabric/src/contract/mod.rs`（整文件 `COMPAT`） | `command` 的 last caller 同上；`fabric::contract::envelope_v2` 在基线无外部 direct caller | 仅旧 Fabric facade root 单向 re-export Gateway protocol；零 caller 的 `envelope_v2` re-export 同在 CGP-03 删除 | CGP-02 完成后、CGP-03 前，且不晚于 2026-10-31 | `CGP-03` |
| `crates/fabric/src/protocol/client.rs::{ClientRpcRequest,to_json_rpc}`（`SPLIT` 文件内 legacy half） | request writers：`interact::{single_message,tui::{app,debug,goal,workflow,session_protocol}}` 与 `aletheon::{review_cli,extension_cli}`；reader/dispatcher：Executive daemon `server/RequestHandler` method dispatch。`ClientMessage<ClientRequest/Event>` 是另一条 v1 typed reader/writer，不随 legacy half 删除 | legacy enum/JSON-RPC mapper 只能单向翻译到 `gateway-protocol` typed command；禁止 typed `ClientMessage` 或 `contracts` 反向依赖 `ClientRpcRequest` | CGP-02 typed client/server cutover 后、CGP-03 legacy route drain 前，且不晚于 2026-10-31 | `CGP-03` |

两条整文件 `COMPAT` disposition 与上表前两行严格 `1:1`；第三行是 `SPLIT` 文件内部必须单独计时的 legacy half，不改变 disposition 汇总，但同样受禁止新增 caller、deadline 与 exact deletion PR 门约束。

`IpcBackendAdapter` 不是 compat seam：基线对 `IpcBackendAdapter`、`new` 与任何构造路径的 workspace production caller 都为零，config 只命中 frozen public-type inventory。D0 在这里只是 evidence gate code，不是 PR 编号；该文件的 canonical deletion PR 是 `D6`。

下列 legacy backend/bus 的 exported constructor/type 在整个基线 Rust tree 中均无 Fabric 外引用；但 `P=∅` 不能排除安装态配置、feature、route 或外部加载。D0 evidence gate 必须在已安装二进制、systemd/config、feature matrix 上结案。若仍为零，non-extension 文件统一由 canonical `D6` 删除；若观察到 live load，则本 ledger 先改为真实 owner/adapter 与 writer，并由对应 `RA-*`/`CGP-*` 唯一 owner cutover，再允许 D1：

- `ipc/backends/io_uring.rs`、`io_uring_transport.rs`（仅见 Cargo feature/public inventory）；
- `ipc/backends/shared_mem.rs`、`shared_mem_transport.rs`（源码中 SQLite shared-memory 字样不是这些类型的 caller）；
- `ipc/backends/manager.rs`、`json_rpc.rs`、`json_rpc_transport.rs`、`priority_queue.rs`；
- `ipc/backends/unix_socket.rs`、`ipc/transport/unix_socket_transport.rs`（Executive daemon 的 Unix socket 是另一实现，不能算 caller）；
- `ipc/bus/communication_bus.rs`、`in_process.rs`、`pubsub.rs`、`request_response.rs`、`ipc/bus_handle.rs`；
- `ipc/ipc_msg.rs` 的 `IpcMessage`/`ForkDirective`/`ForkResult`、`ipc/protocol.rs` 与 `ipc/transport/mod.rs`。

`CanonicalEventBus` 另行重判：它是 non-durable broadcast notification，不是 EventSpine/journal，不能提供 replay、terminal 或 permission evidence。只有 `RA-02` notification projection 的 caller 仍需它时才以新名字保留为可替换 adapter，否则在同一个 canonical `RA-02` caller-cutover PR 删除；不得另造删除编号。

### 3.2 D0 implementation artifact 与 hard gate

D0 的第一笔实现 PR 必须生成并提交 `config/architecture/fabric-boundary-census.tsv`，每个 public symbol 一行；没有 public symbol 的模块用 `symbol=-` 保留 path 行。最小字段固定为：

```text
path  symbol  kind  prod_caller_path  prod_caller_symbol  cfg
wire_role  persistence_role  schema_or_format_version
last_writer  last_reader  target_owner  cutover_pr  deletion_pr  deadline  evidence_commit
```

D0 只有同时满足以下机械条件才可关闭：

1. baseline Fabric path、逐文件 ledger path、overlay path、symbol artifact path 四个集合均为 `174/174/174/174`，missing/extra/duplicate 全零；
2. 每个 `P=∅` 路径都有 `no-prod-caller` 与 installed/config 证据，不能直接据本 overlay 删除；
3. 每个 `A>0` 的 symbol 以 fully-qualified path 或 compiler/rustdoc evidence 消歧；
4. 每个带 `B2`/`B4` 的路径均有确切 last writer、last reader、版本/格式、迁移/回滚；任一 `typed R/W` 的 `∅` 在 artifact 中未关闭即阻断对应 owner slice 与 D1；
5. 每个 `COMPAT` 行均有单向 seam、禁止新增 caller gate、日历 deadline 与 exact deletion PR；每个 `DELETE` 行均有 production + installed/config caller 清零证明；
6. `contracts` candidate 逐 symbol 证明 ownerless value semantics；任何 codec、repository、policy、live permit、state machine、UI/wire model 进入 `contracts` 都使 D1 失败。

因此，本 ledger 的准确状态是：**D0 implementation-ready，D1 blocked until evidence closure**。不能把本节的 lexical census 宣称为动态 caller 真相。

Deletion ownership 严格串行且唯一：`XRET-03` 只完成 preserved-extension caller/re-export drain；`E7` 删除 Gmail/Hardware/Robot 等 extension-specific Fabric 行与 re-export；随后 `D6` 才删除 non-extension Fabric 行并唯一收掉空 root。`RA-*`/`CGP-*` 若在自己的 caller-cutover PR 同时删除 owner-specific seam，D6 只验证其已清零，不能重复拥有删除。

## 4. 机械完整性与硬门

生成本 ledger 后必须以基线 tree 重新执行集合核对：

```text
actual = sorted(crates/fabric/src/**/*.rs)
ledger = sorted(section 2 Current path column)
overlay = sorted(section 3 Fabric path column)

actual_count == 174
ledger_count == 174
overlay_count == 174
unique_ledger_count == 174
unique_overlay_count == 174
missing == 0
extra == 0
duplicate == 0
```

本版机械解析结果：

```text
actual=174, ledger=174, overlay=174
unique_ledger=174, unique_overlay=174, sets_equal=true
missing=0, extra=0, duplicate=0
CONTRACTS=0, MOVE=51, SPLIT=72, COMPAT=2, DELETE=12, INVESTIGATE=37
P_empty=33, ambiguous_export_paths=108
B2_or_B4_paths=72, typed_rw_unknown=70, typed_rw_closed=2
```

任何后续新增 Fabric 文件都必须同时更新本 ledger，且不能让 `INVESTIGATE` 在 D0 后继续作为迁移借口。

硬门：

1. **D1 前置门**：每个 `CONTRACTS` 或 `SPLIT -> Contracts` 项都要证明是 ownerless value primitive；不能含 repository、live permit、policy evaluator、codec/backend、UI model 或领域状态机。
2. **单 owner 门**：MOVE/SPLIT 的 rich model 必须先有唯一 writer/authority，不能复制到目标 crate 后保留 Fabric 第二 writer。
3. **wire/persistence 门**：所有 `COMPAT`/wire/persistence 项必须有版本矩阵、双读或迁移回滚、production caller census 与删除日期。
4. **EventSpine 门**：Runtime EventSpine 是 durable canonical ordering/journal；Gateway transport 与 Runtime notification bus 只能 project/broadcast。当前 `CanonicalEventBus` 必须 rename 为非权威 notification adapter 或删除，不能提供 replay、terminal、permission evidence；Kernel 只追加 operation facts，不再出现第二个 canonical event authority。
5. **AK2-25 最终门**：只有在 D1 收缩完成、所有 rich model/ports/adapters 已迁出、`fabric::` production caller 与兼容 re-export 清零、D1 architecture gate 通过后，才允许做 `fabric -> contracts` 的机械 rename。禁止先 rename 再慢慢清理。

提交前必须重新生成核对结果；若 baseline/ledger/overlay 不是 `174/174/174`、任一集合差非零，或 disposition 总和不是 `174`，本 ledger 不可进入实施阶段。`typed_rw_unknown=70` 是显式 D0 工作量，不得在 D1 前保持非零；本版闭合的 2 条也必须在 implementation artifact 复核，不能把 lexical 命中当动态真相。
