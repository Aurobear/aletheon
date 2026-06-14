> Migrated from docs/design/perception/fuse-interface.md — code paths updated to aletheon-* crate structure

# FUSE 虚拟文件系统接口

> 通过 FUSE 挂载点暴露 Agent 状态、感知数据和控制接口，让 shell 脚本和系统工具可以直接与 Agent 交互。

**模块编号:** 08
**关联模块:** [编排引擎](../orchestration/orchestration-engine.md), [IPC 与内核](../platform/kernel-ipc.md), [感知层](perception-layer.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| AgentFs (in-memory API) | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/filesystem.rs` | In-memory virtual filesystem with read/write/readdir, pause support |
| FsNode types | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/filesystem.rs` | Directory/File/DynamicFile node types |
| FUSE module | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/mod.rs` | Module entry point |
| FuseMount | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/mount.rs` | `fuse3::path::PathFileSystem` integration behind `fuse` feature flag; stub mode when feature disabled |
| PathFileSystem impl | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/mount.rs` | `AgentFs` implements `fuse3::path::PathFileSystem` (lookup, getattr, read, write, readdir) behind `#[cfg(feature = "fuse")]` |
| StateProvider trait | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/provider.rs` | Abstraction for data sources (`get_sensor_data`, `get_context`, `get_log`, `get_agent_status`) |
| LiveStateProvider | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/provider.rs` | Reads live system state from `/proc` (loadavg, meminfo, diskstats, net/dev) |
| MockStateProvider | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/provider.rs` | In-memory mock with pre-settable responses and call recording for testing |
| ControlsValidator | ✅ Implemented | `crates/aletheon-self/src/impl/perception/fuse/controls.rs` | Write validation for `/controls/` (toggle "0"/"1", TOML syntax check, allowlist) |

> **Note:** The real FUSE mount is implemented behind the `fuse` feature flag. When `fuse` is enabled, `AgentFs` implements `fuse3::path::PathFileSystem` and `FuseMount` performs an actual mount via `fuse3::Session`. Without the feature, `FuseMount` operates in stub mode (always reports unmounted). The directory structure `/context/`, `/controls/`, `/sensors/`, `/logs/`, `/agents/` is fully wired.

---

## 目录

1. [概述](#1-概述)
2. [当前设计](#2-当前设计)
3. [已识别缺陷](#3-已识别缺陷)
4. [改进设计](#4-改进设计)
5. [实现要点](#5-实现要点)
6. [参考来源](#6-参考来源)

---

## 1. 概述

FUSE 虚拟文件系统为 Agent 提供 Unix 风格的交互接口。通过挂载在 `/mnt/agent/` 的虚拟目录树，用户和系统工具可以用标准的 `cat`、`echo`、`tail -f` 命令与 Agent 交互，无需专用客户端。

核心价值：
- **零学习成本**: shell 用户天然熟悉文件系统操作
- **脚本友好**: 可直接在 shell 脚本中集成 Agent 控制
- **可观测**: `watch`、`tail -f` 等工具直接可用
- **Unix 哲学**: "一切皆文件"

---

## 2. 当前设计

### 2.1 挂载结构

```
/mnt/agent/                    # Agent 的 FUSE 挂载点
│
├── context/                   # Agent 上下文 (只读)
│   ├── focus                 # 当前关注什么
│   ├── tasks                 # 任务队列
│   ├── status                # Agent 状态
│   └── memory/               # 记忆概览
│       ├── core_blocks       # Core Memory block 列表
│       ├── recall_count      # 回忆记忆条目数
│       └── archival_count    # 归档记忆条目数
│
├── controls/                  # 控制接口 (只写)
│   ├── schedule              # echo "..." > schedule
│   ├── notify                # echo "..." > notify
│   ├── execute               # echo "cmd" > execute
│   └── memory/               # 记忆编辑
│       ├── append            # echo "label:content" > ...
│       └── replace           # echo "label:old:new" > ...
│
├── sensors/                   # 感知数据 (只读)
│   ├── system                # 系统状态
│   ├── network               # 网络状态
│   ├── processes             # 进程列表
│   └── events                # 最近事件流
│
├── logs/                      # 日志 (只读)
│   ├── decisions             # Agent 决策记录
│   ├── reasoning             # 推理过程
│   └── audit                 # 审计日志
│
└── agents/                    # 多 Agent 视图 (只读)
    ├── coordinator/           # 协调器 Agent
    ├── fs_agent/              # 文件系统 Agent
    ├── net_agent/             # 网络 Agent
    └── ...                    # 其他 Agent
```

### 2.2 交互方式

```bash
# 查看 Agent 当前关注点
$ cat /mnt/agent/context/focus

# 创建定时任务
$ echo "明天9点开会" > /mnt/agent/controls/schedule

# 实时监控系统事件
$ tail -f /mnt/agent/sensors/events

# 查看子 Agent 状态
$ cat /mnt/agent/agents/fs_agent/status

# 向 Core Memory 追加内容
$ echo "user_prefs:偏好深色主题" > /mnt/agent/controls/memory/append

# 执行命令（通过 Agent 沙箱）
$ echo "make build" > /mnt/agent/controls/execute
```

### 2.3 访问权限模型

| 路径 | 权限 | 说明 |
|------|------|------|
| `context/` | 0444 (只读) | Agent 状态，任何人可读 |
| `controls/` | 0200 (只写) | 控制命令，仅 owner 可写 |
| `sensors/` | 0444 (只读) | 感知数据，任何人可读 |
| `logs/` | 0400 (owner 只读) | 日志，仅 owner 可读 |
| `agents/` | 0444 (只读) | Agent 视图，任何人可读 |
| `controls/execute` | 0200 (owner 只写) | 命令执行，需 L2 权限 |

---

## 3. 已识别缺陷

当前设计在 FUSE 接口本身上较为完整，但与会话持久化 (Session Persistence) 的集成存在设计空白：

### 与 Session Persistence 的集成

**问题:** FUSE 挂载点暴露的是当前运行时状态。如果 Agent 重启或崩溃，`context/` 和 `agents/` 下的状态会丢失或变为陈旧数据。

**影响:**
- 用户通过 `tail -f /mnt/agent/sensors/events` 监控时，Agent 重启会导致流中断
- `context/focus` 和 `context/tasks` 在重启后需要从检查点恢复
- `agents/*/status` 在 Agent 崩溃后可能显示过期状态

**建议:** FUSE 层应读取检查点 (Checkpoint) 数据而非仅缓存在内存中。Agent 重启后，FUSE 层从 SQLite/文件检查点恢复状态，对外表现一致。

---

## 4. 改进设计

### 4.1 会话持久化集成

FUSE 层应实现一个 `StateProvider` trait，抽象状态来源：

```rust
trait StateProvider: Send + Sync {
    /// 获取当前 Agent 状态
    fn get_context(&self) -> Result<AgentContext>;

    /// 获取指定 Agent 的状态
    fn get_agent_status(&self, agent_id: &str) -> Result<AgentStatus>;

    /// 获取感知事件流（支持 tail -f）
    fn subscribe_events(&self, since: Option<Timestamp>) -> EventStream;

    /// 执行控制命令
    fn execute_control(&self, path: &str, input: &[u8]) -> Result<ControlResponse>;
}
```

两种实现：
- **LiveStateProvider**: Agent 运行时，直接读取内存状态
- **CheckpointStateProvider**: Agent 离线时，从检查点恢复状态

FUSE 层自动选择：运行时用 Live，离线用 Checkpoint。

### 4.2 事件流的 tail -f 支持

对于 `sensors/events` 和 `logs/` 等流式文件，`open()` 返回带 offset 的 file handle，`read()` 中 offset 表示"从这个位置之后的新事件"。如果没有新事件，阻塞等待（或返回 EAGAIN 配合 poll）。

### 4.3 controls/ 写入验证

写入 `controls/` 下的文件时，应经过安全策略引擎验证：
- L0 权限: 自动执行
- L2 权限: 需要确认（返回 EPERM 或触发确认流程）
- L3 权限: 禁止（返回 EACCES）

---

## 5. 实现要点

- **使用 fuse3 crate**: libfuse 3.x 的 Rust 绑定，支持 async/await。
- **挂载点权限**: `/mnt/agent/` 应由 aletheond 创建和管理，挂载时使用 `allow_other` 或 `allow_root` 选项。
- **缓存策略**: `context/` 和 `sensors/` 的内容变化频繁，不应启用页缓存 (direct_io)。
- **日志轮转**: `logs/` 下的文件应支持大小限制和轮转，避免 FUSE 返回无限大的文件。
- **优雅关闭**: aletheond 停止时应执行 `fusermount -u /mnt/agent/`，确保挂载点清理。
- **systemd 集成**: FUSE 挂载应作为 aletheond.service 的一部分，或独立为 agent-fuse.service。
- **与 Session Persistence 的集成**: FUSE 层的状态读取应通过 StateProvider trait 抽象，支持从检查点恢复。
- **Poll 支持**: 对 `sensors/events` 和 `logs/` 等流式文件实现 `poll()`，让 `select()`/`epoll()` 可用。

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| **Linux FUSE** | fuse3 API、PathFileSystem trait、direct_io 选项 |
| **fuse3 crate** | Rust async FUSE 绑定，`fuse3::path::PathFileSystem` |
| **/proc /sys** | 内核虚拟文件系统的设计范式（只读/只写分离、流式读取） |
| **systemd** | FUSE 挂载的 systemd 集成（MountUnit、自动卸载） |

---

## Implementation Summary

**Code location:** `crates/aletheon-self/src/impl/perception/fuse/` (5 files: mod.rs, filesystem.rs, mount.rs, provider.rs, controls.rs)

**What IS implemented:**
- `FsNode` enum (`filesystem.rs`) — Directory/File/DynamicFile node types
- `AgentFs` struct (`filesystem.rs`) — in-memory virtual filesystem with read(), write(), readdir(), pause/resume support
- Directory structure: context/, controls/, sensors/, logs/, agents/
- Dynamic file content generated from /proc via `StateProvider` trait
- `FuseMount` (`mount.rs`) — manages FUSE mount lifecycle; implements `fuse3::path::PathFileSystem` for `AgentFs` behind `#[cfg(feature = "fuse")]`
- PathFileSystem trait impl (`mount.rs`) — full bridge: lookup, getattr, read (offset-aware), write, readdir
- `StateProvider` trait (`provider.rs`) — abstraction for data sources
- `LiveStateProvider` (`provider.rs`) — reads /proc/loadavg, /proc/meminfo, /proc/diskstats, /proc/net/dev for live sensor data
- `MockStateProvider` (`provider.rs`) — in-memory mock with pre-set responses and call recording for testing
- `ControlsValidator` (`controls.rs`) — write validation for /controls/ paths: allowlist check, toggle "0"/"1" validation, TOML syntax validation

**What is NOT implemented (remaining work):**
- No systemd mount unit for FUSE integration
- `StateProvider` not yet wired into `AgentFs` — `AgentFs` currently generates dynamic content internally rather than delegating to a `StateProvider`
- `ControlsValidator` not yet integrated into the write path of `AgentFs`/`FuseMount`

**Test coverage:**
- `mount.rs` — 3 tests (stub mode, mount_point accessor, unmount idempotent)
- `provider.rs` — 7 tests (mock sensor/context/log/agent_status, missing key, error response, recorded calls, clear calls)
- `controls.rs` — 9 tests (valid/invalid toggle, non-UTF8, unknown control, valid/invalid TOML, custom validator, whitespace rejection)
- `filesystem.rs` — 5 tests (read, write, readdir, pause functionality)
