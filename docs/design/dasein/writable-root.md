> Merged from docs/design/security/writable-root.md + docs/design/security/security-model.md §4.4-4.8 — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# WritableRoot 路径隔离

> 沙箱路径权限的精细化层——即使父目录可写，受保护子路径仍被拦截。

**模块编号:** 05-子模块
**父模块:** [安全模型](../corpus/security.md)
**关联模块:** [循环检测器](loop-detector.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| WritableRoot | ⬜ Planned | — | Path isolation not implemented |
| FileSystemSandboxPolicy | ⬜ Planned | — | Entry list model not implemented |
| PathAccessGuard | ⬜ Planned | — | Runtime path enforcement not implemented |
| ProtectedMetadataNames | ⬜ Planned | — | Metadata creation blocking not implemented |

---

## 目录

- [1. 已识别缺陷](#1-已识别缺陷)
  - [1.1 P2: WritableRoot 只读子路径](#11-p2-writableroot-只读子路径)
  - [1.2 P1: WritableRoot 子进程绕过风险](#12-p1-writableroot-子进程绕过风险)
- [2. 改进设计](#2-改进设计)
  - [2.1 WritableRoot 概览](#21-writableroot-概览)
  - [2.2 FileSystemSandboxPolicy 入口模型](#22-filesystemsandboxpolicy-入口模型)
  - [2.3 ProtectedMetadataNames 机制](#23-protectedmetadatanames-机制)
  - [2.4 WritableRoot 核心算法](#24-writableroot-核心算法)
  - [2.5 PathAccessGuard](#25-pathaccessguard)
  - [2.6 与 bubblewrap 集成](#26-与-bubblewrap-集成)
  - [2.7 与策略引擎集成](#27-与-策略引擎集成)
- [3. 实现要点](#3-实现要点)

---

## 1. 已识别缺陷

### 1.1 P2: WritableRoot 只读子路径

**问题：** 即使工作目录（如 `/home/user/project/`）整体可写，其中的元数据目录（`.git/`、`.ssh/`）以及系统敏感路径（`/etc/agent/`）不应被 Agent 工具直接修改。当前沙箱配置只有粗粒度的 `--bind`（可写）和 `--ro-bind`（只读），缺少路径级的只读子路径排除。

**典型风险：**

| 路径 | 风险 |
|------|------|
| `.git/` | Agent 误操作可能损坏仓库元数据 |
| `.ssh/` | 私钥泄露或 authorized_keys 被篡改 |
| `/etc/agent/` | 策略文件被 Agent 自行修改，绕过安全约束 |
| `.env` | 环境变量泄露 |
| `*.pem`, `*.key` | 证书/密钥文件被覆盖 |

**Codex 对照发现的额外缺陷：**

| 编号 | 缺陷 | Codex 对应机制 | 影响 |
|------|------|----------------|------|
| G1 | 无 FileSystemSandboxPolicy 抽象 | `FileSystemSandboxEntry` + deny > write > read 优先级 | 无法组合灵活的路径策略 |
| G2 | 无 `protected_metadata_names` 机制 | 阻止创建不存在的元数据目录 | `.git/` 不存在时可先创建再写入 |
| G3 | 无显式写入规则覆盖 | `has_explicit_write_entry_for_metadata_path()` | 无法覆盖默认只读 |
| G4 | 符号链接解析不完整 | `canonicalize_preserving_symlinks()` | 悬挂符号链接上失败 |
| G5 | 不处理 git worktree/submodule 的 gitdir 指针 | `resolve_gitdir_from_file()` | `.git` 是文件时不被识别 |
| G6 | Extension/Prefix 规则不在沙箱层面强制 | 双层强制：bwrap + runtime PathAccessGuard | 子进程绕过时保护失效 |
| G7 | 无正式的 `PROTECTED_METADATA_PATH_NAMES` 常量 | 全局常量 + helper | 保护名称列表分散 |
| G8 | 无 `forbidden_agent_metadata_write()` 预执行检查 | 执行前统一检查点 | 缺少权威写入阻断入口 |

### 1.2 P1: WritableRoot 子进程绕过风险

**问题：** WritableRoot 的 Extension/Prefix 规则（`*.pem`、`.env*`）和 ProtectedMetadataNames 仅在运行时 `PathAccessGuard` 层强制执行，无法通过 bubblewrap 静态绑定实现内核级隔离。

| 缺陷 | 描述 | 绕过方式 |
|------|------|----------|
| bubblewrap 无法表达 Extension/Prefix | `--ro-bind` 仅支持精确路径或子树绑定 | `*.pem` 保护完全依赖用户态 |
| 子进程绕过 PathAccessGuard | 只检查顶层工具调用路径 | `bash("python3 -c 'open(...)'"` 绕过 |
| 符号链接逃逸 | 子进程可跟随符号链接写入外部路径 | `ln -s /etc/passwd link && write link` |
| ProtectedMetadataNames 仅用户态 | 子进程可直接 `mkdir .git` | 创建 `.git/hooks/pre-push` 恶意脚本 |

---

## 2. 改进设计

### 2.1 WritableRoot 概览

`WritableRoot` 是沙箱路径权限的精细化层。它在 bubblewrap 的粗粒度 bind 之上，提供路径级的只读排除。即使父目录通过 `--bind` 设为可写，匹配只读规则的子路径仍会被 `--ro-bind` 覆盖。

```
用户指定 WritableRoot: /home/user/project/

自动生成的沙箱绑定:
  --bind     /home/user/project/          (可写)
  --ro-bind  /home/user/project/.git/     (只读 - 元数据，已存在)
  --ro-bind  /home/user/project/.git      (只读 - gitdir 指针文件)
  --ro-bind  /home/user/project/.ssh/     (只读 - 密钥)
  --ro-bind  /home/user/project/.env      (只读 - 环境变量)
  --ro-bind  /home/user/project/*.pem     (只读 - 证书，运行时拦截)

运行时拦截（PathAccessGuard）:
  blocked: .git/ 创建请求   ← protected_metadata_names 阻止不存在的元数据目录创建
  blocked: .agents/ 创建请求
  allowed: src/lib.rs 写入   ← 正常可写路径
```

三层保护机制：

| 层 | 机制 | 覆盖范围 | 何时生效 |
|----|------|----------|----------|
| **L1: 策略层** | `FileSystemSandboxPolicy` 入口列表 | deny > write > read 优先级 | 策略加载时 |
| **L2: 沙箱层** | bubblewrap `--ro-bind` | Directory / Exact 的已存在路径 | 沙箱构建时 |
| **L3: 运行时层** | `PathAccessGuard` + `protected_metadata_names` | Extension / Prefix / 元数据创建 | 工具执行前 |

### 2.2 FileSystemSandboxPolicy 入口模型

引入入口列表模型替代扁平的 `readonly_rules: Vec<PathPattern>`。入口模型支持冲突优先级解析和显式写入覆盖。

**访问模式：**

| 模式 | 优先级 | 说明 |
|------|--------|------|
| Deny | 3 (最高) | 禁止访问 |
| Write | 2 | 允许读写 |
| Read | 1 | 允许读取 |

冲突时取高优先级：deny > write > read。

**路径匹配模式：**

| 模式 | 示例 | 说明 |
|------|------|------|
| Exact | `.git` | 精确路径匹配 |
| Directory | `.ssh/` | 目录及其所有子路径 |
| Extension | `*.pem` | 文件扩展名匹配 |
| Prefix | `.env` | 文件名前缀匹配 |
| TopLevelComponent | `.git` | root 下第一个相对组件 |

**默认只读保护入口（自动生成）：**

- 版本控制：`.git/`, `.svn/`, `.hg/`
- SSH 和密钥：`.ssh/`
- Agent 元数据：`.agents/`, `.codex/`
- 敏感文件扩展名：`*.pem`, `*.key`, `*.p12`, `*.pfx`
- 环境和配置：`.env*`, `.secret*`
- 包管理：`node_modules/`, `.venv/`, `__pycache__/`

**策略操作：**
- `add_readonly_rule(pattern)` — 添加自定义只读规则
- `add_write_exception(pattern)` — 添加显式写入规则（覆盖默认只读保护）
- `add_deny_rule(pattern)` — 添加显式拒绝规则（最高优先级）
- `query_access_mode(path)` — 查询路径的最终访问模式
- `can_write_path(path)` — 检查路径是否可写

### 2.3 ProtectedMetadataNames 机制

与 `read_only_subpaths` 不同，`protected_metadata_names` 阻止的是**创建**操作——即使 `.git/` 目录尚不存在，Agent 也不能在 WritableRoot 下创建它。

```
只读子路径 vs 受保护元数据名:

  read_only_subpaths:     .git/ 已存在 → --ro-bind 保护
  protected_metadata:     .git/ 不存在 → 阻止 mkdir 创建

  两者互补：
    已存在的元数据目录 → 被 read_only_subpaths 覆盖
    不存在的元数据目录 → 被 protected_metadata_names 覆盖
    用户显式授权      → has_explicit_write_entry_for_metadata_path() 覆盖两者
```

受保护元数据名常量：`.git`, `.ssh`, `.codex`, `.agents`

### 2.4 WritableRoot 核心算法

**WritableRoot 结构字段：**
- `root: PathBuf` — WritableRoot 根路径
- `read_only_subpaths: Vec<PathBuf>` — 只读子路径（已存在，用于 `--ro-bind`）
- `protected_metadata_names: Vec<String>` — 受保护元数据目录名（阻止不存在路径的创建）
- `policy: FileSystemSandboxPolicy` — 文件系统沙箱策略
- `system_readonly: Vec<PathBuf>` — 系统级只读路径（`/etc/agent`, `/etc/ssh`, `/etc/ssl/private`, `/var/log/agent`）

**动态生成默认只读子路径（`generate_default_read_only_subpaths`）：**
1. 普通 `.git/` 目录
2. gitdir 指针文件（worktree/submodule 的 `.git` 文件，格式: `"gitdir: /path"`）
3. `.ssh/` — 始终保护（如果存在）
4. `.agents/` — 始终保护（如果存在）
5. `.codex/` — 如果已存在则加入只读子路径

**三重写入权限检查（`is_path_writable`）：**
1. 系统级只读路径检查 → 任何系统路径下返回 false
2. 路径必须在 root 下
3. 路径不能在任何 read_only_subpath 下
4. 路径不能包含 protected_metadata_name（除非有显式写入规则）
5. 通过 FileSystemSandboxPolicy 入口列表做最终判定

### 2.5 PathAccessGuard

在工具执行前检查文件路径是否允许写入。

**核心方法：**
- `canonicalize_preserving_symlinks(path)` — 解析中间路径符号链接但保留最终组件，对悬挂符号链接优雅降级到原始路径
- `check_write(path)` — 三重写入权限检查，返回 `Ok(canonical_path)` 或 `Err(reason)`，reason 包含具体拒绝原因和建议操作
- `forbidden_agent_metadata_write(path)` — 预执行检查，在沙箱执行前统一阻断对受保护元数据的写入，对应 Codex `forbidden_agent_metadata_write()`
- `check_write_batch(paths)` — 批量检查

### 2.6 与 bubblewrap 集成

| 路径规则类型 | bwrap --ro-bind | PathAccessGuard | 备注 |
|-------------|----------------|-----------------|------|
| Directory (已存在) | Y | Y | 双层强制 |
| Directory (不存在) | N | Y (protected) | protected_metadata_names 阻止创建 |
| Exact (已存在) | Y | Y | 双层强制 |
| Extension (*.pem) | N | Y | 无法静态绑定，运行时拦截 |
| Prefix (.env*) | N | Y | 无法静态绑定，运行时拦截 |
| TopLevelComponent | N | Y | 运行时拦截，阻止创建 |
| gitdir 指针 | Y (文件 + gitdir) | Y | 解析后双重绑定 |

**bubblewrap 参数生成（`to_bwrap_args`）：**
1. 根目录可写绑定（`--bind root root`）
2. 只读子路径覆盖（已存在的路径，`--ro-bind subpath subpath`）
3. 系统级只读路径（`--ro-bind sys_path sys_path`）

**配置示例（`/etc/agent/agent.toml`）：**

```toml
[security.writable_root]
protected_metadata_names = [".git", ".ssh", ".codex", ".agents"]
extra_readonly = [
    "secrets/",
    "deploy/k8s/*.secret.yaml",
]
write_exceptions = [
    # ".codex/prompts/",  # 示例：允许 Agent 写入
]
deny = [
    "*.pem",
    "*.key",
]
```

### 2.7 与策略引擎集成

WritableRoot 作为 PolicyEngine 的子组件，在工具调用链中介入路径权限检查。完整调用链见 [security-model.md](../corpus/security.md) §4.9。

集成点：PolicyEngine 在 `check_and_execute()` 的步骤 3（路径检查）中调用 `PathAccessGuard.check_write()` 和 `forbidden_agent_metadata_write()`。

---

## 3. 实现要点

| 要点 | 说明 |
|------|------|
| **WritableRoot 优先级** | deny > write > read（入口列表冲突优先级）。系统级只读 > 工作目录只读规则 > 默认可写 |
| **符号链接** | `canonicalize_preserving_symlinks()` 解析中间路径但保留最终组件，悬挂符号链接优雅降级 |
| **Extension/Prefix 规则** | 无法在 bwrap 层面静态绑定，需运行时 `PathAccessGuard` 动态拦截 |
| **protected_metadata_names** | `.git`、`.ssh` 等即使不存在也阻止创建。显式写入规则可覆盖 |
| **gitdir 指针处理** | `.git` 可能是文件（worktree/submodule），解析 `"gitdir: ..."` 内容后双重绑定 |
| **forbidden_agent_metadata_write** | 预执行检查点，比 `check_write()` 更早，提供沙箱启动前的权威阻断入口 |
| **审计完整性** | 所有路径阻断都必须写入 audit log |
| **迁移策略** | Phase 1: 核心 WritableRoot 基础。Phase 2: FileSystemSandboxPolicy 入口模型 + gitdir 指针。Phase 3: Extension/Prefix 内核级强制 |

---

*源文档: [安全模型](../corpus/security.md) §3.2, §3.4, §4.4-4.9*

---

## Implementation Summary

**Code Locations:**
- `crates/corpus/src/security/sandbox/mod.rs` — WritableRoot (planned, not yet implemented)

**Key Types/Traits to Implement:**
- `WritableRoot` — root path, read_only_subpaths, protected_metadata_names, system_readonly
- `FileSystemSandboxPolicy` — entry list model with AccessMode (Read/Write/Deny), PathPattern (Exact/Directory/Extension/Prefix/TopLevelComponent)
- `PathAccessGuard` — canonicalize_preserving_symlinks(), check_write(), forbidden_agent_metadata_write()
- `AccessMode` — priority: deny(3) > write(2) > read(1)

**Test Coverage:** Not yet implemented. Tests should cover: three-level write check algorithm, protected metadata name blocking, gitdir pointer resolution, symlink safety, explicit write exception override.

**Design References:** Codex `WritableRoot` (`protocol.rs:909-956`), Codex `FileSystemSandboxPolicy` (`permissions.rs`), Codex `canonicalize_preserving_symlinks()`.


---

## Appendix: Additional Design Details (from security-model.md)

### 4.4 WritableRoot 概览

`WritableRoot` 是沙箱路径权限的精细化层。它在 bubblewrap 的粗粒度 bind 之上，提供路径级的只读排除。即使父目录通过 `--bind` 设为可写，匹配只读规则的子路径仍会被 `--ro-bind` 覆盖。

```
用户指定 WritableRoot: /home/user/project/

自动生成的沙箱绑定:
  --bind     /home/user/project/          (可写)
  --ro-bind  /home/user/project/.git/     (只读 - 元数据，已存在)
  --ro-bind  /home/user/project/.ssh/     (只读 - 密钥)
  --ro-bind  /home/user/project/.env      (只读 - 环境变量)

运行时拦截（PathAccessGuard）:
  blocked: .git/ 创建请求   ← protected_metadata_names 阻止不存在的元数据目录创建
  allowed: src/lib.rs 写入   ← 正常可写路径
```

三层保护机制：

| 层 | 机制 | 覆盖范围 | 何时生效 |
|----|------|----------|----------|
| **L1: 策略层** | `FileSystemSandboxPolicy` 入口列表 | deny > write > read 优先级 | 策略加载时 |
| **L2: 沙箱层** | bubblewrap `--ro-bind` | Directory / Exact 的已存在路径 | 沙箱构建时 |
| **L3: 运行时层** | `PathAccessGuard` + `protected_metadata_names` | Extension / Prefix / 元数据创建 | 工具执行前 |

### 4.5 FileSystemSandboxPolicy 入口模型

借鉴 Codex 的 `FileSystemSandboxPolicy`，引入入口列表模型替代扁平的 `readonly_rules`。

**访问模式与优先级：** `deny (3) > write (2) > read (1)`，冲突时取高优先级。

**路径匹配模式：**

| 模式 | 示例 | 说明 |
|------|------|------|
| Exact | `.git` | 精确路径 |
| Directory | `.ssh/` | 目录及其所有子路径 |
| Extension | `*.pem` | 扩展名匹配 |
| Prefix | `.env` | 前缀匹配 |
| TopLevelComponent | `.git` | root 下第一个相对组件匹配 |

**默认只读保护入口：** `.git/`, `.svn/`, `.hg/`, `.ssh/`, `.agents/`, `.codex/`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `.env*`, `.secret*`, `node_modules/`, `.venv/`, `__pycache__/`

### 4.6 ProtectedMetadataNames 机制

与 `read_only_subpaths` 不同，`protected_metadata_names` 阻止的是**创建**操作——即使 `.git/` 目录尚不存在，Agent 也不能在 WritableRoot 下创建它。

```
read_only_subpaths:     .git/ 已存在 → --ro-bind 保护
protected_metadata:     .git/ 不存在 → 阻止 mkdir 创建

两者互补：
  已存在的元数据目录 → 被 read_only_subpaths 覆盖
  不存在的元数据目录 → 被 protected_metadata_names 覆盖
  用户显式授权      → has_explicit_write_entry_for_metadata_path() 覆盖两者
```

受保护元数据名常量：`.git`, `.ssh`, `.codex`, `.agents`

### 4.7 WritableRoot 完整实现

**WritableRoot 核心结构：** root, read_only_subpaths, protected_metadata_names, policy, system_readonly

**PathAccessGuard** — 在工具执行前检查文件路径是否允许写入：
- `canonicalize_preserving_symlinks()` — 解析中间路径符号链接但保留最终组件
- `check_write()` — 三重写入权限检查（root/subpath/protected_metadata）
- `forbidden_agent_metadata_write()` — 预执行检查，阻止创建受保护元数据目录
- `check_write_batch()` — 批量检查

**三重写入权限检查算法（对应 Codex `WritableRoot::is_path_writable()`）：**
1. 系统级只读路径检查
2. 路径必须在 root 下
3. 路径不能在 read_only_subpaths 下
4. 路径不能包含 protected_metadata_name（除非有显式写入规则）
5. 通过 FileSystemSandboxPolicy 入口列表做最终判定

### 4.8 与 bubblewrap 集成

| 路径规则类型 | bwrap --ro-bind | PathAccessGuard | 备注 |
|-------------|----------------|-----------------|------|
| Directory (已存在) | Y | Y | 双层强制 |
| Directory (不存在) | N | Y (protected) | protected_metadata_names 阻止创建 |
| Extension (*.pem) | N | Y | 无法静态绑定，运行时拦截 |
| Prefix (.env*) | N | Y | 无法静态绑定，运行时拦截 |
| gitdir 指针 | Y (文件 + gitdir) | Y | 解析 gitdir 指针后双重绑定 |
