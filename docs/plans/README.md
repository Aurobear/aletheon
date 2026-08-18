# Aletheon 计划状态

截至 2026-08-17，当前可执行计划：

- [`2026-08-16-aletheon-wiring-ownership-migration.md`](./2026-08-16-aletheon-wiring-ownership-migration.md) — **规范源；ACTIVE；M0–M6 已完成，M7.1/7.2/7.3/7.4(SCOPED)/7.5 已完成，当前收口 M8；后续核心、测试和 deploy 已获统一授权。**
- [`2026-08-17-aletheon-wiring-migration-deepseek-execution-handoff.md`](./2026-08-17-aletheon-wiring-migration-deepseek-execution-handoff.md) — **DeepSeek 后续执行入口；包含当前稳定点、M6.43 以及 M7–M10 文件级步骤、验证和回退。规范冲突时以迁移规范源及当前代码为准。**
- [`2026-08-16-architecture-coupling-closeout.md`](./2026-08-16-architecture-coupling-closeout.md) — **耦合收敛草稿（P0–P2）。诊断尚未 accepted。**
- [`2026-08-16-architecture-coupling-deepseek-packets.md`](./2026-08-16-architecture-coupling-deepseek-packets.md) — **DeepSeek 逐包领取。一次一个。**
- [`2026-08-15-architecture-hardening.md`](./2026-08-15-architecture-hardening.md) — **硬化清单（锁 poison / 死代码核实）。P1 膨胀拆分以 closeout 为准。**

诊断原文：[`docs/arch/evidence/2026-08-16-architecture-coupling-diagnosis.md`](../arch/evidence/2026-08-16-architecture-coupling-diagnosis.md)。

旧 crate 删除和物理 cutover 已在当前工作区出现，但 wiring 的 policy、authority、adapter、
host 所有权尚未完成迁移。不得把物理搬迁等同于架构收敛；当前迁移基线见
[`2026-08-17-wiring-migration-m0-baseline.md`](../arch/evidence/2026-08-17-wiring-migration-m0-baseline.md)。

> ⚠️ 工作区仍有大量未提交迁移路径。提交由用户明确授权后按包执行。禁止 `git add -A`。

## 给执行者

1. Wiring migration 按 M0→M10 顺序执行；每次只推进一个可回退 packet。
2. 当前工作区和生产调用链是代码事实源；locator 漂移时先更新清单，不要为旧文本增加兼容层。
3. 后续核心、测试、fixture 和最终 deploy 已统一授权；核心 packet 仍须记录文件、符号、运行风险和验证证据。
4. Architecture checker/fixture 已启用，不得以统一授权绕过边界或行为 gate。
5. 禁止以减少 crate 数量或跑通旧 architecture grep 代替本包验证命令。
6. 计划与代码冲突时先改计划，不要加 compatibility seam 让文本 gate 变绿。
7. 未提交工作区很大。禁止 `git add -A`。
