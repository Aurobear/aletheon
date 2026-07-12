# Aletheon 宏内核迁移 PR 计划与验收

## 1. PR 顺序

| PR | 内容 | 禁止夹带 |
|---|---|---|
| 0A | Turn contracts + Linear adapter | 行为重写 |
| 0B | TurnService + characterization tests | 删除旧入口 |
| 0C | daemon 切换唯一主链 | crate 重命名 |
| 0D | exec 切换唯一主链 | Process 模型 |
| 0E | 删除重复 loop + 修 SandboxFirst | Agora 重构 |
| 1A | ID、ExitReason、Clock contracts | 调度算法 |
| 1B | OperationTable + cancellation tree | SubAgent UI 改版 |
| 1C | ProcessTable + SubAgent 执行 | IPC transport 重写 |
| 2A | Chronos + VirtualClock | Dasein temporality 修改 |
| 2B | SupervisorTree | 自动无限重启 |
| 3A | ContextSpace snapshot/overlay | 分布式 COW |
| 3B | Agora version/proposal/commit | CRDT |
| 3C | Agora/Mnemosyne commit persistence | 全量 Memory 重写 |
| 4A | EnvelopeV2 + mailbox | (旧 EventBus 已删除；迁移完成) |
| 4B | Agent 协作迁移 | DDS |
| 4C | Legacy Event 清理 | kernel IPC |
| 5A | CapabilityInvoker + Permit | Robot backend |
| 5B | Budget/Quota/Lease/Accounting | 高级 Scheduler |
| 6A | Service Ports + CoreSystems 收缩 | 全仓库目录移动 |

## 2. 每个 PR 必须包含

```text
Motivation
Old path / New path
Compatibility adapter
Tests
Rollback method
Remaining migration count
```

建议在 PR 描述中维护：

```text
legacy call sites before: N
legacy call sites after: M
```

## 3. 通用检查命令

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo doc --workspace --no-deps
```

若全 workspace 测试过慢，每个 PR 至少先跑修改 crate，再由 CI 跑全量。

## 4. Phase Gate

### Gate 0：唯一主链

- daemon 和 exec 共用 TurnService；
- 只有一个生产 Harness factory；
- `handle_chat` 不再编排 Cognit 细节；
- SandboxRequired fail closed。

### Gate 1：Process/Operation

- Main Agent 和 SubAgent 都存在 ProcessTable；
- 每个 Turn 都有 OperationId；
- wait/cancel/exit 可测试；
- 没有孤儿 Tokio task。

### Gate 2：Chronos/Supervision

- timeout 使用 monotonic clock；
- VirtualClock 测试稳定；
- restart 有边界；
- Dasein lived time 未与系统 clock 混合。

### Gate 3：Space/Agora

- private overlay 不泄漏；
- shared write 必须 proposal/commit；
- conflict 和 TTL 有测试；
- Agora commit 可恢复。

### Gate 4：Communication

- Agent mailbox 使用 EnvelopeV2；
- 所有 Stream 有 backpressure；
- 新代码不实现旧 Event；
- schema 不匹配结构化失败。

### Gate 5：Governance

- 副作用 Capability 必须持有 Permit；
- budget/quota/resource 无竞态超发；
- usage/audit 可按 Operation 追踪；
- Required sandbox 不降级。

## 5. 性能基线

重构前先记录：

```text
daemon cold start
first token latency
single no-tool turn latency
single tool turn latency
memory growth over 100 turns
event stream dropped count
```

第一阶段不追求更快，但不能出现数量级退化。

建议后续增加：

```text
benches/turn_pipeline.rs
benches/mailbox.rs
benches/agora_commit.rs
```

## 6. 故障注入

在合并宏内核基础设施前必须测试：

```text
LLM timeout
LLM stream disconnect
Tool panic
Sandbox unavailable
Approval client disconnect
Memory write failure
Agora conflict
Child agent panic
Daemon SIGTERM during tool execution
```

每个故障必须产生结构化 ExitReason/Audit，而不是只写日志。

## 7. 文档同步规则

能力状态只能使用：

```text
Production：生产主链启用并有集成测试
Implemented：有实现但未进入生产主链
Experimental：feature/config opt-in
Design：只有文档
```

禁止因为“存在代码”就在 README 标记 Stable/Done。

## 8. 第一轮可直接创建的 Issues

1. `arch: introduce TurnRequest/TurnResult contracts`
2. `arch: wrap ReActLoop as CognitiveSession (done; ReActLoop renamed/absorbed into TurnService pipeline)`
3. `refactor: extract TurnService from daemon chat handler`
4. `refactor: route exec mode through TurnService`
5. `cleanup: remove duplicate AletheonExecutive and Controller loops`
6. `security: make SandboxFirst fail closed`
7. `kernel: introduce OperationId and structured ExitReason`
8. `kernel: add OperationTable and cancellation tree`
9. `kernel: make SubAgentSpawner execute real processes`
10. `kernel: add Clock abstraction and VirtualClock tests`

## 9. 首个里程碑

建议里程碑名称：

```text
M0 — One Kernel, One Turn Path
```

完成范围只包括 Issues 1–6。不要在 M0 同时实现 Space、DDS、Robot 或完整 Governance。

M0 的用户可见结果：

- TUI 与 exec 行为一致；
- 安全规则一致；
- 会话和记忆一致；
- 更容易持续增加 Harness；
- 后续 Agent Process 有唯一可承载的执行入口。

## 10. M0 具体执行拆分

M0 只覆盖 Issues 1–6。（`07_M0_DETAILED_IMPLEMENTATION_PLAN.md` 原为独立 M0 拆解文件，已删除；任务细节合并至本节。）

| Issue | PR | 主要文件 | 验收命令 |
|---|---|---|---|
| 1 | 0A | `crates/fabric/src/types/turn.rs`, `crates/fabric/src/include/turn.rs` | `cargo test -p fabric turn` |
| 2 | 0A | `crates/cognit/src/harness/session.rs`, `crates/cognit/tests/cognitive_session.rs` | `cargo test -p cognit cognitive_session` |
| 3 | 0B | `crates/executive/src/service/*`, `crates/executive/tests/turn_service_equivalence.rs` | `cargo test -p executive turn_service` |
| 4 | 0D | `crates/bin/src/main.rs` | `cargo check -p aletheon-bin --all-targets` |
| 5 | 0E | `crates/executive/src/core/orchestrator.rs` (Controller 已删除) | `rg 'ReActLoop::new|build_harness' crates/executive crates/bin` |
| 6 | 0E | `crates/executive/src/service/daemon_turn/execute.rs`, `crates/executive/src/core/orchestrator.rs` | `rg 'Proceeding without sandbox|selffield-note>SandboxFirst|sandbox review' crates/executive/src` |

M0 禁止夹带：ProcessTable、ContextSpace、EnvelopeV2、Agora proposal、Budget/Quota/Lease 完整实现。
