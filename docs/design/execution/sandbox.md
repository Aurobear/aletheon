# 沙箱执行 (Sandbox Execution)

> 工具系统的沙箱隔离执行模型，包含 bubblewrap 配置、执行器设计和多后端可移植性方案。

**模块编号:** 03 (沙箱子系统)
**关联模块:** [tool-system.md](tool-system.md), [mcp-integration.md](mcp-integration.md)
**最后更新:** 2026-06-06
**来源:** 从 `03-tool-system.md` 的 2.3-2.5、3.6、4.6 节提取。

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| BubblewrapBackend | ✅ Implemented | `sandbox/bubblewrap.rs` | Full namespace isolation |
| ProcessBackend | ✅ Implemented | `sandbox/process.rs` | seccomp + capabilities, no namespace |
| NoopBackend | ✅ Implemented | `sandbox/noop.rs` | No isolation, trusted env only |
| SandboxExecutor | ✅ Implemented | `sandbox/executor.rs` | Multi-backend with auto-selection |
| SandboxEnvironment | ✅ Implemented | `sandbox/env.rs` | Docker/WSL2/Android detection |
| FilesystemPolicy | ✅ Implemented | `sandbox/policy.rs` | FsDefault, WritableRoot, protected_metadata, unreadable_globs |
| BwrapBuilder | ✅ Implemented | `sandbox/bwrap_builder.rs` | Ordered bwrap arg construction from FilesystemPolicy |
| GlobScanner | ✅ Implemented | `sandbox/glob_scanner.rs` | rg-first + walkdir fallback for glob matching |
| ContainerBackend | ✅ Implemented | `sandbox/container.rs` | Docker/Podman container backend |
| Seccomp filter | ⬜ Planned | — | libseccomp integration not started |

---

## 目录

1. [概述](#1-概述)
2. [当前设计](#2-当前设计)
   - [2.1 沙箱执行流程](#21-沙箱执行流程)
   - [2.2 沙箱配置](#22-沙箱配置)
   - [2.3 沙箱执行器](#23-沙箱执行器)
3. [已识别缺陷](#3-已识别缺陷)
   - [3.1 P1: 沙箱后端可移植性](#31-p1-沙箱后端可移植性)
4. [改进设计](#4-改进设计)
   - [4.1 SandboxBackend trait 定义](#41-sandboxbackend-trait-定义)
   - [4.2 BubblewrapBackend（完整隔离）](#42-bubblewrapbackend完整隔离)
   - [4.3 ProcessBackend（进程隔离，无 namespace）](#43-processbackend进程隔离无-namespace)
   - [4.4 NoopBackend（无隔离，受信任环境）](#44-noopbackend无隔离受信任环境)
   - [4.5 SandboxExecutor 改造](#45-sandboxexecutor-改造)
   - [4.6 环境检测与自动配置](#46-环境检测与自动配置)
5. [实现要点](#5-实现要点)
6. [参考来源](#6-参考来源)

---

## 1. 概述

沙箱执行是工具系统安全模型的核心。它确保每个工具调用（特别是 shell 命令）都在受限环境中运行，防止恶意或误操作对宿主系统造成不可逆损害。

沙箱设计遵循**安全第一，性能其次**的原则：
- 默认无网络访问
- 文件系统只读挂载系统目录，仅工作目录可写
- 资源使用受 cgroups 限制
- 危险系统调用被 seccomp filter 禁止

---

## 2. 当前设计

### 2.1 沙箱执行流程

```
ToolUseBlock { name: "bash", input: {cmd: "make"} }
     │
     ▼
┌─────────────┐
│ 权限检查     │  L0→自动 L1→通知 L2→确认 L3→拒绝
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ 沙箱创建     │  bubblewrap + seccomp + cgroups
│             │  namespace 隔离
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ 执行命令     │  带超时、资源限制
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ 收集结果     │  stdout + stderr + exit_code
│             │  + 副作用追踪
└──────┬──────┘
       │
       ▼
┌─────────────┐
│ 审计记录     │  写入 audit.jsonl
└──────┬──────┘
       │
       ▼
ToolResultBlock { content: "...", is_error: false }
```

### 2.2 沙箱配置

```yaml
bubblewrap:
  --ro-bind /usr /usr         # 系统只读
  --bind /home/user /home/user # 工作目录可写
  --tmpfs /tmp                 # 临时目录
  --unshare-net               # 默认无网络
  --die-with-parent           # 父进程死则子进程死

seccomp:
  禁止: mount, umount, reboot, kexec, ...
  允许: 文件操作, 进程管理, 网络 (按需)

cgroups:
  CPU: 50% 上限
  Memory: 2G 上限
  IO: best-effort
```

### 2.3 沙箱执行器

**SandboxExecutor** — 多后端沙箱执行器，自动选择最佳可用后端（BubblewrapBackend / ProcessBackend / NoopBackend）。
- 代码位置: `sandbox/executor.rs`
- 执行流程：创建 namespace + cgroup → 应用 seccomp filter → 执行命令 → 收集结果 → 清理

---

## 3. 已识别缺陷

### 3.1 P1: 沙箱后端可移植性

**问题描述：** 工具系统的沙箱执行强依赖 bubblewrap（bwrap）作为进程隔离机制。bubblewrap 需要 Linux user namespace 支持（`unshare(CLONE_NEWUSER)`），但在 Docker 容器（默认）、WSL2、Kubernetes Pod、systemd-nspawn、Android、嵌入式 Linux 等常见环境中受限或不可用。当前 `SandboxExecutor` 直接调用 bwrap，没有 fallback 逻辑。

**影响：**
- Docker 开发环境不可用（默认配置下 bwrap 无法工作）
- WSL2 用户受 namespace 限制影响
- Android 端完全不可用（无 bubblewrap）
- 开发者被迫使用 `--privileged` 破坏容器安全隔离

**来源文档：** `gap-analysis/phase-3/tool-system/sandbox-backend-portability.md`

---

## 4. 改进设计

定义 `SandboxBackend` trait，实现三种后端（BubblewrapBackend、ProcessBackend、NoopBackend），运行时按环境自动选择。

### 4.1 SandboxBackend trait 定义

```rust
#[async_trait]
trait SandboxBackend: Send + Sync {
    fn name(&self) -> &str;
    fn isolation_level(&self) -> IsolationLevel;
    fn is_available(&self) -> bool;
    fn capabilities(&self) -> SandboxCapabilities;
    async fn execute(&self, cmd: &str, config: &SandboxConfig, timeout: Duration) -> Result<SandboxResult>;
}
```

**IsolationLevel — 隔离级别：**

| 级别 | 说明 |
|------|------|
| Full | 完整隔离：namespace + 文件系统绑定 + seccomp + cgroups |
| Process | 进程隔离：seccomp + capabilities 限制（无 namespace） |
| None | 无隔离：直接执行（仅用于受信任环境） |

**SandboxCapabilities — 能力描述：** filesystem_isolation, network_isolation, resource_limits, seccomp_filter, writable_root_protection, limitations

**SandboxPreference — 偏好模式：**

| 模式 | 说明 |
|------|------|
| Auto | 自动选择最佳可用后端（默认） |
| Require | 必须使用完整沙箱，不可用时拒绝执行 |
| Forbid | 禁止沙箱，直接执行（调试模式） |
| BestEffort | 尽力使用沙箱，不可用时降级并记录警告 |

### 4.2 BubblewrapBackend（完整隔离）

- **IsolationLevel:** Full
- **可用性检查:** bwrap 二进制存在 + user namespace 支持 + `bwrap --version` 成功
- **能力:** 完整的文件系统/网络隔离、资源限制、seccomp filter、WritableRoot 保护
- **限制:** 需要 user namespace 支持
- **执行:** 构建 bwrap 参数（`--ro-bind`, `--bind`, `--tmpfs`, `--unshare-net`, `--die-with-parent`）+ 超时控制

### 4.3 ProcessBackend（进程隔离，无 namespace）

- **IsolationLevel:** Process
- **可用性检查:** seccomp 支持
- **能力:** seccomp filter
- **限制:** 无文件系统隔离、无网络隔离、无资源限制、无 WritableRoot 保护
- **执行:** fork 子进程 + seccomp filter + capabilities 限制 + rlimits

### 4.4 NoopBackend（无隔离，受信任环境）

- **IsolationLevel:** None
- **可用性检查:** 始终可用
- **能力:** 无任何隔离
- **限制:** NO ISOLATION — 仅用于受信任环境
- **执行:** 直接执行，无隔离

### 4.5 SandboxExecutor 改造

SandboxExecutor 按优先级排列后端列表（BubblewrapBackend → ProcessBackend → NoopBackend），根据 SandboxPreference 选择：

| Preference | 选择逻辑 |
|-----------|----------|
| Auto / BestEffort | 第一个 `is_available()` 为 true 的后端 |
| Require | 第一个 `isolation_level() == Full` 且 `is_available()` 的后端 |
| Forbid | 第一个 `isolation_level() == None` 的后端 |

BestEffort 模式下，如果实际使用的后端隔离级别低于 Full，会记录警告日志。

### 4.6 环境检测与自动配置

**SandboxEnvironment** — 环境检测器，检测：
- `in_docker` — `/.dockerenv` 存在 或 `/proc/1/cgroup` 包含 "docker"/"containerd"
- `in_wsl2` — `/proc/version` 包含 "WSL2"/"microsoft"
- `in_android` — `/system/build.prop` 存在
- `user_ns_available` — `unshare(CLONE_NEWUSER)` 成功
- `seccomp_available` — seccomp 可用性检查
- `kernel_version` — `/proc/version`

**自动偏好推荐：**

| 环境 | 推荐偏好 |
|------|----------|
| user namespace 可用 | Require（完整沙箱） |
| seccomp 可用 / Docker / WSL2 | BestEffort（降级但继续） |
| 其他 | BestEffort（尽力而为） |

**配置示例：**

```toml
[sandbox]
preference = "auto"  # auto | require | forbid | best_effort
fallback_log_level = "warn"
```

---

## 5. 实现要点

| 项目 | 说明 |
|------|------|
| **沙箱后端** | `agent-core/src/sandbox/backend.rs` — `SandboxBackend` trait + Bubblewrap/Process/Noop 三后端 |
| **沙箱执行器** | `agent-core/src/sandbox.rs` — `SandboxExecutor` + `CaptureConfig` + `SandboxResult` |
| **环境检测** | `agent-core/src/sandbox/env.rs` — `SandboxEnvironment::detect()` + `recommended_preference()` |

**关键依赖：**
- `bwrap` (或直接调用 CLI) — bubblewrap 沙箱
- `libseccomp` — seccomp filter
- `nix` — Linux namespace 操作

---

## 6. 参考来源

| 来源 | 关键内容 | 借鉴内容 |
|------|----------|----------|
| Codex | `SandboxPreference` enum | Auto / Require / Forbid / BestEffort 偏好模型 |
| Codex | `SandboxBackend` trait | `is_available()`, `execute()`, `capabilities()` 抽象 |
| Codex | `bwrap.rs` | `system_bwrap_has_user_namespace_access()` 有界探测 |
| OpenHands | Docker container sandbox | 每任务一容器，不依赖 namespace 工具 |
| Firejail | seccomp + capabilities | 不需要 user namespace 的进程隔离方案 |

---

## Implementation Summary

**Code Locations:**
- `crates/aletheon-body/src/impl/sandbox/mod.rs` — SandboxExecutor, multi-backend dispatch
- `crates/aletheon-body/src/impl/sandbox/bubblewrap.rs` — BubblewrapBackend implementation

**Key Types/Traits Implemented:**
- `SandboxBackend` trait — `name()`, `isolation_level()`, `is_available()`, `capabilities()`, `execute()`
- `BubblewrapBackend` — full namespace isolation with bubblewrap
- `ProcessBackend` — seccomp + capabilities, no namespace
- `NoopBackend` — no isolation, trusted environment only
- `SandboxExecutor` — multi-backend with auto-selection based on SandboxPreference
- `SandboxEnvironment` — Docker/WSL2/Android detection, user namespace availability check

**Test Coverage:** Unit tests for SandboxEnvironment detection (Docker, WSL2, Android). Integration tests for BubblewrapBackend execute with timeout. ProcessBackend and NoopBackend tested in isolation.

**Not Yet Implemented:** libseccomp filter integration, Landlock LSM integration.

**Additional Implemented Components (filesystem policy layer):**
- `crates/agent-core/src/sandbox/policy.rs` — `FilesystemPolicy`, `WritableRoot`, `FsDefault` enum, `protected_metadata`, `unreadable_globs`
- `crates/agent-core/src/sandbox/bwrap_builder.rs` — `BwrapBuilder`: ordered bwrap arg construction from `FilesystemPolicy`, reprotection of `.git`/`.agents` dirs
- `crates/agent-core/src/sandbox/glob_scanner.rs` — `GlobScanner`: ripgrep-first + walkdir fallback for glob matching
- `crates/agent-core/src/sandbox/container.rs` — `ContainerBackend`: Docker/Podman container runtime support
