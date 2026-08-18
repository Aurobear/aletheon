# Wiring ownership migration M1 consumer census

状态：**EXECUTED — owner/import cutover complete; compatibility and ghost files removed**

日期：2026-08-17

对应计划：`docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md` M1

## 1. 目的与判定规则

M1 删除六个 `wiring::application` compatibility re-export。删除前必须满足：

1. production 与 test consumer 都改用实际 owner；
2. wrapper 自身和 `application/mod.rs` declaration 删除；
3. 不新增 alias/re-export；
4. 涉及 Turn/Agent/Evaluation 核心路径和测试源码的修改分别获得 §0.4 授权；
5. 所有 target symbol 在当前代码中存在，不能按计划记忆推断。

本 census 的 `production` 计数排除 wrapper 自身、`#[cfg(test)]`、`tests/**` 和
`test_support.rs`。行号是 2026-08-17 当前工作区 locator，实施时必须重新 grep。

## 2. Consumer 与唯一 target

| Compatibility module | Production consumers | Test consumers | 唯一 target | 受保护生产路径 |
|---|---:|---:|---|---|
| `session_service` | 6 | 3 | `runtime::session_service` | `request_use_cases.rs`、`turn_pipeline.rs`、`daemon_turn/orchestrator.rs` |
| `governed_capability` | 3 | 7 | `kernel::capability::governed` | `turn_pipeline.rs`、`turn_runtime_ports.rs`、`adapters/runtime/native_cognit.rs` |
| `harness_factory` | 4 + 1 doc locator | 5 | `cognit::harness`; binary config helpers remain in `composition::harness_factory` | `daemon_react.rs`、`turn_pipeline.rs`、`adapters/runtime/native_cognit.rs` |
| `memory_gateway` | 2 | 5 | `mnemosyne` | `turn_pipeline.rs` |
| `capability_benchmark` | 10 | 1 | types → `application::capability_benchmark`; durable sink → `adapters_sqlite::SqliteCapabilityRollupProjectionSink` | `application/evaluation/service.rs`、`composition/turn_coordinator.rs`、`daemon/bootstrap/services.rs` |
| `cognitive_role_workflow` | 7 | 0 | `agora::cognitive_role_workflow` | `turn_pipeline.rs` |

已完成的非核心 cutover：GBrain adapter 的三处 Memory Gateway trait 引用已直接使用
`mnemosyne`。表中 `memory_gateway` production count 因此只剩 TurnPipeline 两处。

## 3. Symbol evidence

| Target symbol family | 当前定义 |
|---|---|
| `SessionService`, `ResumeResult`, `InterruptOutcome` | `crates/runtime/src/session_service.rs:30-42` |
| `CapabilityExecutionContext`, `GovernedActionLoopResolver`, `SelectedActionContext` | `crates/kernel/src/capability/governed.rs:29-85` |
| `CognitiveSessionFactory`, `LinearCognitiveSessionFactory` | `crates/cognit/src/harness/factory.rs:27,279` |
| `SupplementalBindingNegotiator`, `SupplementalBindingRecallPort`, `MemoryGatewayService` | `crates/mnemosyne/src/memory_gateway.rs:36-54` |
| `CapabilityRollupKey`, `CapabilityReceiptRollup` | `crates/application/src/capability_benchmark.rs:19-63` |
| durable capability rollup sink | `crates/adapters/sqlite/src/capability_benchmark.rs:17-75` |
| `RoleWorkflowFactory`, `TurnRoleLaunchContext`, `CodingWorkflowRequest`, `CognitiveRoleWorkflow` | `crates/agora/src/cognitive_role_workflow/mod.rs:85-372` |

`CapabilityRollupProjectionSink` 不能机械替换为 application 的同名 in-memory projection：
当前 Aletheon production callers 调用 `.open(...)`，对应唯一 durable implementation 是
`adapters_sqlite::SqliteCapabilityRollupProjectionSink`。key/receipt value types仍归 Application。

## 4. Ghost files

下列文件没有父模块 declaration、没有 Rust consumer，也没有 Cargo/build locator；Agora 已有
自己被声明的同名实现：

| 文件 | LOC | SHA-256 | 结论 |
|---|---:|---|---|
| `wiring/application/cognitive_role_workflow/stages.rs` | 269 | `2acdb9a00b8456c78c31272332b3c5de08a8273cb1ebd37e19da0c4f16718779` | ghost；M1 删除 |
| `wiring/application/cognitive_role_workflow/state_machine.rs` | 250 | `63eb422db463379bc414b8030f4725e80581f6ae88d0a7a5ef9f411b11067d1e` | ghost；M1 删除 |

父 wrapper `cognitive_role_workflow.rs:9-14` 仅 `pub use agora::cognitive_role_workflow`；Agora
在 `crates/agora/src/cognitive_role_workflow/mod.rs:5-6` 声明自己的 `stages` 和
`state_machine`。因此旧目录的 519 LOC 不进入任何编译单元。删除仍随完整 M1 packet 执行，
避免当前工作区形成“wrapper 尚在但旁支已删”的难审状态。

## 5. 实施顺序与验证

已按一个原子 packet 执行：

1. production callers 逐 owner 改写；
2. test imports 改到同一 owner，不改测试语义；
3. 删除六个 wrapper、两个 ghost 文件和六个 `application/mod.rs` declaration；
4. 运行计划 M1 的零引用/零文件 gate；
5. 运行 `aletheon --all-targets` check，以及 runtime/application/agora 现有 lib tests；
6. 任一行为或编译失败时回退整个 M1 packet，不恢复局部 alias。

## 6. 执行结果

- 旧 `wiring::application::{session_service,governed_capability,harness_factory,memory_gateway,
  capability_benchmark,cognitive_role_workflow}` 路径全仓 Rust 引用为零；
- 六个 wrapper 与两个 ghost 文件不存在，且 `application/mod.rs` 不再声明或 re-export；
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets` 通过；
- runtime 212、application 63、agora 99 个 lib tests 全部通过；
- `bash scripts/aletheon.sh test architecture` 通过（0 findings），`git diff --check` 通过。

当前状态为 `COMPLETED`；变更仅替换 owner/import 并删除 compatibility/ghost 源码，没有
修改 Turn、Agent 或 Evaluation 的状态转换、时序、cancel/deadline/fence 或持久化语义。
