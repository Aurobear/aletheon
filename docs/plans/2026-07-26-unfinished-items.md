# Aletheon 未完成项追踪

> 基于 2026-07-25/26 实际分析+改码+部署全流程,记录仍待解决的项目。
> 已完成项见 dev 分支 PR #124–#136。最后更新:2026-07-27。

## 1. gbrain 语义召回自动化 ✅ 完成(#135)

**原假设(错误)**:pre-turn recall 在 `pre_turn.rs` 有意留空,gbrain 从未自动检索。

**实测结论**:gbrain pre-turn recall **已完整实现**在 `context_assembler.rs:133-158`(`ProductionContextSource::load()`)。调用链: `memory_service.recall()` → `CompositeMemoryService` → `SupplementalMcpAdapter` → MCP `query`/`search`。

**修正**:
- `config/default.toml`: `memory.gbrain.enabled = true`, `server_name = "gbrain"`
- 补充了 MCP server 模板注释(含 `trust = "RemoteTrusted"` 和 `bearer_token_env`)
- 更新了注释说明 fail-open 安全性

## 2. diff 交互审批 ✅ 完成(#135 + #136)

**patch_delta 管道**(#135): 结构化 diff 数据从 tool result → TUI 的完整管道:
`ToolResultEvent` → `TurnEventV1::ToolResult` → `ClientEvent::ToolCallResult` → `ExecEntry`
TUI expanded 模式渲染文件列表(+hunks, +bytes delta),失败项红色显示。

**approval detail 渲染**(#136): daemon 已经发送 `detail`(完整 tool input JSON)在 `approval_request` 通知中(`turn_pipeline.rs:862`)。TUI 的 `ApprovalDialog` 现在读取并渲染 detail(最多 14 行,dark gray),popup 动态扩展高度。

## 3. executive unwrap/panic 硬化(低-量太大)

无变化。~1200+ `.unwrap()` 分散在 executive 中,已知热路径 panic 均位于 `#[cfg(test)]`。建议按热度优先逐步做,不急。

## 4. code-agent/admin-agent 补 git_push ✅ 完成(#135)

`git_push` 加到 `code-agent` 和 `admin-agent`(`.md` + `.toml`)。其他 git 工具已在 `UNIVERSAL_TOOLS` 中自动生效。

## 5. 计划中的"代码级 planner→agent_spawn"流水线(极低-待立项)

无变化。独立工程,建议单独立项。

## 6. 未验证的 provider 降级链(低-缺硬件)

无变化。`config/default.toml` 已有 Ollama 脚手架,需装 Ollama + pull 模型 + 取消注释即生效。

---

*最后更新:2026-07-27 | 基于 dev 分支 `1e02ad7`*
