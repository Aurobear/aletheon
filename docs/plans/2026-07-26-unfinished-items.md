# Aletheon 未完成项追踪

> 基于 2026-07-25/26 实际分析+改码+部署全流程。本次加固回合已收口。
> 已合并 PR: #124–#137。最后更新: 2026-07-27。

## 1. gbrain 语义召回自动化 ✅ 完成(#135)

**结论**: gbrain pre-turn recall 已完整实现于 `context_assembler.rs:133`。修正了 `default.toml` 默认值。

## 2. diff 交互审批 ✅ 完成(#135 + #136)

**patch_delta 管道**: 结构化 diff 从 tool result → TUI 全链路贯通,expanded 模式渲染文件列表+hunks+bytes。

**approval detail 渲染**: TUI 审批弹窗显示完整 tool input(命令/diff),最多 14 行。

## 3. executive unwrap/panic 硬化 ✅ 完成(#137)

76 处 `lock().unwrap()` → `unwrap_or_else(|e| e.into_inner())`(poison-safe 模式),16 个文件。生产代码 0 剩余。592 测试全过。

## 4. code-agent/admin-agent 补 git_push ✅ 完成(#135)

`git_push` 加入 code-agent/admin-agent(.md + .toml)。其他 git 工具已在 `UNIVERSAL_TOOLS` 自动生效。

## 5. 计划中的"代码级 planner→agent_spawn"流水线(待立项)

`crates/cognit/src/core/planner.rs` 未接线,独立工程。单独立项。

## 6. provider 降级链验证(缺硬件)

`config/default.toml` 已有 Ollama 脚手架,需装 Ollama + pull 模型。

---

*最后更新:2026-07-27 | 基于 dev `5e2f8f4`*
