# Aletheon 当前架构审计与收敛原则

> **Status:** Current architecture decision record
>
> **Verified:** 2026-07-20

## 1. 结论

Aletheon 保留宏内核与领域模块，但不再按 `api`、`broker`、操作系统或
未来 provider 横向拆 crate。当前 workspace 有 16 个领域/入口 crate 和 2 个 example
package，登记于 `Cargo.toml:3-22`。

当前核心结构：

```text
aletheon entry
    -> Executive composition root
         |-> Kernel lifecycle/admission
         |-> Cognit reasoning
         |-> Corpus capabilities
         |-> Runtime external executors
         |-> Mnemosyne / Dasein / Agora / Metacog
         `-> Gateway / execd

Corpus -> Platform
Hardware (experimental, not wired)
```

详细依赖矩阵见 `CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md`。

## 2. 已确认的正确边界

- Kernel 独立拥有 admission、operation、process、chronos、space 与
  supervision，见 `crates/kernel/src/lib.rs:1-12`。
- Executive 是 composition root；其领域依赖见
  `crates/executive/Cargo.toml:9-19`。
- TurnEngine 声明唯一权威 Turn 接口，见
  `crates/executive/src/service/turn_engine.rs:14-22`。
- Runtime 只定义外部执行 manifest 与 selector contract；实例生命周期、registry、
  result、verification 和 settlement 归 Executive/Fabric，见 `crates/runtime/src/lib.rs:1-8`。
- Platform 在单 crate 内拥有 contract、selector 与三 OS backend，见
  `crates/platform/src/lib.rs:1-38`。
- Execd 是独立安全/故障隔离进程，不是 Agent Runtime。

## 3. 当前真实缺口

### 3.1 Executive 仍然过大

Executive 有约 10.9 万行 Rust。作为 composition root，它可以依赖多数领域，
但不能继续吸收 Kernel 机制、认知算法、工具实现或 OS backend。

### 3.2 Fabric 仍是依赖漏斗

Fabric 是多数成熟领域的基础依赖，且包含大量兼容实现。新类型不能因为
“多个 crate 都想用”就自动放入 Fabric；必须先确定领域 owner。

### 3.3 MCP owner 已收敛到 Corpus

MCP server/transport/trust/OAuth/tool-policy schema 与执行生命周期现在都归 Corpus：
`crates/corpus/src/tools/mcp/config.rs:11-202`。Cognit 不再定义 MCP server 配置，
Corpus 也不再依赖 Cognit。MCP endpoint credential 使用 Corpus 私有、无 secret 的
`McpEndpointCredentialGrant`，不再复用 Mnemosyne embedding grant：
`crates/corpus/src/tools/mcp/auth.rs:12-60`。

### 3.4 Runtime selector 已成为生产选择入口

Pi adapter 的 manifest 由 Executive registry 注册并通过 Runtime selector 解析；adapter
继续位于 Executive，因为它参与 AgentControl 生命周期。边界保持：

```text
Executive 选择、准入、监督、最终验证
Runtime adapter 执行任务并返回 Fabric AgentResult
```

### 3.5 Platform Linux contract 与强路径约束已接通

Linux backend 已导出真实实现：
`crates/platform/src/backend/linux/mod.rs:20-31`；selector 已在对应 target 选择
原生 probe：`crates/platform/src/selector.rs:26-59`。Linux 统一 contract suite 已覆盖
filesystem/process/PTY/service/sandbox；filesystem 使用 pinned directory fd 与 openat2
消除路径验证后重开造成的 root escape。Windows/macOS 仍缺原生 runner 证据。

### 3.6 Hardware 已形成实验性纵向切片，但不是生产能力

Hardware simulator 已通过 Kernel Permit、lease、deadline、sequence、fail-safe、stop
与 receipt 的测试纵向链；caller 仍只有 Executive integration test，因此保持
`experimental_wired`，没有真实 actuator 生产声明。

## 4. 五条不可绕过的系统语义

```text
1. 所有工作成为 Operation
2. 所有执行主体成为受监督 Process 或 Runtime Session
3. 所有副作用经过 Permit 约束的 Capability Invocation
4. 所有结果形成 Evidence + Receipt + Verification + Settlement
5. 每类 durable fact 只有一个 Authority，其他存储只是 Projection/Cache
```

## 5. 优化顺序

1. 修复已存在主链的真实性，不增加新 crate；
2. ~~迁移 MCP 配置所有权；~~ 已归 Corpus；
3. 完成 Platform Host contract suite 与原生 runner 验证；
4. 接通 Runtime registry/selector 与 Executive verification；
5. ~~缩窄 `execd -> corpus` 依赖；~~ 已改为 `execd -> platform` 最小 patch/filesystem contract；
6. 以 simulator 证明 Hardware 的 permit/lease/deadline/fail-safe 纵向闭环；
7. 建立真实 `tests/coding`，用独立验收证明 Agent 能力。

## 6. 禁止事项

- 创建没有生产调用者的 crate；
- 用 `api/types/common/broker` 名称代替领域边界；
- 为每个 OS 创建独立 Platform crate；
- 在设备需求出现前创建 ROS/CAN/Serial/Vendor 空壳；
- 让 Runtime 或模型自证全局任务完成；
- 用 mock 自返回成功代替端到端 benchmark；
- 因为 Executive 很大就把代码随机拆成更多 crate。

## 7. 验收

架构变更必须同时通过：

- 最窄相关 package 测试；
- `bash scripts/architecture-check.sh`；
- 无新增未经登记的本地依赖；
- 文档中 current/target/experimental 状态明确；
- 新抽象存在真实生产 caller 或明确删除期限。
