# Aletheon Daemon

> **Status:** Current design and implementation map
>
> **Verified:** 2026-07-19

## 1. 定位

`aletheon daemon` 是持久应用入口。它装配 Executive、Kernel、领域服务、channel、
storage 和可选 `execd`，通过 Unix socket 提供请求入口。

```text
aletheon CLI
    -> executive::host::launcher
    -> daemon bootstrap
    -> UnixServer
    -> typed RPC handler
    -> Turn / Goal / Approval / Admin use case
```

CLI 入口位于 `crates/aletheon/src/main.rs:1-35`；daemon launcher 位于
`crates/executive/src/host/launcher.rs:56-111`；Unix server 位于
`crates/executive/src/impl/daemon/server.rs:622-708`。

## 2. 责任边界

Daemon 负责：

- 加载并验证顶层配置；
- 创建唯一 composition root；
- 打开 durable stores；
- 装配 TurnRuntimePorts 与领域服务；
- 启动 channel/worker/Unix RPC；
- 传播 cancel、shutdown 与 health；
- 对 optional feature 明确报告启用/不可用状态。

Daemon 不负责：

- 复制 Cognit loop；
- 实现具体工具；
- 实现 Kernel lifecycle；
- 实现 OS backend；
- 在 handler 中直接写多个状态 authority。

## 3. Bootstrap 分层

当前 bootstrap 已按关注点拆分：

```text
crates/executive/src/impl/daemon/bootstrap/
  request       composition entry
  services      domain services and ports
  runtime       model/agent runtime assembly
  turn_runtime  TurnRuntimePorts
  storage       durable stores
  channels      channel workers
  google        Google integration
  extensions    plugins/runtime extensions
  approval_gate approval wiring
```

新依赖必须放入拥有它的 stage，不能重新堆回一个巨型 constructor。

## 4. 请求路径

```text
socket frame
  -> connection validation
  -> RPC method parse
  -> typed request use case
  -> TurnEngine / GoalService / ApprovalService / AdminService
  -> structured response + streamed events
```

Turn 的唯一接口定义于
`crates/executive/src/service/turn_engine.rs:14-22`。RPC handler 只做协议转换、
身份/上下文解析和 use-case 调用。

## 5. Host 模式

`aletheon` 可以按环境选择前台、systemd 或 container host。Systemd host 的
READY/WATCHDOG 行为位于 `crates/executive/src/host/systemd.rs:31-131`。

Host 选择不应改变 Turn、permission、session 或 recovery 语义。

## 6. Execd

当配置 `execd = true` 时，Executive 创建随机 shared secret、限制 workspace root
并启动 `execd`：
`crates/executive/src/impl/daemon/bootstrap/request.rs:452-479`。

Execd 只执行已批准的低层副作用。它不是第二个 daemon authority，也不是 Runtime。

## 7. Shutdown 与恢复

Shutdown 顺序必须：

1. 停止接收新请求；
2. 取消或排空 live Operations；
3. 停止 channel/worker；
4. 终止并回收 `execd` 与外部 Runtime child；
5. flush durable authority；
6. 发布最终 health/stopping 状态。

Restart reconciliation 必须在新工作 admission 前完成，且不得重复已结算副作用。

## 8. 当前风险

- Executive 仍承担大量实现代码，bootstrap 修改需防止 owner 回流；
- Runtime registry 尚未统一真实外部 executor selection；
- MCP 配置仍由 Cognit 提供；
- `execd` 暂时依赖完整 Corpus；
- optional features 仍需持续验证不会静默 no-op。

## 9. 验收

- daemon 可从 repo 外目录启动；
- socket 权限与身份绑定正确；
- Turn/Goal/Approval use case 不被 RPC 旁路；
- SIGTERM、client disconnect、timeout 能清理子进程；
- systemd READY/WATCHDOG 有真实测试；
- restart 不重复已结算 Operation；
- disabled optional integration 在 doctor/health 中可见；
- `bash scripts/architecture-check.sh` 通过。
