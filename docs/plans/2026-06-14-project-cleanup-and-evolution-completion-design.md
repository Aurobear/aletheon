# Aletheon 项目清理与进化闭环完善

**日期:** 2026-06-14
**状态:** 已批准
**目标:** 清理残留命名、打通进化闭环、架构增强

---

## Phase 1: 清理残留命名

### 1.1 systemd 服务文件
- 重命名 `systemd/agentd.service` → `systemd/aletheond.service`
- 更新所有路径: `/usr/bin/agentd` → `/usr/bin/aletheond`
- 更新 socket: `/run/agentd/agentd.sock` → `/run/aletheond/aletheond.sock`
- 更新 EnvironmentFile: `/etc/agentd/env` → `/etc/aletheond/env`
- 更新描述: "OS-Agent Daemon" → "Aletheon Daemon"

### 1.2 Cargo.toml 统一
- 所有 crate 统一使用 `version.workspace = true` 和 `edition.workspace = true`
- 所有 crate 使用 `license.workspace = true`
- 涉及: aletheon-abi, aletheon-body, aletheon-brain, aletheon-comm, aletheon-memory, aletheon-runtime, aletheon-self

### 1.3 README 扫描
- 扫描根 README.md 和所有 docs/ 下文件，替换残留 argos/agentd 引用

---

## Phase 2: 进化闭环完善

### 2.1 技能自动生成器 (SkillExtractor)

**灵感:** Hermes Agent 的自主技能创建

**设计:**
- 位于 `crates/aletheon-self/src/core/skill_extractor.rs`
- 输入: Vec<ReflectionEntry>
- 检测模式:
  - 反复同类任务 → 生成技能模板
  - 成功策略重复出现 → 提取为可复用技能
  - 失败后找到新方法 → 生成"避坑指南"技能
- 输出: Markdown 技能文件到 `~/.aletheon/skills/`
- 格式兼容 Claude Code skills (SKILL.md + references/)

### 2.2 进化触发器 (EvolutionTrigger)

**灵感:** SOAR impasse-driven chunking

**设计:**
- 位于 `crates/aletheon-self/src/core/evolution_trigger.rs`
- 三种触发模式:
  - **事件驱动:** 连续 N 次失败 (impasse) → 自动触发反思 + 行为调整
  - **定时驱动:** 可配置间隔 (默认 6h) 运行进化周期
  - **手动驱动:** `/evolve` 命令
- 进化周期流程:
  1. 收集近期 ReflectionEntry
  2. ExperienceSummarizer 检测模式
  3. SkillExtractor 提取可复用技能
  4. 行为调整 (CareLayer weights, BoundaryLayer rules)
  5. 记录 EvolutionLogEntry
  6. 验证效果

### 2.3 效果验证 (EvolutionValidator)

**设计:**
- 位于 `crates/aletheon-self/src/core/evolution_validator.rs`
- 调整前记录 baseline:
  - 成功率 (成功/总任务)
  - 平均置信度
  - 常见失败模式
- 调整后 (N 次交互后) 对比:
  - 成功率提升 > 5% → 有效
  - 成功率下降 → 回滚
  - 无显著变化 → 标记为待观察
- 回滚机制: SelfFieldStore 中保存调整历史，可恢复

---

## Phase 3: 架构增强

### 3.1 层级记忆块 (MemoryBlocks)

**灵感:** Letta 的 labeled memory segments

**设计:**
- 扩展 `crates/aletheon-memory` 添加 memory block 系统
- 三个持久化块:
  - `persona` — agent 身份/价值观 (稳定，SelfField Identity 控制)
  - `human` — 用户画像 (渐进更新，从交互中学习)
  - `learned` — 经验知识 (快速增长，来自反射)
- 每个块有: label, content, max_tokens, last_updated
- 注入到 LLM system prompt 的固定位置

### 3.2 双模型协作 (DualModel)

**灵感:** Reasonix executor + planner 模式

**设计:**
- 扩展 `crates/aletheon-brain` 添加 dual model 协调器
- Planner 模型: 分析任务、制定计划 (read-only, 不执行工具)
- Executor 模型: 执行计划、报告结果 (可调用工具)
- 共享缓存稳定的系统前缀 (persona + memory blocks)
- 配置: 可选择单模型模式 (当前行为) 或双模型模式

### 3.3 代码图谱 (CodeGraph)

**灵感:** Reasonix CodeGraph (tree-sitter)

**设计:**
- 新 crate `crates/aletheon-codegraph` 或在 `aletheon-body/tools/` 中添加
- tree-sitter 解析项目 AST
- 功能:
  - 符号查询: 函数/类型/变量定义位置
  - 调用图: 谁调用了谁
  - 引用查找: 某符号在哪里被使用
- JSON-RPC 接口暴露给 daemon
- CLI 命令: `/symbols`, `/callers`, `/refs`

---

## 实施顺序

1. Phase 1 (清理) — 单个 developer agent
2. Phase 2 (进化闭环) — 3 个并行 developer agents
3. Phase 3 (架构增强) — 3 个并行 developer agents

每阶段完成后运行测试验证。
