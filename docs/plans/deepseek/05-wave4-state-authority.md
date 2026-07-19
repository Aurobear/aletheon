# Wave 4：状态权威与恢复

**状态：** 被阻塞。**解锁条件：** W3-08 通过。

## 固定权威

- Turn/Item：`events.db` EventSpine。
- Active operation：Kernel OperationTable 加 `events.db.recovery_points`。
- Agent run：`agent_control.db` AgentRunRepository。
- `sessions-v1.db`、`event-projections.db`、`protocol-events-v1.db` 全是 projection。

## 串行任务

| ID | 内容 | 验收命令 |
|---|---|---|
| W4-01 | 创建 `impl/storage/manifest.rs`；登记全部 19 个生产库 | `bash scripts/cargo-agent.sh test -p executive manifest_` |
| W4-02 | MigrationCoordinator 统一 `PRAGMA user_version`；任一失败阻止 ready | `bash scripts/cargo-agent.sh test -p executive migration_` |
| W4-03 | ReconciliationCoordinator；recovery_points 表固定建在 events.db | `bash scripts/cargo-agent.sh test -p executive reconcile_` |
| W4-04 | Turn 写路径固定 EventSpine 先写、projection 后物化 | `bash scripts/cargo-agent.sh test -p executive session_event_recovery` |
| W4-05 | CanonicalSessionStore 注释和权限降为 projection | `bash tests/architecture_check.sh` |
| W4-06 | 删除 LegacySessionUseCases 与生产 SessionManager 写路径 | `bash scripts/cargo-agent.sh test -p executive session_` |
| W4-07 | TrajectoryReader 保留完整 tool_use/result pair | `bash scripts/cargo-agent.sh test -p executive trajectory_` |
| W4-08 | TokenBudgetCompactor 替换固定 6 条 | `bash scripts/cargo-agent.sh test -p executive compaction_` |
| W4-09 | branch/checkpoint 事件落 EventSpine | `bash scripts/cargo-agent.sh test -p executive checkpoint_` |
| W4-10 | daemon 启动恢复 resumable runtime | `bash scripts/cargo-agent.sh test -p executive runtime_resume` |
| W4-11 | kill-9 turn/session 恢复 | `bash scripts/cargo-agent.sh test -p executive --test turn_kill9 -- --ignored` |
| W4-12 | kill-9 agent/lease/approval 恢复 | `bash scripts/cargo-agent.sh test -p executive --test agent_kill9 -- --ignored` |
| W4-13 | kill-9 checkpoint 与全库恢复序 | `bash scripts/cargo-agent.sh test -p executive --test checkpoint_kill9 -- --ignored` |

Token 估算首版固定：Unicode scalar count 除以 4并向上取整；禁止引入 tokenizer。旧 turn 生成结构化摘要；最近 tool pair 不得拆开。kill-9 三项必须在真实文件系统启动 daemon 子进程，禁止以内存库替代。
