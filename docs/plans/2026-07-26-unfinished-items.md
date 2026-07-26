# Aletheon 未完成项追踪

> 基于 2026-07-25/26 实际分析+改码+部署全流程,记录仍待解决的项目。
> 已完成项见 dev 分支 PR #124–#134。最后更新:2026-07-27。

## 1. gbrain 语义召回自动化(高) ✅ 已修正认知

**原假设(错误)**:pre-turn recall 在 `pre_turn.rs` 有意留空,gbrain 从未自动检索。

**实测结论(2026-07-27)**:

gbrain 的 pre-turn recall **已经完整实现**在 `context_assembler.rs:133-158`(`ProductionContextSource::load()`)。调用链:

```
ProductionContextSource::load()
  → memory_service.recall(RecallRequest { query: user_input, ... })
    → CompositeMemoryService::recall()
      → SupplementalMemoryBackend::recall_inner()
        → SupplementalMcpAdapter::query() / search()
          → McpManager::call_tool("gbrain", "query", ...)
```

本地 `~/.aletheon/config.toml` 已配置 `memory.gbrain.enabled = true` + `server_name = "gbrain"`,故**本地 gbrain recall 已实际生效**(fail-open,gbrain 不可达时 silence 降级为空上下文)。

**本次修正**:
- `config/default.toml`: `memory.gbrain.enabled` false→true, `server_name` "supplemental"→"gbrain"
- 补充了 MCP server 模板注释(含 `trust = "RemoteTrusted"` 和 `bearer_token_env`)
- 更新了注释说明 fail-open 安全性

**待验证**:实际 daemon 日志中是否有 recall hit/miss 的 trace。

## 2. diff 交互审批(中) 🔄 patch_delta 管道已通

**本次完成**:patch_delta 结构化数据从 tool result → TUI 的全链路管道:

| 层 | 文件 | 变更 |
|---|---|---|
| ToolResultEvent | `crates/cognit/src/harness/event_sink.rs` | +`patch_delta: Option<PatchDelta>` |
| TurnEventV1::ToolResult | `crates/fabric/src/ipc/stream.rs` | +`patch_delta` field |
| ClientEvent::ToolCallResult | `crates/fabric/src/events/ui_event.rs` | +`patch_delta` field |
| 转换层 | `turn_pipeline.rs`, `format.rs` | 传递 patch_delta |
| TUI 渲染 | `crates/interact/src/tui/chat.rs` | expanded 时渲染文件列表(+ hunks + bytes delta),失败项红色显示 |
| TUI 事件处理 | `crates/interact/src/tui/response.rs` | `update_exec_with_delta()` |

**仍待实现**:交互式 approve/reject(用户按快捷键批准后 daemon 重新执行无 dry_run)。需要 daemon 侧审批管线 + TUI 快捷键绑定。建议复用现有 `ApprovalDialog` 模态框(Path A)。

## 3. executive unwrap/panic 硬化(低-量太大)

无变化。~1200+ `.unwrap()` 分散在 executive 中,已知热路径 panic 均位于 `#[cfg(test)]`。建议按热度优先逐步做,不急。

## 4. code-agent/admin-agent/safe-agent 补 git 工具(低) ✅ 完成

**已修正**:`git_push` 加到 `code-agent` 和 `admin-agent`(`.md` + `.toml`)。其他 git 工具已在 `UNIVERSAL_TOOLS` 中自动生效。`safe-agent` 保持只读(不加 git_push)。

## 5. 计划中的"代码级 planner→agent_spawn"流水线(极低-待立项)

无变化。独立工程,建议单独立项。

## 6. 未验证的 provider 降级链(低-缺硬件)

无变化。`config/default.toml` 已有 Ollama 脚手架,需装 Ollama + pull 模型 + 取消注释即生效。

---

*最后更新:2026-07-27 | 基于 dev 分支当前 HEAD*
