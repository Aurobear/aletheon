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
| `runtime` | 外部执行器 lifecycle、manifest、WorkOrder、event、receipt、selection |
| `cognit` | cognition、reasoning、planning、review、harness |
| `corpus` | 工具、MCP、provider adapter 与受治理 capability execution |
| `platform` | Host OS contract、selector 与 Linux/Windows/macOS backend |
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

- MCP 执行归 Corpus，但配置仍从 Cognit 重导出：
  `crates/corpus/src/tools/mcp/config.rs:54-56`；
- Platform selector 已接通 target 对应的原生 probe：`crates/platform/src/selector.rs:26-59`，完整 Host contract 仍在收敛；
- Runtime selector 尚未统一所有真实外部执行路由；
- machine-wide provider concurrency/cooldown 尚未统一；当前重试已遵循指数退避和
  `Retry-After`，但跨 session、主 Agent 与 subagent 的请求协调仍需收敛到 machine core；
- Pi RPC 已回收 `agent_end` 中的 terminal assistant text 与 usage；完整 diff/artifact
  receipt 仍需继续完善：`crates/executive/src/adapters/runtime/pi_rpc.rs`；
- Hardware 没有生产调用者；
- Coding benchmark 尚未以真实 fixture/harness/receipt 落地。

## 8. 架构纪律

- 不创建没有生产 caller 的 crate；
- 不以 `api/types/common/broker` 命名代替 owner；
- 不按 OS 拆 Platform crate；
- 不让 Runtime、Corpus 或模型自行授予权限；
- 不在主进程执行应隔离的不可信低层副作用；
- 不用 mock 自返回成功代替真实端到端验收。
