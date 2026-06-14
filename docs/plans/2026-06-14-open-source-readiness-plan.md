# Aletheon 开源发布就绪 实施计划

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 Aletheon 开源发布的所有准备工作，包括基础设施、文档、CI/CD、Demo 和发布流程。

**Architecture:** 基于设计文档 `2026-06-14-open-source-readiness-design.md`，分 5 个阶段实施：基础设施 → 文档 → CI/CD → Demo → 发布。

**Tech Stack:** Rust, GitHub Actions, Markdown, crates.io

---

## Phase 1: 开源基础设施（P0）

### Task 1: 添加 MIT LICENSE 文件

**Files:**
- Create: `LICENSE`

- [ ] **Step 1: 创建 MIT LICENSE 文件**

```text
MIT License

Copyright (c) 2026 Aurobear

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 2: 更新 Cargo.toml 中的 license 字段**

修改 `Cargo.toml` 中 `[workspace.package]` 的 `license` 字段：

```toml
[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
```

- [ ] **Step 3: 为所有 crate 添加 license.workspace = true**

检查并更新以下 crate 的 `Cargo.toml`，确保包含 `license.workspace = true`：

- `crates/aletheon-abi/Cargo.toml` ✅ 已有
- `crates/aletheon-body/Cargo.toml` - 需添加
- `crates/aletheon-brain/Cargo.toml` - 需添加
- `crates/aletheon-comm/Cargo.toml` - 需添加
- `crates/aletheon-memory/Cargo.toml` - 需添加
- `crates/aletheon-meta/Cargo.toml` - 需添加
- `crates/aletheon-runtime/Cargo.toml` - 需添加
- `crates/aletheon-self/Cargo.toml` - 需添加
- `crates/binaries/aletheond/Cargo.toml` - 需添加
- `crates/binaries/aletheon-exec/Cargo.toml` - 需添加
- `crates/binaries/aletheon-cli/Cargo.toml` - 需添加

在每个 crate 的 `[package]` 部分添加：
```toml
license.workspace = true
```

- [ ] **Step 4: 验证**

```bash
cargo metadata --no-deps --format-version 1 | jq '.packages[].license'
```

Expected: 所有包显示 `"MIT"`

- [ ] **Step 5: 提交**

```bash
git add LICENSE Cargo.toml crates/*/Cargo.toml crates/binaries/*/Cargo.toml
git commit -m "chore: add MIT license and update workspace metadata"
```

---

### Task 2: 创建 CONTRIBUTING.md

**Files:**
- Create: `CONTRIBUTING.md`

- [ ] **Step 1: 创建 CONTRIBUTING.md**

```markdown
# Contributing to Aletheon

感谢你对 Aletheon 的兴趣！我们欢迎各种形式的贡献。

## 快速开始

### 开发环境

1. 安装 Rust 工具链：
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup default stable
   ```

2. 克隆仓库：
   ```bash
   git clone https://github.com/Aurobear/aletheon.git
   cd aletheon
   ```

3. 运行测试：
   ```bash
   cargo test --workspace
   ```

4. 检查代码风格：
   ```bash
   cargo fmt --check
   cargo clippy -- -D warnings
   ```

## 架构概览

Aletheon 采用三体架构：

- **SelfField** (自我意识层): 自我连续性、边界感知、叙事构建
- **BrainCore** (认知核心): 推理、规划、反思
- **BodyRuntime** (执行层): 工具执行、系统交互

详见 [docs/architecture/overview.md](docs/architecture/overview.md)

## 贡献领域

我们特别欢迎以下方面的贡献：

1. **Self-Evolution 算法**: 改进反思和行为进化机制
2. **新 Tool 实现**: 扩展 Agent 的工具集
3. **文档**: 改进文档、添加示例、翻译
4. **测试**: 提高测试覆盖率
5. **Bug 修复**: 修复已知问题

## Pull Request 流程

1. Fork 仓库
2. 创建功能分支: `git checkout -b auro/feat/your-feature`
3. 提交更改: `git commit -m "feat: add your feature"`
4. 推送到 Fork: `git push origin auro/feat/your-feature`
5. 创建 Pull Request

### 分支命名规范

- 功能: `auro/feat/feature-name`
- 修复: `auro/fix/bug-name`
- 文档: `auro/docs/topic`

### 提交信息规范

使用 [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

类型:
- `feat`: 新功能
- `fix`: Bug 修复
- `docs`: 文档更新
- `style`: 代码格式（不影响功能）
- `refactor`: 重构
- `test`: 测试相关
- `chore`: 构建/工具链相关

## 代码风格

- 使用 `rustfmt` 格式化代码
- 使用 `clippy` 检查代码质量
- 所有公开 API 必须有文档注释
- 测试覆盖率目标: 80%+

## 报告 Bug

使用 [GitHub Issues](https://github.com/Aurobear/aletheon/issues) 报告 Bug，请包含:

1. 环境信息 (OS, Rust 版本)
2. 复现步骤
3. 期望行为
4. 实际行为
5. 相关日志

## 行为准则

本项目遵循 [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md)。
```

- [ ] **Step 2: 提交**

```bash
git add CONTRIBUTING.md
git commit -m "docs: add contributing guide"
```

---

### Task 3: 创建 CODE_OF_CONDUCT.md

**Files:**
- Create: `CODE_OF_CONDUCT.md`

- [ ] **Step 1: 创建 CODE_OF_CONDUCT.md**

使用 Contributor Covenant 2.1 全文，从 https://www.contributor-covenant.org/version/2/1/code_of_conduct/ 获取。

- [ ] **Step 2: 提交**

```bash
git add CODE_OF_CONDUCT.md
git commit -m "docs: add code of conduct"
```

---

### Task 4: 创建 SECURITY.md

**Files:**
- Create: `SECURITY.md`

- [ ] **Step 1: 创建 SECURITY.md**

```markdown
# Security Policy

## 报告安全漏洞

如果你发现安全漏洞，请**不要**通过公开 Issue 报告。

请通过以下方式联系我们:

- Email: [你的邮箱]
- 或者通过 GitHub 的私人漏洞报告功能

## 响应时间

我们会在 48 小时内确认收到报告，并在 7 天内提供修复计划。

## 安全更新

安全更新会通过以下方式发布:

1. GitHub Security Advisory
2. CHANGELOG 中的安全部分
3. crates.io 上的新版本

## 赏金计划

目前没有赏金计划，但我们非常感谢安全研究者的贡献。
```

- [ ] **Step 2: 提交**

```bash
git add SECURITY.md
git commit -m "docs: add security policy"
```

---

### Task 5: 创建 GitHub Issue 和 PR 模板

**Files:**
- Create: `.github/ISSUE_TEMPLATE/bug_report.md`
- Create: `.github/ISSUE_TEMPLATE/feature_request.md`
- Create: `.github/PULL_REQUEST_TEMPLATE.md`

- [ ] **Step 1: 创建 Bug 报告模板**

```markdown
---
name: Bug Report
about: 报告一个 Bug
title: '[BUG] '
labels: bug
assignees: ''
---

## 环境信息

- OS: [e.g., Ubuntu 22.04, Arch Linux]
- Rust 版本: [e.g., 1.75.0]
- Aletheon 版本: [e.g., 0.1.0]

## 复现步骤

1. 运行 '...'
2. 执行 '...'
3. 看到错误

## 期望行为

描述你期望发生什么。

## 实际行为

描述实际发生了什么。

## 相关日志

```
粘贴相关日志
```

## 补充信息

任何其他有助于诊断问题的信息。
```

- [ ] **Step 2: 创建 Feature 请求模板**

```markdown
---
name: Feature Request
about: 提出一个新功能建议
title: '[FEATURE] '
labels: enhancement
assignees: ''
---

## 问题描述

描述你遇到的问题或需求。

## 解决方案

描述你希望的解决方案。

## 替代方案

描述你考虑过的替代方案。

## 补充信息

任何其他相关信息。
```

- [ ] **Step 3: 创建 PR 模板**

```markdown
## 描述

简要描述这个 PR 做了什么。

## 相关 Issue

Fixes #(issue number)

## 变更类型

- [ ] Bug 修复
- [ ] 新功能
- [ ] 重构
- [ ] 文档更新
- [ ] 测试
- [ ] 其他

## 测试

描述你如何测试这些变更。

## 检查清单

- [ ] 代码符合项目风格指南
- [ ] 已添加/更新测试
- [ ] 已更新文档
- [ ] 所有测试通过
- [ ] clippy 无警告
```

- [ ] **Step 4: 提交**

```bash
git add .github/ISSUE_TEMPLATE/ .github/PULL_REQUEST_TEMPLATE.md
git commit -m "chore: add GitHub issue and PR templates"
```

---

## Phase 2: 文档体系（P1）

### Task 6: 创建用户指南 - getting-started.md

**Files:**
- Create: `docs/guide/getting-started.md`

- [ ] **Step 1: 创建 getting-started.md**

```markdown
# 快速开始

5 分钟内从零开始体验 Aletheon。

## 前置条件

- Linux 系统 (推荐 Ubuntu 22.04 或 Arch Linux)
- Rust 工具链 (1.75.0+)
- 至少 4GB RAM
- 2GB 磁盘空间

## 安装

### 1. 克隆仓库

```bash
git clone https://github.com/Aurobear/aletheon.git
cd aletheon
```

### 2. 构建项目

```bash
cargo build --release
```

### 3. 运行测试

```bash
cargo test --workspace
```

## 运行第一个 Agent

### 1. 创建配置文件

```bash
mkdir -p ~/.config/aletheon
cat > ~/.config/aletheon/config.toml << 'EOF'
[agent]
name = "my-first-agent"
model = "local"

[memory]
backend = "sqlite"
path = "~/.local/share/aletheon/memory.db"

[tools]
enabled = ["bash", "file", "http"]
EOF
```

### 2. 启动 Agent

```bash
./target/release/aletheond
```

### 3. 与 Agent 交互

```bash
./target/release/aletheon-cli "你好，请介绍一下你自己"
```

## 体验 Self-Evolution

Self-Evolution 是 Aletheon 的核心特性。运行完整 demo:

```bash
cd examples/self-evolution-demo
./setup.sh
./run-demo.sh
```

详见 [Self-Evolution Demo](../../examples/self-evolution-demo/README.md)

## 下一步

- [核心概念](concepts.md) - 了解 SelfField、BrainCore、BodyRuntime
- [配置参考](configuration.md) - 详细的配置选项
- [架构概览](../design/architecture-overview.md) - 深入了解系统架构
```

- [ ] **Step 2: 提交**

```bash
git add docs/guide/getting-started.md
git commit -m "docs: add getting started guide"
```

---

### Task 7: 创建用户指南 - concepts.md

**Files:**
- Create: `docs/guide/concepts.md`

- [ ] **Step 1: 创建 concepts.md**

```markdown
# 核心概念

Aletheon 的核心架构概念。

## 三体架构

Aletheon 采用三体架构，模拟人类认知的三个层次:

```
┌─────────────────────────────────────────────────────────────┐
│                      Aletheon                               │
├─────────────────────────────────────────────────────────────┤
│  SelfField (自我意识)                                        │
│  ├── 自我连续性: Agent 知道"我是谁"                          │
│  ├── 边界感知: 区分自我与环境                                │
│  └── 叙事构建: 记录和理解自己的经历                          │
├─────────────────────────────────────────────────────────────┤
│  BrainCore (认知核心)                                        │
│  ├── 推理: 分析问题、制定计划                                │
│  ├── 规划: 分解任务、安排执行顺序                            │
│  └── 反思: 评估结果、提取经验                                │
├─────────────────────────────────────────────────────────────┤
│  BodyRuntime (执行层)                                        │
│  ├── 工具执行: 调用各种工具完成任务                          │
│  ├── 系统交互: 与操作系统深度集成                            │
│  └── 结果收集: 汇总执行结果                                  │
└─────────────────────────────────────────────────────────────┘
```

## Self-Evolution (自我进化)

Self-Evolution 是 Aletheon 的核心特性，使 Agent 能够:

1. **反思 (Reflect)**: 分析自己的行为和结果
2. **进化 (Evolve)**: 根据反思改进自己的行为
3. **Genome 生成**: 将成功的行为模式固化为 Genome

### 进化循环

```
任务执行 → 结果评估 → 反思分析 → 行为进化 → Genome 更新
    ↑                                              │
    └──────────────────────────────────────────────┘
```

### Genome

Genome 是 Agent 的行为基因，记录了:

- 成功的行为模式
- 失败的教训
- 优化的策略

详见 [Self-Evolution 详解](../architecture/self-evolution.md)

## Memory System (记忆系统)

Aletheon 的记忆系统分为三层:

1. **Episodic Memory**: 记录具体事件
2. **Semantic Memory**: 存储知识和概念
3. **Procedural Memory**: 记录操作步骤

详见 [记忆系统](../design/memory/README.md)

## Plugin System (插件系统)

Aletheon 支持通过插件扩展功能:

- **Native Plugins**: Rust 编写的高性能插件
- **WASM Plugins**: WebAssembly 插件，安全隔离
- **Script Plugins**: 脚本插件，快速开发

详见 [插件系统](../design/runtime/plugin.md)

## Linux Integration (Linux 集成)

Aletheon 深度集成 Linux 系统:

- **eBPF**: 内核级事件感知
- **systemd**: 服务生命周期管理
- **FUSE**: 用户态文件系统
- **D-Bus**: 进程间通信

详见 [Linux 集成](../architecture/linux-integration.md)
```

- [ ] **Step 2: 提交**

```bash
git add docs/guide/concepts.md
git commit -m "docs: add core concepts guide"
```

---

### Task 8: 创建架构文档 - self-evolution.md

**Files:**
- Create: `docs/architecture/self-evolution.md`

- [ ] **Step 1: 创建 self-evolution.md**

```markdown
# Self-Evolution 机制详解

Self-Evolution 是 Aletheon 的核心卖点，使 Agent 从"执行者"进化为"自我完善的实体"。

## 与传统 Agent 的本质区别

```
传统 Agent:
  用户指令 → 模型推理 → 工具调用 → 返回结果
  (无记忆，无学习，无进化)

Aletheon:
  用户指令 → 模型推理 → 工具调用 → 返回结果
                                    ↓
                              结果评估
                                    ↓
                              反思分析
                                    ↓
                              行为进化
                                    ↓
                              Genome 更新
  (有记忆，有学习，持续进化)
```

## 反思机制

### 反思触发条件

1. **任务完成**: 每次任务完成后触发反思
2. **周期性反思**: 定期回顾最近的行为
3. **失败触发**: 任务失败时立即反思
4. **手动触发**: 用户主动要求反思

### 反思内容

```rust
pub struct Reflection {
    pub task: String,           // 任务描述
    pub action: String,         // 执行的动作
    pub result: ActionResult,   // 执行结果
    pub analysis: String,       // 分析
    pub lessons: Vec<String>,   // 学到的教训
    pub improvements: Vec<String>, // 改进建议
}
```

### 反思流程

```
1. 收集任务执行数据
2. 分析成功/失败原因
3. 提取可复用的经验
4. 生成改进建议
5. 存储到记忆系统
```

## 行为进化

### 进化触发条件

1. **连续失败**: 同类任务连续失败 3 次
2. **周期性进化**: 定期评估是否需要进化
3. **用户反馈**: 用户明确指出需要改进
4. **反思积累**: 积累足够多的反思后触发

### 进化策略

1. **工具优化**: 为重复任务创建专用工具
2. **策略调整**: 修改任务执行策略
3. **参数调优**: 优化工具调用参数
4. **流程重构**: 重新设计任务执行流程

### 进化流程

```
1. 收集相关反思
2. 分析进化需求
3. 生成进化方案
4. 测试进化方案
5. 应用进化结果
6. 更新 Genome
```

## Genome

### Genome 结构

```rust
pub struct Genome {
    pub version: u32,                    // 版本号
    pub created_at: DateTime<Utc>,       // 创建时间
    pub updated_at: DateTime<Utc>,       // 更新时间
    pub behaviors: Vec<BehaviorGene>,    // 行为基因
    pub tools: Vec<ToolGene>,           // 工具基因
    pub strategies: Vec<StrategyGene>,   // 策略基因
}

pub struct BehaviorGene {
    pub name: String,                    // 行为名称
    pub pattern: String,                 // 行为模式
    pub success_rate: f64,              // 成功率
    pub usage_count: u32,               // 使用次数
    pub last_used: DateTime<Utc>,       // 最后使用时间
}
```

### Genome 应用

1. **行为选择**: 根据 Genome 选择最佳行为
2. **工具选择**: 根据 Genome 选择最合适的工具
3. **策略应用**: 应用 Genome 中的成功策略

### Genome 持久化

Genome 存储在 SQLite 数据库中，支持:

- 版本管理
- 历史回溯
- 导入导出

## 完整进化循环示例

### 场景: Agent 学会使用新工具

1. **初始状态**: Agent 只有基础工具 (bash, file)
2. **任务**: 监控系统 CPU 使用率
3. **第一次尝试**: 用 bash 轮询 `/proc/stat`，效率低
4. **反思**: "这个任务重复性高，应该写成专用工具"
5. **进化**: 自动生成 `cpu_monitor` 工具
6. **第二次执行**: 使用新工具，效率提升 10x
7. **Genome 更新**: 新增 "为重复任务创建专用工具" 行为模式

## 实现细节

### 核心模块

- `aletheon-self`: 自我意识层实现
- `aletheon-brain`: 认知核心实现
- `aletheon-memory`: 记忆系统实现

### 关键数据结构

- `Reflection`: 反思记录
- `BehaviorGene`: 行为基因
- `Genome`: 基因组

### 配置选项

```toml
[self_evolution]
enabled = true
reflection_interval = 3600  # 反思间隔（秒）
evolution_threshold = 3     # 进化触发阈值
max_genome_size = 1000      # 最大基因数量
```

## 最佳实践

1. **定期反思**: 保持反思频率，积累经验
2. **渐进进化**: 避免激进的进化策略
3. **验证充分**: 进化前充分测试
4. **备份 Genome**: 定期备份基因组
```

- [ ] **Step 2: 提交**

```bash
git add docs/architecture/self-evolution.md
git commit -m "docs: add self-evolution architecture documentation"
```

---

### Task 9: 创建架构文档 - linux-integration.md

**Files:**
- Create: `docs/architecture/linux-integration.md`

- [ ] **Step 1: 创建 linux-integration.md**

```markdown
# Linux 系统集成

Aletheon 深度集成 Linux 系统，实现真正的"系统级 Agent"。

## 集成层次

```
┌─────────────────────────────────────────────────────────────┐
│                    Aletheon Agent                           │
├─────────────────────────────────────────────────────────────┤
│  应用层集成                                                  │
│  ├── D-Bus: 进程间通信                                       │
│  ├── systemd: 服务管理                                       │
│  └── FUSE: 用户态文件系统                                    │
├─────────────────────────────────────────────────────────────┤
│  内核层集成                                                  │
│  ├── eBPF: 内核事件监控                                      │
│  ├── /proc: 进程信息                                         │
│  └── /sys: 系统信息                                          │
├─────────────────────────────────────────────────────────────┤
│  硬件层集成                                                  │
│  ├── 传感器数据                                              │
│  ├── 设备控制                                                │
│  └── 资源监控                                                │
└─────────────────────────────────────────────────────────────┘
```

## eBPF 集成

### 什么是 eBPF

eBPF (Extended Berkeley Packet Filter) 是 Linux 内核的可编程技术，允许在内核空间运行自定义代码。

### Aletheon 的 eBPF 使用

1. **系统调用监控**: 监控进程的系统调用
2. **网络监控**: 监控网络流量和连接
3. **文件系统监控**: 监控文件访问和修改
4. **性能分析**: 收集性能数据

### 示例: 监控 CPU 使用率

```rust
// eBPF 程序示例
#[no_mangle]
pub extern "C" fn trace_cpu_usage(ctx: *mut c_void) -> i32 {
    // 读取 CPU 使用率数据
    let cpu_usage = read_cpu_usage();
    
    // 发送到用户空间
    send_event(cpu_usage);
    
    0
}
```

## systemd 集成

### 服务管理

Aletheon 作为 systemd 服务运行:

```ini
# /etc/systemd/system/aletheon.service
[Unit]
Description=Aletheon Agent Runtime
After=network.target

[Service]
Type=simple
User=aletheon
ExecStart=/usr/bin/aletheond
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

### 服务命令

```bash
# 启动服务
sudo systemctl start aletheon

# 查看状态
sudo systemctl status aletheon

# 查看日志
journalctl -u aletheon -f
```

## FUSE 集成

### 用户态文件系统

Aletheon 通过 FUSE 提供虚拟文件系统:

```
/aletheon/
├── status/           # Agent 状态
├── memory/           # 记忆系统
├── genome/           # 基因组
├── tools/            # 工具列表
└── logs/             # 日志
```

### 使用示例

```bash
# 挂载文件系统
aletheon-fuse /mnt/aletheon

# 查看 Agent 状态
cat /mnt/aletheon/status/agent.json

# 查看记忆
ls /mnt/aletheon/memory/
```

## D-Bus 集成

### 进程间通信

Aletheon 通过 D-Bus 与其他进程通信:

```rust
// D-Bus 接口示例
#[dbus_interface(name = "org.aletheon.Agent")]
impl AgentInterface {
    fn execute_task(&self, task: &str) -> Result<String, Error> {
        // 执行任务
        Ok(result)
    }
    
    fn get_status(&self) -> Result<AgentStatus, Error> {
        // 获取状态
        Ok(status)
    }
}
```

## /proc 和 /sys 集成

### 进程信息

通过 `/proc` 获取进程信息:

```rust
fn get_process_info(pid: u32) -> ProcessInfo {
    let path = format!("/proc/{}/status", pid);
    let content = std::fs::read_to_string(path).unwrap();
    // 解析进程信息
    parse_process_info(content)
}
```

### 系统信息

通过 `/sys` 获取系统信息:

```rust
fn get_cpu_info() -> CpuInfo {
    let content = std::fs::read_to_string("/proc/cpuinfo").unwrap();
    // 解析 CPU 信息
    parse_cpu_info(content)
}
```

## 安全考虑

### 权限控制

- eBPF 程序需要 root 权限
- FUSE 挂载需要用户权限
- D-Bus 访问需要策略配置

### 沙箱机制

- 使用 cgroups 限制资源
- 使用 namespaces 隔离进程
- 使用 seccomp 限制系统调用

## 最佳实践

1. **最小权限**: 只请求必要的权限
2. **审计日志**: 记录所有系统访问
3. **安全隔离**: 使用沙箱机制
4. **资源限制**: 防止资源滥用
```

- [ ] **Step 2: 提交**

```bash
git add docs/architecture/linux-integration.md
git commit -m "docs: add Linux integration architecture documentation"
```

---

### Task 10: 创建开发者文档 - testing.md

**Files:**
- Create: `docs/development/testing.md`

- [ ] **Step 1: 创建 testing.md**

```markdown
# 测试策略

Aletheon 的测试策略和运行方法。

## 测试类型

### 1. 单元测试

测试单个函数或模块:

```bash
# 运行所有单元测试
cargo test --workspace

# 运行特定 crate 的测试
cargo test -p aletheon-abi

# 运行特定测试
cargo test test_function_name
```

### 2. 集成测试

测试模块之间的交互:

```bash
# 运行集成测试
cargo test --test '*'
```

### 3. 端到端测试

测试完整的工作流:

```bash
# 运行端到端测试
cargo test --test e2e_*
```

## 测试覆盖率

### 安装覆盖率工具

```bash
cargo install cargo-tarpaulin
```

### 生成覆盖率报告

```bash
cargo tarpaulin --workspace --out Html
```

### 覆盖率目标

- 核心模块: 80%+
- 工具模块: 70%+
- 辅助模块: 60%+

## 测试最佳实践

### 1. 测试命名

```rust
#[test]
fn test_function_name_with_input_should_return_expected() {
    // 测试代码
}
```

### 2. 测试结构

```rust
#[test]
fn test_example() {
    // Arrange - 准备测试数据
    let input = "test";
    
    // Act - 执行被测试的函数
    let result = function_under_test(input);
    
    // Assert - 验证结果
    assert_eq!(result, "expected");
}
```

### 3. 测试覆盖

- 正常路径
- 异常路径
- 边界条件
- 并发场景

## 持续集成

GitHub Actions 会自动运行:

1. `cargo fmt --check` - 格式检查
2. `cargo clippy -- -D warnings` - 静态分析
3. `cargo test --workspace` - 全量测试
4. `cargo doc --no-deps` - 文档构建

详见 [CI/CD 管线](../architecture/ci-cd.md)

## 调试测试

### 启用日志

```bash
RUST_LOG=debug cargo test
```

### 运行单个测试

```bash
cargo test test_name -- --nocapture
```

### 使用调试器

```bash
cargo test
# 然后使用 gdb 或 lldb 调试
```

## 性能测试

### 基准测试

```bash
cargo bench
```

### 性能分析

```bash
cargo install cargo-flamegraph
cargo flamegraph
```
```

- [ ] **Step 2: 提交**

```bash
git add docs/development/testing.md
git commit -m "docs: add testing strategy documentation"
```

---

## Phase 3: CI/CD 完善（P1）

### Task 11: 完善 GitHub Actions CI

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: 更新 CI 配置**

```yaml
name: CI

on:
  push:
    branches: [dev, main]
  pull_request:
    branches: [dev, main]

env:
  CARGO_TERM_COLOR: always

jobs:
  fmt:
    name: cargo fmt
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all -- --check

  clippy:
    name: cargo clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace -- -D warnings

  test:
    name: cargo test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace

  doc:
    name: cargo doc
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo doc --workspace --no-deps
        env:
          RUSTDOCFLAGS: "-D warnings"

  build:
    name: cargo build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --workspace --release
```

- [ ] **Step 2: 提交**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: enhance CI pipeline with doc and build checks"
```

---

### Task 12: 创建 Release 工作流

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: 创建 release.yml**

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

permissions:
  contents: write

jobs:
  build:
    name: Build ${{ matrix.target }}
    runs-on: ubuntu-latest
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            archive: tar.gz
          - target: aarch64-unknown-linux-gnu
            archive: tar.gz
    
    steps:
      - uses: actions/checkout@v4
      
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      
      - uses: Swatinem/rust-cache@v2
      
      - name: Build
        run: cargo build --release --target ${{ matrix.target }}
      
      - name: Package
        run: |
          cd target/${{ matrix.target }}/release
          tar czf ../../../aletheon-${{ github.ref_name }}-${{ matrix.target }}.tar.gz \
            aletheond aletheon-exec aletheon
          cd ../../..
      
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: aletheon-${{ matrix.target }}
          path: aletheon-${{ github.ref_name }}-${{ matrix.target }}.tar.gz

  release:
    name: Create Release
    needs: build
    runs-on: ubuntu-latest
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Download artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts
      
      - name: Generate changelog
        id: changelog
        run: |
          # 获取上一个 tag
          PREV_TAG=$(git describe --tags --abbrev=0 HEAD^ 2>/dev/null || echo "")
          
          # 生成 changelog
          if [ -z "$PREV_TAG" ]; then
            CHANGELOG=$(git log --oneline --no-merges)
          else
            CHANGELOG=$(git log --oneline --no-merges ${PREV_TAG}..HEAD)
          fi
          
          # 写入文件
          echo "$CHANGELOG" > CHANGELOG.md
          
          # 设置输出
          echo "changelog<<EOF" >> $GITHUB_OUTPUT
          echo "$CHANGELOG" >> $GITHUB_OUTPUT
          echo "EOF" >> $GITHUB_OUTPUT
      
      - name: Create Release
        uses: softprops/action-gh-release@v1
        with:
          body: |
            ## Changes
            
            ${{ steps.changelog.outputs.changelog }}
            
            ## Installation
            
            Download the appropriate archive for your platform and extract it:
            
            ```bash
            tar xzf aletheon-${{ github.ref_name }}-x86_64-unknown-linux-gnu.tar.gz
            ```
            
            ## Documentation
            
            - [Getting Started](https://github.com/Aurobear/aletheon/blob/main/docs/guide/getting-started.md)
            - [Architecture](https://github.com/Aurobear/aletheon/blob/main/docs/architecture/overview.md)
            - [Contributing](https://github.com/Aurobear/aletheon/blob/main/CONTRIBUTING.md)
          files: |
            artifacts/**/*
          draft: false
          prerelease: ${{ contains(github.ref_name, 'alpha') || contains(github.ref_name, 'beta') }}
```

- [ ] **Step 2: 提交**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release workflow for automated releases"
```

---

## Phase 4: Demo + 示例（P2）

### Task 13: 创建 Self-Evolution Demo 框架

**Files:**
- Create: `examples/self-evolution-demo/README.md`
- Create: `examples/self-evolution-demo/setup.sh`
- Create: `examples/self-evolution-demo/run-demo.sh`
- Create: `examples/self-evolution-demo/config.toml`

- [ ] **Step 1: 创建 Demo README**

```markdown
# Self-Evolution Demo

这个 demo 展示 Aletheon 的核心特性：自我进化。

## 场景

Agent 学会使用新工具：

1. Agent 启动，只有基础工具（bash、file）
2. 用户给 Agent 一个任务：监控系统 CPU 使用率
3. Agent 第一次尝试：用 bash 轮询 `/proc/stat`，效率低
4. Agent 反思：这个任务重复性高，应该写成专用工具
5. Agent 进化：自动生成一个 `cpu_monitor` 工具
6. Agent 第二次执行：使用新工具，效率提升 10x
7. 用户看到：Agent 的 Genome 中新增了「为重复任务创建专用工具」的行为模式

## 快速开始

### 1. 环境准备

```bash
./setup.sh
```

### 2. 运行 Demo

```bash
./run-demo.sh
```

### 3. 查看结果

```bash
# 查看 Genome 变化
cat expected-output/genome-after.json

# 查看反思日志
cat expected-output/reflection-log.md
```

## 预期输出

### 运行前 Genome

```json
{
  "version": 1,
  "behaviors": [],
  "tools": ["bash", "file"],
  "strategies": []
}
```

### 运行后 Genome

```json
{
  "version": 2,
  "behaviors": [
    {
      "name": "create专用工具",
      "pattern": "为重复任务创建专用工具",
      "success_rate": 1.0,
      "usage_count": 1
    }
  ],
  "tools": ["bash", "file", "cpu_monitor"],
  "strategies": [
    {
      "name": "工具优化策略",
      "description": "识别重复任务并创建专用工具"
    }
  ]
}
```

## 复现性保证

- 固定 seed: 42
- 固定输入: 监控 CPU 使用率
- 固定环境: Ubuntu 22.04, Rust 1.75.0

## 文件结构

```
examples/self-evolution-demo/
├── README.md                 # 本文件
├── setup.sh                  # 环境准备脚本
├── run-demo.sh               # 运行 demo 脚本
├── config.toml               # Agent 配置
├── scenarios/                # 场景说明
│   ├── 01-basic-tools.md
│   ├── 02-first-attempt.md
│   ├── 03-reflection.md
│   ├── 04-evolution.md
│   └── 05-result.md
└── expected-output/          # 预期输出
    ├── genome-before.json
    ├── genome-after.json
    └── reflection-log.md
```
```

- [ ] **Step 2: 创建 setup.sh**

```bash
#!/bin/bash
set -e

echo "=== Self-Evolution Demo 环境准备 ==="

# 检查 Rust
if ! command -v rustc &> /dev/null; then
    echo "错误: 未找到 Rust，请先安装: https://rustup.rs"
    exit 1
fi

echo "Rust 版本: $(rustc --version)"

# 构建项目
echo "构建 Aletheon..."
cd ../..
cargo build --release
cd examples/self-evolution-demo

# 创建必要目录
mkdir -p ~/.config/aletheon
mkdir -p ~/.local/share/aletheon

# 复制配置文件
cp config.toml ~/.config/aletheon/

echo "环境准备完成！"
echo "运行 demo: ./run-demo.sh"
```

- [ ] **Step 3: 创建 run-demo.sh**

```bash
#!/bin/bash
set -e

echo "=== Self-Evolution Demo ==="
echo ""
echo "这个 demo 展示 Agent 如何通过反思和进化学会使用新工具。"
echo ""

# 检查是否已运行 setup
if [ ! -f ~/.config/aletheon/config.toml ]; then
    echo "错误: 请先运行 ./setup.sh"
    exit 1
fi

# 运行 demo
echo "启动 Agent..."
cd ../..
cargo run --release --bin aletheond -- --demo self-evolution
cd examples/self-evolution-demo

echo ""
echo "=== Demo 完成 ==="
echo ""
echo "查看结果:"
echo "  cat expected-output/genome-after.json"
echo "  cat expected-output/reflection-log.md"
```

- [ ] **Step 4: 创建 config.toml**

```toml
[agent]
name = "self-evolution-demo"
model = "local"
seed = 42

[memory]
backend = "sqlite"
path = "~/.local/share/aletheon/demo-memory.db"

[self_evolution]
enabled = true
reflection_interval = 0  # 立即反思
evolution_threshold = 1  # 一次失败即触发进化

[tools]
enabled = ["bash", "file"]
```

- [ ] **Step 4: 设置权限并提交**

```bash
chmod +x examples/self-evolution-demo/setup.sh
chmod +x examples/self-evolution-demo/run-demo.sh
git add examples/self-evolution-demo/
git commit -m "docs: add self-evolution demo framework"
```

---

### Task 14: 创建 Basic Agent 示例

**Files:**
- Create: `examples/basic-agent/README.md`
- Create: `examples/basic-agent/main.rs`
- Create: `examples/basic-agent/config.toml`

- [ ] **Step 1: 创建 basic-agent 示例**

```rust
// examples/basic-agent/main.rs
//! 最小 Agent 示例
//!
//! 演示如何创建一个最基本的 Aletheon Agent。

use aletheon_runtime::Runtime;
use aletheon_abi::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::init();
    
    // 加载配置
    let config = Config::load("config.toml")?;
    
    // 创建运行时
    let runtime = Runtime::new(config).await?;
    
    // 执行任务
    let result = runtime.execute("你好，请介绍一下你自己").await?;
    
    println!("Agent 回复: {}", result);
    
    Ok(())
}
```

```markdown
# Basic Agent 示例

最小的 Aletheon Agent 示例。

## 运行

```bash
cd examples/basic-agent
cargo run
```

## 预期输出

```
Agent 回复: 你好！我是 Aletheon，一个系统级的 Agent 运行时...
```

## 代码说明

1. 加载配置文件
2. 创建运行时实例
3. 执行任务并获取结果
```

```toml
# examples/basic-agent/config.toml
[agent]
name = "basic-agent"
model = "local"

[memory]
backend = "sqlite"
path = ":memory:"

[tools]
enabled = ["bash", "file"]
```

- [ ] **Step 2: 提交**

```bash
git add examples/basic-agent/
git commit -m "docs: add basic agent example"
```

---

## Phase 5: 发布准备（P2）

### Task 15: 更新 README 添加徽章

**Files:**
- Modify: `README.md`

- [ ] **Step 1: 更新 README 头部**

在 README.md 的开头添加徽章:

```markdown
# Aletheon: A Persistent Self-Evolving Agent Runtime

[![CI](https://github.com/Aurobear/aletheon/actions/workflows/ci.yml/badge.svg)](https://github.com/Aurobear/aletheon/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Crates.io](https://img.shields.io/crates/v/aletheon.svg)](https://crates.io/crates/aletheon)

> An Agent that is not merely executed, but continuously exists.
> Deep integration with operating system kernels and system services.
```

- [ ] **Step 2: 添加快速开始链接**

在 README 的目录部分添加:

```markdown
- [Quick Start](docs/guide/getting-started.md)
- [Contributing](CONTRIBUTING.md)
- [Self-Evolution Demo](examples/self-evolution-demo/README.md)
```

- [ ] **Step 3: 提交**

```bash
git add README.md
git commit -m "docs: add badges and quick start links to README"
```

---

### Task 16: 创建 CHANGELOG.md

**Files:**
- Create: `CHANGELOG.md`

- [ ] **Step 1: 创建 CHANGELOG**

```markdown
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- MIT License
- CONTRIBUTING.md
- CODE_OF_CONDUCT.md
- SECURITY.md
- GitHub Issue and PR templates
- User guide: getting-started.md
- User guide: concepts.md
- Architecture: self-evolution.md
- Architecture: linux-integration.md
- Development: testing.md
- CI/CD: Enhanced CI pipeline
- CI/CD: Release workflow
- Examples: self-evolution-demo
- Examples: basic-agent

### Changed
- Updated CI configuration
- Updated README with badges

## [0.1.0-alpha] - 2026-06-14

### Added
- Initial release
- Core runtime (aletheon-runtime)
- ABI layer (aletheon-abi)
- Body runtime (aletheon-body)
- Brain core (aletheon-brain)
- Memory system (aletheon-memory)
- Self-evolution system (aletheon-self)
- Meta runtime (aletheon-meta)
- Communication layer (aletheon-comm)
```

- [ ] **Step 2: 提交**

```bash
git add CHANGELOG.md
git commit -m "docs: add changelog"
```

---

### Task 17: 为所有 crate 添加 description 字段（crates.io 发布必需）

**Files:**
- Modify: `crates/aletheon-abi/Cargo.toml`
- Modify: `crates/aletheon-body/Cargo.toml`
- Modify: `crates/aletheon-brain/Cargo.toml`
- Modify: `crates/aletheon-comm/Cargo.toml`
- Modify: `crates/aletheon-memory/Cargo.toml`
- Modify: `crates/aletheon-meta/Cargo.toml`
- Modify: `crates/aletheon-runtime/Cargo.toml`
- Modify: `crates/aletheon-self/Cargo.toml`

- [ ] **Step 1: 为每个 crate 添加 description 字段**

在每个 crate 的 `[package]` 部分添加 `description`：

```toml
# aletheon-abi
description = "Aletheon ABI layer - public API interfaces for agent runtime"

# aletheon-body
description = "Aletheon Body runtime - tool execution and system interaction"

# aletheon-brain
description = "Aletheon Brain core - reasoning, planning, and reflection"

# aletheon-comm
description = "Aletheon Communication layer - inter-process communication"

# aletheon-memory
description = "Aletheon Memory system - episodic, semantic, and procedural memory"

# aletheon-meta
description = "Aletheon Meta runtime - self-update and morphological evolution"

# aletheon-runtime
description = "Aletheon Runtime - core agent runtime and orchestration"

# aletheon-self
description = "Aletheon Self-evolution - reflection, behavior evolution, and genome generation"
```

- [ ] **Step 2: 验证**

```bash
cargo package --list -p aletheon-abi
```

Expected: 无错误输出

- [ ] **Step 3: 提交**

```bash
git add crates/*/Cargo.toml
git commit -m "chore: add description fields for crates.io publishing"
```

---

## 自查清单

完成所有任务后，运行以下检查:

```bash
# 1. 测试通过
cargo test --workspace

# 2. 格式检查
cargo fmt --check

# 3. Clippy 检查
cargo clippy -- -D warnings

# 4. 文档构建
cargo doc --workspace --no-deps

# 5. 检查文件存在
ls -la LICENSE CONTRIBUTING.md CODE_OF_CONDUCT.md SECURITY.md CHANGELOG.md

# 6. 检查 GitHub 模板
ls -la .github/ISSUE_TEMPLATE/ .github/PULL_REQUEST_TEMPLATE.md

# 7. 检查文档
ls -la docs/guide/ docs/architecture/ docs/development/

# 8. 检查示例
ls -la examples/
```

## 里程碑

- [x] M1: LICENSE + CONTRIBUTING 合并
- [ ] M2: 文档体系完成
- [ ] M3: CI/CD 流水线就绪
- [ ] M4: Demo 可复现
- [ ] M5: 首次发布 (v0.1.0-alpha)
