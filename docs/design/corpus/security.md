> Migrated from docs/design/security/security-model.md — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# 安全策略 (Security Policy)

> 分级权限、策略引擎、审计日志、回滚引擎、多 Agent 权限继承。
> 循环检测见 [self/loop-detector.md](../dasein/loop-detector.md)，路径隔离见 [self/writable-root.md](../dasein/writable-root.md)。

---

## 1. 概述

安全模型是 Aletheon 的底线保障。它定义了 Agent 能做什么、不能做什么、做了之后如何审计和回滚。核心设计原则：

- **默认最小权限** — Agent 以最低权限运行，升级需显式授权
- **可审计** — 所有工具调用记录到 audit log，决策链可回溯
- **可回滚** — 关键操作前自动快照，支持单文件/服务/快照级回滚
- **防死循环** — 检测并阻断 Agent 的工具调用死循环，防止 token 浪费和资源滥用

---

### 2.1 权限分级

四级权限模型，从自动执行到完全禁止：

| 级别 | 策略 | 操作示例 |
|------|------|----------|
| **L0** | 自动执行，无需通知 | 读取文件/目录、查看 /proc /sys、grep/find/rg、读取记忆、更新 Core Memory |
| **L1** | 通知后执行（做了告诉用户） | 安装/更新软件包、修改配置文件、管理 systemd 服务、网络配置变更、沙箱内代码执行 |
| **L2** | 需要确认（做之前先问） | 删除非临时文件、修改系统关键配置、sudo 命令、修改防火墙规则、访问密码/密钥、跨 Agent 委托 |
| **L3** | 禁止 | `rm -rf /`、修改内核模块、关闭安全服务、未验证的远程代码执行、修改 read_only Core Memory |

### 2.2 策略引擎

**PolicyEngine** — 策略引擎，包含 rules（规则列表）、audit_log、rollback_engine。
- 代码位置: `security/policy.rs`
- 当前实现使用硬编码规则，非 YAML 驱动

**PolicyRule** — 策略规则：pattern（匹配模式）、level（权限级别 L0-L3）、conditions（附加条件）、action（允许/拒绝/升级）

### 2.3 审计日志

**AuditLogger** — 结构化审计日志，每条记录包含时间戳、agent、tool、args、level、result、duration_ms、side_effects。
- 代码位置: `security/audit.rs`

### 2.4 回滚引擎

关键操作前自动快照：

| 快照类型 | 机制 | 恢复粒度 |
|----------|------|----------|
| 文件系统级 | btrfs snapshot | 整个子卷 |
| 单文件级 | 文件备份 | 单个文件 |
| 服务级 | systemd 状态记录 | 单个服务 |

---

### 3.3 P0: 多 Agent 权限继承缺失

**问题：** 安全模型（PolicyEngine L0-L3）和编排引擎（DELEGATE_BLOCKED_TOOLS）各自定义权限控制，但两者之间缺乏集成，形成安全真空地带。

| 断裂 | 描述 | 风险 |
|------|------|------|
| 权限等级未连通 | PolicyEngine L0-L3 与编排引擎的工具屏蔽列表无对应关系 | 子 Agent 可执行 L3 级操作 |
| 硬编码工具屏蔽 | `DELEGATE_BLOCKED_TOOLS` 是静态列表，不从 PolicyEngine 动态派生 | 新增危险工具需手动同步 |
| 子 Agent 权限继承未定义 | 未规定子 Agent 是否继承父 Agent L 等级 | 权限升级攻击可行 |
| LoopDetector 作用域不明 | 不按 Agent ID 独立追踪 | 分布式循环可绕过检测 |

### 3.5 P1: 回滚引擎 btrfs 可移植性

**问题：** 回滚引擎的文件系统级快照依赖 btrfs snapshot，但 btrfs 在主流 Linux 中采用率极低（ext4 占桌面 Linux 市场份额 70% 以上）。

| 环境 | 默认文件系统 | btrfs 支持 |
|------|-------------|-----------|
| Ubuntu 20.04-24.04 | ext4 | 可选安装，非默认 |
| RHEL/CentOS 9 | XFS | 不支持 |
| WSL2 | ext4 (虚拟化) | 不支持快照 |
| Docker overlay2 | overlay2 | 不支持快照 |

### 4.9 与策略引擎集成

```
工具调用请求
      ▼
┌──────────────────────┐
│ RiskClassifier        │  ← 判定风险等级，动态调整阈值
└──────┬───────────────┘
       ▼
┌──────────────────────┐
│ LoopDetector          │  ← pre-check: 模式检测
│   + CircuitBreaker    │
└──────┬───────────────┘
       ▼
┌──────────────────────┐
│ PolicyEngine          │  ← 检查权限级别（L0-L3）
└──────┬───────────────┘
       ▼
┌──────────────────────┐
│ PathAccessGuard       │  ← 检查路径只读
│   .check_write()      │
└──────┬───────────────┘
       ▼
┌──────────────────────┐
│ Sandbox               │  ← bubblewrap --bind / --ro-bind
│ Executor              │     + seccomp + cgroups
└──────┬───────────────┘
       ▼
┌──────────────────────┐
│ OutputGuardrail       │  ← post-check: 输出验证
│ (validate output)     │     失败 → 注入上下文 + 重试
└──────┬───────────────┘
       ▼
┌──────────────────────┐
│ LoopDetector          │  ← post-check: 记录调用结果
│ (record result)       │
└──────┬───────────────┘
       ▼
┌──────────────────────┐
│ AuditLogger           │  ← 记录完整调用链 + 遥测指标
└──────────────────────┘
```

### 4.10 子 Agent 权限继承与降级策略

**默认降级规则：**

| 父等级 | 子等级 | 说明 |
|--------|--------|------|
| L3 | L2 | 禁止危险操作 |
| L2 | L1 | 禁止系统目录写入 |
| L1 | L0 | 只读 |
| L0 | L0 | 无降级空间 |

子 Agent 可请求更低等级，但不允许升级。移除 `DELEGATE_BLOCKED_TOOLS` 硬编码列表，改为在子 Agent 派生时调用 `PolicyEngine.derive_child_permission()`。

### 4.12 分级回滚策略

将 `RollbackEngine` 定义为 trait，实现三个后端：

| 后端 | Tier | 可用条件 | 能力 |
|------|------|----------|------|
| BtrfsSnapshotEngine | 3 | btrfs 文件系统 | 原子子卷快照 |
| FileBackupEngine | 2 | 有写入权限和磁盘空间 | 文件备份 + systemd 状态记录 |
| AuditOnlyEngine | 1 | 始终可用 | 审计日志 + 手动指引 |

运行时自动选择最佳可用后端。


---

## Implementation Summary

**Code Locations:**
- `crates/corpus/src/security/mod.rs` — PolicyEngine with hardcoded rules
- `crates/corpus/src/security/audit.rs` — AuditLogger
- `crates/dasein/src/impl/security/rollback/mod.rs` — RollbackEngine (3-tier: AuditOnly, FileBackup, BtrfsRollback)
- `crates/corpus/src/security/risk_classifier.rs` — RiskClassifier

**Key Types/Traits Implemented:**
- `PolicyEngine` — rule-based permission checks (L0-L3), hardcoded rules
- `AuditLogger` — structured audit trail with timestamps, tool info, results
- `RollbackEngine` — 3-tier rollback: AuditOnly, FileBackup, BtrfsRollback
- `RiskClassifier` — 4-level risk classification

**Not Yet Implemented:** YAML-driven policy configuration, per-agent permission inheritance, FileSystemSandboxPolicy entry model.
