# Aletheon CLI

> 用户交互入口，支持单消息和 TUI 两种模式。通过 Unix socket 与 aletheond 通信。
> CLI 逻辑已合并到 `aletheon-body/src/impl/cli/`，TUI 在 `aletheon-body/src/impl/ui/`。
> `aletheon-cli` crate 保留为薄包装（向后兼容），实际逻辑在 body crate 中。

**模块编号:** CLI
**关联模块:** [daemon](../daemon/README.md), [body/ui](../body/ui.md)
**Crate:** `aletheon-body` (feature `cli`), `aletheon-cli` (thin re-export)
**最后更新:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| CLI arg parsing | ✅ Implemented | `aletheon-cli/src/main.rs` | clap, -m/--tui/--simple |
| Single message mode | ✅ Implemented | `aletheon-cli/src/main.rs` | `-m "text"` → send → print → exit |
| Simple REPL mode | ✅ Implemented | `aletheon-cli/src/main.rs` | `--simple`, stdin loop |
| TUI mode (default) | ✅ Implemented | `aletheon-body/src/impl/ui/mod.rs` | ratatui, alternate screen |
| Chat widget | ✅ Implemented | `aletheon-body/src/impl/ui/chat.rs` | Message list, scroll, streaming update |
| Input handling | ✅ Implemented | `aletheon-body/src/impl/ui/mod.rs` | CJK-aware, IME delay, cursor movement |
| Command parser | ✅ Implemented | `aletheon-body/src/impl/ui/command.rs` | /help, /clear, /quit, /status, /skills |
| Skill loader | ✅ Implemented | `aletheon-body/src/impl/ui/skill.rs` | ~/.aletheon/skills/ SKILL.md |
| Status bar | ✅ Implemented | `aletheon-body/src/impl/ui/status.rs` | Connection status, model name |
| Markdown renderer | ✅ Implemented | `aletheon-body/src/impl/ui/markdown.rs` | Styled text for ratatui |
| Terminal compat | ✅ Implemented | `aletheon-body/src/impl/ui/term_compat.rs` | Unicode/color detection |
| Computer view | 🔶 Partial | `aletheon-body/src/impl/ui/computer.rs` | Feature-gated (input+display+a11y) |
| Streaming display | ⬜ Planned | — | Response chunks not streamed to TUI |
| History persistence | ⬜ Planned | — | No command history across sessions |
| Multi-line editor | ⬜ Planned | — | Only Shift+Enter newline, no real editor |

---

## 目录

- [1. 概述](#1-概述)
- [2. 架构](#2-架构)
- [3. 三种运行模式](#3-三种运行模式)
  - [3.1 单消息模式 (-m)](#31-单消息模式--m)
  - [3.2 TUI 模式 (默认)](#32-tui-模式-默认)
  - [3.3 简单 REPL 模式 (--simple)](#33-简单-repl-模式---simple)
- [4. 与 Daemon 通信](#4-与-daemon-通信)
- [5. TUI 架构](#5-tui-架构)
  - [5.1 App 状态](#51-app-状态)
  - [5.2 事件循环](#52-事件循环)
  - [5.3 输入处理](#53-输入处理)
  - [5.4 显示布局](#54-显示布局)
  - [5.5 命令系统](#55-命令系统)
  - [5.6 技能系统](#56-技能系统)
- [6. 已识别缺陷](#6-已识别缺陷)

---

## 1. 概述

`aletheon-cli` 是 Aletheon 的用户交互层。它不包含推理逻辑，仅负责:
- 接收用户输入
- 格式化为 JSON-RPC 请求
- 通过 Unix socket 发送到 `aletheond`
- 接收并渲染响应

三种模式满足不同场景需求:
- **单消息** (`-m`) — 脚本集成、快速查询
- **TUI** (默认) — 日常交互，Markdown 渲染、技能系统
- **简单 REPL** (`--simple`) — 管道、非 TTY 环境

---

## 2. 架构

```
┌──────────────────────────────────────────────────────────┐
│                    aletheon-cli                           │
│                                                          │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────────┐ │
│  │ -m mode     │  │ TUI mode     │  │ --simple mode   │ │
│  │ (one-shot)  │  │ (default)    │  │ (stdin REPL)    │ │
│  └──────┬──────┘  └──────┬───────┘  └───────┬─────────┘ │
│         │                │                   │           │
│         │    ┌───────────┴───────────┐       │           │
│         │    │    ui::run()          │       │           │
│         │    │  ┌─────────────────┐  │       │           │
│         │    │  │ App state       │  │       │           │
│         │    │  │ chat/input/skill│  │       │           │
│         │    │  └────────┬────────┘  │       │           │
│         │    │           │           │       │           │
│         │    │  ┌────────┴────────┐  │       │           │
│         │    │  │ ratatui draw()  │  │       │           │
│         │    │  │ header│chat│input│  │       │           │
│         │    │  └─────────────────┘  │       │           │
│         │    └───────────┬───────────┘       │           │
│         │                │                   │           │
│         └────────────────┼───────────────────┘           │
│                          │                               │
│              UnixStream (JSON-RPC over Unix socket)      │
└──────────────────────────┼───────────────────────────────┘
                           │
                           ▼
                    ┌──────────────┐
                    │  aletheond   │
                    └──────────────┘
```

---

## 3. 三种运行模式

入口文件: `aletheon-cli/src/main.rs`

```rust
#[derive(Parser)]
#[command(name = "aletheon-cli", about = "Aletheon CLI client")]
struct Args {
    #[arg(short, long, default_value = "/tmp/aletheon/aletheon.sock")]
    socket: PathBuf,

    #[arg(short, long)]
    message: Option<String>,

    #[arg(long)]
    tui: bool,

    #[arg(long)]
    simple: bool,
}
```

路由逻辑:

```
if -m present → single_message()
else if not --simple → ui::run() (TUI)
else → simple_cli()
```

### 3.1 单消息模式 (-m)

```rust
async fn single_message(socket: &PathBuf, msg: &str) -> Result<()> {
    let mut stream = UnixStream::connect(socket).await?;
    // send JSON-RPC "chat" request
    // read one response line
    // print result.response or error
}
```

用途: 脚本集成、快速查询。连接 → 发送 → 接收 → 退出。

### 3.2 TUI 模式 (默认)

调用 `aletheon_body::impl::ui::run(socket_path)`。详见 [TUI 架构](#5-tui-架构)。

非 TTY 环境（管道、重定向）自动降级为简单行模式 (`simple_line_mode`)。

### 3.3 简单 REPL 模式 (--simple)

```rust
async fn simple_cli(socket: &PathBuf) -> Result<()> {
    // connect to socket
    // loop: print "> ", read stdin line, send JSON-RPC, print response
    // "quit" exits
}
```

无 TUI 依赖，纯 stdin/stdout 循环。

---

## 4. 与 Daemon 通信

协议: **行分隔 JSON-RPC**（每条消息以 `\n` 结尾）

请求格式:

```json
{"jsonrpc": "2.0", "id": 1, "method": "chat", "params": {"message": "hello"}}
```

响应格式:

```json
{"jsonrpc": "2.0", "id": 1, "result": {"response": "Hello! How can I help?"}}
```

错误格式:

```json
{"jsonrpc": "2.0", "id": 1, "error": {"code": -32000, "message": "LLM error: ..."}}
```

连接方式: `tokio::net::UnixStream::connect(socket_path)`

---

## 5. TUI 架构

代码位置: `aletheon-body/src/impl/ui/`

### 5.1 App 状态

```rust
struct App {
    chat: ChatWidget,         // 消息列表 + 滚动
    input_buf: String,        // 当前输入缓冲
    cursor: usize,            // 光标位置 (byte index)
    stream: UnixStream,       // daemon 连接
    read_buf: Vec<u8>,        // 读取缓冲 (8192 bytes)
    running: bool,            // 主循环控制
    streaming: bool,          // 等待 daemon 响应
    response_buf: String,     // 响应拼接缓冲
    caps: TermCaps,           // 终端能力检测
    skill_loader: SkillLoader,// 技能加载器
    model_name: String,       // 模型名称显示
    status: StatusBar,        // 状态栏
    last_ctrl_c: Option<Instant>, // 双击 Ctrl+C 检测
    has_cjk: bool,            // CJK 输入检测
    pending_submit: Option<Instant>, // IME 延迟提交
    scroll_offset: u16,       // 聊天区滚动偏移
    first_render: bool,       // 首次渲染标记
}
```

### 5.2 事件循环

```
while app.running:
    1. terminal.draw()           // 渲染 UI
    2. check pending_submit      // IME 延迟提交 (100ms)
    3. crossterm::event::poll()  // 键盘/粘贴/resize 事件
    4. handle_key() / handle_paste()
    5. try_read_response()       // daemon 响应读取
```

poll 超时:
- streaming 或 pending_submit 时: 50ms（快速刷新）
- 空闲时: 200ms（降低 CPU）

### 5.3 输入处理

代码位置: `aletheon-body/src/impl/ui/mod.rs` `handle_key()`

| 按键 | 行为 |
|------|------|
| Enter | 提交消息（CJK 时延迟 100ms 等待 IME） |
| Shift+Enter / Alt+Enter | 插入换行 |
| `\` + Enter | 续行（移除 `\`，插入换行） |
| Backspace / Delete | 删除字符（UTF-8 安全） |
| Left / Right / Home / End | 光标移动 |
| PageUp / PageDown | 聊天区滚动 |
| Esc | 清空输入 |
| Ctrl+C | 清空输入（双击 2s 内退出） |
| Ctrl+D | 空输入时退出 |
| Ctrl+L | 清屏 |

CJK 检测范围: `U+4E00-9FFF`, `U+3400-4DBF`, `U+3000-303F`, `U+FF00-FFEF`, `U+AC00-D7AF`, `U+3040-309F`, `U+30A0-30FF`

### 5.4 显示布局

```
┌──────────────────────────────────┐
│  header (3 rows first render)    │  aletheon v0.1.0, model, hints
│  header (1 row after)            │  compact title bar
├──────────────────────────────────┤
│  chat area (flexible)            │  ChatWidget: styled messages
│                                  │  User/Assistant/System roles
│                                  │  Markdown rendering
│                                  │  Scroll support
├──────────────────────────────────┤
│  separator line                  │  horizontal rule
│  input line with cursor          │  ❯ text[cursor]  [CJK]
│  hint line                       │  Enter/Shift+Enter/Esc hints
├──────────────────────────────────┤
│  status bar (1 row)              │  connection status, model
└──────────────────────────────────┘
```

`ChatWidget` 渲染:
- 每条消息预渲染为 `Vec<Line<'static>>`（缓存）
- 支持 `update_content()` 用于流式更新
- Markdown 标题、粗体、代码块样式

`TermCaps` 检测:
- Unicode 支持 → 使用 `│`, `❯`, `─` 等字符
- 降级 → 使用 `|`, `>`, `-`
- 颜色支持检测

### 5.5 命令系统

代码位置: `aletheon-body/src/impl/ui/command.rs`

以 `/` 开头的输入被解析为命令:

| 命令 | 说明 |
|------|------|
| `/help` | 显示帮助信息 |
| `/clear` | 清空聊天记录 |
| `/quit` `/exit` | 退出 |
| `/status` | 查询 daemon 状态（发送 JSON-RPC） |
| `/input` `/i` | (保留) |
| `/computer <args>` | 计算机控制命令 |
| `/<skill-name> [args]` | 执行技能 |

### 5.6 技能系统

代码位置: `aletheon-body/src/impl/ui/skill.rs`

技能目录: `~/.aletheon/skills/`

```
~/.aletheon/skills/
├── code-review/
│   └── SKILL.md
├── explain/
│   └── SKILL.md
└── ...
```

加载逻辑:
1. 扫描 skills 目录下的子目录
2. 读取每个子目录的 `SKILL.md`
3. 首段作为 description，全文作为 content
4. `/skill-name args` → 将 SKILL.md content + args 作为消息发送到 daemon

---

## 6. 已识别缺陷

### 6.1 响应非流式显示

当前 `try_read_response()` 读取完整 JSON 后才调用 `process_response()`。LLM 长响应时用户看到空白等待。

**修复方向:** 支持 JSON-RPC notification chunk 推送，逐步 `update_content()`。

### 6.2 无输入历史

无 readline 风格的历史记录。上下箭头未绑定历史回溯。

### 6.3 Computer View 未完整

`computer.rs` 存在但受 feature gate 限制 (`input + display + a11y`)，功能未完全集成到 TUI。

### 6.4 非 TTY 降级粗糙

`simple_line_mode()` 直接使用 `stdin.read_line()` 阻塞调用，无法与 async stream 读取并发。长响应期间无法输入。
