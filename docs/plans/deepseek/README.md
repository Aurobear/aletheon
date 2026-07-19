# DeepSeek 单任务执行包

## 强制协议

1. 只执行“当前任务”字段指向的文档；禁止跨文档修改。
2. 开始前运行 `git status --short`。已有改动不是当前任务创建时，禁止覆盖、删除、暂存或提交。
3. 严格按任务编号执行。任一前置检查失败，记录命令和输出并停止。
4. Rust 命令必须使用 `bash scripts/cargo-agent.sh <参数>`；禁止直接运行 `cargo`。
5. 先写失败测试，再写最小实现，再运行指定验证。
6. 只暂存任务“提交边界”列出的文件。提交前运行 `git diff --cached --check` 并检查 `git diff --cached`。
7. 非平凡提交必须使用 conventional subject、空行、问题/方案说明和改动列表。
8. 验收命令未全部通过时禁止提交，禁止把失败写成完成。
9. 文档与代码冲突时禁止猜测。记录 `文档路径:行` 与 `代码路径:行`，交由用户裁决。
10. 遇到标记“前置条件未满足，禁止执行”的任务，禁止创建骨架、占位实现或跳过测试。

## 状态定义

| 状态 | 含义 | 允许动作 |
|---|---|---|
| 已完成 | 本地代码和提交证据均存在 | 只运行复核命令 |
| 待执行 | 所有前置条件已满足 | 按文档执行 |
| 被阻塞 | 软件前置阶段未完成 | 只复核解锁条件 |
| 前置条件未满足，禁止执行 | 缺少指定 OS、设备、实验室或凭据 | 禁止改代码 |

## 唯一执行顺序

| 顺序 | 文档 | 当前状态 | 解锁条件 |
|---:|---|---|---|
| 0 | `00-local-completion-audit.md` | 已完成审计 | 无 |
| 1 | `01-wave1-turn-engine.md` | 待执行 | Wave 0 复核通过 |
| 2 | `02-host-h0-h1.md` | 被阻塞 | Wave 1 全部验收通过 |
| 3 | `03-wave2-capability-substrate.md` | 被阻塞 | Host H1 与 Wave 1 全部验收通过 |
| 4 | `04-wave3-pi-verifier.md` | 被阻塞 | Wave 2 全部验收通过 |
| 5 | `05-wave4-state-authority.md` | 被阻塞 | Wave 3 全部验收通过 |
| 6 | `06-wave5-profiles-eval.md` | 被阻塞 | Wave 4 全部验收通过 |
| 7 | `07-host-h2-h5.md` | 前置条件未满足，禁止执行 | Wave 5 完成且对应原生 OS runner 已登记 |
| 8 | `08-hardware-d0-d6.md` | 被阻塞 | Wave 5 完成；各硬件子阶段另受设备门禁 |

**当前任务：** `01-wave1-turn-engine.md` 的 W1-01。

## 每次执行的固定回报

```text
TASK: <任务编号>
STATUS: PASS | FAIL | BLOCKED
CHANGED_FILES: <逐行列出>
TESTS: <命令及退出码>
COMMIT: <hash；未提交写 NONE>
BLOCKER: <无则写 NONE>
NEXT_TASK: <唯一任务编号；无则写 NONE>
```
