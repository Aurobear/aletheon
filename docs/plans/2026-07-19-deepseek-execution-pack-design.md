# DeepSeek 执行包设计

## 目标

保留 `docs/arch/` 与现有 `docs/plans/` 作为需求和决策来源，在
`docs/plans/deepseek/` 建立能够逐项执行、逐项验证、逐项提交的任务包。

## 固定决策

- 执行顺序固定为 Wave 0 审计 → Wave 1 → Host H0/H1 → Wave 2 → Wave 3 → Wave 4 → Wave 5 → Host H2–H5 → Hardware D0–D6。
- 每次只执行一份任务文档，前一份未验收时不得开始后一份。
- 现有实现与上游计划冲突时停止修改，在执行记录中列出双方锚点并请求裁决。
- 无法在当前 Linux 环境验证的 Windows、macOS 与真实硬件任务仍完整描述，但状态固定为“前置条件未满足，禁止执行”。
- 所有 Rust 构建和测试只通过 `bash scripts/cargo-agent.sh` 执行。
- 每份任务必须明确状态、前置条件、修改文件、操作步骤、验证命令、期望结果、禁止事项和提交边界。

## 输出结构

```text
docs/plans/deepseek/
├── README.md
├── 00-local-completion-audit.md
├── 01-wave1-turn-engine.md
├── 02-host-h0-h1.md
├── 03-wave2-capability-substrate.md
├── 04-wave3-pi-verifier.md
├── 05-wave4-state-authority.md
├── 06-wave5-profiles-eval.md
├── 07-host-h2-h5.md
└── 08-hardware-d0-d6.md
```

## 验收

- `docs/plans/deepseek/README.md` 给出唯一执行顺序和状态机。
- 本地完成情况必须由提交、文件或符号证据支持。
- 每个未完成阶段都有唯一入口条件和唯一完成条件。
- 文档不得让执行者自行选择接口位置、实现路径、测试命令或提交范围。
