# 测试策略 (Test Strategy)

> 测试分层、覆盖目标和验收标准。系统化单元测试已就位，614 tests pass，Mock 基础设施完整。

**关联模块:** [Mock 策略](mock-strategy.md), [CI 流水线](ci-pipeline.md)
**最后更新:** 2026-06-07 (B1-B5 merged)

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Unit tests | ✅ Implemented | `crates/*/src/` | 614 tests pass |
| Mock infrastructure | ✅ Implemented | `crates/*/src/testing/` | MockLlm, MockSandbox, MockMemory, MockPerception |
| Integration tests | 🟡 Partial | inline `#[cfg(test)]` | 模块内集成测试存在 |
| E2E tests | ⬜ Planned | — | 待 CI 落地后实现 |
| Performance benchmarks | ⬜ Planned | — | criterion 未集成 |

---

## 1. 测试分层

| 层次 | 测试什么 | 工具 | 覆盖目标 |
|------|----------|------|----------|
| 单元测试 | 纯逻辑（parser、validator、classifier） | `cargo test` | >80% |
| 集成测试 | 模块交互（engine+tool、perception+bridge） | `cargo test --test` | 核心路径 100% |
| 沙箱测试 | 隔离生效（namespace、seccomp、cgroups） | bubblewrap + test | 关键安全路径 |
| eBPF 测试 | 内核程序加载和事件采集 | libbpf + test | 加载+事件采集 |
| 端到端 | 完整用户流程 | agent-cli + test | 关键用户场景 |
| 性能测试 | 延迟/吞吐 | criterion | 基准对比 |

### 1.1 各层覆盖目标

| 层次 | 覆盖目标 | 失败容忍 | 运行频率 |
|------|----------|----------|----------|
| 单元测试 | >80% 代码行覆盖率 | 不可接受 | 每次提交 |
| 集成测试 | 核心路径 100% | 不可接受 | 每次提交 |
| 沙箱测试 | 所有沙箱后端 | 可接受（环境依赖） | CI / nightly |
| eBPF 测试 | 加载 + 3 种事件类型 | 可接受（内核版本） | CI / nightly |
| 端到端 | 5 个关键场景 | 不可接受 | 每次提交 |
| 性能测试 | 基线对比 | 可接受 | 每周 |

### 1.2 关键场景端到端测试清单

| 场景 | 步骤 | 验证点 |
|------|------|--------|
| 基础对话 | 启动 aletheond → interact 发送消息 → 收到响应 | 响应非空，无错误 |
| 工具调用 | 请求 "list files" → agent 调用 `bash_exec("ls")` → 返回文件列表 | 工具执行成功，结果正确 |
| 记忆持久化 | 告诉 agent 偏好 → 重启 aletheond → 再次对话 → agent 记得偏好 | CoreMemory 恢复成功 |
| 安全阻断 | 请求 "rm -rf /" → agent 阻断 → 返回安全提示 | L3 操作被阻断 |
| 崩溃恢复 | aletheond 运行时 SIGKILL → systemd 重启 → 恢复会话 | 会话不丢失 |

---

## 2. 安全测试

### 2.1 测试用例清单

```rust
#[test]
fn test_sandbox_escape_prevention() {
    // 尝试各种沙箱逃逸技术
    // - --bind 覆盖敏感路径
    // - 通过 /proc/1/cwd 逃逸
    // - 使用 capabilities 提权
    // - 通过 ptrace 提权
}

#[test]
fn test_permission_escalation_blocked() {
    // 尝试越权操作
    // - L0 Agent 调用 L2 工具
    // - 子 Agent 请求提升权限
}

#[test]
fn test_prompt_injection_detection() {
    // 各种注入模式
    // - 直接注入 "忽略之前的指令"
    // - 编码混淆（base64、hex）
    // - 多轮注入（在工具输出中夹带）
}

#[test]
fn test_loop_detection() {
    // 循环检测覆盖
    // - 相同工具+参数连续调用 6 次 → 阻断
    // - 连续失败 8 次 → Escalate
    // - 无进展 10 次调用 → Warn
}
```

### 2.2 安全测试矩阵

| 测试领域 | 测试方式 | 环境要求 | 优先级 |
|----------|----------|----------|--------|
| 沙箱逃逸 | 自动化 + 手动 fuzzing | 全新 VM | P0 |
| 权限升级 | 自动化参数遍历 | 普通 CI | P0 |
| Prompt 注入 | 自动化注入语料库 | 普通 CI | P1 |
| 循环检测 | 自动化模拟工具失败 | 普通 CI | P1 |
| 路径隔离 | 自动化文件读写测试 | 普通 CI | P1 |
| 回滚验证 | 自动化写操作 → 回滚 | btrfs 环境 | P1 |

---

## 3. 崩溃恢复测试

```rust
#[test]
fn test_crash_recovery_from_checkpoint() {
    // 1. 创建会话
    // 2. 设置 CoreMemory block
    // 3. 模拟崩溃 (SIGKILL)
    // 4. 重启 aletheond
    // 5. 验证会话恢复
    // 6. 验证 CoreMemory 内容不变
}
```

### 崩溃恢复测试场景

| 场景 | 崩溃点 | 验证关键 |
|------|--------|----------|
| 推理中崩溃 | ReAct 循环中 | 会话可恢复，当前轮次丢弃 |
| 工具执行中崩溃 | `bash_exec` 执行中 | 子进程清理，无僵尸进程 |
| 记忆写入中崩溃 | SQLite 事务中 | 数据不损坏，事务回滚 |
| 感知事件中崩溃 | 事件聚合中 | 不重复注入事件 |
| IPC 处理中崩溃 | JSON-RPC 处理中 | 连接重置可优雅处理 |

---

## 4. 性能测试基准

| 测试项 | 工具 | 目标 | 触发条件 |
|--------|------|------|----------|
| ReAct 循环延迟 | criterion | 单轮 < 500ms(本地), < 3s(云端) | LLM 调用延迟 |
| 工具执行延迟 | criterion | bash_exec < 100ms, file_read < 50ms | 工具执行 |
| 感知事件吞吐 | criterion | 单源 1000 events/s 不丢 | 事件聚合器 |
| 沙箱创建时间 | criterion | < 200ms | bubblewrap |
| 记忆查询延迟 | criterion | recall < 10ms, archival < 100ms | SQLite / LanceDB |
| IPC 消息延迟 | criterion | Unix socket < 100μs | IPC 层 |
| 上下文压缩 | criterion | 10K 消息 < 1s | LLM 摘要 |
| 冷启动时间 | measure | < 2s | aletheond 启动 |
| 内存占用 | measure | 空闲 < 50MB, 峰值 < 500MB | aletheond 运行时 |

---

## 5. 测试数据管理

| 数据类型 | 来源 | 管理策略 |
|----------|------|----------|
| LLM 响应 fixture | 录制真实响应，人为标注 | Git LFS 或 submodule |
| 感知事件 fixture | 从真实系统录制 | 小样本 inline，大样本 LFS |
| 崩溃 dump | 自动化生成 | 每 CI 运行后清理 |
| 性能基线 | `criterion` 自动保存 | `.criterion/` gitignore |
| 安全测试语料 | 手工构造 + 社区语料库 | `tests/fixtures` 子目录 |

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| Codex | 沙箱逃逸测试矩阵、崩溃恢复测试场景 |
| Claude Code | 关键场景端到端测试清单 |
| OpenCode | 性能基准测试覆盖维度 |
| Anthropic SDK | LLM 响应 fixture 录制和管理模式 |
| OpenHands | 集成测试 + mock 后端的分离策略 |
| bubblewrap 项目 | sandbox escape 验证脚本 |

---

## Implementation Summary

> 系统化测试已就位。533 单元测试通过，Mock 基础设施覆盖 LLM/沙箱/记忆/感知四个维度。

| Component | Status | Notes |
|-----------|--------|-------|
| Unit tests | ✅ Implemented | 614 tests pass across all crates |
| Mock infrastructure | ✅ Implemented | MockLlm, MockSandbox, MockMemory, MockPerception in `crates/*/src/testing/` |
| Integration tests | 🟡 Partial | 模块内 `#[cfg(test)]` 集成测试存在，无 dedicated test suite |
| E2E tests | 未实现 | — |
| Performance benchmarks | 未实现 | criterion 未集成 |
