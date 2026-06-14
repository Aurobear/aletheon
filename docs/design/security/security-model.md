# 安全模型

> 分级权限、策略审计、循环防护、路径隔离——Agent 的安全底线。

**模块编号:** 05
**关联模块:** [工具系统与沙箱执行](../execution/tool-system.md) · [感知层](../perception/perception-layer.md) · [认知引擎](../core/cognitive-engine.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| PolicyEngine | ✅ Implemented | `security/policy.rs` | Hardcoded rules, not YAML-driven |
| LoopDetector | ✅ Implemented | `security/loop_detector.rs` | Stagnation + fail-streak detection |
| CircuitBreaker | ✅ Implemented | `security/circuit_breaker.rs` | Per-turn consecutive block detection |
| RiskClassifier | ✅ Implemented | `security/risk_classifier.rs` | 4-level risk classification |
| OutputGuardrail | ✅ Implemented | `security/output_guardrail.rs` | Output validation |
| Audit logging | ✅ Implemented | `security/audit.rs` | Structured audit trail |
| ToolRunnerWithGuard | ✅ Implemented | `security/runner.rs` | Guard-wrapped tool execution |
| Rollback engine | ✅ Implemented | `security/rollback/mod.rs` | 3-tier: AuditOnly, FileBackup, BtrfsRollback |
| WritableRoot | ⬜ Planned | — | Path isolation not started |

---

## 目录

- [1. 概述](#1-概述)
- [2. 当前设计](#2-当前设计)
  - [2.1 权限分级](#21-权限分级)
  - [2.2 策略引擎](#22-策略引擎)
  - [2.3 审计日志](#23-审计日志)
  - [2.4 回滚引擎](#24-回滚引擎)
- [3. 已识别缺陷](#3-已识别缺陷)
  - [3.1 P0: 工具调用循环检测/Guardrail](#31-p0-工具调用循环检测guardrail)
  - [3.2 P2: WritableRoot 只读子路径](#32-p2-writableroot-只读子路径)
  - [3.3 P0: 多 Agent 权限继承缺失](#33-p0-多-agent-权限继承缺失)
  - [3.4 P1: WritableRoot 子进程绕过风险](#34-p1-writableroot-子进程绕过风险)
  - [3.5 P1: 回滚引擎 btrfs 依赖](#35-p1-回滚引擎-btrfs-依赖)
  - [3.6 P1: LoopDetector 全局追踪不区分 Agent](#36-p1-loopdetector-全局追踪不区分-agent)
- [4. 改进设计](#4-改进设计)
  - [4.1 LoopDetector 概览](#41-loopdetector-概览)
  - [4.2 循环模式检测](#42-循环模式检测)
  - [4.3 完整 LoopDetector 实现](#43-完整-loopdetector-实现)
  - [4.4 WritableRoot 概览](#44-writableroot-概览)
  - [4.5 FileSystemSandboxPolicy 入口模型](#45-filesystemsandboxpolicy-入口模型)
  - [4.6 ProtectedMetadataNames 机制](#46-protectedmetadatanames-机制)
  - [4.7 WritableRoot 完整实现](#47-writableroot-完整实现)
  - [4.8 与 bubblewrap 集成](#48-与-bubblewrap-集成)
  - [4.9 与策略引擎集成](#49-与策略引擎集成)
  - [4.10 子 Agent 权限继承与降级策略](#410-子-agent-权限继承与降级策略)
  - [4.11 内核级路径访问强制](#411-内核级路径访问强制)
  - [4.12 分级回滚策略](#412-分级回滚策略)
  - [4.13 Per-Agent LoopDetector 状态隔离](#413-per-agent-loopdetector-状态隔离)
- [5. 实现要点](#5-实现要点)
- [6. 参考来源](#6-参考来源)

---

## 1. 概述

安全模型是 OS-Agent 的底线保障。它定义了 Agent 能做什么、不能做什么、做了之后如何审计和回滚。核心设计原则：

- **默认最小权限** — Agent 以最低权限运行，升级需显式授权
- **可审计** — 所有工具调用记录到 audit log，决策链可回溯
- **可回滚** — 关键操作前自动快照，支持单文件/服务/快照级回滚
- **防死循环** — 检测并阻断 Agent 的工具调用死循环，防止 token 浪费和资源滥用

---

## 2. 当前设计

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

## 3. 已识别缺陷

### 3.1 P0: 工具调用循环检测/Guardrail

**问题：** Agent 可能陷入工具调用死循环——反复调用同一失败工具、两个工具之间无限互相触发、或在 token 消耗不断增加的情况下没有任何实质进展。

**典型场景：**

| 场景 | 表现 | 后果 |
|------|------|------|
| 同工具重复失败 | `bash("make")` 连续失败 10 次，每次错误相同 | token 浪费 |
| 双工具互锁 | tool_a 调用 tool_b，tool_b 调用 tool_a，无限循环 | CPU 和 token 双重浪费 |
| 无进展循环 | 连续调用 20 个工具，但系统状态未发生任何变化 | 毫无产出 |

### 3.2 P2: WritableRoot 只读子路径

**问题：** 即使工作目录整体可写，其中的元数据目录（`.git/`、`.ssh/`）以及系统敏感路径不应被 Agent 直接修改。详见 [writable-root.md](writable-root.md)。

### 3.3 P0: 多 Agent 权限继承缺失

**问题：** 安全模型（PolicyEngine L0-L3）和编排引擎（DELEGATE_BLOCKED_TOOLS）各自定义权限控制，但两者之间缺乏集成，形成安全真空地带。

| 断裂 | 描述 | 风险 |
|------|------|------|
| 权限等级未连通 | PolicyEngine L0-L3 与编排引擎的工具屏蔽列表无对应关系 | 子 Agent 可执行 L3 级操作 |
| 硬编码工具屏蔽 | `DELEGATE_BLOCKED_TOOLS` 是静态列表，不从 PolicyEngine 动态派生 | 新增危险工具需手动同步 |
| 子 Agent 权限继承未定义 | 未规定子 Agent 是否继承父 Agent L 等级 | 权限升级攻击可行 |
| LoopDetector 作用域不明 | 不按 Agent ID 独立追踪 | 分布式循环可绕过检测 |

### 3.4 P1: WritableRoot 子进程绕过风险

**问题：** WritableRoot 的 Extension/Prefix 规则和 ProtectedMetadataNames 仅在运行时 `PathAccessGuard` 层强制执行，无法通过 bubblewrap 静态绑定实现内核级隔离。子进程可绕过 PathAccessGuard。

### 3.5 P1: 回滚引擎 btrfs 可移植性

**问题：** 回滚引擎的文件系统级快照依赖 btrfs snapshot，但 btrfs 在主流 Linux 中采用率极低（ext4 占桌面 Linux 市场份额 70% 以上）。

| 环境 | 默认文件系统 | btrfs 支持 |
|------|-------------|-----------|
| Ubuntu 20.04-24.04 | ext4 | 可选安装，非默认 |
| RHEL/CentOS 9 | XFS | 不支持 |
| WSL2 | ext4 (虚拟化) | 不支持快照 |
| Docker overlay2 | overlay2 | 不支持快照 |

### 3.6 P1: LoopDetector 全局追踪不区分 Agent

**问题：** LoopDetector 的检测状态使用全局状态，在多 Agent 场景下产生误判和漏检。

| 缺陷 | 全局追踪的问题 | 后果 |
|------|---------------|------|
| Same-Call Detection 不区分 Agent | 不同代理相同调用被误判为循环 | 合法调用被阻断 |
| Fail-Streak 全局累计 | 一个代理的失败影响其他代理 | 故障隔离失败 |
| CircuitBreaker 状态全局共享 | 子代理阻断累积导致父代理中断 | 正常工作被错误中断 |

---

## 4. 改进设计

### 4.1 LoopDetector 概览

`LoopDetector` 作为工具调用链的看门人，独立于策略引擎运行。它按 `turn_id` 维护滑动窗口，记录每个推理轮次内的工具调用历史，实时检测循环模式、输出异常和风险等级。检测范围包含五个子系统：

- **SameCall / FailStreak / Stagnation** — 三种调用模式检测
- **CircuitBreaker** — 连续阻断达到阈值时中断整个推理轮次
- **OutputGuardrail** — 工具执行后验证输出是否符合预期
- **RiskClassifier** — 按工具类型和参数将调用分类为不同风险等级
- **Metrics** — 每次判定都发射遥测指标

```
工具调用请求 ──▶ ┌──────────────────────────────────────────────────┐
                 │  LoopDetector (per-turn scoped)                  │
                 │  ┌──────────────┐                                │
                 │  │ RiskClassifier│── 动态阈值 ──┐                │
                 │  └──────────────┘               │                │
                 │                                  ▼                │
                 │  ┌─────────────┐  ┌────────────┐  ┌──────────┐ │
                 │  │ SameCall    │  │ FailStreak │  │ Stagnation│ │
                 │  │ Detector    │  │ Detector   │  │ Detector  │ │
                 │  └──────┬──────┘  └─────┬──────┘  └─────┬─────┘ │
                 │         │               │               │        │
                 │  ┌──────┴───────────────┴───────────────┴──────┐ │
                 │  │ Verdict: Allow / Warn / Block / Escalate     │ │
                 │  └────────────────────┬────────────────────────┘ │
                 │  ┌────────────────────▼────────────────────────┐ │
                 │  │ CircuitBreaker (per-turn)                   │ │
                 │  │ consecutive_blocks ≥ 3 → InterruptTurn      │ │
                 │  └────────────────────┬────────────────────────┘ │
                 │  ┌────────────────────────────────────────────┐ │
                 │  │ OutputGuardrail                           │ │
                 │  │ validate(output) → pass / fail+retry       │ │
                 │  └────────────────────┬────────────────────────┘ │
                 │  Allow / Warn / Block / Escalate / InterruptTurn │
                 │  + Metrics emission                              │
                 └──────────────────────────────────────────────────┘
```

### 4.2 循环模式检测

**核心模式：**

| 模式 | 检测条件 | 动作 |
|------|----------|------|
| **相同调用重复** | 同一工具名 + 参数哈希，连续出现 N 次 | `Block` |
| **连续失败** | 工具返回 `is_error: true`，连续 M 次 | `Escalate` |
| **无进展** | 最近 K 次调用后，token 消耗变化 < 阈值 且 无成功结果 | `Warn` |

**风险分级（动态阈值）：**

| 风险等级 | 工具示例 | same_call_threshold | fail_streak_threshold | 首次失败动作 |
|----------|----------|---------------------|-----------------------|-------------|
| **ReadOnly** (L0) | read_file, grep, ls | 5 | 7 | Allow |
| **FileModification** (L1) | write_file, bash("make") | 3 | 5 | Allow |
| **SystemChange** (L2) | systemctl, pacman, iptables | 2 | 3 | Warn |
| **Destructive** (L2+) | rm, mkfs, dd | 2 | 2 | Warn + 立即记录审计 |

**熔断器（CircuitBreaker）模式：**

| 触发条件 | 阈值 | 动作 |
|----------|------|------|
| 连续阻断次数（同一 turn） | 3 次 | `InterruptTurn` |
| 滑动窗口内阻断次数（窗口=50） | 10 次 | `InterruptTurn` |

**输出验证（OutputGuardrail）模式：**

| 验证规则 | 适用范围 | 失败动作 |
|----------|----------|----------|
| 非空输出检查 | 所有工具 | 注入错误上下文，允许重试（上限 2 次） |
| 退出码检查 | bash/shell 工具 | 非零退出码视为失败 |
| JSON schema 验证 | 结构化数据工具 | 输出不符合 schema 时重试 |

### 4.3 完整 LoopDetector 实现

> 完整实现见 [loop-detector.md](loop-detector.md)。以下为关键组件摘要。

**核心组件：**
- **RiskCategory** — 四级风险分类：ReadOnly / FileModification / SystemChange / Destructive
- **RiskClassifier** — 根据工具名和参数判定风险等级，支持用户可配置规则
- **LoopDetectorConfig** — 滑动窗口(50)、阈值、熔断器参数等配置
- **ToolCallRecord** — 调用记录（tool_name, args_hash, is_error, token_cost, turn_id）
- **LoopVerdict** — Allow / Warn / Block / Escalate / InterruptTurn
- **OutputGuardrail** — 输出验证（非空、退出码），失败注入上下文+重试(max 2)
- **LoopCircuitBreaker** — 连续 Block 达阈值(3)或滑动窗口累积(10/50)时 InterruptTurn
- **LoopDetectorMetrics** — 遥测指标

**核心接口：**
- `pre_check(tool_name, args, turn_id)` — 调用前模式匹配
- `post_check(tool_name, args, is_error, token_cost, turn_id)` — 调用后记录
- `record_and_check()` — 合一接口，Fail-closed
- `validate_output(tool_name, output)` — 输出验证

**ToolRunnerWithGuard** — 集成循环检测的工具运行器，执行流程：权限检查 → pre-check → 执行(含重试) → post-check → 审计

**RiskClassifier 内置默认规则（部分）：**

| 工具模式 | 风险等级 |
|----------|----------|
| `rm`, `mkfs*`, `dd`, `shutdown`, `reboot` | Destructive |
| `systemctl`, `pacman`, `iptables`, `mount`, `useradd` | SystemChange |
| `read_file`, `ls`, `cat`, `grep`, `find` | ReadOnly |
| 其他工具 | FileModification（默认） |

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

### 4.11 内核级路径访问强制

将 Extension/Prefix 规则的强制执行从用户态 PathAccessGuard 下沉到内核层：

| 层 | 机制 | 作用 |
|----|------|------|
| seccomp BPF | 拦截 `openat(2)` 等系统调用 | 消除子进程绕过 |
| Landlock LSM (5.13+) | `landlock_restrict_self()` 对所有子进程生效 | 内核级强制 |
| 沙箱构建时枚举 | 遍历匹配文件，逐个添加规则 | 弥补后缀匹配限制 |

### 4.12 分级回滚策略

将 `RollbackEngine` 定义为 trait，实现三个后端：

| 后端 | Tier | 可用条件 | 能力 |
|------|------|----------|------|
| BtrfsSnapshotEngine | 3 | btrfs 文件系统 | 原子子卷快照 |
| FileBackupEngine | 2 | 有写入权限和磁盘空间 | 文件备份 + systemd 状态记录 |
| AuditOnlyEngine | 1 | 始终可用 | 审计日志 + 手动指引 |

运行时自动选择最佳可用后端。

### 4.13 Per-Agent LoopDetector 状态隔离

将 LoopDetector 的全局状态拆分为 per-agent 状态：

```rust
struct MultiAgentLoopDetector {
    agent_detectors: HashMap<String, AgentLoopState>,
    aggregate_view: AggregateLoopState,
    global_config: LoopDetectorConfig,
}

struct AgentLoopState {
    agent_id: String,
    call_window: VecDeque<ToolCallRecord>,
    fail_streak: u32,
    circuit_breaker: CircuitBreakerState,
    threshold_override: Option<RiskThresholds>,
}
```

**Per-Agent 阈值配置** — 父代理可为每个子代理配置差异化阈值。**聚合报告** — 父代理可查看所有子代理的安全健康摘要。Fail-closed 保持：单个 agent 的 LoopDetector 异常只阻断该 agent。

---

## 5. 实现要点

| 要点 | 说明 |
|------|------|
| **P0 优先** | LoopDetector 是安全底线，必须在 Phase 1 就实现 |
| **Pre-check vs Post-check** | 调用前做模式匹配，调用后更新历史并验证输出 |
| **Turn-scoped 历史** | 每个推理轮次独立维护调用历史，turn 结束时清理 |
| **风险分级阈值** | ReadOnly(5/7) vs Destructive(2/2)，由 `RiskClassifier.classify()` 动态决定 |
| **熔断器** | 连续 Block 3 次或滑动窗口 10/50 时 InterruptTurn |
| **输出验证** | 失败时注入错误上下文并允许重试（默认 2 次） |
| **Fail-closed** | LoopDetector 自身出错时阻断调用 |
| **WritableRoot 优先级** | deny > write > read。显式写入规则可覆盖默认只读 |
| **protected_metadata_names** | `.git`、`.ssh` 等即使不存在也阻止创建 |
| **gitdir 指针处理** | `.git` 可能是文件（worktree/submodule），需解析 gitdir 路径 |
| **审计完整性** | 所有 Block/Escalate/InterruptTurn 都必须写入 audit log |

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| **Codex Guardian** | `GuardianRejectionCircuitBreaker` 熔断机制、fail-closed 设计 |
| **Codex Guardian Policy** | 风险分类法（Data Exfiltration / Credential Probing 等） |
| **Codex WritableRoot** | 三重写入检查、`is_path_writable()` 算法、gitdir 指针解析 |
| **Codex FileSystemSandboxPolicy** | 入口列表模型、deny > write > read 冲突优先级 |
| **Hermes tool_guardrails** | `ToolCallRecord` + 滑动窗口 + 三种检测模式 |
| **CrewAI Guardrail** | `GuardrailResult` + 重试循环 + 错误上下文注入 |

---

## Implementation Summary

**Code Locations:**
- `argos/crates/agent-core/src/security/policy.rs` — PolicyEngine with hardcoded rules
- `argos/crates/agent-core/src/security/loop_detector.rs` — LoopDetector with stagnation + fail-streak detection
- `argos/crates/agent-core/src/security/policy.rs` — PolicyRule, permission level checks

**Key Types/Traits Implemented:**
- `PolicyEngine` — rule-based permission checks (L0-L3), hardcoded rules
- `LoopDetector` — stagnation detection, fail-streak detection (turn-scoped)
- `AuditLogger` — structured audit trail with timestamps, tool info, results

**Test Coverage:** Unit tests for PolicyEngine rule matching, LoopDetector same-call and fail-streak detection. Integration tests verify end-to-end tool execution with guard checks.

**Not Yet Implemented:** WritableRoot path isolation, FileSystemSandboxPolicy entry model, PathAccessGuard, per-agent LoopDetector isolation, RollbackEngine (btrfs dependency), YAML-driven policy configuration, OutputGuardrail with retry.
