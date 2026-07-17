# Typed Lifecycle Extensions

## 1. Grok 的设计原则

Grok 的 lifecycle crate 明确规定：contributor 在 dispatch 时只接收 data-only input；它能使用的 capability 在安装时注入；contributor 永不拥有 loop control（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-agent-lifecycle/src/lib.rs:1-16`）。注册表 builder 按 turn lifecycle、session lifecycle、turn input、command 四类保存 typed contributor（`/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-agent-lifecycle/src/send/registry.rs:8-35`）。

这个原则比“所有插件拿一个可变 Agent/Executive 句柄”安全得多。

## 2. Aletheon 当前情况

Aletheon 已有 `SessionLifecycleUseCases`，负责 reset turn token、finish、start；生产实现还执行 Corpus hooks（`crates/executive/src/service/request_use_cases.rs:330-377`）。这说明生命周期行为已经存在，但边界仍偏 use-case/具体实现，尚未形成通用、类型化、可排序的 contributor 模型。

## 3. 推荐边界

```text
Executive / UnifiedTurnCoordinator     <-- 唯一 loop owner
     |
     +-- build immutable LifecycleInput
     |
     +-- dispatch ordered contributors
     |      +-- observability
     |      +-- memory projection
     |      +-- corpus hooks
     |      +-- checkpoint
     |      `-- client notifications
     |
     `-- interpret bounded LifecycleEffects
```

建议 contributor 只返回声明式 effect，而不直接推进 turn：

- `Continue`
- `AddContextFragment`
- `EmitEvent`
- `RequestCheckpoint`
- `RequestCancellation`
- `RejectInput { reason }`

高风险 effect 必须由 Executive 重新授权或执行，contributor 不能直接调用 tool。

## 4. 候选 hook 点

| Session | Turn | Tool/Capability |
|---|---|---|
| before_session_start | before_turn_input | before_admission |
| after_session_start | after_context_projection | after_admission |
| before_session_end | before_model_call | after_tool_progress（只观察） |
| after_session_end | before_tool_batch | after_tool_terminal |
| idle | after_turn_terminal | on_abort/on_error |

不要一次实现所有 hook。第一批应只覆盖 Aletheon 已经存在且有明确调用点的 session start/end、turn start/end、tool terminal。

## 5. 顺序与错误语义

- 注册时确定稳定顺序：阶段 + priority + contributor id。
- 同一 contributor id 重复注册应报错。
- 输入不可变；effect 有大小与数量上限。
- observability contributor 失败默认降级；authority/security contributor 失败必须 fail closed。
- 每次 dispatch 记录耗时和 outcome，防慢 hook 隐藏 turn 延迟。
- contributor 不能持有 Executive 内部具体类型；依赖通过最小 capability port 注入。

## 6. 多用户与安全

Lifecycle input 可包含 principal/thread/session 的只读标识，但不能暴露可伪造 approval authority。若 contributor 需要发起操作，应返回 request effect，由 Executive 使用当前可信 context 决策。

这延续 Aletheon 当前做法：可信 execution context 由 Executive 注入，模型只提供 capability call（`crates/executive/src/service/governed_capability.rs:1-5`）。

## 7. 验收方向

- contributor 无法自己推进/重入 turn loop。
- 固定输入产生固定 contributor 顺序。
- non-critical contributor 故障被隔离并可观察。
- security contributor 故障 fail closed。
- context fragment 有总字节上限并带来源。
- session/turn/tool terminal 的现有行为在迁移后不改变。

## 8. Grok's Concrete Hook System vs. Typed Contributor Model

### 8.1 Two Design Philosophies

Doc 05 proposes a **typed, ordered contributor model** where lifecycle participants return declarative effects that the Executive re-authorizes. Grok chose a different path: a **command-execution hook system** where external processes receive JSON on stdin and produce JSON on stdout.

Neither is universally superior — they optimize for different goals:

| Dimension | Typed Contributor (proposed) | Command Hook (Grok) |
|---|---|---|
| **Safety** | Contributor can't call tools, only return effects | Hook receives read-only input, can only return allow/deny (PreToolUse) |
| **Extensibility** | New contributor requires Rust code + recompile | New hook is a shell script / binary in a directory |
| **Observability** | Structured effect types, easy to audit | JSON envelopes with typed payloads, structured but external |
| **Performance** | In-process, no serialization overhead | Process spawn per event (acceptable for non-hot-path events) |
| **Failure mode** | Panic in contributor = compile-time safety (ideally) | Hook crash = fire-and-forget timeout + log |
| **Trust model** | Contributors are trusted (compiled in) | Hooks can be untrusted — trust-gated by Folder Trust |

### 8.2 Grok's 15 Hook Events

Grok supports significantly more hook points than the candidate list in §4:

| Category | Events |
|---|---|
| **Session lifecycle** | `SessionStart`, `SessionEnd`, `Stop`, `StopFailure` |
| **Tool events** | `PreToolUse` (blocking), `PostToolUse`, `PostToolUseFailure`, `PermissionDenied` |
| **User / notification** | `UserPromptSubmit`, `Notification` |
| **Subagent** | `SubagentStart`, `SubagentStop` (alias: `SubagentEnd`) |
| **Compaction** | `PreCompact`, `PostCompact` |

Source: `/home/aurobear/Bear-ws/grok-build/crates/codegen/xai-grok-hooks/src/event.rs:13-49`

### 8.3 Key Design Details Worth Borrowing

**Blocking vs. non-blocking**: Only `PreToolUse` is blocking (the hook can deny the tool call). All other events are fire-and-forget — the hook's exit code is logged but never blocks the agent. This is a clean separation: one gate, everything else is observation.

**Matcher patterns**: Non-lifecycle events support matchers so a hook only fires for specific tools or conditions. Lifecycle events (`SessionStart`, `SessionEnd`, `Stop`, `UserPromptSubmit`) fire unconditionally.

**Fail-open by default**: Hook failures (crash, timeout, bad JSON) do not block the agent. The hook system explicitly states "fail-open by default" in its module docs.

**Payload truncation**: Hook input is capped at 128KB with char-boundary-safe truncation. This prevents a hook from being fed a 10MB tool result that would OOM the hook process.

**Trust integration**: Hooks from untrusted repos don't run. The hook trust module (`xai-grok-hooks/src/trust.rs`) gates loading based on the Folder Trust decision.

**Canonical event names with aliases**: Event names accept PascalCase, snake_case, and camelCase during deserialization for migration compatibility. `SubagentEnd` is a documented alias for `SubagentStop`.

### 8.4 Recommended Hybrid Approach for Aletheon

Rather than choosing one model, Aletheon should combine both:

```text
                    Typed Contributors (in-process)
                    - Security/authority hooks
                    - Memory projection
                    - Budget enforcement
                    - Conscious arbitration
                    -------------------------------
Hook Point (Session/Turn/Tool boundaries)
                    -------------------------------
                    Command Hooks (external process)
                    - User-defined notifications
                    - CI/CD integration scripts
                    - Custom logging/telemetry
                    - Repo-local workflow triggers
```

The rule: **typed contributors for anything that affects authority, safety, or the correctness of the turn loop. Command hooks for user-defined observation and integration.** Grok's insight that only `PreToolUse` is blocking and everything else is fire-and-forget is the right boundary.

### 8.5 Suggested Aletheon Hook Events

Starting from Grok's list, adapting to Aletheon's domain:

| Event | Type | Blocking? | Aletheon Notes |
|---|---|---|---|
| `SessionStart` | Lifecycle | No | Principal + workspace identity in payload |
| `SessionEnd` | Lifecycle | No | Reason + turn/tool counts |
| `PreCapabilityCall` | Tool | **Yes** | Equivalent to PreToolUse; can deny |
| `PostCapabilityCall` | Tool | No | Result + duration + usage |
| `CapabilityCallFailure` | Tool | No | Error classification |
| `PermissionDenied` | Tool | No | What was denied and why |
| `UserPromptSubmit` | Lifecycle | No | Prompt text (truncated) |
| `PreConsciousArbitration` | Compaction | No | Before context slot compaction |
| `PostConsciousArbitration` | Compaction | No | What was kept/dropped/summarized |
| `SubagentSpawn` | Agent | No | Child agent identity + budget |
| `SubagentComplete` | Agent | No | Outcome + resource settlement |
| `Notification` | UI | No | Permission prompts, idle, status |

These map naturally to Aletheon's existing event spine (`TurnEventV1`) while adding the external-process extension point that Grok demonstrates.

