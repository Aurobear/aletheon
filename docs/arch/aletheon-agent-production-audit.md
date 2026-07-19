# Aletheon Agent 生产能力审计

> **Status:** Production coding evidence converged on Executive/Fabric ownership
>
> **Verified:** 2026-07-20

## 1. 审计标准

Agent 生产能力不是模块数量，而是下面闭环能否真实完成：

```text
input
  -> typed Turn/Goal
  -> Operation + admission
  -> cognition or selected Runtime
  -> governed tools
  -> evidence
  -> independent verification
  -> durable settlement
  -> restart recovery
```

## 2. 已存在的基础

- `TurnEngine` 定义统一 Turn 请求、上下文、事件与结果接口：
  `crates/executive/src/service/turn_engine.rs:14-70`。
- Kernel 提供 Operation、Process、Admission 与 Supervision：
  `crates/kernel/src/lib.rs:1-12`。
- Corpus 提供文件、搜索、shell、MCP 与 provider 工具执行。
- Execd 提供隔离文件/进程副作用，Executive 的启动接线位于
  `crates/executive/src/impl/daemon/bootstrap/request.rs:452-479`。
- Pi RPC 以 manifest 注册到 Executive 的 Runtime registry：
  `crates/executive/src/impl/runtime/pi_rpc.rs:584-605`。

## 3. P0 真实性问题

### 3.1 Runtime contract 与生产生命周期已经拆清

旧 `runtime::CapabilityRuntime` 没有 production caller，也没有 operation/workspace
输入，已经删除。进一步 caller 审计确认 `RuntimeReceipt`、`RuntimeEvent`、`WorkOrder`
和 replacement `CodingVerifier` 也只有定义或测试 caller，因此一并删除。`runtime`
现在只拥有 manifest 与 selector：`crates/runtime/src/lib.rs:1-8`。Agent 实例、取消、
恢复和结算归 Executive `AgentControl`；结果 schema 归 Fabric `AgentResult`：
`crates/fabric/src/types/agent_control.rs:538-559`。

### 3.2 Verification 已归生产 owner

无 production caller 的文本启发式 verifier 和 replacement verifier 均已删除。
Executive Goal verification 校验 attempt identity、bounded diff 与 diff hash：
`crates/executive/src/impl/goal/verification.rs:85-159`；`tests/coding` 在进程外独立运行
acceptance commands，并把 operation、workspace fingerprints、command digests、diff
与 terminal status 写入完整性保护的 receipt。

### 3.3 Runtime selector 已接入 Executive registry

`RuntimeSelector` 只解析 manifest；Executive registry 同时保存 manifest 与 launcher，
daemon bootstrap 原子注册 Pi RPC。这样 selector 是选择入口，但 admission、cancel、
verification 与 settlement 仍只有 Executive 一个 authority。

### 3.4 MCP 配置与 credential 已归 Corpus

Corpus 同时持有 MCP schema、连接、鉴权、工具发现和执行：
`crates/corpus/src/tools/mcp/config.rs:11-202`。Cognit 不再提供 MCP server schema；
endpoint grant 也由 Corpus 按 MCP 生命周期定义，且不保存 secret：
`crates/corpus/src/tools/mcp/auth.rs:12-60`。

## 4. 工具闭环要求

生产工具必须提供：

- 明确 workspace root；
- permission/risk/permit；
- deadline 与 cancel；
- bounded output；
- structured error；
- 可关联 Operation/Process 的 evidence；
- 对写操作的冲突检测或前置哈希；
- 可验证 receipt。

Execd 是低层执行边界，不应理解 Prompt、Goal 或任务完成。Corpus 是工具领域，
不应自行授予权限。

## 5. Runtime 与 Subagent

```text
Native child Agent
  使用 Aletheon Turn/Kernel 语义

External Runtime
  由 Executive launcher 驱动，返回 Fabric AgentResult

execd
  执行单次获准的文件或进程副作用
```

三者不能因为都会“执行”而共享同一个模糊接口。

## 6. 上下文与状态

生产闭环必须保留完整的 tool call/result 关联，而不是只保留文本摘要。压缩产生的
summary 是 projection，不是原始 trajectory authority。恢复后不能重复执行已结算
副作用。

## 7. 评测门禁

真实 Coding Benchmark 位于 `tests/coding`，包含：

```text
fixtures   真实可复制仓库和缺陷
tasks      prompt、限制与独立验收条件
harness    启动真实 Executive/Turn 路径
receipts   diff、命令输出、usage、verification、terminal status
```

禁止用闭包直接返回成功，或用 30 条重复描述冒充 30 个任务。

## 8. 当前优先级

1. 保持 AgentResult、Goal coding evidence 与独立 acceptance receipt 的唯一主链；
2. 不恢复文本启发式或无 caller 的平行 verifier；
3. 保持 Runtime selector 与 Executive 最终验证的单向边界；
4. ~~迁移 MCP 配置 owner；~~ 已归 Corpus；
5. ~~建立 3 个真实 coding fixtures 的纵向闭环；~~ 已生成三个独立 verified receipt；
6. 加入 restart、cancel、timeout、残留进程和 false-success 门禁。

## 9. 非目标

- 用更多“意识层”代替工具闭环；
- 为 verifier、benchmark 或 provider 预建独立 crate；
- 让外部 Runtime 直接修改全局 Goal 状态；
- 以单元测试数量代表生产任务成功率。
