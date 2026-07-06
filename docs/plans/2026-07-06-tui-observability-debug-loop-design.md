# Aletheon TUI 可观测 + 自动化调试闭环 — 设计文档

**Date**: 2026-07-06
**Status**: Approved (design), pending implementation plan
**Scope**: 本期只做**观测与自动化能力**。已定位的 T/D/I 类 bug 留待下一轮用这套能力回归修复，不在本设计实现范围内。

---

## 1. 背景与动机

当前 aletheon 的调试自动化（`aletheon-tester` 技能 + `aletheon-monitor` MCP）对 **TUI 渲染层是全盲的**：

- `aletheon-monitor` 的 9 个工具（`health/snapshot/analyze/journal/logs/memory/sessions/ask/watch`）**全部是 daemon 侧 JSON-RPC 内省**。
  - `ask.py` 走 `session.ask` RPC；`watch.py` 轮询 `session.journal`。
  - `grep pty/tmux/render` 在 monitor 源码中全空 → **零终端捕获能力**。
- 而真实使用中观察到的一半问题**只存在于 TUI 渲染层**，RPC 路径根本不经过：
  - 整块输出重复绘制（dup-render）。
  - markdown 表格原样打印、宽字符换行破碎。
  - `Reflection: Reflection:` 双前缀 — 加在 `crates/interact/src/tui/response.rs:212`。
  - 绝对路径被当技能 `未知技能: /home/...` — 在 `crates/interact/src/tui/app/submit.rs:25`。

**结论**：现有自动化只能测"agent 答得对不对"，测不了"用户实际看到的对不对"。要做到"基于日志**和实际 TUI**"的调试，必须补上一个**真实 TUI 捕获层**，并把它与 daemon 内省信息关联。

## 2. 目标 / 非目标

**目标**
- Claude 能像用户一样"看到"真实渲染的 TUI 帧。
- 把 TUI 帧 ⟷ journal 事件 ⟷ daemon 日志 按时间对齐，形成一站式诊断信息。
- 驱动 `aletheon-tester` 全自动闭环：发任务 → 看渲染 → 诊断 → 改 → 重建 → 回归。
- 纯函数模块可独立单测；用真实 session 输出做 fixture 复现 dup-render。

**非目标（本期不做）**
- 不修 T/D/I 类具体 bug（重复渲染、daemon 权限、超时、/dev/null、slash 解析、session 持久化、audit session_id）。
- 不改 aletheon TUI 渲染路径本身（避免"调试渲染路径与真实路径漂移"）。

## 3. 关键设计决策

| 决策 | 选择 | 理由 |
|---|---|---|
| TUI 捕获方式 | **tmux capture-pane** | tmux 3.7b 已装、零新依赖；抓到的就是用户肉眼所见；复用生态里已有的 tmux `debug-loop` 模式。否决 pyte（需安装）、否决 Rust headless 渲染（与真实路径漂移，未必复现真实 bug）。 |
| 集成方式 | **混合式**：纯 `tui_session.py` + 薄 MCP 封装 | 同时拿到"可单测的纯核心"与"Claude 面对统一 MCP 表面"两个好处。 |
| 信息形态 | **`aletheon_diagnose` 一站式诊断包** | 一次调用覆盖 用户所见 + agent 内部 + 系统日志 三层。 |

## 4. 架构

```
                 ┌─────────────────────────────────────────────┐
Claude ──MCP──▶  │ aletheon-monitor (FastMCP)                   │
                 │  既有 9 工具: health/journal/logs/ask/watch… │
                 │  新增薄封装:  aletheon_tui_* / aletheon_diagnose│
                 └───────┬───────────────────────┬─────────────┘
                         │ import                 │ import
                 ┌───────▼────────┐      ┌────────▼─────────┐
                 │ tui_session.py │      │ frame.py         │
                 │ tmux 生命周期  │      │ 归一化/diff/稳定 │
                 │ start/send/cap │      │ 判定 (纯函数)    │
                 └───────┬────────┘      └────────┬─────────┘
                         │ tmux send-keys/capture │ 输入帧文本
                 ┌───────▼────────┐      ┌────────▼─────────┐
                 │ tmux pane:     │      │ tui_checks.py    │
                 │ 真实 aletheon  │◀socket▶ 渲染断言:dup/    │
                 │ TUI ── daemon  │      │ markdown/错误横幅 │
                 └────────────────┘      └──────────────────┘
```

### 4.1 隔离单元（职责/接口/依赖）

| 单元 | 做什么 | 接口 | 依赖 |
|---|---|---|---|
| `src/tui_session.py` | 在 tmux 里起/停真实 TUI，send-keys 输入，capture-pane 抓帧（含 `-S -` 抓 scrollback） | `start(cmd) -> session`; `send(session, text, submit)`; `capture(session, scrollback) -> str`; `stop(session)` | tmux, subprocess |
| `src/frame.py` | 去 ANSI、去尾部空行、帧间 diff、稳定判定（连续 N 帧不变 = 渲染完成） | `normalize(raw) -> str`; `is_stable(frames) -> bool`; `diff(a, b) -> str` | 纯函数，无 IO |
| `src/tui_checks.py` | 对渲染帧做断言：重复块、raw markdown 管道、`未知技能`、`Permission denied`、`Reflection: Reflection:` | `run_checks(frame) -> list[Finding]` | 纯函数，输入帧文本 |
| `src/tools/tui.py` + `server.py` | 把上面暴露成 MCP 工具 + `aletheon_diagnose` 关联 | MCP Tool 定义 | 上述三模块 + 既有 client |

## 5. 工具面（Claude 可调用）

新增：

- **`aletheon_tui_start(task?)`** — 在 tmux 起 TUI，返回 session 名 + 首帧。
- **`aletheon_tui_send(text, submit=true)`** — 输入文本（支持多行/slash/Enter），可选自动回车。
- **`aletheon_tui_capture(scrollback=true, wait_stable=true)`** — 返回渲染帧文本 + 稳定状态 + 触发的 `tui_checks` 命中项。
- **`aletheon_tui_stop()`** — 清理 tmux。
- **`aletheon_diagnose(task)`** — 一站式：起 TUI → 发任务 → 等稳定 → 抓帧，并联既有 `analyze`（snapshot+perf+journal+anomaly）+ `logs`，按时间戳把 TUI 帧 ⟷ journal 事件 ⟷ 日志 关联，输出：

```json
{
  "rendered_frame": "<归一化后的 TUI 文本>",
  "tui_checks": [{"kind": "dup_render", "evidence": "...", "severity": "high"}],
  "daemon": { "analyze": {...}, "logs": [...] },
  "timeline": [{"ts": "...", "source": "tui|journal|log", "event": "..."}],
  "verdict": "pass|fail",
  "audit_tail": [...]      // 并入 .aletheon-audit.jsonl 末 N 条
}
```

> `audit_tail` + config 快照按用户确认并入 `aletheon_diagnose`，作为"大部分我们需要的信息"的一部分。

## 6. 数据流（一次自动化回归）

1. `aletheon_tui_start` → tmux pane 跑 `aletheon`（连 daemon）。
2. `aletheon_tui_send("<测试任务>")`。
3. 轮询 `capture-pane`，`frame.is_stable` 判定稳定（1.5s 不变或输入提示复现）。
4. `tui_checks.run_checks` 扫描帧：重复块 / raw markdown / 错误横幅。
5. 并联拉 `analyze` + `logs` + `audit_tail`，按 ts 对齐成 timeline。
6. `verdict` = TUI 断言 ∧ daemon 无 CRITICAL ∧ 错误率 < 10%。
7. 失败 →（下一轮）定位到 `crates/interact/src/tui/*` 或 daemon，改 → `cargo build` → `systemctl restart` → 回到 1 回归。

## 7. 错误处理

- tmux/pane 起不来 → 明确报错，不静默。
- TUI 连不上 socket → 回退提示"先 restart daemon"。
- 帧永不稳定（流式卡死/无限刷新）→ 超时上限 90s 后返回"疑似 dup-render/死循环"并附最后 3 帧。
- capture 空/乱码 → 保留 raw 供人工看。
- 全程只读地拉日志；改代码/重启走 `aletheon-tester` skill 既有 guardrail（只动 `aletheon/`、一次一改、restart 允许、其它破坏性操作需确认）。

## 8. `aletheon-tester` 技能改造

在现有 6 阶段闭环里加一条 **TUI track**：

- **Phase 2 (Test)**：除 `aletheon_ask`（RPC）外，增加**经真实 TUI 发任务**这条同源路径。
- **Phase 3/4 (Monitor/Analyze)**：`aletheon_diagnose` 补充纯 RPC 分析，断言里加入 TUI 渲染检查。
- 新增「渲染回归」判据表（dup / markdown / 换行 / 前缀）。
- Root-cause 表补 TUI 层映射：
  - `重复块 → tui/response.rs + tui/chat.rs`
  - `双前缀 → tui/response.rs:212`
  - `路径误判 → tui/app/submit.rs:25`
  - `markdown 不渲染 → tui/markdown.rs`

## 9. 测试策略

- `frame.py` / `tui_checks.py` 纯函数 → 用真实 session 输出（如 `Bear-ws/tmp.md`）做 fixture 单测；**重复块、raw markdown 必须被检出**。
- `tui_session.py` → 对一个假的 `cat`/`bash` TUI 做 smoke（起/发/抓/停）。
- 端到端 → 对真实 daemon 跑一次 `aletheon_diagnose`，确认能复现 T1 重复渲染。

## 10. 已知问题清单（下一轮用本能力回归修复）

**TUI 渲染层**
- T1 🔴 整块输出重复绘制 — `tui/response.rs` + `tui/chat.rs`（`86c4dd9` 去重未生效）。
- T2 🟡 `Reflection: Reflection:` 双前缀 — `tui/response.rs:212` 拼前缀，summary(`reflection.rs:188`) 已含。
- T3 🟡 markdown 表格原样打印 / JSON 拦腰换行 — `tui/markdown.rs` + 按字节换行。

**Daemon 权限 / 执行**
- D1 🔴 降权后写不进用户项目，落到 `/tmp` — daemon UID ≠ 项目属主。
- D2 🟠 `cargo check` 30s 必超时 — `bash_exec.rs:48` 默认 10s、无长命令上限。
- D3 🟠 `/dev/null: Permission denied` — 非 root 沙箱（`bubblewrap.rs:63`），`1b46ec4` 未覆盖 bash 重定向。

**输入 / 持久化**
- I1 🟠 绝对路径被当技能 — `tui/app/submit.rs:25` `starts_with('/')` 无区分。
- I2 🔴 session 完全不落盘，`/resume` 空壳 — `~/.aletheon/sessions/*` 全空，journal/checkpoint 未写。
- I3 🟠 audit `session_id` 恒为空 — tool_exec 路径未透传真实 session_id。

## 11. 验收标准

- [ ] Claude 可通过 MCP 起真实 TUI、发任务、抓到渲染帧文本。
- [ ] `aletheon_diagnose` 返回帧 + tui_checks + daemon analyze + 时间线 + audit_tail。
- [ ] `tui_checks` 能在 fixture 上检出 dup-render 与 raw markdown。
- [ ] `aletheon-tester` 技能文档更新，含 TUI track 与渲染回归判据。
- [ ] 端到端能复现 T1（作为后续修复的回归基线）。
