# Wave 1：唯一 TurnEngine

**状态：** W1-01 待执行；W1-02 至 W1-06 被阻塞。
**上游：** `docs/plans/2026-07-19-wave1-turn-engine.md`。
**完成定义：** daemon、CLI、native child 只通过一个 `TurnEngine` 执行；`TurnService` 不存在；只有 cognit 定义 `CognitiveSessionFactory`。

## W1-01 ResolvedTurnProfile

- 前置：`00-local-completion-audit.md` 的 Wave 0 复核命令全部通过。
- 修改：`crates/executive/src/service/turn_runtime_ports.rs`、`crates/executive/src/impl/daemon/bootstrap/turn_runtime.rs`、对应单元测试。
- 决策：定义 `ResolvedTurnProfile`；字段固定为 profile_name、allowed_tools、system_prompt、model_policy、max_iterations、max_input_tokens、max_output_tokens、max_tool_calls、max_elapsed_ms、approval_policy、tool_timeout_ms。删除 `ActiveAgentProfileSnapshot`，一次性更新全部引用。
- TDD：先增加 `resolved_turn_profile_carries_behavior_and_authorization`，断言 profile 的 prompt、model、预算、审批和工具集合全部保持。
- 验证：`bash scripts/cargo-agent.sh test -p executive resolved_turn_profile`，期望退出码 0。
- 提交边界：上述实现和测试文件。
- 完成后唯一下一项：W1-02。

## W1-02 Profile 接入主 Turn

- 前置：W1-01 提交存在。
- 修改：`crates/executive/src/service/daemon_turn/execute.rs`、`crates/executive/src/service/harness_factory.rs`、对应测试。
- 决策：删除 `execute.rs:175` 的 `model_policy: None`；prompt、model、迭代数、token、工具调用数、elapsed、审批和工具 timeout 全部从同一个 `ResolvedTurnProfile` 注入。
- TDD：两个 profile 使用不同 model/prompt/budget，断言构造出的 cognit config 不同。
- 验证：`bash scripts/cargo-agent.sh test -p executive turn_profile_is_applied`。
- 禁止：保留 daemon 局部默认值覆盖 profile；只接 model 不接预算。
- 完成后唯一下一项：W1-03。

## W1-03 Agent 工具两阶段注册

- 前置：W1-02 通过。
- 修改：`crates/executive/src/impl/daemon/bootstrap/runtime.rs`、`request.rs`、`services.rs`、agent tool definitions 所在文件、`agents/code-agent.toml`。
- 决策：profile 编译前注册 `agent_spawn/agent_wait/agent_send/agent_cancel/agent_list` definitions；profile 编译后绑定 executor。新增并只向主 coding profile暴露 `delegate_code/delegate_review/delegate_research`。
- TDD：profile 编译能解析 delegate 名；未授权 profile 看不到 delegate；授权 profile 能调用绑定 executor。
- 验证：`bash scripts/cargo-agent.sh test -p executive agent_tool_registration`。
- 禁止：definition-only 工具在 executor 未绑定时返回成功；用空 executor 吞掉调用。
- 完成后唯一下一项：W1-04。

## W1-04 TurnEngine contract 与 parity harness

- 前置：W1-03 通过。
- 创建：`crates/executive/src/service/turn_engine.rs`、`crates/executive/tests/turn_engine_parity.rs`；修改 `service/mod.rs`。
- 决策：trait 固定为 `execute(request, context, events) -> Result<TurnExecution, TurnError>`；context 固定携带 principal、workspace、operation、deadline、cancel token、ResolvedTurnProfile。测试从首次提交即启用，禁止 `#[ignore]`。
- parity 必须比较：工具授权集合、deadline、cancel terminal、compaction输入、receipt、settlement 事件数量和顺序。
- 验证：`bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity`。
- 完成后唯一下一项：W1-05。

## W1-05 三入口迁移并删除旧 facade

- 前置：W1-04 parity 通过。
- 修改：`turn_pipeline.rs`、`daemon_turn/orchestrator.rs`、`exec_session.rs`、`agent_control/mod.rs`、模块导出；删除 `turn_service.rs`。
- 顺序固定：daemon → CLI → native child。每迁移一个入口运行 parity；三入口完成后删除 `TurnService`。
- native child 固定调用 TurnEngine；禁止等待 Wave 2 Broker。
- 验证：`bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity`；`! rg -n 'TurnService' crates/executive/src`。
- 禁止：保留 re-export、deprecated facade 或第二套 settlement。
- 完成后唯一下一项：W1-06。

## W1-06 合并 CognitiveSessionFactory

- 前置：W1-05 通过。
- 修改：`crates/cognit/src/harness/session.rs`、`crates/executive/src/service/harness_factory.rs` 及引用。
- 决策：trait 只保留在 cognit；executive 文件只保留 `LinearCognitiveSessionFactory` 实现，不再定义同名 trait。
- 验证：`bash scripts/cargo-agent.sh test -p cognit cognitive_session`；`bash scripts/cargo-agent.sh test -p executive turn_engine`；`test "$(rg -l 'trait CognitiveSessionFactory' crates | wc -l)" -eq 1`。
- 完成后：Wave 1 完成，唯一下一文档为 `02-host-h0-h1.md`。
