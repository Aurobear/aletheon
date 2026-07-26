# Aletheon 未完成项追踪

> 基于 2026-07-25/26 实际分析+改码+部署全流程,记录仍待解决的项目。
> 已完成项见 dev 分支 PR #124–#133。

## 1. gbrain 语义召回自动化(高)

**现状**:gbrain MCP 已连接(配置 `memory.gbrain.enabled = true`)、daemon 启动时有 40+ `gbrain__*` 工具注册。但 pre-turn recall 路径在 `crates/executive/src/application/pre_turn.rs:13` **有意留空**:

```rust
// Memory is projected only through Mnemosyne -> Agora candidates ->
// conscious context. The legacy TurnServices recall hook remains a
// compatibility contract but is intentionally not prompt-injected.
```

**问题**:gbrain 有全部检索能力(语义搜索/代码引用/本体查询),但从未在 turn 前**自动检索并注入上下文**。配置里的 `memory.recall.inject_into_context` 和 `recall_limit` 目前不经过 gbrain。

**可能入口**:
- `crates/executive/src/application/conscious_workspace.rs` — 每轮前从 Agora workspace 取 candidate slots(黑板上推的)
- `crates/mnemosyne/src/recall/pipeline.rs` — mnemosyne 侧召回管线
- `crates/executive/src/host/daemon/bootstrap/memory.rs` — memory 组初始化

**建议**:用 `aletheon exec` 做一个 controlled test:"在一个有已知 gbrain 页面的话题上提问,观察模型是否能引用 gbrain 的结果"——先证明确实没进,再决定补哪个注入点。

## 2. diff 交互审批(中)

**现状**:`apply_patch dry_run` 已实现(#125),agent 可以预览不改盘。但没有 **TUI 层交互**:改前不能点"接受/拒绝"——apply 就是 apply。

**涉及文件**:
- `crates/corpus/src/tools/tools/apply_patch.rs` — dry_run 已有
- `crates/interact/src/tui/` — 需新增 diff 预览 + 批准流

**建议**:在 TUI 的 tool-result 渲染中,对 `apply_patch`(`dry_run=false`)的结果新增一个"approve/reject"交互区,渲染 unified diff + 文件列表 + 行数 delta,用户按快捷键批准后重新执行(无 dry_run 真正落盘)。

## 3. executive unwrap/panic 硬化(低-量太大)

**现状**:`attempt_coordinator.rs` 11 处 `Mutex::lock().unwrap()` → `into_inner()` 已硬化。executive 还有其他 ~1200+ `.unwrap()`——分散、非系统性风险。已知确切热路径 panic:

| 文件 | 行 | 风险 |
|---|---|---|------|
| `agent_control/settlement.rs` | (test 模块内) | 子 agent settlement db 写失败 → 理论上 test-only(原点名位置不在生产) |
| `daemon_turn/helpers.rs:171` | (test 模块内) | panic on unexpected block type — test-only |

**原分析报告中的"30+ settlement unwrap"和"helpers panic"都是 `#[cfg(test)]` 内。**

> 实际生产 hotspot 未完全盘点;建议打一次 `grep -rn "\.unwrap()" crates/executive/src --include=*.rs | grep -v test` 按热度优先挑选调用频率最高的函数做。

## 4. code-agent/admin-agent/safe-agent 补 git 工具(低-consistency)

**现状**:`general-agent` 有全套 git(读+写+push),但 code-agent/admin-agent 只配到 `git_status/diff/log/show`,**没有 undo(stash/restore/reset)和写(add/commit/branch/push)**。git 读+undo 已在 `UNIVERSAL_TOOLS` 里自动有效,但写的名字不对就不行。

**修改文件**:
- `agents/code-agent.md` + `agents/admin-agent.md` + `agents/safe-agent.md` — 更新 `tools:` 列表
- 以及对应的 `.toml` 副本(若有)

## 5. 计划中的"代码级 planner→agent_spawn"流水线(极低-待立项)

原始"5 项增强 #5"最终按模型驱动分解实现(general-agent 自动列 task_create 计划)。真正把 `crates/cognit/src/core/planner.rs`(未接线,只 output 抽象 Action name 无执行器)接到 agent_spawn/执行流水线**是多轮/多天的独立工程**,需要:任务复杂度判定、spawn 编排与排队、结果 merge/reduce、失控防护、UI 进展展示。建议**单独立项**,不在日常加固范围内。

## 6. 未验证的 provider 降级链(低-缺硬件)

**现状**:`scheduler.rs` 的跨-provider 故障转移代码(含 context-overflow→下一个 provider)已落地。但本机**只配了 leju 一个 provider、未装 Ollama**,故障转移无法实测。`config/default.toml` 里已有 Ollama 脚手架(注释掉),安装 Ollama + pull 模型 + 取消注释即生效。

---

*最后更新:2026-07-26 | 基于 dev `8a12d89` + 线上部署 `7fdc6e4a`*
