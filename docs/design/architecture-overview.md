# Aletheon 架构总览

> **Status:** Current design overview
>
> **Verified:** 2026-07-26

## 1. 系统定位

Aletheon 是 native-first、长期运行、受治理的 Agent 系统。它维护身份、认知、
目标、经验与外部行动，但不修改 Linux 内核，也不把所有领域拆成服务。

当前架构边界由本文件、`config/architecture/` 门禁、
`docs/arch/CORE_REFACTOR_COMPLETION_REPORT.md` 和实际 workspace 共同约束。

## 2. 运行结构

```text
User / Channel / Automation
          |
          v
       aletheon
    application entry
          |
          v
       Executive
 composition + orchestration + final verification
   /        |          |          \
  v         v          v           v
Kernel    Cognit     Corpus      Runtime
lifecycle reasoning  capabilities external executor lifecycle
                       |
                       v
                    Platform

Executive -> Gateway / Mnemosyne / Dasein / Agora / Metacog
Executive -> execd (optional isolated side effects)
Executive -> Hardware (experimental; not production-wired)
```

## 3. 核心边界

| Crate | Owner |
|---|---|
| `aletheon` | CLI、daemon、exec、TUI/ACP 顶层装配 |
| `executive` | Turn/Session/Goal/Agent 编排、approval、最终验证与 settlement |
| `kernel` | Operation、Process、Admission、Chronos、Space、Supervision |
| `runtime` | 外部执行器 lifecycle、manifest、event、receipt、selection |
| `cognit` | cognition、reasoning、planning、review、harness |
| `corpus` | 工具、MCP、provider adapter 与受治理 capability execution |
| `platform` | Host OS contract、selector 与 Linux backend（Android/macOS/Windows 未实现） |
| `execd` | 独立进程中的受约束文件/进程副作用 |
| `hardware` | 设备身份、租约、命令、遥测与 simulator；当前 experimental |
| `fabric` | 跨领域协议、ID、envelope 与兼容基础设施 |
| `mnemosyne` | 经验、记忆、召回与知识持久化 |
| `dasein` | identity、care、continuity、lived temporality |
| `agora` | 共享认知工作空间 |
| `metacog` | 受治理候选评估与演化 |
| `gateway` | 外部请求与 channel adapter |
| `interact` | TUI 和用户交互 adapter |

当前 workspace 列表以 `Cargo.toml:3-21` 为准。

## 4. 统一请求流

```text
Input
  -> Gateway/Interact normalization
  -> Executive Turn or Goal use case
  -> Kernel Operation + Admission Permit
  -> Cognit harness or selected Runtime
  -> Corpus governed capability
  -> Platform / execd / external provider / Hardware
  -> Evidence + Receipt
  -> Executive verification and settlement
  -> durable authority + projections
```

### 4.1 权威运行事实

模型关于自身、上下文和运行状态的自然语言陈述不是系统事实。每轮模型选择后，
host 从实际路由对象生成 `ModelRuntimeFacts`：

```text
machine provider registry
  -> resolved model specification
  -> PortLlmProvider::runtime_facts()
  -> TurnPipeline system context
  -> model answer / logs / diagnostics
```

当前事实包含 effective model ID、显示名称和最大上下文长度。它们来自
`crates/fabric/src/types/llm_types.rs` 的共享契约及
`crates/executive/src/application/inference_port.rs` 的 resolved adapter；模型不得
根据训练先验猜测厂商、版本或上下文窗口。

同一原则适用于 session ID、Agent runtime、capability 与预算：authority/projection
可以提供事实，模型输出不能反向覆盖 authority。

`TurnEngine` 的权威接口位于
`crates/executive/src/application/turn_engine.rs:14-22`。入口不得自行运行另一套 LLM
loop。

## 5. Runtime、Platform 与 execd

```text
Runtime   完整 WorkOrder 的受监督执行主体，可维护 session
Platform  Host OS 能力库，不拥有 Agent 或权限策略
execd     单次低层副作用的隔离执行进程，不理解 Prompt/Goal
```

Runtime receipt 是证据，不是最终成功裁决。Executive 保留 admission、cancel、
verification 与 settlement 权威。

## 6. 状态与恢复

- 每类 durable fact 只有一个 Authority；
- projection/cache 可重建，不能反向覆盖 authority；
- tool call/result 与副作用 receipt 必须关联 Operation；
- retry 创建新 attempt，不覆盖失败历史；
- restart 不得重复已结算副作用；
- approval、lease、deadline 在恢复后不能自动放宽。

## 7. 当前未完成项

### 7.1 通用 Governed Review 服务

外部控制器通过现有的 authenticated user socket 提交有界证据审查，不获得工具、
文件系统或领域适配器权限：

```text
review.submit
  -> Fabric N/N-1 bounded job contract
  -> Executive principal-scoped durable queue
  -> shared InferencePort (tools = [])
  -> allowlist/output/budget validation
  -> fsync-backed terminal receipt
  -> review.wait / status / cancel
```

Fabric 只拥有 wire-safe DTO；Executive 拥有幂等、执行、取消和终态结算；daemon RPC
仅投影该 authority。`queued` 和 `running` 不是成功，调用方必须观察 terminal receipt。
实现位于 `crates/fabric/src/types/governed_review.rs`、
`crates/executive/src/application/governed_review/` 与
`crates/executive/src/host/daemon/handler/rpc/rpc_review.rs`。生产验收使用官方 user
socket 发起真实推理，并同时要求候选/安装/运行进程摘要一致且 systemd restart counter
稳定。

- MCP 执行归 Corpus，但配置仍从 Cognit 重导出：
  `crates/corpus/src/tools/mcp/config.rs:54-56`；
- Platform selector 已接通 target 对应的原生 probe：`crates/platform/src/selector.rs:26-59`，完整 Host contract 仍在收敛；
- Platform 的 `LinuxSandboxHost::apply` 仍是未接线的 host-contract stub，**不是**
  governed command 的生产沙箱。安装态工具执行由 Corpus `SandboxExecutor` 选择
  Bubblewrap/Process/Noop backend（`crates/corpus/src/security/sandbox/executor.rs:11-54`）；
  coding runtime 则在 bootstrap 中只接受实际 probe 成功的 Bubblewrap namespace backend，
  否则不注册（`crates/executive/src/host/daemon/bootstrap/request.rs:1056-1074`）。
- Runtime selector 已选择 manifested `native-cognit` / `pi-rpc`，但兼容
  `pi-coder`、Goal provider worker 与 package executable runtime 尚未全部进入同一
  manifest/selection 路径：`crates/executive/src/host/daemon/bootstrap/request.rs:953-1103`；
- machine-wide provider permit/cooldown 的 authority 位于 machine core：LLM 由
  canonical factory 直接消费，user daemon 的 remote embedding 经 authenticated core
  RPC 持有 socket-backed permit lease；health 也从 core RPC 读取权威 snapshot。当前工作树
  已完成这条跨进程接线，剩余安装态并发/故障验收：
  `crates/cognit/src/composition/inference_factory.rs:76-187`、
  `crates/executive/src/host/core_rpc/server.rs:200-280`、
  `crates/executive/src/host/daemon/bootstrap/request.rs:693-708`；
- Pi RPC 已回收 `agent_end` 中的 terminal assistant text 与 usage，但其
  `AgentResult.artifacts` 仍为空；legacy `pi-coder` 的 bounded diff artifact 不能替代
  resident RPC receipt：`crates/executive/src/adapters/runtime/pi_rpc.rs:420-434`、
  `crates/executive/src/adapters/runtime/pi.rs:657-703`；
- Hardware 已由 production embodiment composition 调用：
  `crates/executive/src/host/daemon/bootstrap/request.rs:823-850`；
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
