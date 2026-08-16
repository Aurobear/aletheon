# Aletheon 计划状态

截至 2026-08-16，当前可执行计划：

- [`2026-08-16-architecture-coupling-closeout.md`](./2026-08-16-architecture-coupling-closeout.md) — **耦合收敛草稿（P0–P2）。诊断尚未 accepted。**
- [`2026-08-16-architecture-coupling-deepseek-packets.md`](./2026-08-16-architecture-coupling-deepseek-packets.md) — **DeepSeek 逐包领取。一次一个。**
- [`2026-08-15-architecture-hardening.md`](./2026-08-15-architecture-hardening.md) — **硬化清单（锁 poison / 死代码核实）。P1 膨胀拆分以 closeout 为准。**

诊断原文：[`docs/arch/evidence/2026-08-16-architecture-coupling-diagnosis.md`](../arch/evidence/2026-08-16-architecture-coupling-diagnosis.md)。

迁移收尾（七 crate 删除 + gateway 收敛 + config 拆解 + extension 合入）已在本工作区落地，尚未作为独立提交。相关历史设计文件已删除。

> ⚠️ 工作区仍有大量未提交迁移路径。提交由用户明确授权后按包执行。禁止 `git add -A`。

## 给执行者

1. DeepSeek：只贴 packets 文件 §0 的第一句话，领取 `CC-P0.1`。不要从旧 Kernel V2 / Luna / 08-14 文件重新开任务。
2. 当前工作区和生产调用链是代码事实源；locator 漂移时先更新清单，不要为旧文本增加兼容层。
3. 一次只做一个编号步骤，验证后停下。
4. 本包完成定义见 closeout P0–P2。Finding F 证据检查不在本包。安装态 `sudo deploy` 不在本包。
5. 禁止以减少 crate 数量或跑通旧 architecture grep 代替本包验证命令。
6. 计划与代码冲突时先改计划，不要加 compatibility seam 让文本 gate 变绿。
7. 未提交工作区很大。禁止 `git add -A`。
