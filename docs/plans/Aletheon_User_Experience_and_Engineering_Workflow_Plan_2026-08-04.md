# Aletheon 面向用户的实际使用、CLI/TUI 与工程任务闭环计划

> 文档类型：可执行实施计划
> 基线日期：2026-08-04
> 审计对象：`Aurobear/aletheon`；**基线 SHA 见 `Aletheon_Unified_Execution_Plan_2026-08-04.md` §0（唯一声明处）**，本文件不再自行声明 SHA
> 基线规则：不使用“比 main 超前 N 个提交”描述现状；执行前若 `origin/dev` 含未登记提交，先按统一计划 §0 的 external-change reconciliation 更新
> 状态：`revised-for-owner-approval`；统一计划 B0/B1 完成、owner approval 入库且 X0 baseline reconciliation 通过前不得批量实现
> **执行权威**：节点顺序、依赖、状态词表与完成判定以 `Aletheon_Unified_Execution_Plan_2026-08-04.md`（§3 节点清单 / §4 验收 ID 归属 / §6 关键裁决 / §7 状态词表）为准。
> **DO-NOT-SCHEDULE**：本文件 §7 的 U0–U8 PR 表**仅为历史记录，禁止作为调度输入**；新旧 ID 映射见统一计划 §5。deepseek C-plan 保留自己的 C0–C7 编号，不在 X 轨道 DAG 内调度（约束见统一计划 §1.1/§1.2）。
> 关联计划：`Aletheon_Architecture_Stabilization_and_Convergence_Plan_2026-08-04.md`（契约来源）、`deepseek-cache-and-message-optimization-plan.md`（provider/cache 事实 owner）、`robot-vla-production-closure-plan.md`（robot 域关闭 owner）；边界见 §1.4
> 目标：让 Aletheon 从“底层能力很多但难以使用”收敛为可以日常完成工程任务的 Agent 产品

---

## 0. 执行摘要

Aletheon 当前并不缺工具。代码已经存在工具注册、managed command 和 change transaction（`crates/corpus/src/tools/tools/registry.rs:453-559`），以及 versioned validation receipt（`crates/fabric/src/types/change_transaction.rs:60-75,122-130`）、session/TUI/sub-agent/Skill/Memory/Robot 等能力。

真正的问题是：这些能力没有被组织成一条用户可理解、可控制、可恢复、可验证的主流程。用户感知到的是命令分散、TUI 状态混乱、启动依赖 daemon、内部术语过多、变更和验证不可见；而不是底层已经完成的事务、收据和治理能力。

本计划不增加新的认知框架，不重写 Agent，不新增一级 crate。它只完成以下产品主链：

```text
启动项目
  -> 输入任务
  -> 查看计划与执行状态
  -> 审批危险动作
  -> 查看代码变更
  -> 查看验证结果
  -> 接受 / 修复 / 回退
  -> 重启后继续
```

最终目标不是追平其他编码 Agent 的命令数量，而是让用户可以在不理解 Aletheon 内部架构的情况下，稳定完成真实任务。

### 0.1 已验证现状与执行边界

| 事实 | 当前证据 | 本计划处理方式 |
|---|---|---|
| 安装 binary 只有一个实际 parser | `crates/aletheon/src/main.rs:357` 的 `Cli::parse()` | 保持为唯一权威 parser |
| 兼容 parser 的 `Args::parse()` 无调用方 | `crates/interact/src/tui/cli.rs:228`，仅同文件单测 `:750-752` 使用 | 可直接删除，无外部阻塞 |
| **兼容模块承载权威 parser 依赖的类型** | `crates/aletheon/src/main.rs:15` `use interact::cli::TaskKindArg;`，用于 `:66` 与 `:705`；类型定义在 `crates/interact/src/tui/cli.rs:97-107` | caller inventory 已完成：生产调用 1 个。必须先迁移类型（U1b-1），再删除模块（U1b-2） |
| TUI 连接失败不会自动启动 daemon | `crates/interact/src/tui/mod.rs:148-154` | 建立 install-mode aware ensure-running |
| `TurnCheckpoint` 已存在 | `crates/fabric/src/types/workspace_checkpoint.rs:55-74` | 不重新定义同名类型；增加 projection 或版本化扩展 |
| 架构门禁、wire/persistence inventory 已存在 | `config/architecture/wire-surfaces.tsv:1-20`、`config/architecture/persistence-surfaces.tsv:1-18`、`scripts/libexec/aletheon/architecture-check.sh:62-171` | 复用并增量扩展 |
| machine/provider backpressure 与 runtime observability 已有基础 | `crates/cognit/src/adapters/inference/backpressure.rs:14-80`、`crates/fabric/src/types/attempt.rs:71-80` | TUI 只投影 typed runtime facts，不重新计算 |

因此，本计划不是 TUI 重写，也不是重新建立架构基础设施；它是建立在现有 host/runtime 契约上的增量产品收敛。

---

## 1. 边界与强制约束

### 1.1 本计划负责

- 统一用户命令入口与命令语义。
- TUI 信息架构、输入体验、状态可视化和快捷操作。
- daemon 自动发现、自动启动、连接恢复和诊断。
- Session 的创建、恢复、搜索、分叉和继续。
- 每轮工程修改的 checkpoint、diff、validation、accept、rollback。
- Tool、managed command、sub-agent、approval 的统一活动时间线。
- 非交互式 `run/exec` 的稳定输出与退出码。
- 面向真实用户任务的端到端验收。

### 1.2 本计划不负责

- 不改变 Native Cognit 的认知权威。
- 不重新设计 Mnemosyne/GBrain 的内部记忆策略。
- 不把 TUI 变成第二个业务状态权威。
- 不在 `interact` 内直接操作数据库、工具或 Agent Runtime。
- 不把 Aletheon 拆成微服务。
- 不增加新的 `Manager`、`Coordinator`、`Service` 来绕过现有主链。
- 不通过增加 Prompt 文字代替代码级状态和验收。

### 1.3 依赖关系

本计划依赖架构计划先完成以下契约的 migration matrix，而不是默认全部新增：

- `CommandSpec`
- `ClientIntent`
- `SessionSnapshot`
- `TaskSnapshot`
- `ActivityEvent`
- `ApprovalRequest`
- `TurnCheckpoint`
- `ValidationReceipt`
- `TaskSettlement`

UI 可以先做只读投影，但不得自行发明另一套同义结构。

冻结前必须逐项标记：

```text
existing：直接复用
extend：兼容新增字段
project：由现有权威状态生成 UI/read-model
v2：存在不兼容 wire/persistence 变化
new：确认没有当前同义类型后才允许新增
delete：列出 caller、替代路径和删除门禁
```

`ApprovalRequest`（`crates/fabric/src/protocol/client.rs:916-934`）和 `TurnCheckpoint`（`crates/fabric/src/types/workspace_checkpoint.rs:55-74`）已有当前定义，默认从 `existing/extend/project` 开始评估；禁止先创建另一份同名 DTO。`TaskSnapshot`、`ActivityEvent` 若新增，只能是 daemon-owned projection contract，不能成为新的任务或事件写入权威。

### 1.4 兄弟计划的 owner 边界

`docs/plans/` 下另有两份同基线计划，本计划**只投影、不定义**它们拥有的契约：

| 计划 | 它拥有什么 | 本计划的动作 |
|---|---|---|
| `deepseek-cache-and-message-optimization-plan.md`（C0–C7） | provider usage 解析、provider capability/代理差异、prompt 稳定前缀 shape、Mnemosyne recall cache、Corpus tool result cache 的**字段定义与写入** | §5.1 `runtime_facts` 与 U2 任务 8 只读取其 typed 输出；cache 语义分歧以 C-plan 为准 |
| `robot-vla-production-closure-plan.md`（R0–R8） | Robot/Policy typed config、Perception/FrameRef、VLA gateway、Episode artifact、实机安全 | Phase U8 只做统一主链的用户表面投影，域内关闭由 R7/R8 判定 |

架构边界的权威表述见架构计划 §0.2；两处冲突时以架构计划为准。

### 1.5 已合入工作与执行前置

2026-08-05 复核时，C0–C7 和原 14 条 feature/fix/test 分支均已合入 `origin/dev@c080b08b...`，不再作为在途依赖。统一计划 B0 先把四份计划、Robot evidence 与 status ledger 入库；B1 固定 Goal 控制面。B0 后任何新增未登记分支或生产代码改动都必须先登记 owner，U1–U7 不得隐式依赖。

---

## 2. 当前实际问题清单

### 2.1 启动与安装体验

#### UX-BOOT-01：文档声称自动启动，实际连接失败

`crates/aletheon/src/main.rs:3-5` 注释描述无子命令时 TUI 会自动启动 daemon；但 `crates/interact/src/tui/mod.rs:148-154` 连接 Unix Socket 失败后只提示用户手动运行 `aletheon daemon &`。

影响：

- 第一次运行失败。
- 用户必须理解 daemon、user socket、core socket 和 systemd。
- systemd 已安装但 socket 未就绪时无法自愈。
- README、代码注释、真实行为不一致。

#### UX-BOOT-02：错误诊断没有进入启动链

`doctor` 已存在（`crates/aletheon/src/main.rs:181-192`），但 TUI 连接失败路径不会自动执行或展示结构化诊断（`crates/interact/src/tui/mod.rs:148-154`）。

#### UX-BOOT-03：源码操作与安装后操作入口不同

用户交互用 `aletheon`，部署运维大量使用 `bash scripts/aletheon.sh ...`。运维脚本可以保留为内部部署接口，但面向用户的常用状态、日志、诊断不应要求记住第二套入口。

---

### 2.2 CLI 与命令系统

#### UX-CLI-01：一个生产 parser，一个兼容 parser，以及一条反向类型依赖

权威 binary 在：

- `crates/aletheon/src/main.rs:26-208,355-518`

兼容命令解析仍在：

- `crates/interact/src/tui/cli.rs:25-148,226-307`
- `crates/interact/src/lib.rs:28` 中的 `pub use tui::cli`

安装 binary 只调用 `crates/aletheon/src/main.rs:357` 的 parser；旧 parser 的 `Args::parse()` 未被安装入口调用，但仍是公开 Rust API，包含 `debug`、`goal`、`workflow`、`daemon start/stop/status` 等不同命令。

关键修正：这里**不是**纯粹的"无人调用的 API 债务"。权威 parser 反向依赖兼容 parser 文件里的类型：

```text
crates/interact/src/tui/cli.rs:97-107   pub enum TaskKindArg + From<TaskKindArg> for fabric::TaskKind
        ^
        | use interact::cli::TaskKindArg
crates/aletheon/src/main.rs:15,66,705   权威 CLI 的 --task-kind 字段与单测
```

即 assembly（`crates/aletheon`）通过兼容 re-export 依赖 presentation（`interact`）的兼容模块。后果：

- 兼容 parser 测试通过不等于安装后的命令可用。
- 同一能力有不同参数和语义。
- `--help`、README、测试和实际 binary 可能不一致。
- 新功能不知道应该注册在哪一层。
- **`caller count=0` 的删除证据当前不可能达成**，删除顺序被强制为先迁移类型、后删除模块（见 §7 的 U1b-1/U1b-2）。

#### UX-CLI-02：消息入口重复

当前存在：

- 默认 TUI 输入。
- `-m/--message`。
- `exec --prompt`。
- 旧 parser 的位置参数消息。
- 非 TTY line mode。

这些路径未明确区分“聊天”“工程任务”“脚本协议”，容易产生不同的 workspace、permission、session 和 output 行为。

#### UX-CLI-03：用户命令暴露内部名词

`core`、`memory-agent`、runtime requirement、profile、mode、agent、Skill、Memory 同时出现在用户表面。用户很难知道哪些是日常命令，哪些是运维命令，哪些只是开发/治理接口。

#### UX-CLI-04：缺少日常工程入口

当前缺少稳定、明确的：

- `resume`
- `review`
- `checkpoint`
- `rewind`
- `diff --file`
- 面向用户的稳定 `aletheon completion`（运维脚本已有自己的 ops completion，不能冒充产品 CLI completion）
- `mcp doctor`
- `skill doctor`

#### UX-CLI-05：非交互输出契约不够完整

`exec` 当前只声明 `text/json`（`crates/aletheon/src/main.rs:151-171`），还缺少：

- JSONL 流式事件。
- 明确的 schema version。
- task/session/turn/checkpoint id。
- 分类退出码。
- stdin prompt。
- approval 行为定义。
- resume/idempotency 语义。

---

### 2.3 TUI 信息架构

#### UX-TUI-01：Chat Transcript 承担过多职责

系统消息、状态、工具调用、审批结果、session 操作和实际回答大量混在聊天区。用户无法快速判断：

- 当前在分析、修改、测试还是等待。
- 哪个命令仍在运行。
- 哪些文件已修改。
- 验证是否通过。
- 任务能否安全接受。

#### UX-TUI-02：功能很多，但没有稳定任务主视图

当前已有 session picker、approval dialog、plan、sub-agent view、completion 等模块（`crates/interact/src/tui/mod.rs:328-352`），但主要通过零散 overlay 和快捷键进入。

问题不是缺模块，而是缺统一布局和状态导航。

#### UX-TUI-03：快捷键不可发现且语义冲突风险高

现有 Ctrl/Alt 组合较多：Ctrl+T、Ctrl+D、Ctrl+O、Ctrl+B、Ctrl+M、Ctrl+P、Alt+Up/Down 等（`crates/interact/src/tui/help_overlay.rs:70-86`）。用户无法从界面稳定发现，也容易与终端、shell、编辑器习惯冲突。

#### UX-TUI-04：命令补全仍是命令名前缀补全

`CompletionPopup` 已能展示说明、来源和 unavailable reason（`crates/interact/src/tui/completion.rs:13-96`），但搜索主要围绕命令名/alias/fuzzy subsequence，没有成为统一 Action Palette。

#### UX-TUI-05：输入历史不持久

`CommandHistory` 只保留进程内 50 条（`crates/interact/src/tui/input.rs:1-34`），退出丢失；草稿也没有跨 session 保存。

#### UX-TUI-06：文件引用和长文本输入仍偏命令式

存在 `/mention <path>` 和 `/input`（`crates/interact/src/tui/registry.rs:346-365`），但缺少：

- `@` 文件模糊选择。
- 最近文件和 changed files 排序。
- 图片/Artifact 引用入口。
- `$EDITOR` 长文本编辑。
- 清楚的多行提交提示。

#### UX-TUI-07：Activity Detail 与任务结算没有关联

Tool card 可以选择/展开（`crates/interact/src/tui/app/key_handler.rs:263-280`），但用户看完工具输出后仍不知道：它属于哪个计划步骤、是否改变工作区、是否产生 validation receipt、是否满足完成标准。

---

### 2.4 Session、Checkpoint 与恢复

#### UX-SESSION-01：session 能力存在但入口不统一

已有 `/sessions`、`/resume`、`/fork`、`/new`、`/clear`（`crates/interact/src/tui/registry.rs:151-225`、`crates/interact/src/tui/help_overlay.rs:113-118`），CLI 却没有统一的 `aletheon resume` 主入口。

#### UX-SESSION-02：缺少面向用户的 Turn Checkpoint

底层已经存在 ChangeTransaction、workspace version、restore point、diff 和 rollback 能力（`crates/fabric/src/types/change_transaction.rs:48-130`、`crates/corpus/src/tools/tools/change_transaction.rs:35-75,355-455`），但没有映射成“这一轮修改”的 checkpoint。

#### UX-SESSION-03：无法清楚地区分三种恢复

必须支持并明确区分：

1. 仅恢复对话/任务状态，不动文件。
2. 仅回退本轮文件修改，保留对话。
3. 从历史 Turn 分叉，同时恢复对应文件状态。

#### UX-SESSION-04：重启恢复缺少可见证据

重启后用户应看到：

- 当前 Goal/Plan。
- 已接受 checkpoint。
- 未结算 transaction。
- 正在运行或失联的 child runtime。
- 剩余预算。
- 最近 validation receipt。

---

### 2.5 Diff、Review、验证与完成

#### UX-VERIFY-01：底层强，用户表面弱

`apply_patch` 已支持 dry-run、digest 和 patch receipt（`crates/corpus/src/tools/tools/apply_patch.rs:43-44,121-244,255-309`）；ChangeTransaction 已支持 validation plan、receipt、conflict 和 rollback（`crates/corpus/src/tools/tools/change_transaction.rs:260-269,400-461,486-543`）。但这些能力没有形成一致的用户工作流。

#### UX-VERIFY-02：缺少独立 Review 产品入口

至少需要支持：

- Review 未提交修改。
- Review 相对 base branch。
- Review 指定 commit。
- Review 当前 Turn 的 diff。
- Finding 严重度、文件/行、证据、修复状态。

#### UX-VERIFY-03：模型仍可能用自然语言自报完成

工程任务必须由 Host acceptance 决定，不能只根据模型输出中的“已完成”。

#### UX-VERIFY-04：验证结果没有形成可操作结算

用户需要看到：

- 修改了什么。
- 为什么修改。
- 执行了哪些验证。
- 哪些验证缺失以及原因。
- 当前风险。
- Accept、Repair、Rollback 三种动作。

---

### 2.6 Approval、权限和安全体验

#### UX-APPROVAL-01：approval 仍以阻塞弹窗为中心

当前 dialog 支持一次批准、session 批准、path scope、拒绝（`crates/interact/src/tui/approval_dialog.rs:9-28,61-74`），但需要进一步绑定 task/transaction/tool risk，显示可理解的影响范围。

#### UX-APPROVAL-02：权限 profile 与实际动作边界未统一呈现

CLI 有 `safe/dev/full`（`crates/aletheon/src/main.rs:32-44,92-113`），TUI 有 `/permissions`（`crates/interact/src/tui/registry.rs:236-245`），workspace policy 和 tool approval 又有各自信息。用户需要一个权威 Permission Snapshot。

#### UX-APPROVAL-03：审批后缺少可追踪性

必须能回答：谁在什么 task/turn 中，批准了哪个工具，对哪些路径生效，何时失效，最终产生了什么 receipt。

#### UX-APPROVAL-04：输入快捷入口扩大了攻击面

`/`、`@`、`!`、paste、外部编辑器和 Artifact 预览必须由显式 editor state machine 管理：

- IME composing 和 bracketed paste 期间不得意外触发 palette/shell action。
- `!` 只构造受治理 command intent，不得直接执行；必须显示 workspace、permission、transaction coverage 和审批影响。
- `@` attachment 必须做 workspace authority、canonical path、symlink/path traversal 和 size/type 校验。
- tool/runtime 输出视为不可信数据；过滤危险 ANSI/OSC、OSC 8/52、控制字符和伪造 UI 标记。
- prompt/history/draft/artifact 中的 secrets 必须支持 redaction，不进入普通 telemetry。
- hidden CLI 只解决 discoverability，不解决 authorization；internal service 命令必须验证调用身份和运行环境。

### 2.7 终端兼容、可访问性和隐私

当前计划还必须覆盖：

- `NO_COLOR`、低色深、色盲友好主题和无动画模式。
- Unicode 宽度、emoji/组合字符、中日韩 IME、非 UTF-8 locale 的降级行为。
- 不依赖鼠标的完整键盘导航、明确 focus 和 screen-reader/纯文本输出模式。
- tmux、SSH、resize、terminal disconnect、macOS/Linux 差异。
- session/history/draft 的保存位置、文件权限、retention、删除和 project scope。
- workspace 移动/改名、branch/worktree 变化后的 resume 身份校验。
- 多用户或共享机器上不得把其他 principal 的 session/history 暴露在 picker 中。

---

## 3. 目标用户体验

### 3.1 第一次启动

```text
$ aletheon

Detect project
  -> Load layered config
  -> Resolve user socket
  -> Start/activate daemon when absent
  -> Wait for readiness
  -> Run bounded doctor checks on failure
  -> Open or create workspace session
  -> Render TUI
```

要求：

- 正常路径不要求用户理解 systemd。
- 失败必须显示下一步和机器可读 reason code。
- 不允许注释、README 和行为不一致。

### 3.2 日常工程任务

```text
用户输入任务
  -> Host 建立 Task + Turn + ChangeTransaction
  -> Cognit 形成/更新 Plan
  -> 工具和 child runtime 执行
  -> UI 显示活动、审批、diff
  -> Validation Controller 执行验收
  -> TaskSettlement: accepted / repair / blocked / rolled_back
  -> 生成 evidence receipt
```

### 3.3 TUI 固定信息模型

逻辑上固定三个区域，窄屏可以折叠：

#### Conversation

- 用户请求。
- Agent 关键结论。
- 阻塞问题。
- 最终交付。

#### Task

- Goal。
- Plan steps 与状态。
- 当前阶段。
- managed command。
- sub-agent/runtime。
- approval。
- budget/context。

#### Changes

- changed files。
- diff。
- findings。
- validations。
- receipts。
- Accept / Repair / Rollback。

聊天文本不是状态权威；所有状态来自 daemon snapshot + event projection。

---

## 4. 命令系统收敛设计

### 4.1 唯一顶层 CLI

目标命令按普通用户、扩展管理和内部运维分层。普通 `--help` 只展示高频内置能力：

```text
aletheon                         # 交互 TUI
aletheon run [PROMPT]            # 一次受治理任务
aletheon exec                    # 稳定脚本/CI 协议
aletheon resume [SESSION]
aletheon goal <create|list|show|run|pause|cancel>
aletheon review [TARGET]
aletheon diff [OPTIONS]
aletheon checkpoint <ACTION>
aletheon doctor
aletheon config <ACTION>
aletheon permissions
aletheon model <ACTION>
aletheon completion <SHELL>
aletheon version
```

扩展安装管理只使用一个低频、权威的 package 生命周期入口。Skill、Hook、MCP 是 extension asset kind，不建立三套平行的 install/enable/upgrade/rollback 状态机：

```text
aletheon extension <inspect|validate|install|list|show|enable|disable|upgrade|rollback|remove|purge|doctor>
```

如需按资产类型查看，使用 `extension list/show` 的 kind filter 或 Action Palette 分类；`skill`、`hook`、`mcp` 最多是无状态查询 alias，不能成为第二写入入口。当前 `crates/aletheon/src/extension_cli.rs:16-61` 已具备上述统一 package 生命周期，后续工作只补可发现性和过滤。

`core`、`memory-agent`、reflection、memory maintenance、compaction policy 等系统治理入口不出现在普通用户命令面。它们由 systemd/host policy 自动触发，或进入经过身份验证的 internal/admin service contract；hidden flag 不能替代 authorization。

### 4.2 兼容策略

- 建立一份 `CommandSpec` 作为 help、completion、parser 文档的单一来源。
- 零生产调用的内部 compatibility parser 在 caller inventory 通过后直接删除，不伪装成公共兼容承诺。
- 确有外部调用者的公共命令最多保留一个 release cycle，并打印明确 warning、替代命令和删除 deadline。
- 禁止旧入口与新入口走不同的业务 handler。
- 删除 `interact::tui::cli::Args` 第二套 parser。

### 4.3 `run` 与 `exec` 的区别

#### `run`

- 面向人。
- 默认创建/恢复 workspace session。
- 可交互审批。
- 输出最终摘要。
- 支持 `--resume`。

#### `exec`

- 面向脚本和 CI。
- 默认无交互。
- 支持 stdin。
- `--output jsonl|json|text`。
- 固定 schema version。
- 分类退出码。
- approval 不可满足时返回 blocked，不得等待 stdin。
- stdout 只输出所选协议；日志、诊断和非协议文本进入 stderr。
- JSONL 保序，包含 schema version、sequence、session/task/turn/activity IDs，并以唯一 terminal event 结束。
- `SIGINT`、timeout、cancelled、blocked、provider unavailable/rejected、validation failed 使用稳定分类退出码。
- 支持 caller idempotency key；重试不得重复应用 patch 或重复启动不可幂等 child。
- 输出 consumer backpressure 有界；达到上限时产生 typed failure，不能静默丢事件。
- 非交互模式不得弹出 TUI 或等待隐式 stdin approval。

### 4.4 Slash commands 重新分层

普通 TUI 首屏只保留可由用户直接理解的高频动作：

```text
/help
/new
/resume
/fork
/plan
/diff
/review
/checkpoint
/rewind
/permissions
/model
/doctor
/quit
```

Memory、Profile、Agent runtime、Skill、Hook、MCP、Extension 和 compaction 等低频管理进入 Action Palette 或顶层 CLI；reflection、memory maintenance 等自动治理能力不提供外部手动命令。任何新增 slash command 必须证明它是高频用户意图，而不是内部函数的直接映射。

---

## 5. 核心数据契约

### 5.1 TaskSnapshot

这是 daemon-owned read model，不拥有 Task transition。至少包含：

```text
task_id
session_id
goal
phase
plan_revision
steps[]
active_turn_id
active_runtime_children[]
active_commands[]
pending_approvals[]
budget
checkpoint_head
settlement
runtime_facts
```

`runtime_facts` 只投影 typed host/effective-config 数据：effective provider/model/context capacity、active context occupancy、累计 usage、cache usage/unknown、inference rounds、provider retries、tool calls、terminal tool results。不得采信模型自报身份，也不得从累计 token 推导 active context。

其中 provider/usage/cache 相关字段的定义与写入属于 C-plan（见 §1.4）。本计划只做只读投影：字段名、单位和 `unknown` 语义一律以 C-plan 冻结结果为准；若 C-plan 在 U2 之后仍变更这些字段，按 wire surface 变更处理并更新 `config/architecture/wire-surfaces.tsv`，不得在 UI 层做兼容换算或本地推导。

### 5.2 ActivityEvent

所有长期活动统一**投影为 ActivityEvent**；这不是拿同一个 wire schema 替换 provider stream、tool result、runtime progress 或 Session event：

```text
activity_id
task_id
turn_id
parent_activity_id
kind: tool | command | runtime | validation | approval | memory | robot
label
state: queued | running | waiting | completed | failed | cancelled | lost
started_at
updated_at
progress
artifact_refs[]
receipt_ref
```

`completed` 只表示相应 domain 的权威 terminal event 已被观察；accepted/queued/start/cancel_requested 不能映射为 completed。

### 5.3 TurnCheckpointProjection

当前 `fabric::TurnCheckpoint` 已经持久化 checkpoint identity、workspace、FS domain refs、integrity 和 finalize state（`crates/fabric/src/types/workspace_checkpoint.rs:55-74`）。本计划不得重定义同名结构；UI 通过兼容扩展或独立 read projection 展示：

```text
checkpoint_id
session_id
task_id
turn_id
parent_checkpoint_id
workspace_before
workspace_after
changed_paths[]
diff_artifact_ref
validation_receipts[]
conversation_cursor
plan_revision
settlement
created_at
```

如果上述字段需要持久化到现有 `TurnCheckpoint`，必须登记 schema version、old/new reader/writer、backup/restore 和 compatibility window；若只用于 UI，则必须可以从现有 checkpoint、transaction、validation 和 event authority 确定性重建。

### 5.4 TaskSettlement

```text
accepted
repair_required
blocked
cancelled
rolled_back
failed
```

模型的自然语言不能直接写 `accepted`。

### 5.5 Contract migration matrix

| 目标契约 | 当前起点 | 默认策略 | 禁止事项 |
|---|---|---|---|
| `CommandSpec` | 顶层 Clap + 兼容 parser + completion assets | 新增 canonical metadata，逐入口消费 | 再写第三套 parser |
| `ClientIntent` | 现有 client RPC/request types | extend/project，先列 method mapping | 新增平行业务 handler |
| `SessionSnapshot` | session store/event projections | project | UI 成为 writer |
| `TaskSnapshot` | Turn/Goal/Plan/runtime/checkpoint 权威状态 | project | 作为第二套 Task store |
| `ActivityEvent` | domain events/receipts | project with domain refs | 复用为所有 runtime diagnostic schema |
| `ApprovalRequest` | 现有 Fabric protocol type | extend/v2 by compatibility analysis | 同名重复定义 |
| `TurnCheckpointProjection` | 现有 `fabric::TurnCheckpoint` + transaction/event | project，必要时兼容 extend | 替换现有持久化结构而无 migration |
| `ValidationReceipt` | ChangeTransaction/versioned validation receipt | reuse/extend | 仅凭 natural-language summary |
| `TaskSettlement` | Host acceptance/Turn lifecycle outputs | new projection only after owner freeze | 模型直接写 accepted |

---

## 6. 分阶段实施计划

> **Phase ≠ 调度单位。** 本节的 U0–U8 是**契约分组**，不是执行节点；执行节点是统一计划 §3 的 X0–X14。
> 每个 `U-*` 验收 ID 的调度归属见统一计划 §4.3（U1 被拆到 X2/X3a/X3b/X3c/X4a–X4d，
> U2 被拆到 X5b/X5c/X6a，U4 被拆到 X7/X8a/X8b，U5 被拆到 X8c/X8d）。
>
> **验收 ID 必须有代码绑定。** 基线实测：本节所有 `U-*` ID 在仓库中（排除 `docs/plans/`）命中为 0，
> 即它们目前只是散文。统一计划 §4.1 定义了唯一机械判定手段——测试函数名以 ID 小写、`-` 换 `_` 为前缀
> （`U-BOOT-001` → `fn u_boot_001_*`），并由 X1 建立的 `config/architecture/acceptance-ids.tsv`
> 台账 + `architecture-check.sh` 校验。安装态/外部服务类 ID（`U-INST-*`、`U-ROBOT-*`）允许
> `kind=manual-evidence`，但必须落到 `docs/testing/` 下的具名证据文件，**不得只写在文档里**。

### Phase U0：冻结用户主链与基线

目标：停止继续堆叠 UI 功能，先建立可测基线。

任务：

1. 输出当前所有 CLI、slash、快捷键、RPC、UI overlay 清单。
2. 标记每个入口的真实 handler 和状态所有者。
3. 建立 12 个 golden user journeys。
4. 记录当前启动时间、首次成功率、任务完成率、平均审批次数、恢复成功率。
5. 为 `CommandSpec`、`TaskSnapshot`、`TurnCheckpoint` 建立 ADR。
6. 记录 corpus 缺口：`tests/coding/acceptance/` 现有 8 个 fixture，需补 12 个，登记为 U-corpus PR。
7. 与 C-plan、R-plan 互相登记 owner 边界（§1.4），确认无契约双写。
8. 保存 14 条历史分支均为 `already_merged` 的证据，并确认 B0 后没有未登记代码改动。

验收：

- 每个用户入口都能追踪到唯一 handler。
- 所有 ghost/dead/compat command 被列出，且区分"零调用可删"与"仍有生产调用需先迁移"。
- 不实现新 UI 功能。
- corpus 缺口、兄弟计划边界、历史分支已合入状态和 B0/B1 前置均有书面结论。

### Phase U1：唯一 CLI 与启动闭环

目标：`aletheon` 可以直接进入可用状态。

代码重点：

- `crates/aletheon/src/main.rs`
- `crates/interact/src/tui/cli.rs`
- `crates/interact/src/host.rs`
- `crates/interact/src/tui/mod.rs`
- `crates/executive/src/host/launcher.rs`

任务：

1. 建立唯一 parser 和 `CommandSpec`。
2. 将旧 CLI handler 迁移到应用层 command handler。
3. 把 `TaskKindArg`（`crates/interact/src/tui/cli.rs:97-107`）迁到 `CommandSpec` 权威位置，`crates/aletheon/src/main.rs:15` 改引用新位置；此步不删文件，退出条件是 `interact::cli` 生产调用计数归零。
4. 计数归零后删除第二套 Args/Command 定义与 `crates/interact/src/lib.rs:28` 的 re-export。
5. 实现 daemon ensure-running：区分 system install/user-local/dev foreground，按策略 resolve、activate/spawn、lock、readiness、timeout、doctor；处理 stale socket、多 client 竞态和 client/daemon version mismatch。
6. 收敛 `-m`、位置消息和 `exec`。
7. 增加 `run`、`resume`、`completion`。
8. 为现代 Goal RPC 增加 canonical `goal create/list/show/run/pause/cancel` CLI/typed client；禁止复用待删除的 legacy `goal.set/status` parser。
9. 增加 CLI contract test：help snapshot、exit code、deprecated route、Goal control surface。

验收用例：

- U-BOOT-001：全新用户运行 `aletheon`，无需手动启动 daemon。
- U-BOOT-002：daemon 配置错误时 5 秒内得到结构化诊断。
- U-BOOT-003：两个 client 并发首次启动不会产生两个 daemon authority；stale socket 可诊断恢复。
- U-CLI-001：所有帮助、completion 和 parser 来自同一 spec。
- U-CLI-002：旧命令只能转发到相同 handler。
- U-CLI-003：`exec --output jsonl` 可被 jq/CI 稳定消费。
- U-CLI-004：`crates/aletheon` 不再 import 任何 `interact::cli::*`，且 `interact::cli` 的生产调用计数为 0（删除前置证据）。

### Phase U2：TUI Task Console

目标：用户无需读日志即可知道任务正在做什么。

任务：

1. 建立 Task header：project、session、task、phase、model、permission。
2. 建立 Activity timeline。
3. 将 tool、managed command、sub-agent、approval、validation 映射到 ActivityEvent。
4. 将 Conversation 与系统诊断分离。
5. 增加 changed-files summary。
6. 实现一致的 keyboard hint footer。
7. 所有 overlay 支持 Esc、搜索、上下移动和明确关闭提示。
8. model/provider/context/budget/cache 等只展示 typed runtime facts；provider error 不得被 final answer 或 monitor PASS 覆盖。
9. Conversation、runtime progress、diagnostic、receipt 使用不同 reducer/schema，只在 Activity projection 汇合引用。

验收：

- U-TUI-001：用户 3 秒内能识别当前任务阶段。
- U-TUI-002：运行 5 分钟的命令可查看增量输出并取消。
- U-TUI-003：sub-agent 失败不会只显示一段系统文本。
- U-TUI-004：终端宽度 80x24、120x40、200x60 均可用。
- U-TUI-005：中文 IME、多行输入、paste 不产生重复提交。
- U-TUI-006：`NO_COLOR`、低色深、纯键盘和 screen-reader/line mode 可完成核心旅程。
- U-TUI-007：provider unavailable/rejected 在 frame、session evidence 和 exit/settlement 中一致失败。

### Phase U3：输入、搜索和 Action Palette

目标：减少记命令和快捷键。

任务：

1. `/` 打开 Action Palette，按名称、说明、alias、category 搜索。
2. `@` 打开 file/artifact picker。
3. `!` 进入受治理 shell action，不绕过 approval。
4. Ctrl+R 搜索持久化输入历史。
5. 支持 `$EDITOR` 编辑长 prompt。
6. 保存和恢复未提交草稿。
7. Command availability 来自 Host snapshot，不能由 TUI 猜测。
8. 输入 editor state 明确区分 normal/composing/paste/palette/attachment/shell-confirmation。
9. 过滤不可信 tool/runtime 输出中的危险 ANSI/OSC/control sequence。
10. history/draft 按 principal+workspace 隔离，定义 retention/delete/redaction。

验收：

- U-INPUT-001：可在 1 万文件仓库中 300ms 内返回首屏候选。
- U-INPUT-002：历史和草稿在进程重启后仍存在。
- U-INPUT-003：`@` 引用被解析为带 workspace authority 的 typed attachment。
- U-INPUT-004：`!` command 产生 ActivityEvent 和 receipt。
- U-INPUT-005：IME 或 paste 中的 `/@!` 不会意外触发动作。
- U-INPUT-006：symlink/path traversal、超大附件和越权 workspace attachment 被 fail closed。

### Phase U4：Checkpoint、Diff、Accept 与 Rewind

目标：使每轮修改可检查、可接受、可回退。

任务：

1. 修改型 Turn 自动建立 ChangeTransaction，并为每项 mutation 声明 `full/best_effort/non_rollbackable` coverage。
2. ChangeTransaction terminal projection 生成 TurnCheckpoint。
3. TUI 显示 changed files、diff stat、逐文件 diff。
4. 增加 Accept、Repair、Rollback。
5. 实现 `/rewind` checkpoint picker。
6. 支持仅回退代码、仅分叉会话、分叉并回退。
7. 检测外部工作区变化，拒绝静默覆盖。
8. 明确 untracked/ignored/staged、binary/large file、symlink、权限、submodule、workspace 外写入和并发进程的行为。

验收：

- U-CHK-001：回退本轮修改不影响修改前用户文件。
- U-CHK-002：外部并发修改触发 conflicted，不自动覆盖。
- U-CHK-003：重启后 checkpoint 链可恢复。
- U-CHK-004：所有 accepted checkpoint 至少有一个 validation receipt，或明确的 omission。
- U-CHK-005：best-effort/non-rollbackable effect 不显示虚假的“一键完全回退”。
- U-CHK-006：部分 rollback 失败产生 terminal `partial/conflicted` receipt，保留重试和人工恢复证据。

### Phase U5：Review 与强制验证闭环

目标：工程任务不再由模型自报成功。

任务：

1. 实现 `aletheon review`。
2. 定义 ReviewFinding：severity、location、evidence、status、repair link。
3. Validation Controller 根据 repository context 和 changed paths 生成验证计划。
4. format/check/test/lint/build/deploy 使用 managed command。
5. validation 失败自动将 plan 修改为 Repair。
6. Host acceptance 计算 TaskSettlement。
7. 最终回答只投影权威 settlement 和 receipts。

验收：

- U-VERIFY-001：测试失败时模型即使输出“完成”也不能 accepted。
- U-VERIFY-002：Review finding 可修改 plan graph。
- U-VERIFY-003：验证命令、exit code、output artifact、workspace version 可追踪。
- U-VERIFY-004：遗漏集成测试必须展示风险，不能静默成功。

### Phase U6：Session 与长任务恢复

目标：会话、任务、child runtime 和预算可恢复。

任务：

1. `aletheon resume` 支持项目优先、最近使用和全文搜索。
2. 统一 `/sessions`、`/resume` 和 CLI resume handler。
3. 恢复 TaskSnapshot、plan revision、checkpoint head、budget。
4. 将重启时 running activity 标为 recovering/lost，经过 reconcile 后更新。
5. child runtime 通过 authoritative receipt 恢复或结算。
6. 恢复未完成 approval 时默认 fail closed。
7. generation/epoch fencing 阻止旧 daemon/child 的迟到 terminal result 推进新状态。
8. 处理项目移动/改名、branch/worktree mismatch、session retention/delete 和 principal isolation。
9. child 已产生副作用但 receipt 丢失时进入 orphan reconciliation，不假设 exactly-once。

验收：

- U-RESUME-001：daemon 重启不丢 Goal、Plan、预算、checkpoint。
- U-RESUME-002：失联命令不会继续显示 running。
- U-RESUME-003：child 不得通过自报文本推进父任务。
- U-RESUME-004：恢复后不会重复应用同一个 patch。
- U-RESUME-005：旧 generation 的迟到 receipt 被拒绝并留下审计记录。
- U-RESUME-006：其他 principal 的 session/history 不出现在 picker/search 中。

### Phase U7：安装后真实工程验收

目标：证明 repository 内测试通过的 client/daemon/IPC/persistence 变更已经进入系统安装主链。

任务：

1. 完成 system deploy、binary/runtime SHA 对账和 systemd restart stability。
2. 通过 official user socket 完成真实 LLM 请求。
3. 同一 TUI session 完成多轮工程旅程；模型控制 routing/arguments 做三次 fresh-session 验证。
4. 对照 frame、session、receipt/audit、daemon log 和 monitor verdict；任何冲突按失败处理并修复 monitor。
5. 运行固定 20-task engineering corpus，记录 provider/model/budget/fixture/receipt。

验收：

- U-INST-001：system deploy 和 SHA provenance 通过。
- U-INST-002：units active、`NRestarts` 稳定、official socket 真实请求成功。
- U-INST-003：20 个核心工程任务至少 16 个成功，且没有 scope leakage 或 evidence mismatch。

### Phase U8：机器人任务体验（独立扩展门）

目标：让 Aletheon 的差异化能力成为可见产品。

该阶段依赖核心工程 UX，但 bridge/simulator/外部团队条件不反向阻塞 U0-U6 的完成；物理实机验证由独立 HIL/production plan 管理。

领域实现与关闭条件由 `docs/plans/robot-vla-production-closure-plan.md` 的 R0–R8 拥有。本阶段只负责用户表面：robot 任务是否复用同一 TaskSnapshot/ActivityEvent/Settlement 并可被用户理解。不得在此重新定义 Policy/VLA、Perception 或 EpisodeReport 契约。

任务：

1. Robot task 使用相同 TaskSnapshot/ActivityEvent/Settlement。
2. TUI 展示 Observe→Plan→Authorize→Execute→Verify→Settle。
3. 展示 device、scene version、bridge digest、attempt、evidence。
4. rosbag/log/plot 作为 Artifact 引用，不内联大对象。
5. Safety Supervisor denial 明确显示为 blocked，不转成普通工具错误。

验收：

- U-ROBOT-001：完成至少一个 Kuavo 仿真闭环。
- U-ROBOT-002：自然语言目标、受控动作、验证和 EpisodeReport 可追踪。
- U-ROBOT-003：云端/Agent 不直接承担硬实时控制。

---

## 7. 历史 PR 执行表（DO-NOT-SCHEDULE）

> **本节仅为历史记录，禁止作为调度输入。** 唯一执行顺序是
> `Aletheon_Unified_Execution_Plan_2026-08-04.md` §3 的节点清单；U0–U8 → X0–X14 的映射见统一计划 §5。
> 本表的价值在于它证明了 U1a/U1b-1/U1b-2/U1c/U1d/U2-projection/U2-TUI/U3/U4-projection/U4-rewind/U5
> 当初是**各自独立的 PR**——统一计划因此把跨层节点拆为 X3a–X3c、X4a–X4d、X5b/X5c、X8a–X8d，并把 X9 拆成 X9a–X9c，而不是合并成大 PR。
> 自动执行器应跳过本表：这里没有验收 ID 归属、状态词表和失败/重试策略。

本表消费架构计划第 7 节的共同 DAG，不再另建平行契约 PR。每个实现 PR 开始前，executor 必须重新 grep 表中符号并将实际文件列表写入 task packet。

| PR | 依赖 | 允许写入重点 | 交付/删除 | 退出证据 |
|---|---|---|---|---|
| U0/P0 baseline reconciliation | 无 | 两份计划、`config/architecture/*` | 现状矩阵、caller inventory | reviewed baseline SHA/diff |
| U1a command contract | P1 architecture gates | `crates/aletheon/src/main.rs`、现有 Fabric client protocol、completion assets | canonical command metadata | parser/help/completion snapshots |
| U1b-1 `TaskKindArg` relocation | U1a | CommandSpec 权威位置、`crates/aletheon/src/main.rs`、`crates/interact/src/tui/cli.rs` | 纯迁移，不删文件 | `interact::cli` 生产调用计数 1→0 + 编译通过 |
| U1b-2 compatibility parser deletion | U1b-1 | `crates/interact/src/tui/cli.rs`、`crates/interact/src/lib.rs`、调用测试 | 删除 `Args::parse()`/旧 handler/re-export，或记录外部兼容阻塞 | caller count=0 + negative gate |
| U1c ensure-running | U1a | Interact host/TUI、Executive launcher/doctor | install-mode aware activation/readiness | stale socket/concurrent start/version-skew tests |
| U-corpus fixtures | U0/P0 | `tests/coding/acceptance/` | 补齐 8→20 个 fixture 与评分 rubric | 20 个 versioned fixture 可运行；U7 的前置条件 |
| U1d run/exec protocol | U1a,U1c | top-level CLI、Fabric wire、Executive exec host | versioned JSONL/exit codes/idempotency | jq/PTY/signal/backpressure tests |
| U2 task/activity projection | P3 architecture contract | Executive projection、Fabric read protocol、Interact reducer | deterministic Task/Activity read model | replay/duplicate/out-of-order tests |
| U2 TUI task console | U2 projection | `crates/interact/src/tui/` | task/activity/changes layout，删除 transcript 重复状态 | golden frames + accessibility/PTY |
| U3 secure input | U2 TUI | TUI editor/completion/history/attachment | palette、history、draft、typed attachment | IME/paste/path/ANSI/security fixtures |
| U4 checkpoint projection | P4 capability coverage,U2 | existing checkpoint/transaction/validation projection | no duplicate `TurnCheckpoint` | schema/replay/integrity tests |
| U4 rewind UI | U4 projection,U3 | TUI checkpoint/diff/review | coverage-aware Accept/Repair/Rollback | conflict/partial/binary/symlink fixtures |
| U5 review/settlement | U4 | Executive validation/acceptance、TUI/CLI review | Host-owned settlement，删除 model-complete shortcut | failing-test/finding/omission tests |
| U6 resume reconciliation | U2,U4,U5,P7 recovery | session/recovery ports、picker/projection | generation fencing、privacy/retention | restart/orphan/principal/worktree tests |
| U7 installed acceptance | U1-U6,U-corpus,P8 deletion | acceptance fixtures/docs；产品缺陷另开 PR | 20-task corpus 与 installed evidence | system deploy + real TUI |
| U8 robot episode view | U6,U7；域内关闭见 R-plan R7/R8 | robot projection/Interact view | domain-specific receipt/artifacts | simulator acceptance；不含物理实机 |

禁止将 CLI 重构、TUI 重写、session schema、memory 迁移和 robot UI 放入同一个 PR。每个 PR 必须列出旧路径删除条件、wire/persistence delta、rollback、最窄验证命令和 owner。

---

## 8. 测试策略

### 8.1 Contract tests

- CommandSpec 与 help/completion/parser 一致。
- ClientIntent 序列化 schema 稳定。
- JSONL event schema version。
- TaskSnapshot projection 可重放。
- Checkpoint settlement state machine。
- client/daemon/core version negotiation、generation fencing 和 idempotency。
- stdout/stderr、JSONL sequence/terminal event、分类 exit code。
- runtime facts 中 inference rounds、provider retries、tool calls、active context 和 cache usage 分维度守恒。

### 8.2 Golden TUI tests

现有 frame recorder 继续使用，但 fixture 必须覆盖真实用户旅程，而不是只检查组件存在：

- 首次启动。
- daemon 启动失败。
- 普通聊天。
- 修改两个文件。
- 长命令流式输出。
- approval。
- diff review。
- test failure/repair。
- checkpoint rewind。
- session resume。
- sub-agent failure。
- robot episode（独立扩展 golden，不计入核心 UX gate）。

### 8.3 PTY integration tests

- crossterm raw mode 恢复。
- Ctrl+C 单次取消、双次退出。
- resize。
- paste。
- IME 延迟。
- terminal disconnect/reconnect。
- tmux/SSH、`NO_COLOR`、低色深、Unicode width 和纯键盘旅程。
- ANSI/OSC/control-sequence sanitization。

### 8.4 真实工程任务基准

corpus 落点是既有目录 `tests/coding/acceptance/`，当前只有 8 个 fixture（`approval_blocked_patch`、`budget_exhaustion`、`clippy_cleanup`、`config_schema_sync`、`dirty_workspace_preservation`、`rustdoc_contract`、`rust_multifile`、`rust_regression_test`），**距要求缺 12 个**。补齐由统一计划 **X11** 负责（依赖只有 X0，**从第一天起可与 X1–X10 全程并行**，不排在兼容删除之后），是 X12 的前置条件，不得在 X12 内临时拼凑，也不得新建平行 corpus 目录。

固定至少 20 个 versioned task fixtures，成功不少于 16 个：

- 5 个定位/解释任务。
- 5 个小型 bug fix。
- 4 个跨文件修改。
- 2 个失败测试修复。
- 2 个 review finding 修复。
- 1 个无在途副作用的 daemon/session resume。
- 1 个 daemon/core 重启与在途 child orphan reconciliation。

成功必须同时满足：

- 产生正确代码或报告。
- validation/evidence receipt 完整。
- 没有 scope 泄漏。
- settlement 由 Host 计算。
- 重放时可以解释关键动作。
- rendered frame、persisted session、receipt/audit、daemon log 和 monitor verdict 一致。
- `provider_unavailable`、`provider_rejected_request` 或渲染 inference error 一律失败。

Robot simulator 使用独立扩展 corpus，不占核心 20 个工程任务名额，也不阻塞核心 UX acceptance。

### 8.5 Installed runtime acceptance

影响 client、daemon、IPC、persistence、config、tools 或 profiles 的最终验收必须：

1. 执行 `sudo bash scripts/aletheon.sh deploy`；user-local deploy 只作为单独模式证据。
2. 验证 `target/release/aletheon`、`/usr/bin/aletheon` 与动态枚举到的所有运行中 Aletheon daemon executable 的 SHA-256 相同；至少核对 machine core、user daemon，以及运行中的 Memory Agent。
3. systemd units active 且 `NRestarts` 在观察窗口内稳定。
4. `/usr/bin/aletheon` 经 official user socket 完成真实 LLM 请求。
5. 保持一个真实 TUI session 连续多轮，并对模型控制 routing/arguments 做三次 fresh-session 验证。

### 8.6 Rust 验证资源规则

- 禁止裸 `cargo`；所有 check/test/clippy/doc/build 通过 `bash scripts/cargo-agent.sh <arguments>`。
- 每个 PR 使用最窄 package 和 test target；只有 integration owner 运行 workspace-wide lane。
- 不并发运行 Executive 或 workspace build。
- 格式检查：`bash scripts/cargo-agent.sh fmt --all -- --check`。

---

## 9. 指标

### 用户指标

- 首次启动成功率 ≥ 95%。
- 首次有效 prompt 时间 ≤ 10 秒（不含模型响应）。
- 常规任务无需查帮助即可完成率 ≥ 80%。
- 恢复成功率 ≥ 95%。
- 误触危险操作为 0。

### 工程指标

- 20 个真实任务至少 16 个成功。
- 100% accepted 修改有 receipt。
- 100% fallback 可观测。
- 100% running activity 在重启后被 reconcile。
- CLI/TUI/line mode 不再拥有不同的 command handler。
- provider inference rounds、provider retries、tool calls、terminal tool results 分开显示和统计。
- active context occupancy 与累计 billed/session tokens 分开；cache unknown 不记作 0。

### 指标测量协议

每项百分比/延迟必须记录：fixture revision、样本数、冷/热启动、硬件/OS、TTY/SSH、provider/model、预算以及 p50/p95/p99。首次启动时间不含模型响应，但必须包含 config/socket/daemon readiness。行为 telemetry 如需采集，必须 opt-in，并定义脱敏、保留期、owner 和删除方式。

"工程指标"中属于架构不变量的项（CLI/TUI/line mode 不再拥有不同 command handler）落点为 `config/architecture/metrics.env` 与 `compatibility-debt.tsv`，由 `scripts/libexec/aletheon/architecture-check.sh` 棘轮，不在本计划另建指标文件。用户指标与任务成功率落点为 `tests/coding/acceptance/` 的 run 记录，必须随 fixture revision 一起版本化。

---

## 10. 完成定义

### 10.1 核心工程 UX 完成

只有满足以下条件，核心 UX 才算完成：

1. 用户运行 `aletheon` 可以直接进入可用会话。
2. 项目只有一个用户 CLI 规范和一个 command handler 路径。
3. 用户可以清楚看到 Task、Plan、Activity、Diff、Validation、Settlement。
4. 每轮修改都有 coverage-aware checkpoint/receipt；`full` mutation 可接受、修复或回退，其他等级不会显示虚假完整回退。
5. `resume` 可以恢复对话、任务、预算、checkpoint 和 child 状态。
6. 工程任务完成由 Host acceptance 判定。
7. 20 个真实任务至少成功 16 个。
8. installed runtime acceptance 全部通过，且 monitor 与 runtime evidence 无冲突。
9. Action Palette、attachment、history/draft 和不可信输出通过安全/隐私验收。
10. 兼容 parser/API 债务已删除：`TaskKindArg` 已迁至 CommandSpec 权威位置，`crates/aletheon` 不再 import `interact::cli::*`，`Args::parse()`/旧 handler/re-export 已移除；或有明确 external compatibility 阻塞、owner、deadline 和 gate。

### 10.2 Robot UX 扩展完成

Robot UX 独立关闭：至少一个 Kuavo simulator 闭环通过同一用户主链，具有 domain receipt、safety denial 和 Artifact evidence。物理实机验证不属于本文档完成条件。

在这些条件完成以前，不应继续增加新的顶层 slash command、TUI overlay 或并行用户入口。

## 11. Owner / independent review 清单

审核只需回答以下阻断项；任一为“否”则回到对应阶段修改计划：

1. baseline SHA 与列出的 current-code facts 是否一致？
2. 是否复用了现有 architecture inventories、`ApprovalRequest`、`TurnCheckpoint` 和 runtime observability，而非重复定义？
3. Contract migration matrix 是否足以区分 reuse/extend/project/v2/new/delete？
4. 每个 PR 是否有唯一 owner、依赖、写入范围、删除项、rollback 和最窄证据？
5. daemon startup、version skew、generation fencing、orphan reconciliation 是否 fail closed？
6. Action Palette、`@`、`!`、history/draft 和 untrusted output 是否覆盖安全/隐私/IME/paste？
7. checkpoint 是否诚实区分 full/best-effort/non-rollbackable？
8. installed system runtime 是否是最终产品验收，而非临时 binary/socket？
9. 核心工程 UX 和 Robot extension 是否可以独立关闭？
10. 兼容层删除是否遵守"先迁 `TaskKindArg`、后删 parser"的两步顺序，而不是把 `caller count=0` 当作已成立事实？
11. `runtime_facts` 的 provider/cache 字段是否只做投影，且 owner 已指向 C-plan？
12. 20-task corpus 是否锚定 `tests/coding/acceptance/`，并把 8→20 的补齐作为 U7 前置 PR 而非 U7 内产物？
