# Aletheon 项目清理设计

**日期:** 2026-06-14
**状态:** 待审批
**范围:** 修复 workspace 不匹配 + 清理 ~310 个 argos 残留引用 + 建立 CI + 更新文档

---

## 1. 问题清单

### 1.1 🔴 CRITICAL: Workspace 构建失败

`Cargo.toml` workspace members 引用了 `aletheon-self-field` 和 `aletheon-brain-core`，但实际目录是 `aletheon-self` 和 `aletheon-brain`。Crate name 与目录名不一致导致 `cargo build` 失败。

**决策：** 将 crate name 改短匹配目录名（用户选择）。

### 1.2 🟡 ~310 个 argos 残留引用

| 类别 | 文件数 | 行数 | 风险 |
|---|---|---|---|
| 代码标识符 | ~20 | ~160 | 编译安全 |
| 运行时路径 | ~10 | ~20 | 破坏已部署环境 |
| 协议/设备标识 | ~6 | ~10 | 运行时行为变化 |
| 文档注释 | ~35 | ~120 | 无风险 |

### 1.3 🟠 无 CI 配置

`.github/workflows/` 不存在。

### 1.4 🟢 文档过时

`README.md`、`docs/design/` 中多处引用旧的 `argos/crates/` 路径。

---

## 2. Phase 1: 修复 Workspace 不匹配

### 变更

| 文件 | 变更 |
|---|---|
| `crates/aletheon-brain/Cargo.toml` | `name = "aletheon-brain-core"` → `name = "aletheon-brain"` |
| `crates/aletheon-self/Cargo.toml` | `name = "aletheon-self-field"` → `name = "aletheon-self"` |
| `Cargo.toml` (workspace) | members: `aletheon-self-field` → `aletheon-self`, `aletheon-brain-core` → `aletheon-brain` |
| `crates/aletheon-runtime/Cargo.toml` | deps: `aletheon-brain-core` → `aletheon-brain`, `aletheon-self-field` → `aletheon-self` |
| `crates/aletheon-meta/Cargo.toml` | deps: 同上 |
| 所有 `.rs` 文件 | `use aletheon_brain_core::*` → `use aletheon_brain::*`, `use aletheon_self_field::*` → `use aletheon_self::*` |

---

## 3. Phase 2: 重命名运行时路径

### 3.1 路径映射表

| 旧路径 | 新路径 | 涉及文件 |
|---|---|---|
| `~/.argos/` | `~/.aletheon/` | daemon/mod.rs, memory/vector_store.rs, ui/skill.rs, hook/config.rs, rollback/mod.rs |
| `~/.argos/config.toml` | `~/.aletheon/config.toml` | daemon/mod.rs |
| `~/.argos/.env` | `~/.aletheon/.env` | daemon/mod.rs |
| `~/.argos/hooks/` | `~/.aletheon/hooks/` | hook/config.rs |
| `~/.argos/skills/` | `~/.aletheon/skills/` | ui/skill.rs |
| `~/.config/argos/` | `~/.config/aletheon/` | mcp/auth.rs |
| `~/.local/share/argos` | `~/.local/share/aletheon` | daemon/mod.rs |
| `/var/run/argos` | `/var/run/aletheon` | discovery.rs (const) |
| `/var/lib/argos/snapshots` | `/var/lib/aletheon/snapshots` | rollback/mod.rs |
| `/etc/argos/hooks` | `/etc/aletheon/hooks` | hook/config.rs |
| `/sys/fs/cgroup/argos-{id}` | `/sys/fs/cgroup/aletheon-{id}` | sandbox_driver/mod.rs |
| `/var/run/argos/*.sock` | `/var/run/aletheon/*.sock` | awareness/mod.rs, discovery.rs |

### 3.2 集中常量

建议在 `aletheon-abi` 中定义路径常量，避免分散硬编码：

```rust
// crates/aletheon-abi/src/paths.rs
pub const CONFIG_DIR: &str = ".aletheon";
pub const SOCKET_DIR: &str = "/var/run/aletheon";
pub const SNAPSHOT_DIR: &str = "/var/lib/aletheon/snapshots";
pub const HOOKS_SYSTEM_DIR: &str = "/etc/aletheon/hooks";
pub const CGROUP_PREFIX: &str = "aletheon";
pub const MCP_TOKENS_DIR: &str = ".config/aletheon";
pub const DATA_DIR: &str = ".local/share/aletheon";
```

---

## 4. Phase 3: 重命名代码标识符

### 4.1 Rust 类型/函数重命名

| 旧名 | 新名 | 文件 |
|---|---|---|
| `ArgosBodyRuntime` | `AletheonBodyRuntime` | core/mod.rs, lib.rs |
| `ArgosPermissionLevel` | `ToolPermissionLevel` | core/conversions.rs |
| `argos_to_abi_permission()` | `tool_to_abi_permission()` | core/conversions.rs |

### 4.2 协议标识符

| 旧值 | 新值 | 文件 |
|---|---|---|
| MCP client `"argos"` | `"aletheon"` | mcp/transport.rs, mcp/client.rs |
| uinput `b"argos-virtual-input"` | `b"aletheon-virtual-input"` | driver/input/uinput.rs |
| X11 `ARGOS_CLIP` | `ALETHEON_CLIP` | driver/display/clipboard_x11.rs |
| X11 `ARGOS_CLIPBOARD_DATA` | `ALETHEON_CLIPBOARD_DATA` | driver/display/clipboard_x11.rs |
| Subsystem name `"argos-body"` | `"aletheon-body"` | core/mod.rs |

### 4.3 测试路径

| 旧值 | 新值 | 文件 |
|---|---|---|
| `"argos_test_experience"` | `"aletheon_test_experience"` | acix/experience.rs |
| `"/tmp/argos-test-*"` | `"/tmp/aletheon-test-*"` | discovery.rs, rollback/mod.rs |
| `"argos-x11-clipboard-test-42"` | `"aletheon-x11-clipboard-test-42"` | clipboard_x11.rs |

---

## 5. Phase 4: 清理文档注释

### 5.1 Rust doc comments

将所有 `"Merged from argos-*"` 注释更新为 `"Migrated from argos-*"` 或移除 origin 注释（代码已经稳定，不需要再标注来源）。

### 5.2 设计文档更新

| 文件 | 变更 |
|---|---|
| `README.md` | 更新项目名和描述 |
| `docs/design/README.md` | 更新目录树 |
| `docs/design/architecture-overview.md` | 更新路径和目录结构 |
| `docs/design/execution/ipc.md` | 更新旧的 `argos/crates/` 路径 |
| `docs/design/execution/tool-system.md` | 同上 |
| `docs/design/execution/sandbox.md` | 同上 |
| `docs/design/security/security-model.md` | 同上 |
| `docs/design/security/writable-root.md` | 同上 |
| `docs/design/platform/agent-awareness.md` | 更新 socket 路径 |
| `docs/design/testing/ci-pipeline.md` | 更新容器镜像名 |
| `docs/design/core/session-lifecycle.md` | 更新原始设计文档引用 |
| `docs/design/orchestration/hybrid-inference.md` | 同上 |
| `docs/design/perception/perception-layer.md` | 同上 |
| `docs/plans/2026-06-14-aletheon-v3-redesign.md` | 保留（历史文档） |
| `docs/plans/2026-06-14-argos-aletheon-migration-design.md` | 保留（历史文档） |
| `docs/plans/2026-06-14-argos-aletheon-migration-plan.md` | 保留（历史文档） |

---

## 6. Phase 5: 建立 CI

创建 `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [dev]
  pull_request:
    branches: [dev]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo check --workspace

  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo clippy --workspace -- -D warnings
```

---

## 7. 执行顺序

```
Phase 1 (workspace fix)  ← 必须先做，否则无法编译
    ↓
Phase 2 (路径重命名)
    ↓
Phase 3 (代码标识符重命名)
    ↓
Phase 4 (文档更新)
    ↓
Phase 5 (CI 配置)
    ↓
验证: cargo check --workspace && cargo test --workspace
```

---

## 8. 风险评估

| 风险 | 等级 | 缓解措施 |
|---|---|---|
| 路径重命名破坏已部署环境 | 低 | 项目尚未发布，无已部署用户 |
| MCP 协议标识符变化 | 低 | MCP 服务器不依赖 client name |
| X11 atom 名变化 | 低 | atom 仅在进程内使用 |
| cgroup 名变化 | 低 | cgroup 仅用于沙箱隔离 |
| 引入编译错误 | 中 | 每个 Phase 后运行 `cargo check` |
