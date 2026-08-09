# Kernel 执行强制边界收敛计划

> 日期：2026-08-08
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
> 上位计划：`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`
> 状态：可执行迁移计划；本文不修改生产代码
> 代号：`K`（Kernel Enforcement）

## 1. 结论

当前 Kernel 不是完全没有实现，而是“有表、有机制、没有不可绕过的边界”：`KernelRuntime` 管 Process、Operation、Admission、Budget、Lease、Mailbox、Space 和 Supervision，但真正的 capability authority、approval、tool pipeline、Hardware executor、execd 选择和部分 settlement 仍在 Executive 中组装。

本轮不再给 Kernel 增加业务职责。目标是把它变成一个小而硬的 execution enforcement boundary：任何 Agent-requested effect 都只能经由同一条 `begin -> verify -> admit -> invoke -> receipt -> settle/revoke` 路径，并且不能由调用者伪造 permit、替换 executor 或绕过 journal。

最终约束：

1. Kernel 只拥有 `ExecutionProcess`、`Operation`、permit/grant、budget、lease、deadline、
   cancellation、effect dispatch、usage accounting、receipt 和 recovery；
2. Runtime 拥有 Agent/AgentRun/Session/Turn/Delegate，Kernel 不理解这些 aggregate；
3. Application 拥有人类 Approval resolution aggregate，Kernel 不反向依赖 Application；
4. Corpus/extension 拥有 capability 的业务 schema，Kernel 只保存 enforcement descriptor；
5. Linux/execd 只实现 Kernel 定义的窄 `ProcessController`/execution adapter；
6. `DecisionRequestId`、correlation ID 或普通 DTO 永不等于授权；
7. Kernel 不成为第二个 Runtime，也不保留只有 getter 的 God object；
8. Fabric 最终重命名为 `contracts`，本文不使用、也不引入 `ABI` 命名。
9. V2 固定保留小型独立 `kernel` crate；是否物理合并只能在 V2 稳定后另开 ADR，本次迁移不把边界决定拖到 Executive 删除阶段。

## 2. 基线源码证据

以下结论以指定 dev commit 的生产源码为准，不以目录名推断 owner。

| 当前路径 | 当前事实 | 耦合/风险 |
|---|---|---|
| `crates/kernel/src/runtime.rs` | `KernelRuntime` 同时持有 space、process、operation、supervisor、mailbox、admission、budget、lease 和多组 registry/map | 组件多且公开 getter；内存状态无法独立恢复，容易成为第二个 Runtime |
| `crates/kernel/src/admission/production.rs` | `ProductionAdmissionController` 管 active/settled permit、budget、lease 和 sandbox availability | permit 生命周期存在，但 authority evidence、executor binding 和 durable journal 不完整 |
| `crates/kernel/src/capability/mod.rs` | `DefaultCapabilityInvoker` 执行 admit、executor、settle/revoke | 调用者传入 `ToolExecutor`；没有 sealed descriptor-to-executor binding |
| `crates/kernel/src/operation/table.rs` | Operation 状态在进程内 table 中推进 | crash 后不能仅凭权威 journal 判断 dispatch/terminal |
| `crates/kernel/src/process/table.rs` | process identity、signal、terminal 管理位于内存表 | OS child 与逻辑 process 的重连、PID generation 和 receipt 边界不够明确 |
| `crates/executive/src/application/governed_capability.rs` | Executive 构造 canonical invoker、permit issuer、authority context 和 action loop | Kernel enforcement 的入口与 authority attachment 实际由 Executive 拥有 |
| `crates/executive/src/application/embodiment_authority.rs` | Hardware executor 实现 `ToolExecutor` 并由 Executive 绑定 admission | Hardware safety、Kernel permit 和 composition glue 混在 Application 目录 |
| `crates/executive/src/core/permission_manager.rs` | Dasein review 规则被搬到 Executive `PermissionManager` | 领域判断与 execution enforcement owner 不清晰 |
| `crates/executive/src/host/daemon/bootstrap/approval_gate.rs` | socket approval、repository、session grant cache 同时参与决策 | Approval aggregate 可被误当 Kernel authority；core 反向读取 Application 状态 |
| `crates/executive/src/host/daemon/bootstrap/security.rs` | daemon bootstrap 选择 execd、构造 sandbox、audit 和 approval gate | host composition 直接形成另一条 tool security path |
| `crates/executive/src/host/daemon/handler/tool_executor.rs` | `TurnToolExecutor` 混合 hook、Corpus、Self、session approval、storm breaker、perf 和 Kernel IDs | 单个工具调用同时承载业务编排、policy 和 effect，无法证明不可绕过 |
| `crates/executive/src/adapters/channel/execd_client.rs` | Executive 直接 spawn execd、握手、RPC、kill、structured tool fallback policy | Linux process adapter 与业务 tool dispatcher 混合，且可形成独立执行入口 |

现有 Kernel 中值得保留的是 admission、budget、lease、operation/process lifecycle 的有效不变量；需要删除或外迁的是 Agent identity、mailbox routing、业务 policy、provider/LLM、presentation 和任意 owner-specific store。不能把 Executive 对应目录整块复制到 Kernel。

## 3. 目标边界

```text
Runtime Turn/Delegate command
  -> Kernel EffectPort
  -> begin Operation
  -> resolve sealed EnforcementDescriptor
  -> verify optional authorization evidence
  -> admit budget/lease/deadline/sandbox
  -> dispatch bound CapabilityExecutor
  -> append dispatch/usage/final receipt
  -> settle or revoke
  -> Runtime receives immutable OperationReceipt reference
```

Kernel 的 public surface 只暴露意图级 command 和不可变 query/receipt：

```text
kernel/
├── command.rs              # BeginEffect / CancelOperation / ReconcileOperation
├── query.rs                # OperationSnapshot / Receipt lookup
├── operation.rs            # 唯一 Operation 状态机与 epoch fence
├── admission.rs            # permit、budget、lease、deadline、sandbox enforcement
├── authorization.rs        # evidence verifier 与 opaque single-use grant
├── descriptor.rs           # enforcement-only descriptor 与 registry revision
├── executor.rs             # CapabilityExecutor port 和 sealed binding
├── journal.rs              # ExecutionJournal port、event 与 receipt
├── resource.rs             # ResourceLedger：budget/quota/lease reservation 与 accounting
├── recovery.rs             # replay、dispatch reconciliation、indeterminate handling
├── process.rs              # ExecutionProcess semantics；不含 Agent semantics
└── ports.rs                # ProcessController、TimeSource、Timer 等窄 port
```

`KernelRuntime` 最终改为 composition-private handle，或拆成上述 command/query service；禁止继续公开 `process_table()`、`budget_controller()`、`lease_manager()` 等 component getter。调用方不能拿到 admission controller 后自行组装另一条 invoke path。

## 4. Kernel 拥有与明确不拥有

| Kernel 唯一拥有 | Kernel 明确不拥有 |
|---|---|
| Operation ID、epoch、state transition、terminal fence | Agent、AgentRun、Session、Turn、Delegate 状态机 |
| opaque permit/grant 的 mint、consume、revoke | Goal、plan、priority、salience、Self、Metacog 语义 |
| descriptor digest、executor binding、registry revision | capability 完整业务 schema、prompt、tool UX |
| budget、quota、lease、deadline、cancellation | provider selection、LLM request/response、`InferencePort` 实现 |
| dispatch fence、usage accounting、receipt、recovery | TUI、CLI、Gateway、RPC、Approval 展示 |
| ExecutionProcess 与 OS child reference/generation | Application `ApprovalStore`、Runtime journal、domain repository |

Kernel 不允许依赖 Runtime、Application、Cognit、Dasein、Metacog、Agora、Mnemosyne、TUI、Gateway、Gmail、Robot 或具体数据库 crate。Kernel port 的 SQLite/Linux 实现由 adapter crate 提供，并在 `aletheon` composition root 注入。

Native cognition 通过 Cognit-owned `InferencePort` 完成单 Turn 推理；Pi 作为 Runtime-owned `DelegateBackend`，仅把 child process spawn/cancel 交给 Kernel。二者都不进入 Kernel 业务模型。

## 5. Descriptor 到 executor 的不可替换绑定

Corpus、Gmail outbound、Hardware bridge、Robot skill 和其他保留扩展在 bootstrap 注册：

```text
EnforcementDescriptor
+ ExecutorRegistrationId
+ CapabilityExecutor
```

`EnforcementDescriptor` 只含强制执行所需字段：capability key/version、risk class、scope bounds、sandbox requirement、budget/lease class、timeout/output limit、authority rule 和 executor kind。完整 JSON schema、领域 validation 和展示文案仍由 capability owner 持有。

注册规则：

- 只有 `aletheon` composition root 能在 serving 前注册；
- registry 完成后 seal，运行期调用方不能插入 trait object；
- duplicate key/version、缺失 executor 或 descriptor digest 不一致时 fail closed；
- permit 绑定 `descriptor_digest + executor_registration_id + registry_revision +
  operation_epoch + invocation_digest`；
- 活跃 Operation 的 entry 不可热替换；升级创建新 revision；
- `CapabilityExecutor` 只接收 Kernel 已验证的内部 invocation/grant，不接收可公开构造的
  `ExecutionPermit`；
- executor 返回 bounded raw outcome/usage，Kernel 写 final receipt；executor 不能自行 settle。

因此 `crates/executive/src/application/governed_capability.rs` 的 canonical invoker 和 `crates/executive/src/host/daemon/handler/tool_executor.rs` 不能整块迁入。前者拆成 Kernel enforcement、Runtime context attachment 和领域 action observation；后者的 hook/Self/Corpus 逻辑回各 owner，真正 effect dispatcher 只实现 `CapabilityExecutor`。

## 6. Authorization evidence 与 Application 解耦

Application 继续拥有 `ApprovalRequest/ApprovalResolution` aggregate、`ApprovalStore`、人类展示和 resolve use case。Kernel 只拥有：

```text
trait AuthorizationEvidenceVerifier {
    verify(challenge, evidence, invocation_digest, principal, scope, now)
        -> OpaqueAuthorizationProof
}
```

outer adapter 可以查询 Application `ApprovalStore` 和 challenge owner evidence，但只能向 Kernel 返回经过验证的 opaque proof。Kernel core 不 import Application 类型、不打开 `ApprovalStore`、不订阅 UI approval event。

`DecisionRequestId` 只是关联 ID，不授权。Application resolution 也不能直接当 permit。
Kernel verifier 至少校验：

- challenge 确实由等待该决定的 authority owner 持久化；
- `DecisionRequestId + action/invocation digest + principal + scope` 完全匹配；
- Runtime generation/领域 revision 与发起时绑定一致；
- evidence 未过期、未撤销、nonce 未消费；
- resolution 类型允许该 operation，且没有扩大 scope；
- verifier 返回的 proof 只能在同一 Kernel operation epoch 消费一次。

Kernel 随后内部 mint opaque、不可 serde、single-use grant。失败、重试、超时或进程重启都不复用旧 grant；需要重新验证 evidence。Dasein 若等待 Self 决策，使用 Dasein 自己拥有的 verifier 和 grant，不借 Kernel grant 代替领域 authority。

Application 的 typed `ResumeWithDecision` 只把 resolution evidence 送回原 challenge owner；它不调用 Kernel 内表，不将 `DecisionRequestId` 转成 capability authority。这样 Application 可关闭，而 Kernel/Runtime 仍保留内部受信 command 的 enforcement 路径。关闭 Application 时不存在人类 resolver：静态 Owner Manifest 已预授权且只读/低风险的受信路径可继续；任何需要新的人类 authority evidence 的高风险请求必须 fail closed，返回 typed `AuthorityUnavailable` 并进入 durable `WaitingForAuthority`。该等待必须带 deadline、可取消；deadline 到达后只能终结为 typed `TimedOut`，显式取消只能终结为 `Cancelled`，不得永久 hanging，也不得退回默认 Allow。

## 7. Operation journal、receipt 与恢复

`ExecutionJournal` 是 Kernel 定义的 durable port，每个 Operation stream 至少写入：

```text
OperationBegun
DescriptorResolved
AuthorizationVerified | AuthorizationRejected
ResourcesReserved
DispatchPrepared
DispatchObserved
UsageObserved
ReceiptFinalized
OperationSettled | OperationRevoked | ReconciliationRequired
```

关键不变量：

- `DispatchPrepared` durable 后才能调用 executor；
- 每个 `operation_id + epoch` 最多一次 active dispatch；
- receipt 含 descriptor/invocation digest、executor revision、authority class、resource refs、
  mediation level、bounded output/artifact refs、usage 和 terminal status；
- 一个 epoch 只能有一个 finalized receipt 和一个 accounting terminal；
- late outcome 只追加 observation，不改写既有 terminal；
- crash 位于 prepared/observed 之间时必须 reconcile，不能凭 timeout 宣称 failed 或 success；
- opaque external effect 无法确认时标记 `Indeterminate`，由 owner 决定补偿，禁止自动重放；
- `ExecutionJournal` 与 `RuntimeJournal` 不假装跨库原子事务，二者以 binding ID、digest、
  causation 和 idempotency 做 saga reconciliation。

SQLite adapter 可以先复用旧物理表，但 schema/migration bundle/writer 归 Kernel；repository `open` 不执行 migration。`aletheon migrate` 是唯一 migration composition owner。Kernel 不直接依赖 `rusqlite`，也不把 receipt 当普通 event bus notification。

## 8. Linux、execd 与 process adapter 边界

Linux 解耦的可借鉴点是稳定机制边界，而不是把所有功能放入“内核”。目标端口：

```text
trait ProcessController {
    spawn(SealedProcessSpec, ProcessLease) -> OsProcessBinding;
    inspect(binding, generation) -> ProcessObservation;
    signal(binding, generation, signal) -> ProcessObservation;
    reconcile(binding, generation) -> ProcessObservation;
}
```

`adapters/linux-process` 负责 `fork/spawn`、process group、stdio、signal、PID/generation 防复用、filesystem namespace 和本机 sandbox。execd 若保留独立进程隔离价值，则成为该 port 的远程实现；若审计证明没有独立价值，则并入 Linux adapter，但不改变 Kernel port。

`crates/executive/src/adapters/channel/execd_client.rs` 迁移时必须拆开：

- handshake/RPC/process read/kill/reconnect -> execd transport adapter；
- workspace root canonicalization -> Linux/filesystem adapter；
- structured tool name/JSON mapping -> 对应 `CapabilityExecutor` adapter；
- fallback/approval/sandbox policy -> Kernel descriptor/admission，不留在 client；
- binary path、secret、enable flag -> `aletheon` composition/config；secret 不进 argv、event、prompt；
- execd 失败禁止回退到 in-process ungoverned execution。

Kernel 只记录 OS binding/reference 和 receipt，不保存完整 stdout 缓冲或 TUI stream。Pi child 仍由 `DelegateBackend` 管协议与 terminal interpretation；Kernel 只治理它的 process lease、deadline、cancel 和 process-level receipt，不虚假声称可观察 Pi 内部所有 effect。

## 9. K/M/D disposition ledger

| 当前源码 | 决定 | 目标/删除条件 |
|---|---|---|
| `kernel/src/admission/**` | **KEEP+MIGRATE** | 合入唯一 admit/grant/settle path；去除公开可伪造 permit |
| `kernel/src/operation/**` | **KEEP+MIGRATE** | 唯一 Operation aggregate + ExecutionJournal；内存 table 降为 projection/cache |
| `kernel/src/process/**` | **KEEP+MIGRATE** | 改称 ExecutionProcess；OS 操作只经 ProcessController |
| `kernel/src/runtime.rs` | **SPLIT+DELETE FACADE** | command/query/recovery 分拆；调用者清零后删除 component getters |
| `kernel/src/admission/{budget,lease}.rs` | **KEEP+PORT** | 不变量留 Kernel；durable adapter 外置 |
| `kernel/src/capability/mod.rs` | **KEEP+REWRITE** | `DefaultCapabilityInvoker/ToolExecutor` 收敛为 sealed `CapabilityExecutor` 单一路径；调用者不可注入任意 executor |
| `kernel/src/chronos/system_clock.rs`、`chronos/timer.rs` | **KEEP+PORT** | production `SystemClock/SystemTimer` 变为 `TimeSource/Timer` adapter；wall/monotonic 语义显式分开 |
| `kernel/src/chronos/mod.rs` | **MERGE** | 收缩为 Kernel time ports 的窄 export，不充当共享 utils |
| `kernel/src/chronos/**` 中 `TestClock/TestTimer` | **FIXTURE/REPLACE** | 只保留保护 deadline/expiry/lease 的最小 deterministic fixture，不迁整套旧测试 |
| `kernel/src/space/**` | **INVESTIGATE** | 仅有真实 isolation/resource consumer 才留；不得变 workspace/session owner |
| `kernel/src/supervision/**` | **INVESTIGATE+SPLIT** | OS execution supervision 留；Agent/Delegate restart 归 Runtime |
| Kernel mailbox routing | **MIGRATE/DELETE** | Agent mailbox 归 Runtime；仅执行控制 channel 有独立必要性才留 |
| `kernel/src/lib.rs` | **REWRITE FACADE** | 只导出 command/query/receipt 与窄 ports；删除 component getter、fixture 和 legacy invoker 的 production re-export |
| `executive/application/governed_capability.rs` | **SPLIT+DELETE** | enforcement 归 Kernel；action loop/context 回 Runtime/Dasein/Agora |
| `executive/application/embodiment_authority.rs` | **SPLIT+DELETE** | Hardware executor adapter + Kernel binding；active device op 归 Hardware |
| `executive/core/permission_manager.rs` | **DELETE AFTER MERGE** | care/confirmation rule 回 Dasein；effect scope enforcement 归 Kernel |
| `executive/host/daemon/bootstrap/approval_gate.rs` | **SPLIT+DELETE** | Application Approval adapter 与 Kernel/Dasein verifier adapter 分离 |
| `executive/host/daemon/bootstrap/security.rs` | **SPLIT+DELETE** | composition 到 `aletheon`；Linux/execd、audit、Kernel registry 分别注入 |
| `executive/host/daemon/handler/tool_executor.rs` | **SPLIT+DELETE** | hook/Self/Corpus/effect 分 owner；旧第二 pipeline 调用者清零 |
| `executive/adapters/channel/execd_client.rs` | **MOVE+SPLIT** | execd process/transport adapter；业务 tool mapping 进入具体 executor adapter |

`INVESTIGATE` 不是永久保留。K0 必须给出 production caller、受保护不变量和替代成本；若没有第二个真实 consumer 或不可替代机制，就删除，而不是为了“Kernel 看起来完整”继续扩张。

## 10. 分 PR 实施顺序

```text
K0 census + bypass inventory + boundary gates
 -> K1 Operation/Journal contracts + command/query seam
 -> K2 sealed descriptor/executor registry
 -> K3 authorization evidence + opaque single-use grant
 -> K4 admission/receipt/recovery single path
 -> K5 Linux/execd ProcessController cutover
 -> K6 Kernel seam/gates ready; E2/E4/E5/E6 own extension cutovers
 -> K7 Executive path deletion + Kernel surface contraction
```

### K0：证据清单与非增长 gate

- 列出所有 direct `ToolExecutor`、`AdmissionController`、permit constructor、OS spawn、execd、
  approval cache 和 operation terminal 调用者；
- 给每条 effect 标明 authority owner、executor、journal writer、recovery owner 和旧删除 PR；
- 冻结 Executive 新 capability path；未知路径 fail review，不自动迁入 Kernel；
- 产物无行为变化，回滚仅删除 inventory/gate。

### K1：Operation authority 与 durable seam

- 定义 Kernel-owned command/query、Operation state/epoch、ExecutionJournal 和 receipt；
- 旧 Kernel table 通过 adapter 写同一 authority stream，禁止新旧双 writer；
- Runtime 只保存 Operation binding/receipt ref，不复制 Kernel terminal；
- rollback 保持 legacy-authoritative，V2 只 shadow replay/read compare。

### K2：sealed descriptor 与 `CapabilityExecutor`

- composition root 建 registry、校验 duplicate/version/digest 并 seal；
- 先接一个无外部副作用的 representative executor，再接 Corpus/Gmail/Hardware/Robot；
- 删除 Runtime/Executive 传入 arbitrary executor 的能力；
- rollback 切回整条 legacy invoker，不允许 descriptor 新、executor 旧的混合路径。

### K3：authorization evidence verifier

- 定义 opaque proof、nonce/expiry/scope/digest/generation 校验与 single-use store；
- Application resolution adapter 只实现 verifier port，不暴露 repository 给 Kernel；
- `DecisionRequestId` 静态/契约 gate 禁止直接构造 permit；
- verifier 不可用时 fail closed；回滚不复用已消费 evidence。

### K4：单一 admit/invoke/receipt/recovery 路径

- 把 budget、lease、deadline、sandbox、dispatch fence、usage 和 receipt 串成唯一状态机；
- prepared/observed crash window 有 explicit reconciliation；
- 删除 standalone permit issuer/settler 和公开 component getter；
- 每个 operation generation 只切一次 writer/executor，禁止 shadow 真实 effect。

### K5：Linux/execd adapter

- 引入 `ProcessController`，迁 process group、signal、stdio、reconnect 和 PID generation；
- execd 无 silent in-process fallback；installed binary 下验证 spawn/read/cancel/restart；
- 旧 execd client 保持 authoritative 到新 adapter smoke 通过，然后单向切换；
- rollback 只切 binary/config，保留新 journal/receipt 并 reconcile。

### K6：Kernel seam/gates 就绪，扩展联合切换

- 本阶段只交付稳定 Kernel enforcement seam、per-family registration gate、writer/executor uniqueness gate 和 cutover runbook，不另建第二组扩展迁移 PR；
- Gmail outbound 由唯一 `E2-K6a` PR 切换，Hardware 由 `E4-K6b`、Robot VLA 由 `E5-K6c`、Pi `DelegateBackend` child 由 `E6-K6d` 切换；联合编号表示同一个 PR 同时满足两份计划，不是两次切流；
- 每个唯一 owner PR 先 drain active operation，再记录 watermark/registry revision/authority generation，并完成 end-to-end writer/executor cutover；
- Hardware veto、Gmail outbox、Pi terminal evidence 的领域 owner 不被 Kernel receipt 替代；一个分片失败不以关闭功能通过验收，退回该分片 legacy-authoritative。

### K7：删除 Executive path 并收缩 Kernel

- production caller census 为零后删除 governed capability、tool executor、approval/security glue；
- 删除公开 permit 构造、parallel settlement、direct process spawn 和第二 registry；
- `KernelRuntime` getter surface 清零，space/mailbox/supervision 未证明部分删除；
- 保持 V2 独立小型 `kernel` crate，只收缩 public surface；未来物理形态变化必须另开稳定后 ADR，不得在本阶段合并进 Runtime。

## 11. Writer cutover 与回滚协议

每个 effect family 只允许以下三态：

| 模式 | writer/executor | V2 行为 |
|---|---|---|
| `legacy-authoritative` | Executive 旧 path | inventory/read-only compare |
| `v2-shadow` | Executive 旧 path | replay/descriptor/evidence compare；禁止 dispatch、reserve、settle |
| `v2-authoritative` | Kernel V2 | 唯一 reserve/dispatch/receipt/settle；旧 facade 单向调用 V2 |

切换顺序固定为：maintenance/drain -> 记录 journal watermark 与 active operation -> reconcile 或 cancel -> seal registry revision -> 启新 binary -> 验证唯一 writer/executor -> 恢复流量。

回滚原则：

- 未 dispatch 的 Operation 可撤销并退回旧 binary；
- 已 dispatch 的外部 effect 不回滚到会遗失 receipt/outbox 的旧数据快照；
- 保留新 journal，旧 binary 只在 compatibility 声明允许时读，不能成为第二 writer；
- `Indeterminate` 必须人工/owner reconciliation，不自动重复 Gmail、Robot、Hardware 等 effect；
- registry revision、migration checksum、cutover generation 和 rollback binary range 必须记录。

## 12. 最小验证与架构 fitness gates

本计划不搬运 Executive 巨量测试，也不按文件数量制造测试。只保留能证明边界的最小集合。

契约/不变量检查：

- 无 grant/permit、digest/scope/generation 不匹配、expired/replayed evidence 均 fail closed；
- descriptor seal 后不能替换 executor，duplicate registration 启动失败；
- 每个 Operation epoch 最多一次 dispatch、一次 finalized receipt、一次 accounting terminal；
- cancel/timeout/crash 释放 budget/lease，late outcome 不改写 terminal；
- process signal 校验 binding generation，防 PID reuse；
- Application core 不被 Kernel import，`DecisionRequestId` 不能直接授权。

installed smoke：

- 正式安装态 Native tool：允许、拒绝、cancel 各一条；
- execd/Linux：spawn/read/timeout/kill/daemon restart reconcile；
- Pi：`DelegateBackend` spawn/wait/cancel，验证 Kernel 只治理 process-level effect；
- Gmail outbound 或 filesystem write 选一条具 outbox/receipt 的幂等 effect；
- Hardware 用 simulator 验证 Kernel permit 不能覆盖 Hardware veto/safe-stop。

静态架构 gate：

- Kernel 禁止依赖 Runtime/Application/Cognit/领域/UI/provider/database crate；
- Runtime/extension 禁止直接 import OS process、公开 permit constructor 或 admission internals；
- 生产代码中 direct `ToolExecutor`/`AdmissionController` 组装点只能在 Kernel/composition allowlist；
- `rusqlite`、filesystem、process imports 服从 owner/adapter inventory 且 non-increasing；
- 新增 Kernel public getter、业务 DTO、Session/Turn/Agent type 或 provider/LLM type 直接拒绝。

## 13. 跨计划依赖与完成定义

| 本计划阶段 | 必须依赖 | 提供给后续 |
|---|---|---|
| K0-K1 | Runtime RA-00/RA-01 的 ID/binding 语义；Application APX-00 storage census | Operation/receipt authority seam |
| K2-K3 | Contracts 最小 ID/DTO；Application approval resolution/verifier adapter 设计 | sealed executor 与 evidence proof |
| K4 | Runtime Turn/Agent binding；owner-specific migration composition | 唯一 enforcement writer/recovery |
| K5 | `aletheon` 唯一 composition root 与 config owner | Linux/execd process boundary |
| K6 | Extensions 保全计划的 Gmail、Robot VLA、Hardware、Pi 等价清单 | enforcement seam/gates；`E2-K6a/E4-K6b/E5-K6c/E6-K6d` 各唯一 owner PR 产生 authoritative path |
| K7 | Runtime/Application/Composition/Extensions 相应 caller 已清零 | Executive Kernel 耦合可删除 |

Kernel 收敛完成必须同时满足：

- 所有 Agent-requested effect 都能映射到一个 sealed descriptor、一个 bound
  `CapabilityExecutor`、一个 Operation writer 和一个 finalized receipt；
- Executive 中 Kernel authority、approval gate、tool pipeline、execd process 的 production
  调用点为零；
- Application resolution aggregate 与 Kernel verifier 完全单向解耦；
- Runtime 只能通过窄 port 使用 Kernel，不能访问表、permit 或 executor registry；
- Pi 名为 `DelegateBackend`，effect 名为 `CapabilityExecutor`，LLM port 名为 `InferencePort`；
- 没有 `ABI` 命名，没有把 Fabric/Fabric 类型整体原样搬入 Kernel；共享最小类型进入
  `contracts`；
- installed smoke 和最小 invariant/architecture gates 通过；
- 关闭 Application/TUI 后，core Runtime + Kernel + Cognit + Dasein/Metacog 仍可经受信 typed
  command 完成无 UI 的基本 agent loop。

达到这些条件后，Kernel 才不是“鸡肋”：它的价值不来自拥有更多模块，而来自任何真实 effect
都无法绕过它所保护的少数不变量。
