# Aletheon 架构总览

> **Status:** Current design overview
>
> **Verified:** 2026-07-19

## 1. 系统定位

Aletheon 是 native-first、长期运行、受治理的 Agent 系统。它维护身份、认知、
目标、经验与外部行动，但不修改 Linux 内核，也不把所有领域拆成服务。

Canonical 宪法见
`../arch/Aletheon_MacroKernel_Architecture_Final(2).md`；当前依赖审计见
`../arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md`。

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

`TurnEngine` 的权威接口位于
`crates/executive/src/service/turn_engine.rs:14-22`。入口不得自行运行另一套 LLM
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
- Pi receipt 仍缺真实 output/usage/diff evidence：
  `crates/executive/src/impl/runtime/pi_rpc.rs:632-639`；
- Hardware 没有生产调用者；
- Coding benchmark 尚未以真实 fixture/harness/receipt 落地。

## 8. 架构纪律

- 不创建没有生产 caller 的 crate；
- 不以 `api/types/common/broker` 命名代替 owner；
- 不按 OS 拆 Platform crate；
- 不让 Runtime、Corpus 或模型自行授予权限；
- 不在主进程执行应隔离的不可信低层副作用；
- 不用 mock 自返回成功代替真实端到端验收。
