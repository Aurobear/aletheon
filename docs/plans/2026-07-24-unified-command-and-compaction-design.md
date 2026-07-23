# Aletheon 统一命令平台与高保真上下文压缩设计

**日期：** 2026-07-24
**状态：** 设计已确认，等待实施计划
**受众：** TUI、协议、daemon、Skill、测试和部署维护者
**范围：** 统一斜杠命令注册、动态 Skill 命令、核心命令语义、自动/手动上下文压缩、真实 TUI 与安装运行时验收

## 1. 目标

本阶段交付以下能力：

1. 建立单一 `CommandRegistry`，统一驱动命令发现、帮助、解析、补全、分派和验收清单。
2. 输入 `/` 时自动展示命令面板，支持实时模糊筛选、参数提示、来源和禁用原因。
3. 让 daemon 提供权威 Skill catalog；外置 Skill 可通过 `/skill-name [args]` 发现并执行。
4. 实现并验收核心命令，尤其是 `/compact`、`/clear`、`/status`。
5. `/clear` 创建新会话并清屏；旧会话仍可通过 `/resume` 恢复。
6. 实现高保真、自适应、可回滚的自动上下文压缩；`/compact` 复用同一引擎强制触发。
7. 每个可见命令都必须有真实 PTY/tmux TUI 用例，并通过安装运行时最终验收。

## 2. 非目标

本阶段不实现：

- Codex 的桌面端、账号、反馈、主题、宠物、rollout 和测试专用命令；
- side conversation、后台 shell 管理、会话导出和 Skill bundle；
- 完整通用 `@` 文件补全系统；本期只实现 `/mention` 所需路径；
- 让第三方 Skill 覆盖内置命令；
- 把所有本地 UI 行为强制下沉到 daemon；
- 删除或改写 canonical 会话历史来完成压缩。

## 3. 当前代码事实

| 当前事实 | 精确定位 |
|---|---|
| 内置命令枚举和字符串解析写在同一大 `match` 中 | `crates/interact/src/tui/command.rs:3-103` |
| 绝对路径会绕过斜杠命令解析 | `crates/interact/src/tui/command.rs:106-120` |
| Tab 补全维护独立硬编码列表 | `crates/interact/src/tui/app/key_handler.rs:256-287` |
| `/help` 再维护一份独立文本列表 | `crates/interact/src/tui/app/submit.rs:75-77` |
| `/clear` 当前只重建本地 `ChatWidget` | `crates/interact/src/tui/app/submit.rs:41-43` |
| `/status` 当前发送 RPC 后向聊天追加临时文本 | `crates/interact/src/tui/app/submit.rs:80-84` |
| `/compact` 当前已有客户端 RPC 调用 | `crates/interact/src/tui/app/submit.rs:127-131` |
| TUI 直接扫描 `~/.aletheon/skills` | `crates/interact/src/tui/skill.rs:31-75` |
| 未知斜杠名默认进入 TUI Skill 分支 | `crates/interact/src/tui/command.rs:99-102` |
| TUI 会把 Skill 内容拼入发给 daemon 的消息 | `crates/interact/src/tui/app/submit.rs:302-318` |
| daemon 启动时另行加载权威 Skill 目录 | `crates/executive/src/host/daemon/bootstrap/request.rs:555-581` |
| 协议已有 status、sessions、compact、resume、fork、interrupt | `crates/fabric/src/protocol/client.rs:23-80` |
| daemon 已在成功 turn 后检查自动压缩 | `crates/executive/src/host/daemon/bootstrap/turn_runtime.rs:321-345` |
| 当前自动压缩阈值固定为上下文窗口 80% | `crates/mnemosyne/src/application/compressor/mod.rs:68-78` |
| 当前 compressor 保留尾部并对旧消息生成摘要 | `crates/mnemosyne/src/application/compressor/mod.rs:95-148` |
| 切割已通过 `safe_tail_cut` 保护 tool pair | `crates/mnemosyne/src/application/compressor/mod.rs:95-105` |
| v2 已具备失败不修改上下文和丰富 outcome 的基础 | `crates/mnemosyne/src/application/compressor/mod.rs:151-190` |
| v2 默认仍关闭 | `crates/executive/src/composition/config/agent.rs:55-60` |
| canonical session 与进程内 working projection 已分离 | `crates/executive/src/host/daemon/session_manager.rs:10-18` |

## 4. 总体架构

```text
Builtin command descriptors ─┐
                             ├──> CommandRegistry
Daemon SkillCatalog ─────────┘         │
                                       ├──> command popup / help / skills
                                       ├──> parser / argument validation
                                       ├──> availability calculation
                                       └──> dispatcher
                                             ├── LocalCommandExecutor
                                             ├── RpcCommandExecutor
                                             └── SkillCommandExecutor

Conversation projection
  └──> ContextBudgetPlanner
         ├── no action
         ├── soft-watermark background compaction
         └── hard-watermark synchronous compaction
                    └──> deterministic reducer
                           └──> cheap-model structured summarizer
                                  └──> validator
                                         └──> atomic working-projection commit
```

权威边界：

- daemon 是会话、Skill 内容、Skill 启用状态和上下文压缩的权威来源；
- TUI 是输入、展示、本地剪贴板和 popup 的权威所有者；
- `CommandRegistry` 是用户可见命令元数据的唯一来源；
- canonical session 保持追加式和可重放，压缩只替换 working projection。

## 5. CommandRegistry

### 5.1 描述结构

共享描述只包含可序列化元数据，不包含执行闭包：

```text
CommandDescriptor
├── name
├── aliases
├── description
├── category
├── usage / argument_hint
├── source: builtin | skill(extension_id)
├── executor: local | rpc(method) | skill(skill_id)
├── availability rules
└── acceptance_case_id
```

注册表负责：

- kebab-case 名称校验；
- 别名归一化；
- 内置保留字；
- 冲突诊断；
- 稳定排序与模糊匹配；
- 按任务状态计算可用性；
- 为 `/help`、`/skills` 和测试导出同一 inventory。

### 5.2 Skill 命名与冲突

1. 优先使用 `SKILL.md` frontmatter 的 `name`。
2. 缺少 `name` 时使用目录名。
3. 内置命令及其别名均为保留字，Skill 不得覆盖。
4. Skill 冲突不得静默覆盖；`/skills` 和启动诊断都显示冲突。
5. 冲突项可用 `/extension-id:skill-name` 显式调用。
6. catalog 遍历和冲突决议必须确定性排序。

## 6. 首批命令

### 6.1 对齐的生产力核心命令

```text
/help /new /clear /compact /status /model /permissions
/sessions /resume /fork /diff /mention /skills /hooks
/agents /interrupt /copy /quit
```

### 6.2 保留的 Aletheon 命令

```text
/reflect /reflect_now /evolution /genome /mode /plan
/approve /context /profile /computer
```

### 6.3 暂缓命令

```text
/app /logout /feedback /theme /pets /rollout /test-approval
```

## 7. TUI 交互

### 7.1 命令发现

- 输入 `/` 后自动打开命令面板，不要求先按 Tab。
- 输入继续变化时实时模糊筛选，精确前缀优先。
- 每项显示名称、说明、参数提示、来源和当前可用状态。
- 禁用项保持可见并说明原因。
- 上下键选择、Enter 补全、Esc 关闭；Tab 保留为补全快捷键。
- 未知命令不发送给模型，而是显示最接近命令和 `/skills` 提示。
- `/home/...` 等绝对路径继续作为普通用户消息。

### 7.2 关键命令反馈

- `/status` 使用可刷新状态卡，不污染聊天历史。
- `/compact` 显示处理中状态和压缩前后 token、策略、保留轮数、压缩比。
- `/clear` 在 daemon 成功创建新会话后切换 session 并清屏；失败时保持旧界面。
- `/skill-name args` 显示调用名、参数和来源，不把整份 `SKILL.md` 显示为用户消息。
- `/help` 按会话、模型、工具、Skill 和 Aletheon 自省分类生成。
- `/skills` 显示来源、启用状态、冲突和 stale 状态。

## 8. 协议与执行

### 8.1 新增或规范化 RPC

- `command.catalog`：返回 daemon 权威 Skill 命令目录和诊断。
- `skill.invoke`：接收 `session_id + skill_id + user_args`。
- `session.new`：供 `/new` 和 `/clear` 创建新会话。
- `session.status`：返回结构化会话和运行时状态。
- 规范现有 `compact`、`sessions`、`session.resume`、`session.fork`、`session.interrupt` 的类型化响应。

### 8.2 Skill 执行流

```text
TUI /code-review src/
  -> registry resolves skill id
  -> skill.invoke(session_id, skill_id, "src/")
  -> daemon verifies loaded + enabled skill
  -> daemon binds trusted skill instructions and user args
  -> normal governed turn execution
```

TUI 不读取、注入或伪造 Skill 系统指令。

### 8.3 原子状态转换

- `/clear`：创建成功 → 切换成功 → 清屏；失败保持旧状态。
- `/compact`：生成、验证、持久化 working projection 成功后再发布完成事件。
- `/resume`：加载并验证目标会话后才替换 TUI 当前会话。
- catalog 刷新失败：保留最近一次有效目录并标记 stale。

### 8.4 错误模型

用户可见错误统一包含：

```text
code + concise_message + recovery_hint + retryable
```

命令冲突、运行中禁用、Skill 已卸载、daemon 断连、RPC 超时和压缩失败都必须给出恢复动作。

## 9. 自动上下文压缩

### 9.1 设计目标

自动压缩为 P0 功能，并选择高保真优先策略：允许额外调用一次较便宜的模型，但不得静默丢失任务状态、约束、关键证据或未完成工作。

### 9.2 自适应预算

每轮由 `ContextBudgetPlanner` 计算：

```text
history_budget =
  model_context_window
  - system_and_skill_prefix_tokens
  - tool_schema_tokens
  - reserved_output_tokens
  - pending_user_input_tokens
  - safety_margin_tokens
```

预算使用实际生效模型和 profile 限制，不能使用全局固定窗口。

两个水位：

- **Soft watermark**：预计 1–2 轮内将触及预算，在 turn 完成后预压缩。
- **Hard watermark**：当前请求无法安全容纳，在调用主模型前同步压缩。

每次请求前和成功 turn 后都评估；不得只依赖 turn 后检查。

### 9.3 两阶段压缩

#### 阶段 A：确定性瘦身

- 删除重复进度和中间状态；
- 折叠已完成且不再活跃的 tool output；
- 对超长日志保留命令、退出状态、关键错误和证据定位；
- 保留文件变更、审批、安全决策和失败结果；
- 规范 tool call/result 边界。

#### 阶段 B：廉价模型结构化总结

摘要模型输出固定 checkpoint，而不是自由文本：

```text
Current objective
User requirements and preferences
Constraints and approvals
Completed work
Decisions and rationale
Files, symbols and current state
Important tool results and evidence
Failures and rejected approaches
Open tasks and exact next action
Conversation commitments
```

### 9.4 原文保护

始终原样保留：

- 当前目标及最近一次用户修订；
- 最近完整轮次；
- 正在执行的计划和未完成任务；
- 未闭合 tool pair；
- 最近错误及恢复上下文；
- 需要精确引用的路径、符号、命令和用户原话。

尾部大小应在最低保留轮数、token 预算和 tool 边界之间自适应，而非固定 25%。

### 9.5 验证与原子提交

checkpoint 必须通过：

1. 必填字段完整性；
2. tool 边界合法性；
3. 压缩后低于目标预算；
4. token 收益达到最低阈值；
5. 当前目标、约束和 open tasks 可恢复；
6. canonical session 可重放。

只有全部通过才原子替换 working projection。失败保持原上下文不变。

若 hard watermark 下首次压缩失败，允许一次更激进但仍保真的策略；仍失败则阻止新 turn，提示 `/new`、切换更大窗口模型或导出诊断，不得截断后继续。

### 9.6 防止摘要逐代退化

每次压缩保存 lineage receipt：

```text
run_id
source range / checkpoint parent
strategy
summarizer model
protected tail range
tokens_before / tokens_after
validation result
```

后续压缩输入为“上次结构化 checkpoint + 新增原始历史”，不反复总结整段自然语言摘要。

### 9.7 手动与自动统一

- 自动压缩和 `/compact` 使用相同 planner、reducer、summarizer、validator 和 commit path。
- `/compact` 只改变触发方式为 force，不绕过边界与验证。
- 自动压缩显示简短事件；`/status` 展示最近时间、次数、策略和压缩比。

## 10. 测试策略

### 10.1 注册表契约

验证命令/别名唯一、保留字、稳定排序、help/补全/解析集合一致，以及每个可见命令都绑定 `acceptance_case_id`。

### 10.2 RPC 与状态机

验证 session.new、status、compact、resume、fork、interrupt、catalog 和 skill.invoke 的成功、失败、超时、断连及重复请求行为。

### 10.3 压缩专项

覆盖：

- soft/hard watermark；
- 请求前与 turn 后触发；
- 不同模型窗口和输出预算；
- tool pair、长日志、错误、审批、文件变更；
- checkpoint 缺字段、退化摘要和无压缩收益；
- 原子失败不修改上下文；
- 多轮重复压缩不丢目标、约束和 open tasks；
- `/compact` 与自动压缩结果契约一致；
- 压缩后真实追问能引用压缩前关键事实。

所有 Cargo 命令必须通过：

```bash
bash scripts/cargo-agent.sh <cargo arguments>
```

### 10.4 真实 TUI 命令矩阵

扩展 `tests/tui_tmux`，每个命令必须经 PTY/tmux 实际输入，并断言：

```text
TUI input -> rendered state -> daemon/persistence state -> receipt
```

覆盖首批全部内置命令、普通/带参数 Skill、冲突、卸载后 stale、自动提示、选择键、运行中禁用、clear/resume、compact 连续性和 status 一致性。

### 10.5 安装运行时验收

最终必须通过：

```bash
bash scripts/aletheon.sh deploy
```

验收必须证明：

- `target/release/aletheon`、`/usr/bin/aletheon`、system daemon 和 user daemon 运行文件 SHA-256 一致；
- systemd restart counter 在观察窗口稳定；
- 使用 `/usr/bin/aletheon` 和官方用户 socket 完成真实 LLM 请求；
- 真实验证普通对话、自动/手动压缩连续性和 `/skill-name` 行为改变。

普通命令全部使用真实 TUI，但只有上述必要路径消耗真实模型调用。

## 11. 分阶段实施边界

1. **注册表基础：** 描述结构、内置 inventory、解析/help/补全统一。
2. **协议与 Skill：** catalog、skill.invoke、命名冲突和 daemon 权威化。
3. **会话核心命令：** new/clear/status/compact/resume/fork/interrupt。
4. **TUI 体验：** 自动 popup、状态卡、选择器、错误和参数提示。
5. **自动压缩：** budget planner、结构化 checkpoint、validator、lineage、请求前门禁。
6. **验收：** 每命令 tmux matrix、真实 LLM 连续性、deploy 和摘要一致性。

每个阶段应形成独立、可审查提交；不得把测试和部署验收推迟到无法定位问题的最终大提交。

## 12. 完成判定

只有同时满足以下条件才可声明完成：

1. 命令元数据只有一个权威 registry。
2. TUI 不再直接加载或注入 Skill 内容。
3. 首批命令全部具备真实 TUI 证据。
4. `/clear` 创建新会话且旧会话可恢复。
5. 自动压缩在 soft/hard watermark 生效，并在失败时保持上下文。
6. `/compact` 与自动压缩共享同一安全引擎。
7. 多轮压缩后的真实任务连续性通过。
8. 安装运行时 deploy、SHA-256、systemd 稳定性和官方 socket 真实请求全部通过。
