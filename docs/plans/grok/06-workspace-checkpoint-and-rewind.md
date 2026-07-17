# Workspace Checkpoint 与 Rewind

## 1. Grok 的机制

Grok 用 `prompt_index` 关联 rewind checkpoint，把 filesystem snapshot 与可选 hunk delta、git state 作为同一逻辑 checkpoint 的多个 domain（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-workspace/src/session/checkpoint.rs:1-8`、`:83-100`）。checkpoint 可持久化，写入采用 last-write-wins 并允许 schema additive（同文件 `:192-225`）。

其 restore 顺序是设计重点：先处理 git soft restore/stash guard，再恢复 FS；FS 成功后才 restage 并截断后续 checkpoints；失败时保留状态供重试（同文件 `:364-443`）。

## 2. Aletheon 为什么需要

Aletheon 工具层已有单文件 `FileSnap` 捕获/恢复（`crates/fabric/src/types/tool.rs:178-210`），AgentControl 也有 runtime checkpoint reference（`crates/fabric/src/types/agent_control.rs:26-42`），但两者分别解决“局部文件回滚”和“runtime 可恢复性”，还不是统一的 turn workspace rewind。

建议明确区分：

```text
Runtime checkpoint     恢复 Agent/模型/session 执行状态
Workspace checkpoint   恢复本地 FS/VCS/patch 投影
Memory/Event history   不允许通过文件 rewind 擅自抹除
```

## 3. 候选 checkpoint 模型

```text
TurnCheckpoint
  checkpoint_id
  session_id / thread_id / turn_id / prompt_index
  workspace_identity
  fs_domain_ref
  vcs_domain_ref?          // git HEAD/index/stash metadata
  patch_domain_ref?        // changed hunks and attribution
  runtime_checkpoint_ref?  // optional link, not embedded authority
  created_at / schema_version
  integrity_digest
```

所有 ref 必须是 host-minted；模型只能请求“rewind to turn N”，不能提供任意文件路径或 checkpoint blob。

## 4. Capture 边界

建议在 turn/prompt 边界捕获，不在每个 token 或 progress 事件捕获：

1. 接受用户 prompt 后、模型执行前：begin checkpoint。
2. 工具 terminal settle 后更新 changed-file/hunk 元数据。
3. turn terminal 后 finalize。
4. 非 Completed 结果也应明确 finalize/abort，不能留下无法判断的 open checkpoint。

## 5. Restore 事务语义

最重要的不是“能回滚”，而是“部分失败可解释”：

| 阶段 | 失败行为 |
|---|---|
| 验证 workspace/checkpoint identity | 不修改任何内容 |
| 保护当前未跟踪修改 | stash/snapshot；无法保护则 abort |
| VCS 预处理 | 记录可恢复状态 |
| FS restore | 核心成功点；失败保留 checkpoints |
| index/hunk restore | 失败标记 partial，不伪装成功 |
| truncate future checkpoints | 仅在核心 restore 成功后 |

Memory 和 canonical event spine 不应随 workspace rewind 删除；应追加 `WorkspaceRewound` 事件形成新事实。否则审计和 Agent 行为历史会被重写。

## 6. 多 Agent 与并发

- 同一 workspace 的 rewind 必须取得排他 lease。
- 有活跃 child Agent 时默认拒绝 rewind，或先完成 coordinated cancellation。
- child 与 parent 若共享工作树，checkpoint 必须记录所有 attribution；Grok child 共享 parent hunk tracker/FS/terminal（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs:6-12`），Aletheon 需要同等明确的归属。
- worktree-isolated child 只能 rewind 自己的 workspace identity。
- rewind 后旧 approval/lease 不能自动复用。

## 7. 渐进启用

1. POC：仅 FS domain、显式用户触发、单 Agent。
2. 加入 durable checkpoint 与 crash recovery。
3. 加入 patch/hunk attribution。
4. 最后加入 git index/HEAD 语义和 multi-agent coordination。

每阶段必须有 feature flag、恢复 telemetry 和磁盘配额。

## 8. 验收方向

- rewind 恢复新增/修改/删除文件。
- 当前未提交用户修改不会静默丢失。
- 失败不会截断可重试 checkpoint。
- workspace identity 不匹配时 fail closed。
- child 活跃时不会产生跨 Agent 竞态。
- event history 保留并追加 rewind receipt。

