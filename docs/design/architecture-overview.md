# Aletheon 架构总览

> **Status:** Current design overview
>
> **Verified:** 2026-08-16

## 1. 系统定位

Aletheon 是 native-first、长期运行、受治理的 Agent 系统。它维护身份、认知、
目标、经验与外部行动，但不修改 Linux 内核，也不把所有领域拆成服务。

当前架构边界由本文件、`config/architecture/` 门禁、
`docs/arch/CORE_REFACTOR_COMPLETION_REPORT.md` 和实际 workspace 共同约束。
代码事实优先于历史架构快照。

## 2. 运行结构

```text
User / Channel / Automation
          |
          v
 Gateway / Interact
          |
          v
       aletheon
 composition + host wiring
   |              |                 |
   |              |                 +--> adapters / daemon / exec
   |              |
   |              +--> wiring/application
   |                   Turn / Goal / Agent Control orchestration
   |
   +--> application --------------------> runtime
        narrow pure use cases/contracts    Session / Turn / Agent authority
                    |                         |
                    v                         v
                contracts <---- kernel / cognit / dasein / corpus / mnemosyne
                                             |
                                             v
                                      platform / execd / providers
```

`aletheon` 是二进制 composition root 和 host。它可以依赖多个领域 crate，但不应
成为新的领域 authority。当前主要编排仍在
`crates/aletheon/src/wiring/application/`；selected pure use cases 已进入
`crates/application` 并有生产调用，例如 `DaemonLifecycleService`
（`crates/aletheon/src/wiring.rs:141`）、`TransactionReviewService` 和
`SessionInputCoordinator`
（`crates/aletheon/src/wiring/daemon/handler/ports.rs:44-45`）。

## 3. 核心边界

| Crate | Owner |
|---|---|
| `aletheon` | CLI、daemon、exec 与 user-daemon 的 composition/host wiring；当前还承载主要 Turn/Goal/Agent Control 编排 |
| `application` | 窄、纯的 Application 契约和已接线 use case；不拥有具体 repository，也不 mint Runtime ID |
| `contracts` | 跨 crate 的共享类型、端口、协议、ID 与受治理 envelope |
| `kernel` | Operation、Process、Admission、Chronos、Space、Supervision |
| `runtime` | Session/Turn/Agent 生命周期 authority，以及外部 runtime manifest、selection、event 与 receipt |
| `cognit` | cognition、reasoning、planning、review、harness |
| `corpus` | 工具、MCP、provider adapter 与受治理 capability execution |
| `platform` | Host OS contract、selector 与原生 backend |
| `execd` | 独立进程中的受约束文件/进程副作用 |
| `hardware` | 设备身份、租约、命令、安全、遥测、provider 与 simulator；由 daemon embodiment composition 组装 |
| `mnemosyne` | 经验、记忆、召回与知识持久化 |
| `dasein` | identity、care、continuity、lived temporality |
| `agora` | 共享认知工作空间 |
| `metacog` | 受治理候选评估与演化 |
| `gateway` | 外部请求、typed protocol 与 channel routing |
| `interact` | TUI、ACP 和用户交互 adapter |

`crates/application` 是窄纯契约/已接线用例 owner；Turn、Goal、Agent Control 的 host
编排唯一位于 `crates/aletheon/src/wiring/application/`。无生产调用的
`ApplicationFacade` 已删除，不再作为入口或兼容层。

当前 workspace 列表以 `Cargo.toml:3-24` 为准。

## 4. 统一请求流

Daemon 当前生产路径：

```text
Input
  -> Gateway / Interact normalization
  -> aletheon host admission
  -> TurnEngine
  -> TurnPipeline
  -> Kernel Operation + Admission Permit
  -> Cognit harness or selected Runtime
  -> Corpus governed capability
  -> Platform / execd / external provider / Hardware
  -> Evidence + Receipt
  -> aletheon application orchestration
  -> Runtime authority + durable projections
```

`TurnEngine` 的权威接口位于
`crates/aletheon/src/wiring/application/turn_engine.rs:16-46`。daemon 由
`DaemonTurnEngine` 实现该接口并调用 `TurnPipeline`
（`crates/aletheon/src/wiring/application/daemon_turn_engine.rs:44`）；`aletheon exec`
由 `ExecTurnEngine` 实现同一接口并复用 `TurnCoordinator` reducer
（`crates/aletheon/src/wiring/exec_session.rs`）。保留的 `TurnService`
只是 CLI exec 到 `TurnEngine` 的薄适配，不再拥有独立编排。`aletheon -m` 继续进入
daemon 路径。

### 4.1 权威运行事实

模型关于自身、上下文和运行状态的自然语言陈述不是系统事实。每轮模型选择后，
host 从实际路由对象生成 `ModelRuntimeFacts`：

```text
machine provider registry
  -> resolved model specification
  -> LlmProvider::runtime_facts()
  -> TurnPipeline system context
  -> model answer / logs / diagnostics
```

共享事实类型位于 `crates/contracts/src/types/llm_types.rs:128-185`；provider 从当前
路由对象返回事实（`crates/aletheon/src/wiring/adapters/inference/factory.rs:203`），
`TurnPipeline` 消费并绑定它们
（`crates/aletheon/src/wiring/application/turn_pipeline.rs`）。模型不得根据
训练先验猜测厂商、版本或上下文窗口。

同一原则适用于 Session ID、Agent runtime、capability 与预算：authority/projection
可以提供事实，模型输出不能反向覆盖 authority。

## 5. Runtime、Platform 与 execd

```text
Runtime   Session/Turn/Agent authority；外部 runtime manifest、selection 与 receipt
Platform  Host OS 能力 contract 与原生 backend，不拥有 Agent 或权限策略
execd     单次低层副作用的隔离执行进程，不理解 Prompt/Goal
```

Runtime receipt 是证据，不是最终成功裁决。Admission、cancel、verification 和
settlement 的编排当前仍由 `crates/aletheon/src/wiring/application/` 组合，持久化 authority
必须通过 Runtime 或明确的 durable adapter，而不能由模型输出覆盖。

## 6. 状态与恢复

- 每类 durable fact 只有一个 Authority；
- projection/cache 可重建，不能反向覆盖 authority；
- tool call/result 与副作用 receipt 必须关联 Operation；
- retry 创建新 attempt，不覆盖失败历史；
- restart 不得重复已结算副作用；
- approval、lease、deadline 在恢复后不能自动放宽。

## 7. 边界状态与后续事项

### 7.1 Application 与 Turn 边界（已于 2026-08-16 收敛）

- `crates/application` 只承载窄纯契约与已接线用例；host 编排继续由
  `crates/aletheon/src/wiring/application/` 负责，`ApplicationFacade` 已删除。
- daemon 与 `aletheon exec` 均通过 `TurnEngine`；`TurnService` 仅为薄适配。

### 7.2 通用 Governed Review 服务

外部控制器通过 authenticated user socket 提交有界证据审查，不获得工具、
文件系统或领域适配器权限：

```text
review.submit
  -> application bounded job contract
  -> aletheon principal-scoped durable queue
  -> shared InferencePort (tools = [])
  -> allowlist/output/budget validation
  -> fsync-backed terminal receipt
  -> review.wait / status / cancel
```

Wire-safe DTO 位于 `crates/application/src/governed_review.rs`；幂等、执行、取消和终态
结算位于 `crates/aletheon/src/wiring/governed_review/`；daemon RPC 仅投影该 authority
（`crates/aletheon/src/wiring/daemon/handler/rpc/rpc_review.rs`）。`queued` 和
`running` 不是成功，调用方必须观察 terminal receipt。

### 7.3 仍需收敛或验收的现行路径

- Platform 的 `LinuxSandboxHost::apply` 仍是未接线的 host-contract stub，**不是**
  governed command 的生产沙箱
  （`crates/platform/src/backend/linux/sandbox_host.rs:31-51`）。安装态工具执行由 Corpus
  `SandboxExecutor` 选择 backend
  （`crates/corpus/src/security/sandbox/executor.rs:11-54`）；coding runtime 只在
  Bubblewrap probe 成功后注册（`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:1025-1045`）。
- Machine/provider permit 和 cooldown 已有静态实现，包括 core RPC socket lease
  （`crates/aletheon/src/wiring/core_rpc/server.rs:228-243`）及 remote embedding 接线
  （`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:657-669`）；完整生产调用方和
  安装态并发证据仍按 Finding F 保持 `NEEDS EVIDENCE`，不是当前实现包。
- Pi RPC 已回收 terminal assistant text 与 usage，但 `AgentResult.artifacts` 仍为空
  （`crates/aletheon/src/wiring/adapters/runtime/pi_rpc.rs:697-706`）。
- Hardware 已由 daemon 的 production embodiment composition 组装并注册受治理工具
  （`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:754-767`）。具体真实 provider
  是否可用仍由有效配置与运行 probe 决定。
- Coding fixture/harness/receipt 与 deterministic/release gate 已落地；真实模型 workflow
  仍需取得无 provider error 的连续安装态证据：`tests/coding/README.md`、
  `scripts/libexec/aletheon/release-acceptance.sh`。

## 8. 架构纪律

- 不创建没有生产 caller 的 crate；
- 不以 `api/types/common/broker` 命名代替 owner；
- 不按 OS 拆 Platform crate；
- 不让 Runtime、Corpus 或模型自行授予权限；
- 不在主进程执行应隔离的不可信低层副作用；
- 不用 mock 自返回成功代替真实端到端验收。
