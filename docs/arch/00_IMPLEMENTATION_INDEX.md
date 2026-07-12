# Aletheon 宏内核实施文档索引

## 目标

本目录把宏内核总纲转换为可以直接编码、测试和分 PR 合并的实施规范。

总纲只回答“系统最终是什么”；这里回答：

```text
先改什么文件
新增什么接口
旧代码如何迁移
执行什么测试
满足什么条件才算完成
```

## 阅读与执行顺序

| 顺序 | 文档 | 交付结果 |
|---|---|---|
| 1 | `01_SINGLE_TURN_EXECUTION_PATH.md` | daemon、exec、未来 Automation 共用唯一认知主链 |
| 2 | `02_PROCESS_OPERATION_CHRONOS.md` | 真正可执行、可取消、可 wait 的 Agent Process |
| 3 | `03_CONTEXT_SPACE_AND_AGORA.md` | 私有 Context Space 与事务化 Agora |
| 4 | `04_COMMUNICATION_FABRIC_V2.md` | Call/Command/Event/Mailbox/Stream 语义分离 |
| 5 | `05_ADMISSION_CAPABILITY_SECURITY.md` | 不可绕过的权限、预算、配额、Lease 与沙箱 |
| 6 | `06_PR_PLAN_AND_ACCEPTANCE.md` | PR 顺序、测试矩阵、回滚点和最终验收 |
| 7 | (曾为 `07_M0_DETAILED_IMPLEMENTATION_PLAN.md`，M0 任务拆解已合并至 `06_PR_PLAN_AND_ACCEPTANCE.md` 第 10 节) | M0 可执行任务拆解、文件级改动、测试命令 |

## 实施原则

1. 每个 PR 必须保持 `cargo check --workspace` 通过。
2. 不在同一个 PR 同时做目录重命名和行为修改。
3. 新接口先适配旧实现，再迁移调用方，最后删除旧接口。
4. 生产主链不允许长期存在两套实现。
5. 所有新增异步任务必须属于 Process 或 Operation 的取消树。
6. 安全迁移必须 fail closed。

## 当前基线

目标分支：`Aurobear/aletheon:dev`。

当前关键入口：

```text
crates/executive/src/service/daemon_turn/execute.rs
crates/executive/src/core/orchestrator.rs
crates/executive/src/core/core_systems.rs
crates/cognit/src/harness/
crates/bin/src/main.rs
crates/agora/src/
crates/fabric/src/ipc/
```

## 文档状态

- `Aletheon_MacroKernel_Architecture_Final(2).md` 是代码对齐修订版，建议作为当前总纲。
- `Aletheon_MacroKernel_Architecture_Final.md` 是较早版本，可保留为历史参考，但不应作为实施源。

## M0 当前落地状态

截至当前分支，M0（参见 `06_PR_PLAN_AND_ACCEPTANCE.md` 第 10 节）已完成：

- Turn/Operation contracts：`crates/fabric/src/types/turn.rs`、`crates/fabric/src/types/operation.rs`、`crates/fabric/src/include/turn.rs`；
- CognitiveSession adapter：`crates/cognit/src/harness/session.rs`；
- TurnService scaffold：`crates/executive/src/service/turn_service.rs`、`pre_turn.rs`、`post_turn.rs`；
- exec 路径：`crates/bin/src/main.rs` 已通过 `TurnService` 提交 `TurnRequest`，不再手写 LLM/tool loop；
- daemon 路径：`crates/executive/src/service/daemon_turn/execute.rs` 已构造 `TurnRequest` 并通过 `executive::service::daemon_react::submit_streaming_daemon_turn` 进入 service/composition seam，handler 不再直接构造 `ReActLoop`；
- Harness 创建点：生产 harness 创建集中在 `crates/executive/src/service/harness_factory.rs`；
- SandboxFirst：缺少 sandbox promotion 时 fail closed，并有 `crates/executive/tests/sandbox_first_fail_closed.rs` 覆盖。

M0 验收命令：

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test -p fabric turn
cargo test -p cognit cognitive_session
cargo test -p executive turn_service
cargo test -p executive sandbox_first_fail_closed
rg "ReActLoop::new|build_harness" crates/executive crates/bin
rg "Proceeding without sandbox|selffield-note>SandboxFirst|sandbox review" crates/executive/src
```

允许剩余命中：`crates/executive/src/service/harness_factory.rs` 是唯一生产 harness factory；`crates/executive/src/impl/daemon/session_manager.rs` 的 `AdvancedCompressor` 只用于 session history compaction，不是 ReAct 执行入口；`crates/executive/src/core/config/agent.rs` 为注释。

## 非目标

本轮不做：

- 全仓库 crate 重命名；
- DDS、分布式一致性和 kernel module；
- 抢占式调度；
- Planner/Reviewer/Executor 全部进程化；
- 机器人硬实时控制；
- 页级 Copy-on-write。

