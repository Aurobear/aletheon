> Migrated from docs/design/core/hook-system.md — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# Hook 系统 (Hook System)

> Aletheon 的可扩展性核心，允许用户在推理循环的关键节点注入自定义逻辑。
>
> **从 `session-lifecycle.md` 提取** — 原文 §4.2。

**关联模块:** [Session 生命周期](../runtime/session.md), [安全模型](../corpus/security.md), [工具系统](../corpus/tools.md)

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Hook system | ✅ Implemented | `hook/mod.rs`, `hook/types.rs` | 21 event types, 3-layer TOML config, command hooks |
| PluginHookDef | ✅ Implemented | `plugin/manifest.rs` | Plugin hook registration |

---

## 1. 事件类型系统 (HookEventName)

**HookEventName** — 14 种 hook 事件类型，覆盖工具执行、权限请求、上下文压缩、会话生命周期、子 agent、LLM 调用、感知、安全等推理循环的每个阶段。
- 代码位置: Hook 系统配置

事件类型包括：PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, SessionStart, UserPromptSubmit, SubagentStart, SubagentStop, Stop, PreLLMCall, PostLLMCall, PerceptionEvent, SecurityViolation

## 2. 每事件类型化 Request/Outcome

参考 Codex 的 per-event Request/Outcome pairs，每个事件有强类型的输入和输出结构体。

**核心结构体：**
- **PreToolUseRequest/Outcome** — 工具执行前的完整上下文（session_id, tool_name, tool_input, subagent 上下文等）；Outcome 支持 should_block、block_reason、additional_contexts、updated_input（last-writer-wins）
- **PostToolUseRequest/Outcome** — 工具执行后的上下文（含 tool_result, duration_ms）；Outcome 支持 should_stop、stop_reason
- **SessionStartRequest** — 会话启动（source: Startup/Resume/Clear/Compact）
- **StopRequest** — 会话停止
- **CompactRequest** — 上下文压缩（trigger: Manual/Auto, messages_before, token_usage）
- **SubagentHookContext** — 子 agent 上下文（agent_id, agent_type, parent_session_id）
- **HookCompletedEvent** — 已完成的 hook 事件日志

## 3. Hook 定义与类型

**Hook** — 注册到 HookSystem 的单个 hook 定义，包含 id、name、event_name、matcher、hook_type、execution、scope、trust_status、timeout_sec、source 等字段。

**三种 Hook 类型 (HookType)：**
- **Command** — Shell 命令，通过 stdin/stdout 传递 JSON，exit code 语义（0=成功, 2=阻止, 其他=失败）
- **Prompt** — LLM Prompt 模板，由模型生成决策
- **Agent** — 完整的 agent 实例（WASM / Rust 子进程）

**枚举类型：**
- **ExecutionMode** — Sync（阻塞）/ Async（异步）
- **HookScope** — Thread / Turn / Global
- **HookSource** — System / User / Project / Plugin / Enterprise / SessionFlags
- **HookTrustStatus** — Managed / Trusted / Modified / Untrusted

## 4. Matcher 分发机制

**Matcher 分发** — 参考 Codex 的 `select_handlers`，支持三种 matcher 模式：
- None（匹配所有）
- 字面量管道分隔（`Edit|Write`）
- 正则表达式（`^Bash$`）

通过 `select_handlers()` 从配置的 handler 列表中选择匹配的 handler。`ConfiguredHandler` 包含 event_name、matcher、hook_type、timeout_sec、source、trust_status、enabled 等字段。

## 5. 并发执行与冲突解决

**并发执行** — 参考 Codex 的 `FuturesUnordered` 模式：handler 并发执行，但保留声明顺序用于报告，使用完成顺序解决冲突（last-writer-wins）。

**核心类型：**
- **ParsedHandler** — 带排序信息的 handler 执行结果（configured_order + completion_order）
- **HandlerResult** — Success/Blocked/Failed/TimedOut 四种结果
- **HandlerOutput** — stdout JSON 输出，包含 continue_execution、stop_reason、suppress_output、system_message、additional_context
- **HookSpecificOutput** — 按事件类型区分的特定输出（PreToolUse: permission_decision + updated_input; PostToolUse: additional_context; SessionStart: system_message; Stop: prevent_stop + continuation_fragments; PreCompact: prevent_compaction）
- **PermissionDecision** — Allow/Deny/Ask

## 6. 命令执行与 Exit Code 语义

**命令执行** — 通过 shell 执行 handler 命令，注入环境变量（ALETHEOND_HOOK_EVENT, ALETHEOND_CWD），通过 stdin 传递 JSON 输入，解析 stdout JSON 输出。

**Exit code 语义：**
- 0 = 成功，解析 stdout JSON
- 2 = 阻止，stderr = 阻止原因
- 其他 = 失败

## 7. 信任模型

**信任管理器 (TrustManager)** — 参考 Codex 的 hook trust system：每个 hook 有身份 hash，用户必须明确信任新 hook。

- Managed（系统/MDM 安装）自动信任
- Trusted（用户明确信任，hash 匹配）检查 hash 一致性
- Modified（hash 变更）需要重新信任
- Untrusted（新发现）需要用户确认
- `bypass_hook_trust` 开发模式可绕过检查

## 8. 输出溢出管理

**HookOutputSpiller** — 参考 Codex 的输出溢出管理器，超过 32KB 的 hook 输出写入临时文件，返回文件引用而非内联文本。

## 9. Preview API

**Preview API** — 参考 Codex 的 `preview_*` 方法，在执行前显示哪些 hook 会运行、信任状态、启用状态。

**HookSystem** — Hook 管理核心，包含：
- handlers（按 source 分层的已配置 handler 列表）
- trust（TrustManager）
- spiller（HookOutputSpiller）
- config_layers（配置层栈，用于 precedence 管理）

**配置层 (HookConfigLayer)** — source + hooks + precedence，支持从分层配置加载。

**Preview** — `preview(event, matcher_inputs)` 返回 `Vec<HookRunSummary>`，包含 id、name、source、trust_status、enabled、status、timeout_sec。

**PreToolUse 执行流程：**
1. 从 matcher_inputs 选择匹配的 handler
2. 过滤未信任或禁用的 handler
3. 并发执行，收集结果
4. 按 completion_order 排序（last-writer-wins for updated_input）

## 10. 分层配置发现

```
配置来源（低到高优先级）：
  1. System   (/etc/agent/hooks/          ) — 系统管理员配置
  2. User     (~/.config/agent/hooks/     ) — 用户个人配置
  3. Project  (.agent/hooks/              ) — 项目级配置
  4. Plugin   (plugin-provided hooks      ) — 插件注册的 hook
  5. SessionFlags (runtime overrides      ) — 运行时覆盖

每个来源：hooks.json 或 hooks.toml
状态合并：后层覆盖前层（field-by-field: enabled, trusted_hash）
```

Hook 配置文件示例：

```yaml
# /etc/agent/hooks.yaml (System layer)
hooks:
  - name: block-dangerous-commands
    event: PreToolUse
    matcher: "^rm$|^Bash$"  # 只匹配 rm 和 Bash 工具
    type: command
    program: /usr/lib/agent/hooks/safety-check.sh
    execution: sync
    scope: global
    timeout_sec: 30

  - name: audit-tool-calls
    event: PostToolUse
    type: command
    program: /usr/lib/agent/hooks/audit-logger.py
    execution: async
    scope: global

  - name: inject-system-context
    event: PreLLMCall
    type: command
    program: /usr/lib/agent/hooks/context-inject.sh
    execution: sync
    scope: thread

  - name: block-compaction
    event: PreCompact
    matcher: "auto"  # 只阻止自动压缩，允许手动
    type: command
    program: /usr/lib/agent/hooks/check-compaction.sh
    execution: sync
    scope: global

  - name: subagent-monitor
    event: SubagentStart
    matcher: ".*"  # 匹配所有 agent 类型
    type: command
    program: /usr/lib/agent/hooks/log-subagent.sh
    execution: async
    scope: turn
```

## 11. 与 SessionStore 和 ToolRunner 集成

**SessionStore 集成：**
- `init_for_session(session_id, base_dir)` — session 创建后初始化 spiller
- `cleanup_for_session(session_id)` — session 结束前清理溢出文件

**ToolRunner 集成：**
- `pre_tool_use_hook()` — 工具执行前调用，可阻止执行或修改输入（PreToolUseDecision::Allow/Block）
- `post_tool_use_hook()` — 工具执行后调用，可注入额外上下文或停止（PostToolUseDecision::Continue/Stop）
- Hook 执行事件记录到 SessionEventJournal（HookExecuted 事件体）

---

## Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| Hook system core | `hook/mod.rs` | `HookSystem`, `HookConfigLayer` |
| Hook types | `hook/types.rs` | `HookEventName`, `HookType`, `HookScope`, `HookSource` |
| Plugin hooks | `plugin/manifest.rs` | `PluginHookDef` |
