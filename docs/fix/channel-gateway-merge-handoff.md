# 交接说明：合并 `auro/refactor/channel-gateway`

> 面向负责合并此重构的 agent。目标：把 channel 子系统解耦 + 应用层归位（新 crate `gateway`）合入主线。

## 0. 最关键一条：分支在哪个仓库

- 本重构分支 **`auro/refactor/channel-gateway`** 位于仓库 **`/home/aurobear/Bear-ws/aletheon`**（`aletheon/.git`）。
- worktree 检出点：`/home/aurobear/Bear-ws/.worktrees/aletheon-channel-gateway`。
- **`/home/aurobear/Bear-ws/aletheon-bak` 是另一个独立仓库，没有这个分支。** 若你在 bak 里操作，`git log`/`git branch` 都看不到这 9 个 commit。合并必须在 `aletheon` 仓库进行（或先把分支 push/fetch 到你的合并仓库）。
- 分支基点 base = `8d79823b`（`feat(exec-server): confine filesystem access to workspace roots`）。

## 1. 交付内容（9 个 commit，全绿）

```
ef7d010e Phase 0 — 抽 intent.rs/notify.rs（纯逻辑，零行为变化）
f4b9bf5a Phase 1 — CapabilityRegistry 取代 god-object；ChannelRouter→ChannelDispatcher；5 个 Option<Arc<dyn>> 清零
c35c1d95 Phase 2 — 中立层 google-free；ChatPreprocessor 钩子
00ab6e1a Phase 3 — Gmail→注册 EventCapabilityHandler（领域内部 0 改动）
9eb30b16 Phase 5(部分) — 删死的 GoogleMailIngressSink
82898d64 Phase 4① — ChannelApprovalPort（脱离具体 ApprovalRepository，核心审批契约 0 改动）
8131b9de Phase 4② — 新建 crates/gateway，中立引擎物理搬出 executive
d2aaf750 / da629b48 — 文档
```

**结构变化摘要**：
- 新增 crate **`crates/gateway`**（只依赖 `fabric`）：`dispatcher/effect/intent/notify/ports/registry/store` + `telegram/` transport + `handlers/{chat,goal,greeting,approval,google_read}` + transport/executor 端口 trait。
- `executive` 现依赖 `gateway`（`executive/Cargo.toml` 增 `gateway = { path = "../gateway" }`）；方向 `executive → gateway → fabric` 单向无环。
- executive 侧保留并实现 gateway trait：`daemon_adapter`（`DaemonChannel*Executor`/`ApprovalRepositoryPort`）、`gmail/` 子树、`handlers/gmail_ingest`。
- 根 `Cargo.toml` 的 `workspace.members` 增加了 `crates/gateway`。

## 2. 冲突面（与 `aletheon-bak` 那摊 MCP/tool-exec WIP）

`aletheon-bak` 有大量未提交 WIP（MCP 集成 D3 / 工具执行 D1·G2·S1 / 记忆 / 多用户 D2）。与本分支的文件交集只有 3 个：

| 文件 | 冲突性质 | 处理建议 |
|---|---|---|
| `crates/executive/src/impl/daemon/bootstrap/request.rs` | **真冲突** | bak 的 MCP/tool-exec 与本分支 Phase 4① 接线都改了它。手工合并：保留双方改动（本分支只改了 channel 装配相关行）。 |
| `Cargo.lock` | 常规 | 合并后 `cargo update -w --workspace` 或让 cargo 重新生成。 |
| `docs/fix/channel-app-layer-refactor.md` | 非代码 | bak 里那份是**早期过时拷贝**；以本分支版本为准。 |

> 其余 38 处 bak 改动与本分支不重叠——隔离良好。

## 3. 建议合并顺序

1. **先让 bak 那摊 WIP 各自提交/落地**，把相关工作树弄干净（否则未提交改动会干扰合并与验证）。
2. 在 `aletheon` 仓库合并本分支（或 cherry-pick 9 个 commit）到目标主线。
3. 解决 `request.rs` 冲突（唯一代码冲突点），regenerate `Cargo.lock`。
4. 验证（见 §4）。

## 4. 验证（务必用 cargo 包装脚本，勿用裸 cargo）

> 本仓库多 agent 共享机器，**必须用 `scripts/cargo-agent.sh`**（共享 `CARGO_TARGET_DIR` + `flock` 串行化 + `CARGO_BUILD_JOBS=2`），裸 `cargo` 会引发 OOM/磁盘爆。

```bash
bash scripts/cargo-agent.sh build --workspace
bash scripts/cargo-agent.sh test -p gateway                       # 期望 38 passed
bash scripts/cargo-agent.sh test -p executive --test channel_dispatcher \
  --test telegram_goal_commands --test telegram_restart_recovery \
  --test google_telegram_query --test approval_channel --test gmail_goal_draft \
  --test gmail_channel_policy --test google_event_routing --test goal_worker_flow
bash scripts/cargo-agent.sh clippy --workspace
```

**解耦门（应通过）**：
```bash
rg -n -i "google|gmail" crates/gateway/src/dispatcher.rs        # 仅文档注释
rg -n "executive" crates/gateway/src crates/gateway/Cargo.toml  # 仅 lib.rs/ports.rs 文档散文，无代码/清单依赖
```

## 5. 已知遗留（非阻塞）
- **Phase 4 config 泛化**（`ChannelsConfig`）未做：推测性多渠道扩展、当前仅一个双工 transport、且触碰 bak 正改的 `cognit/config/mod.rs`。真正加第二渠道时再上。
- 既有无关失败 `session_manager::tests::compaction_tail_never_starts_with_tool_result`（属 C1-compaction 区，非本重构引入）。
- `channel/exec_server_client.rs` **保留**（`host/mod.rs:154,167` 实际调用，非死代码）。
