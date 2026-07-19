# 本地完成情况审计

审计基线：本地 `dev` HEAD `49905483`，日期 2026-07-19。结论只覆盖当前工作树；未把未提交的 `docs/arch/` 删除与新增计入实现完成度。

## 总结

| 阶段 | 完成度 | 代码证据 | 裁决 |
|---|---:|---|---|
| Wave 0 | 5/5 | `419ffaa0`、`b488fcbe`、`5d8fc9d6`、`49905483` | 已完成，需复核 |
| Wave 1 | 0/6 | `crates/executive/src/service/turn_service.rs:20`、`turn_pipeline.rs:43` 仍并存 | 未开始 |
| Host H0–H5 | 0/6 | `crates/platform-api`、`platform-host`、各 OS crate 均不存在 | 未开始 |
| Wave 2 | 0/12 | `runtime-api`、`runtime-broker` 不存在；Pi 特判仍存在 | 未开始 |
| Wave 3 | 0/7 | `runtime-pi` 不存在；现有 `pi.rs` 与 `pi_rpc.rs` 未合并 | 未开始 |
| Wave 4 | 0/14 | `MAX_HISTORY_MESSAGES = 6` 与双 Session 表示仍存在 | 未开始 |
| Wave 5 | 0/10 | `coding-bench` 与部署 profile schema 不存在 | 未开始 |
| Hardware D0–D6 | 0/7 | `hardware-api`、`hardware-broker`、`hardware-sim` 不存在 | 未开始 |

## Wave 0 证据

| 工作项 | 提交 | 文件证据 | 复核命令 |
|---|---|---|---|
| `max_iterations=0` 无限语义 | `419ffaa0` | `crates/executive/src/impl/daemon/bootstrap/runtime.rs` | `bash scripts/cargo-agent.sh test -p executive max_iterations` |
| `file_search` cwd 与全局上限 | `b488fcbe` | `crates/corpus/src/tools/tools/file_search.rs` | `bash scripts/cargo-agent.sh test -p corpus file_search` |
| ripgrep 全局上限 | `5d8fc9d6` | `crates/corpus/src/tools/tools/grep.rs` | `bash scripts/cargo-agent.sh test -p corpus grep` |
| 架构账本 | `49905483` | `architecture-status.toml` | `test -s architecture-status.toml` |
| 架构冻结门禁 | `49905483` | `scripts/architecture-check.sh:599-664` | `bash tests/architecture_check.sh` |

Wave 0 只有在四条复核命令退出码均为 0 时保持“已完成”。任何一条失败都把 W1-01 改为“被阻塞”。

## 计划合理性裁决

| 原计划问题 | 证据 | 硬裁决 |
|---|---|---|
| Wave 1 允许保留或改名 profile 类型 | `docs/plans/2026-07-19-wave1-turn-engine.md:49,69` | 类型固定命名为 `ResolvedTurnProfile`；删除旧类型，不留别名 |
| child 可走 TurnEngine 或 Runtime | 同文件 `:183-187` | child 固定走 `TurnEngine`；Runtime Broker 在 Wave 2 只负责外部 runtime |
| TurnService 可删除或保留 re-export | 同文件 `:186` | W1-05 完成时删除文件和导出 |
| Workspace patch 事务语义未裁决 | `2026-07-19-wave2-capability-substrate.md:64` | 单文件事务；多文件请求在 schema 层拒绝 |
| exec-server contract 位置未裁决 | 同文件 `:114` | `structured_patch` 移入 `platform-api`，exec-server 依赖 `platform-api` |
| Verifier 位置未裁决 | `2026-07-19-wave3-pi-verifier.md:83` | 建独立 `runtime-verifier` crate |
| W4 recovery point 库未裁决 | `2026-07-19-wave4-state-authority.md:174` | 表建在 `events.db`，不新增数据库 |
| W4 token estimator 未裁决 | 同文件 `:274` | 首版固定 UTF-8 字符数除以 4、向上取整；不引入 tokenizer |
| Host H2–H5 只有占位 | `2026-07-19-host-platform.md:193-239` | 本执行包完整展开并加原生 OS 门禁 |
| Hardware D3–D6 只有轻量描述 | `2026-07-19-hardware-control.md:216-248` | 本执行包完整展开并加设备/实验室门禁 |
