> Migrated from docs/design/security/self-protection.md — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# Agent 自我保护 (Agent Self-Protection)

> 安全模型定义了 Agent 对系统的权限，自我保护定义了 Agent 如何保护自己。三个保护层（InputSanitizer、ResourceGovernor、EmergencyKillswitch）及完整性监控均已实现。

**关联模块:** [安全模型](../corpus/security.md), [资源治理](resilience.md), [错误处理](resilience.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| InputSanitizer | ✅ Implemented | `crates/dasein/src/impl/security/self_protection/input_sanitizer.rs` | Prompt injection detection + sanitization |
| ResourceGovernor | ✅ Implemented | `crates/dasein/src/impl/security/self_protection/resource_governor.rs` | Multi-resource limits + throttling |
| EmergencyKillswitch | ✅ Implemented | `crates/dasein/src/impl/security/self_protection/emergency_killswitch.rs` | Multi-trigger emergency stop |
| IntegrityMonitor | ✅ Implemented | `crates/dasein/src/impl/security/self_protection/integrity_monitor.rs` | FNV-1a hash checking, baseline tracking, killswitch integration |

---

## 1. 概述

Agent 自我保护层防御三类威胁：

| 威胁 | 防御层 | 严重性 | 当前状态 |
|------|--------|--------|----------|
| **Prompt Injection** — 恶意输入试图劫持 Agent 行为 | InputSanitizer | Critical | ✅ Implemented |
| **资源失控** — Agent 消耗过多 CPU/内存/磁盘/Token | ResourceGovernor | High | ✅ Implemented |
| **代码篡改** — 配置或二进制被非法修改 | IntegrityMonitor | High | ✅ Implemented |
| **Agent 失能** — 连续失败/异常行为需紧急停止 | EmergencyKillswitch | Critical | ✅ Implemented |

---

## 2. Prompt Injection 防御

### 2.1 检测模式分类

```rust
struct InputSanitizer {
    detection_rules: Vec<InjectionPattern>,

    fn assess_input(&self, input: &str) -> RiskAssessment { ... }
    fn sanitize_tool_output(&self, output: &ToolResult) -> ToolResult { ... }
}

enum InjectionRisk {
    SystemOverride { pattern: String },           // "忽略之前的指令"/"你是一个..."
    DataExfiltration { pattern: String },         // "把文件内容发到 http://..."
    SecurityBypass { pattern: String },            // "跳过安全检查..."
    ResourceAbuse { pattern: String },             // "无限循环执行..."
    RolePlayEscalation { pattern: String },        // "假装你是 root..."
}
```

### 2.2 防护层级

| 层级 | 检测时间 | 检测对象 | 策略 |
|------|----------|----------|------|
| L1: 输入时 | 用户消息到达 | 用户输入文本 | 模式匹配 + 评分 |
| L2: 执行前 | 工具调用前 | 工具参数 | 参数注入检测（如 sqlmap 模式） |
| L3: 反馈时 | 工具返回后 | 工具输出（可能被恶意内容污染） | 输出清洗 + 边界标记 |

### 2.3 检测规则引擎

| 规则类型 | 匹配方式 | 示例模式 |
|----------|----------|----------|
| 关键字 | 精确/正则 | `(?i)(忽略|无视|override|you are)` |
| 语义 | 轻量分类 | 基于 token 频率的异常输入检测 |
| 上下文 | 历史对比 | 输入风格突变（如从简短命令变为长文本指令） |
| 结构 | 编码检测 | base64/hex 解码后包含危险指令 |

---

## 3. 资源治理

### 3.1 ResourceGovernor

```rust
struct ResourceGovernor {
    limits: ResourceLimits,
    usage: Arc<Mutex<ResourceUsage>>,
}

struct ResourceLimits {
    max_tokens_per_turn: u32,
    max_tokens_per_hour: u32,
    max_tool_calls_per_turn: u32,
    max_concurrent_tools: u32,
    max_memory_mb: u64,
    max_disk_write_mb_per_hour: u64,
    max_cpu_percent: f32,
}

impl ResourceGovernor {
    fn check_allow(&self, request: &ResourceRequest) -> Result<(), ResourceViolation> { ... }
    fn emergency_throttle(&self) -> ThrottleAction { ... }
}

enum ThrottleAction {
    None,
    ReduceContext,         // 减少上下文窗口
    ForceLocalOnly,        // 强制本地模型
    RejectNewTasks,        // 拒绝新推理请求
    EnterSafeMode,         // 进入安全模式
}
```

### 3.2 默认资源限制

| 资源 | 默认值 | 触发动作 | 检测频率 |
|------|--------|----------|----------|
| Token/每轮 | 100K | ReduceContext | 每次 LLM 调用 |
| Token/小时 | 500K | ForceLocalOnly | 每小时 |
| 工具调用/每轮 | 50 | RejectNewTasks | 每次工具调用 |
| 并发工具 | 8 | 排队等待 | 工具创建 |
| 内存 | 500MB | EnterSafeMode | 每 30s |
| 磁盘写入/小时 | 1GB | Warn + 记录 | 每次文件写入 |

---

## 4. 紧急停止

### 4.1 EmergencyKillswitch

```rust
struct EmergencyKillswitch {
    triggers: Vec<KillswitchTrigger>,

    async fn activate(&self, reason: &str) {
        self.cancel_all_tasks().await;       // 取消所有运行中的工具
        self.save_state_snapshot().await;     // 保存最后一次安全状态
        self.notify_user(format!("Agent 紧急停止: {}", reason)).await;
        self.enter_safe_mode().await;         // 进入安全模式
    }
}

enum KillswitchTrigger {
    ConsecutiveFailures { count: u32 },       // 连续 N 次工具调用失败
    InjectionDetected { confidence: f32 },    // 注入检测置信度 > 0.9
    ResourceExhausted { resource: String },   // 资源耗尽
    UserTriggered,                            // 用户手动触发
    AnomalousBehavior { pattern: String },    // 异常行为模式
    SecurityPolicyViolation { violation: String }, // 安全策略违规
}
```

### 4.2 触发阈值

| 触发条件 | 阈值 | 冷却时间 | 自动恢复 |
|----------|------|----------|----------|
| 连续工具失败 | 10 次 | 30s | 是（5s 后尝试恢复） |
| 注入检测 | 置信度 > 0.9 | 60s | 否（需用户确认） |
| 资源耗尽 | 任意资源 > 95% | 120s | 是（资源回收后） |
| 用户触发 | 手动 | N/A | 否 |
| 异常行为 | 偏离基线 3σ | 300s | 否 |
| 安全策略违规 | L3 操作被阻断 3 次/轮 | — | 是（下轮推理） |

---

## 5. 自我更新

```rust
struct SelfUpdateManager {
    /// 检查新版本（GitHub Release / AUR）
    async fn check_update(&self) -> Option<UpdateInfo>;
    /// 验证签名（cosign / minisign）
    fn verify_signature(&self, update: &UpdateInfo) -> Result<()>;
    /// 应用更新
    async fn apply_update(&self, update: &UpdateInfo) -> Result<()>;
    /// 回滚到上一个版本
    async fn rollback(&self) -> Result<()>;
}
```

| 阶段 | 操作 | 失败安全 |
|------|------|----------|
| 检查 | 对比本地版本与远程版本 | 静默跳过 |
| 下载 | 验证签名 + SHA256 | 删除已下载文件 |
| 备份 | 备份当前二进制和配置 | 中止更新 |
| 应用 | 替换二进制 + 重启 | 自动恢复备份 |
| 验证 | 启动后运行完整性检查 | 回滚到备份 |

---

## 6. 完整性监控

```rust
struct IntegrityMonitor {
    config_hashes: HashMap<PathBuf, Hash>,  // 配置文件的已知 hash
    binary_hash: Hash,                       // aletheon daemon 二进制的已知 hash
    /// 定期完整性检查（默认每 5 分钟）
    async fn check_integrity(&self) -> IntegrityReport { ... }
    /// 检测到篡改时触发紧急停止
    async fn on_tamper_detected(&self, path: &Path) { ... }
}
```

### 完整性检查范围

| 检查项 | 方法 | 检查频率 |
|--------|------|----------|
| aletheon daemon 二进制 | mmap + SHA256 | 启动时 + 每 5min |
| 配置文件 (TOML) | SHA256 | 每次重新加载 |
| 安全策略 (YAML) | SHA256 | 每次策略评估 |
| Agent 定义 (TOML) | SHA256 | 启动时 |
| 技能插件 | SHA256 + sign | 加载时 |
| 内核模块 (.ko) | SHA256 + modinfo | 加载时 |

---

## 7. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| Codex | 三级注入防护 (input/param/output) |
| Codex | EmergencyKillswitch + cancel_all_tasks |
| Hermes Agent | 资源限制阈值 + per-agent resource budget |
| Anthropic SDK | prompt injection 检测模式（系统覆盖/数据泄露） |
| Claude Code | 完整性监控 + periodic hash check |
| OpenCode | self-update + rollback + signature verification |

---

## Implementation Summary

> 自我保护三层防御及完整性监控均已实现。InputSanitizer 提供输入净化，ResourceGovernor 提供资源限制和节流，EmergencyKillswitch 提供多触发紧急停止，IntegrityMonitor 提供 FNV-1a 文件哈希检查和基线追踪。

| Component | Status | Notes |
|-----------|--------|-------|
| InputSanitizer | ✅ Implemented | `crates/dasein/src/impl/security/self_protection/input_sanitizer.rs` |
| ResourceGovernor | ✅ Implemented | `crates/dasein/src/impl/security/self_protection/resource_governor.rs` |
| EmergencyKillswitch | ✅ Implemented | `crates/dasein/src/impl/security/self_protection/emergency_killswitch.rs` |
| IntegrityMonitor | ✅ Implemented | `crates/dasein/src/impl/security/self_protection/integrity_monitor.rs` — FNV-1a, baseline, killswitch integration |
| SelfUpdateManager | ⬜ Planned | Update + rollback designed |
