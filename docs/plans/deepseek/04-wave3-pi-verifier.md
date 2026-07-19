# Wave 3：Pi Runtime 与 Completion Verifier

**状态：** 被阻塞。**解锁条件：** W2-13 通过。

| ID | 修改范围 | 唯一实现要求 | 验证 |
|---|---|---|---|
| W3-01 | 创建 `runtime-pi` manifest/transport/session | RuntimeId 固定 `pi/coding`，alias 固定 `pi` | `bash scripts/cargo-agent.sh test -p runtime-pi manifest` |
| W3-02 | resident JSONL session + isolated worktree | 每 job 一个 worktree 和初始 checkpoint；捕获 stdout/stderr | `bash scripts/cargo-agent.sh test -p runtime-pi session` |
| W3-03 | 标准事件、diff、artifact、cancel | cancel 终止进程树；terminal event 恰好一个 | `bash scripts/cargo-agent.sh test -p runtime-pi contract` |
| W3-04 | model/prompt/tools/budget 映射 | 未能映射任一字段时 prepare 失败 | `bash scripts/cargo-agent.sh test -p runtime-pi launch_policy` |
| W3-05 | 创建独立 `runtime-verifier` crate | CompletionStatus 六态固定；Verifier 不依赖 Pi | `bash scripts/cargo-agent.sh test -p runtime-verifier` |
| W3-06 | 测试选择与 receipt 聚合 | Rust 映射固定为改动 crate的最窄 test target；未知文件运行该 crate lib tests | `bash scripts/cargo-agent.sh test -p runtime-verifier test_selection` |
| W3-07 | 验证失败回送同 session | 未通过不得 Verified；达到预算返回 BudgetExhausted | `bash scripts/cargo-agent.sh test -p executive verification_repair_loop` |
| W3-08 | Broker 默认 coding runtime 与 health | coding profile只选择健康的 pi/coding；无健康实例启动失败 | `bash scripts/cargo-agent.sh test -p executive pi_broker` |

**外部依赖门禁：** 真实 Pi E2E 只有在 executable 路径、版本、SHA-256 和固定 argv 已写入测试配置时执行。门禁未满足时只允许 mock transport contract tests；禁止宣称 W3-08 完成。
