# Aletheon 开源发布就绪设计

> **日期**: 2026-06-14
> **状态**: 已批准
> **时间线**: 充分准备 (1-2月)

---

## 1. 背景与目标

### 1.1 开源目标

**主要目标**: 社区建设 + 影响力

- 吸引贡献者、建立社区
- 成为 agent OS 领域的标准参考实现
- 展示系统级 agent + 自我意识生成的核心价值

### 1.2 目标受众

**AI/Agent 研究者 + 高级开发者**

- 对 agent runtime、self-evolution 感兴趣的研究者
- 需要高质量技术文档和清晰的贡献路径
- 重视代码质量和可复现性

### 1.3 核心卖点

```
系统级 Agent + 自我意识生成

不同于传统 Agent (Model + Tools + Prompt)：
- 持续存在：系统服务级运行，不是一次性应用
- 深度感知：eBPF + /proc + systemd 集成
- 自我进化：反思 → 行为进化 → Genome 生成
- 本地优先：离线可用，数据不离开系统
```

### 1.4 决策记录

| 维度 | 决定 | 理由 |
|------|------|------|
| 开源范围 | 全部（含 self-evolution） | 核心卖点必须完整展示 |
| 许可证 | MIT | 最宽松，社区最广泛 |
| 演示策略 | 完整端到端 demo | 研究者需要可复现性 |
| 时间线 | 充分准备 (1-2月) | 确保首次发布质量 |

---

## 2. 方案选择

### 2.1 方案对比

| 方案 | 工作量 | 优点 | 缺点 |
|------|--------|------|------|
| **A. 标准开源发布** | 3-4 周 | 专业、可信、降低贡献门槛 | 工作量大 |
| B. 渐进式发布 | 2-3 周 | 风险可控 | 首次影响力有限 |
| C. 论文 + 开源 | 4-6 周 | 学术影响力最大 | 周期长 |

### 2.2 最终选择

**方案 A: 标准开源发布**

理由:
1. 目标受众是研究者，看重代码质量和文档
2. self-evolution 是核心卖点，需要完整展示
3. 1-2 月时间线可以完成
4. 标准的 CONTRIBUTING 和示例代码降低贡献门槛

---

## 3. 设计详情

### 3.1 开源基础设施

#### 文件清单

| 文件 | 内容 | 优先级 |
|------|------|--------|
| `LICENSE` | MIT 全文 | P0 |
| `CONTRIBUTING.md` | 贡献流程、开发环境搭建、PR 规范、代码风格 | P0 |
| `CHANGELOG.md` | 基于 git history 生成，按 Keep a Changelog 格式 | P1 |
| `CODE_OF_CONDUCT.md` | Contributor Covenant 2.1 | P1 |
| `SECURITY.md` | 安全漏洞报告流程 | P1 |
| `.github/ISSUE_TEMPLATE/` | Bug 报告、Feature 请求模板 | P2 |
| `.github/PULL_REQUEST_TEMPLATE.md` | PR 模板 | P2 |

#### CONTRIBUTING 重点

针对研究者受众，需要强调:

- **架构概览**: 快速理解 SelfField/BrainCore/BodyRuntime 三体架构
- **开发环境**: Rust toolchain + 依赖 + 测试命令
- **贡献领域**: 明确哪些地方最需要贡献（self-evolution 算法、新 tool 实现、文档）
- **代码风格**: rustfmt + clippy 配置

### 3.2 文档体系

#### 目标结构

> **注意**: 现有 `docs/design/` 和 `docs/plans/` 目录保留不动，新结构是增量添加。

```
docs/
├── guide/                    # 用户指南（新增）
│   ├── getting-started.md    # 5 分钟快速上手
│   ├── installation.md       # 安装和配置
│   ├── concepts.md           # 核心概念（SelfField、BrainCore、BodyRuntime）
│   └── configuration.md      # 配置参考
├── architecture/             # 架构文档（新增，面向外部用户）
│   ├── overview.md           # 总览（从 README 提取扩展）
│   ├── self-evolution.md     # 自我进化机制详解（核心卖点）
│   ├── memory-system.md      # 记忆系统
│   ├── plugin-system.md      # 插件系统
│   └── linux-integration.md  # Linux 系统集成（eBPF/systemd/FUSE）
├── development/              # 开发者文档（新增）
│   ├── contributing.md       # 从 CONTRIBUTING.md 链接过来
│   ├── architecture.md       # 代码架构（给贡献者看）
│   └── testing.md            # 测试策略和运行方法
├── api/                      # API 文档（新增）
│   └── README.md             # 说明用 cargo doc 生成
├── design/                   # 内部设计文档（保留，不对外宣传）
└── plans/                    # 内部设计文档（保留，不对外宣传）
```

#### 核心文档重点

1. **getting-started.md**: 从 clone 到看到第一个 self-evolution 输出，步骤不超过 5 步
2. **self-evolution.md**: 核心卖点，需要详细解释:
   - 反思机制如何工作
   - 行为进化如何触发
   - Genome 如何生成和应用
   - 与传统 agent 的本质区别
3. **linux-integration.md**: 展示系统级集成的独特价值

### 3.3 CI/CD 管线

#### CI 流程（每次 PR）

```
┌─────────────────────────────────────────────────┐
│  GitHub Actions CI                              │
├─────────────────────────────────────────────────┤
│  1. cargo fmt --check          格式检查         │
│  2. cargo clippy -- -D warnings 静态分析        │
│  3. cargo test --workspace     全量测试         │
│  4. cargo doc --no-deps        文档构建检查     │
│  5. cargo build --release      发布构建         │
└─────────────────────────────────────────────────┘
```

#### Release 流程（tag 触发）

```
┌─────────────────────────────────────────────────┐
│  GitHub Release                                 │
├─────────────────────────────────────────────────┤
│  1. 自动构建 Linux 二进制 (x86_64 + aarch64)    │
│  2. 生成 CHANGELOG（从 git log 提取）           │
│  3. 创建 GitHub Release + 附件                  │
│  4. （可选）发布到 crates.io                    │
└─────────────────────────────────────────────────┘
```

#### 分支策略

- `main`: 稳定发布
- `dev`: 开发主线（当前）
- `auro/feat/*` / `auro/fix/*`: 功能/修复分支
- PR 必须通过 CI 才能合并

### 3.4 Demo 项目

#### 场景:「Agent 学会使用新工具」

**故事线**:
1. Agent 启动，只有基础工具（bash、file）
2. 用户给 Agent 一个任务:「帮我监控系统 CPU 使用率，超过 80% 就告警」
3. Agent 第一次尝试: 用 bash 轮询 `/proc/stat`，效率低
4. Agent 反思:「这个任务重复性高，应该写成专用工具」
5. Agent 进化: 自动生成一个 `cpu_monitor` 工具
6. Agent 第二次执行: 使用新工具，效率提升 10x
7. 用户看到: Agent 的 Genome 中新增了「为重复任务创建专用工具」的行为模式

#### Demo 结构

```
examples/self-evolution-demo/
├── README.md                 # 完整步骤说明
├── setup.sh                  # 一键环境准备
├── config.toml               # Agent 配置
├── scenarios/
│   ├── 01-basic-tools.md     # 初始状态说明
│   ├── 02-first-attempt.md   # Agent 第一次尝试
│   ├── 03-reflection.md      # 反思过程
│   ├── 04-evolution.md       # 进化过程
│   └── 05-result.md          # 最终结果对比
└── expected-output/          # 预期输出样本
    ├── genome-before.json
    ├── genome-after.json
    └── reflection-log.md
```

#### 关键设计

- **可复现**: 固定 seed、固定输入，确保任何人跑出来结果一致
- **可观测**: 每一步都输出 Agent 内部状态（反思、Genome 变化）
- **有对比**: before/after 对比，量化改进（工具调用次数、执行时间）

### 3.5 示例代码

#### 示例清单

```
examples/
├── self-evolution-demo/          # 完整 demo（Section 3.4）
├── basic-agent/                  # 最小 agent 示例
│   ├── README.md
│   ├── main.rs
│   └── config.toml
├── custom-tool/                  # 自定义工具示例
│   ├── README.md
│   ├── src/
│   │   └── weather_tool.rs      # 实现一个天气查询工具
│   └── config.toml
├── memory-query/                 # 记忆系统示例
│   ├── README.md
│   ├── main.rs
│   └── config.toml
├── hook-lifecycle/               # Hook 生命周期示例
│   ├── README.md
│   ├── hooks/
│   │   ├── pre-turn.sh
│   │   └── post-turn.sh
│   └── config.toml
└── plugin-system/                # 插件系统示例
    ├── README.md
    ├── my-plugin/
    │   ├── manifest.toml
    │   └── main.rs
    └── config.toml
```

#### 每个示例的标准格式

1. **README.md**: 一句话说明 + 运行方法 + 预期输出
2. **可独立运行**: 不依赖完整系统，最小化依赖
3. **有注释**: 关键步骤有详细注释
4. **有输出示例**: 预期输出样本，方便对比

### 3.6 发布准备

#### crates.io 发布

| Crate | 发布？ | 说明 |
|-------|--------|------|
| `aletheon-abi` | ✅ | 公共 API 接口 |
| `aletheon-body` | ✅ | 工具和执行层 |
| `aletheon-brain` | ✅ | 认知核心 |
| `aletheon-memory` | ✅ | 记忆系统 |
| `aletheon-runtime` | ✅ | 运行时核心 |
| `aletheon-self` | ✅ | 自我进化（核心卖点） |
| `aletheon-meta` | ✅ | 元运行时 |
| `aletheon-comm` | ✅ | 通信层 |

#### GitHub Release

- **版本号**: 语义化版本 (SemVer)
- **Release Notes**: 自动生成 + 手动补充亮点
- **二进制附件**: Linux x86_64 + aarch64 预编译二进制
- **首次发布**: v0.1.0-alpha，表明早期阶段

#### README 最终调整

发布前需要更新 README:
- 添加 crates.io 徽章
- 添加 CI 状态徽章
- 添加 License 徽章
- 添加「快速开始」链接
- 添加「贡献指南」链接
- 添加「演示」链接

#### 发布检查清单

```
□ 所有测试通过
□ clippy 无警告
□ 文档构建成功
□ LICENSE 文件存在
□ CHANGELOG 更新
□ 版本号更新
□ GitHub Release 创建
□ crates.io 发布成功
□ README 徽章正确
□ Demo 可复现
```

---

## 4. 实施计划

### 4.1 阶段划分

| 阶段 | 内容 | 时间 | 依赖 |
|------|------|------|------|
| Phase 1 | 开源基础设施 | 1 周 | 无 |
| Phase 2 | 文档体系 | 1-2 周 | Phase 1 |
| Phase 3 | CI/CD 完善 | 1 周 | 无 |
| Phase 4 | Demo + 示例 | 1-2 周 | Phase 2 |
| Phase 5 | 发布准备 | 1 周 | Phase 1-4 |

### 4.2 关键路径

```
Phase 1 (基础设施) ──┐
                     ├──→ Phase 2 (文档) ──→ Phase 4 (Demo) ──→ Phase 5 (发布)
Phase 3 (CI/CD) ────┘
```

### 4.3 里程碑

| 里程碑 | 标志 | 目标日期 |
|--------|------|----------|
| M1 | LICENSE + CONTRIBUTING 合并 | 第 1 周末 |
| M2 | 文档体系完成 | 第 3 周末 |
| M3 | CI/CD 流水线就绪 | 第 2 周末 |
| M4 | Demo 可复现 | 第 5 周末 |
| M5 | 首次发布 (v0.1.0-alpha) | 第 6-8 周末 |

---

## 5. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 文档质量不够 | 研究者不认可 | 请技术写作 review |
| Demo 不可复现 | 信任度下降 | 固定 seed + CI 验证 |
| 测试覆盖率低 | 贡献者不敢改代码 | 优先补充核心模块测试 |
| 首次发布 bug 多 | 影响声誉 | alpha 标签 + 快速迭代 |

---

## 6. 成功指标

| 指标 | 目标（3 个月内） |
|------|------------------|
| GitHub Stars | 100+ |
| Forks | 20+ |
| Issues | 10+（来自社区） |
| PRs | 5+（来自社区） |
| 文档完整性 | 100% 核心模块有文档 |
| 测试覆盖率 | 核心模块 80%+ |

---

## 7. 总结

本设计为 Aletheon 开源发布提供了完整的路线图:

1. **标准开源基础设施**: LICENSE、CONTRIBUTING、CHANGELOG 等
2. **完整文档体系**: 用户指南、架构文档、开发者文档
3. **自动化 CI/CD**: 质量门禁 + 自动发布
4. **可复现 Demo**: 展示 self-evolution 核心价值
5. **丰富示例代码**: 降低贡献门槛
6. **规范发布流程**: crates.io + GitHub Release

通过以上措施，Aletheon 将以专业、可信的姿态进入开源社区，吸引 AI/Agent 研究者和高级开发者的关注和贡献。
