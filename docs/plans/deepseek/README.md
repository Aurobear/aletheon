# Aletheon 深化架构分析 — 索引

> **日期:** 2026-07-17（README 于同日复核整理）
>
> **本目录含两类文档，请勿混用：**
> - **A. 审计报告（`01`–`09`）** — 已完成的**代码级验证快照**，历史记录。
> - **B. 前瞻计划（`2026-07-17-*.md`）** — 待执行 / 部分执行的**工程计划**，含状态复核。
>
> **方法:** 独立 `Explore` Agent 并行逐行读源码，逐条验证声明，每个发现附 `crates/<name>/src/<path>.rs:line` 锚点，分类为 **CONFIRMED / FALSE·STALE / EXAGGERATED**。

---

## A. 审计报告（代码级验证，历史快照）

### 第一轮：架构文档 vs 代码实际
| 编号 | 报告 | 核心发现 | 文档准确度 |
|------|------|----------|------------|
| 01 | [Executive 耦合现状](./01-executive-coupling-reality.md) | `TurnPipeline` 7/14 concrete；`TurnRuntimeResources` 17 concrete + 8 Mutex | 文档**低估**耦合 |
| 02 | [Agora 事务完整性](./02-agora-transaction-verification.md) | 5 个 CRITICAL bug **已全部修复**；competition/broadcast 生产级 | 文档**严重低估**成熟度 |
| 03 | [Dasein 内部分裂](./03-dasein-split-reality.md) | 6 个 bug 中 5 个**已修复**；真正 gap 是 SelfField/DaseinModule 无因果连接（已补充校正） | 文档**大面积过时** |
| 04 | [CI 架构执行](./04-ci-architecture-enforcement.md) | 485 行 fitness gate + 20+ 删除门 + allowlist 回归防护 | 文档**低估**严格程度 |

### 第二轮：工程能力
| 编号 | 报告 | 核心发现 |
|------|------|----------|
| 05 | [SubAgent Runtime 与 Pi 集成](./05-subagent-pi-runtime-capability.md) | 3 生产 Runtime；Pi 7 阶段 fail-closed 管线；**无 CodexRuntime** |
| 06 | [工具执行、测试、工程硬实力](./06-tool-execution-testing-maturity.md) | 21 工具生产级；7 阶段安全管线；5 sandbox；3 provider；2,766 tests；仅 7 TODO |
| 07 | [MCP、Google、外部集成、IPC](./07-external-integration-maturity.md) | MCP/Google/Telegram 生产级；IPC Unix socket Tier 1 |
| 08 | [工程成熟度综合评估](./08-engineering-maturity-assessment.md) | 综合评分 + 能做/不能做清单（意识闭环结论**已校正**） |

### 第三轮：全量审计
| 编号 | 报告 | 核心发现 |
|------|------|----------|
| 09 | [全量计划文档审计](./09-full-plan-audit.md) | 50 份文档逐份核查：78% 已实现、12% 部分过时、8% 设计、2% 未来 |

---

## B. 前瞻计划（状态已复核 2026-07-17）

| 计划 | 状态 | 复核结论 |
|------|------|----------|
| [capability-hardening-roadmap](./2026-07-17-capability-hardening-roadmap.md) | **INDEX** | 纯索引；§6 两个 quick-win（max_iterations、clock）**已完成** |
| [capability-activation-and-agent-profiles](./2026-07-17-capability-activation-and-agent-profiles-plan.md) | **OUTSTANDING（前提已校正）** | `.md` profile **无 frontmatter**、loader 需 frontmatter → 现 `.md` 不授权；真实授权源是 `.toml`（只 3 工具） |
| [tool-execution-hardening](./2026-07-17-tool-execution-hardening-plan.md) | **OUTSTANDING** | 仅 `bash_exec` 走 sandbox（`runner.rs:400`）；`SandboxProfile` 已定义但无消费者 |
| [structured-code-editing](./2026-07-17-structured-code-editing-plan.md) | **OUTSTANDING（规格最完整）** | Phase 1 自包含于 corpus，验收 P1.1–P1.13 具体 |
| [mcp-integration](./2026-07-17-mcp-integration-plan.md) | **OUTSTANDING** | 两份 `McpServerConfig` 并存待统一；§3.7 疑似 trust-mapping 反转需验证 |
| [multi-user-runtime-m0-m2](./2026-07-17-multi-user-runtime-m0-m2.md) | **COMPLETE** | M0–M2 已实现；剩 **M3–M5**（见设计文档） |
| [codex-inspired-multi-user-runtime-design](./2026-07-17-codex-inspired-multi-user-runtime-design.md) | **设计（M0-M2 已实现）** | M3–M5 是真正剩余工作 |

### B+ 新增设计/计划（2026-07-17）
| 计划 | 主题 | 状态 |
|------|------|------|
| [conscious-core-engineering-plan](./2026-07-17-conscious-core-engineering-plan.md) | **自我意识工程化**：把 Dasein→Agora 闭环从「观察-提交」升级为「care 改变行为」，并纳入 SelfField（R1–R4） | 设计 only |
| [platform-driver-hardware-control-plan](./2026-07-17-platform-driver-hardware-control-plan.md) | **多平台适配 + 实际硬件控制**：现有为桌面 HAL，硬件控制需从零建 effector/fieldbus 栈 + RT 回路 | 设计 only |
| [kernel-application-layer-separation-plan](./2026-07-17-kernel-application-layer-separation-plan.md) | **内核/应用层分离**：kernel 已是干净机制层，剩「封边」——只经 fabric trait 交互 + CI 锁定 | 设计 only |

---

## 核心结论（2026-07-17 校正版）

1. **架构文档显著落后于代码现实。** Agora 的 5 个 "CRITICAL bug" 已修复、Dasein event-sourced ledger 完整运作，但文档从未更新；另一方面 Executive 的 concrete 耦合比文档更严重。

2. **工程硬实力远超文档描述。** 21 工具、7 阶段安全管线、5 sandbox、3 provider、3 SubAgent Runtime、2,766 测试、仅 7 TODO、零无条件 ignored、零生产 `unimplemented!()`。

3. **意识闭环已存在，但只「观察-提交」，未「仲裁」。**（**本次重大校正**）原判「Dasein-Agora 闭环不存在于生产路径」**不准确**——闭环实际已无条件接线（`bootstrap/request.rs:657-676` → `turn_pipeline.rs:215-225` → `governed_capability.rs:148-188` → `conscious_core_coordinator.rs:404-446`）。真实 gap 是：`select_action` 恒定胜出且不改变真实调用、`CareStructure::determine_action()` 空转、SelfField 被排除在闭环外。工程化方案见 `conscious-core-engineering-plan`。

4. **真正缺失的不是工程能力，而是闭环的「仲裁质量」与最独特的自我觉察部分。** 让 care 状态能改序/软否决行为、把 SelfField 纳入闭环，是从 "功能完整的 Agent 运行时" 进化为 "具有持续自我觉察的 Agent 运行时" 的关键。

5. **Pi Agent 已具备实际编码能力。** Fail-closed 7 阶段管线 + git worktree 隔离 + bubblewrap sandbox + SHA-256 验证 + ControlledApply 原子回滚——完整、安全、零 TODO、有 393 行真实 git+bubblewrap 测试。**不存在 CodexRuntime**（Pi 是独立 agent，非 Codex 包装）。

6. **CI 架构执行已超文档描述。** 20+ 删除门 + 7 类扫描 + 依赖图强制 + baseline 回归防护。盲区：corpus 9 处 `SystemClock`、dasein 2 处直接 `Tool::execute` 未纳入 enforcement scope。

7. **硬件控制是空白。** 现有 "platform driver" 是桌面/OS 自动化 HAL，全仓库无 EtherCAT/PDO/GPIO/serial/CAN/ROS/RT 回路。控制实际硬件需从零构建，见 `platform-driver-hardware-control-plan`。

---

## 文档整理状态（2026-07-17）

本次复核（4+2 只读 Agent 逐行验证代码）对本目录做的整理：
- **校正过时结论**：`03`/`08`/`README` 中「意识闭环不存在于生产路径」→ 已改为「闭环已接线、gap 是仲裁质量」。
- **标注计划状态**：`multi-user-runtime-m0-m2` 标 COMPLETE；`capability-activation` 修正 `.md`/frontmatter 前提错误；`capability-hardening-roadmap` 标注已完成的 quick-win。
- **新增 3 份设计**：意识工程化、硬件控制、kernel/app 分离（均 design-only，实现需另行批准）。

上游架构文档（`docs/plans/2026-07-15/16-*.md`）的同步状态见各文件顶部 "Code-Reality Update" 章节，原始计划内容完整保留。
