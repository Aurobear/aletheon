# 子 Agent 资源继承、结算与恢复

## 1. Grok 的可借鉴点

Grok 的 subagent coordinator 将 child session 视为隐藏 session，child 共享 parent 的 hunk tracker、filesystem、terminal 和 environment（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs:1-12`）。tracker 明确记录 parent/child session、prompt、model、cwd、worktree、background 和 cancel token（同文件 `:45-96`）。

更关键的是，parent runtime resources 有明确的生存期语义：

- parent hunk tracker 归集 child edits（同文件 `:182-188`）。
- FS/terminal 与 parent 共享（`:189-194`）。
- background tasks/monitors/scheduled tasks 可在 child exit 后继续存活（`:195-208`）。
- notification handle 需要 reparent，避免事件发往已销毁 child bridge（`:199-204`）。

## 2. Aletheon 已有的强基础

Aletheon 不需要复制 Grok 的 child-session authority。`AgentSpawnRequest` 已包含 root/parent Agent、parent process、runtime、trusted workspace、context fork、broadcast refs、allowlist 和 budget（`crates/fabric/src/types/agent_control.rs:189-207`）。AgentControl service 已统一持有 repository、admission、runtime registry、event spine、live runs 和 agent memory vault（`crates/executive/src/service/agent_control/mod.rs:100-114`）。

真正需要补强的是“child 完成时各类资源怎么结算”。

## 3. 资源分类

| 资源 | 默认继承 | Child 退出时 |
|---|---|---|
| WorkspacePolicy | 收窄继承，不扩大 | 释放 child lease |
| FS backend | shared 或 worktree-isolated | flush attribution |
| Terminal backend | 可共享 | 前台命令必须 settle |
| Background command | 显式标记 | kill / reparent / detach 三选一 |
| Scheduler | 可共享 | reparent 有 ownership receipt |
| Notification route | child-specific | 切 parent 或 durable mailbox |
| Cancellation | parent 派生 child token | parent cancel 必须传播 |
| Budget | 从 parent 分配 | 退回未用 reservation，提交 usage |
| Memory draft | child scope | promotion 前不可泄漏到 parent |
| Worktree | child-owned | 清理、保留 artifact 或进入 recovery |

## 4. 候选 settlement 协议

```text
Running child
   |
   v
Quiescing
   +-- stop accepting new calls
   +-- cancel/await foreground work
   +-- classify background resources
   +-- flush events, usage, memory drafts, artifacts
   v
Settling
   +-- reparent authorized survivors
   +-- release leases/reservations
   +-- persist recovery checkpoint if needed
   v
Terminal
   Completed | Failed | Cancelled | Recoverable
```

Settlement 必须幂等，以 `agent_id + attempt_id + generation` 或等价 key 防重复释放/重复 promotion。

## 5. Reparent 规则

后台资源只有满足全部条件才可 reparent：

1. spawn 时声明 `survive_child=true`，不是 child 自行临终升级。
2. parent 的 workspace/capability authority 覆盖该资源。
3. parent budget 接受剩余 cost/time reservation。
4. notification route 能切换到 parent 或 durable mailbox。
5. 产生不可变 reparent receipt，记录旧 owner、新 owner、资源、原因。

不满足时应 cancel/kill，并把失败写入 terminal evidence。

## 6. 与 Agent Mailbox/Recovery 的关系

- child 完成后的异步通知进入 parent mailbox，而不是丢弃。
- parent 已结束时，资源只能转 durable supervisor，不可悬空。
- daemon crash 后，AgentControl 用现有 `AgentRecoveryDecision`（interrupt/resume/finalize/reclaim，`crates/fabric/src/types/agent_control.rs:45-60`）决定 resource disposition。
- runtime checkpoint 只说明“可恢复”，资源 lease 和 OS process 仍需独立 reconciliation。

## 7. 验收方向

- child 退出后无 orphan foreground process。
- 被批准的 background task 继续运行并向 parent/mailbox 报告。
- parent cancel 传播到所有 child 和不可存活资源。
- usage、lease、budget 只结算一次。
- child memory 不经 promotion 不进入 parent scope。
- crash recovery 能区分需 reclaim、resume、finalize 的资源。

