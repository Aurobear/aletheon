# Grok 二次审核残留项：wiring 所有权迁移计划

状态：**ADDRESSED — R1/R2 与 N1–N4 已写入被审计划，待 Codex 最终验证；未授权 Rust**

日期：2026-08-17

审核对象：`docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md`

审核时计划状态行：`DRAFT — Grok residual R1/R2 与 N1–N4 已写入，待 Codex 最终审核；未授权实施 Rust 代码`

批准设计计划 SHA-256：`c9b927bd7a21a01309d05ae31bbc41ff8024988fb0b3417cf799f3a673f76039`

用途：保留修订前残留项与修订依据；交给 Codex 验证闭合。本文不是实施授权。

前一轮完整意见：`docs/plans/2026-08-16-aletheon-wiring-ownership-migration-grok-review.md`

---

## 0. 结论

以下 Verdict 是修订前审核结论；处置状态以文首 `ADDRESSED` 和批准设计 SHA 为准。

```text
Verdict: REVISE

Blocking findings:
1. [med] §3.3 / M8 — wiring/core_runtime.rs 未入表
2. [med] §3.4 / M2.3 / M7.4 — worktree/command 仍写 “platform 或 corpus”

Non-blocking findings:
1. [low] §3.2 host 树与 §3.3/M8 不一致
2. [low] M5 验证未断言零 GenerationFence 复本
3. [low] adapters-inference 内 core_rpc 与 HTTP provider 必须分模块
4. [low] §0.4 仍写 “P6 Turn/Agent”
```

上次 5 条 blocking（依赖白名单、TurnPipeline 整包迁移、owner matrix 大面积遗漏、Goal 工件/Worker、§9 挡不住 Executive 2.0）已在修订稿闭合。不要再重开那些方向争论。只补本文 §1–§2。

---

## 1. Blocking

### R1. `wiring/core_runtime.rs` 未写入 §3.3 / M8

**计划缺口：** §3.3 矩阵和 M8「其余顶层 wiring 路径」覆盖了 governed_review、workspace_trust、core_rpc、readiness、doctor、exec、user_runtime、embodiment、evolution、approval_service、cognitive_runtime、mode_router、domain、extension。全文没有 `core_runtime`。

计划自己写了：

> 该矩阵必须覆盖 `wiring/` 每个顶层文件/目录。未入表模块会阻止 M9。

因此这不是文风问题，是计划自洽性失败。

**代码事实：** `crates/aletheon/src/wiring/core_runtime.rs`，184 行。

| 符号 | 行 | 应去 |
|---|---|---|
| `RegistryInferencePort` 及 `InferencePort` 实现 | `:22-111` | `adapters-inference`（与 provider registry / backpressure 同 crate） |
| `MachineInferenceRuntime` | `:115-184` | `aletheon::host::core` |
| `bootstrap` 读 host config、建 socket 目录、绑 `CorePeerPolicy`、启动 server | `:123-161` | host 生命周期，不是 adapter policy |
| system core 拒绝 user-scoped 凭据 | `:137-139` | **必须保持 fail-closed** |

`:137-139` 现文：

```text
if telegram.enabled || supplemental.enabled || !mcp_servers.is_empty() {
    anyhow::bail!("system core configuration contains user-scoped integration credentials");
}
```

`wiring.rs:71-80` 的 `run_core` 调用 `core_runtime::MachineInferenceRuntime::bootstrap`。core RPC transport 已正确派给 `adapters-inference::core_rpc`；缺的是这个 **machine-core composition 对象** 的 owner。

**要求修正：**

1. §3.3 增一行，例如：

   | 功能 | Application | Authority | Adapter | aletheon |
   |---|---|---|---|---|
   | Machine core runtime | 无业务 use case | cognit inference/backpressure contract | `adapters-inference` 的 `RegistryInferencePort` / backpressure | `host::core` 拥有 `MachineInferenceRuntime` 的 config/socket/peer-policy/lifecycle |

2. M8 表增一行：`core_runtime.rs` → 拆到 `adapters-inference` + `aletheon::host::core`；旧文件删除。
3. 明确保留「system core 不得装载 telegram / supplemental memory / MCP 用户凭据」。
4. §3.2 `host/` 树补 `core.rs` 已有名字的话，写明它接收的是 `MachineInferenceRuntime`，不是空壳。

### R2. worktree / command 仍是「或」

计划禁止 M0 ledger 出现“或”，正文却留下双候选：

| 位置 | 原文 |
|---|---|
| §3.4:341 | `worktree recovery 不进入 adapters-agent-backend，由 platform/Corpus 对应 port 实现` |
| M2.3:665 | `` `corpus` 或 `platform`：受约束 patch、Git/worktree、command implementation `` |
| M7.4:1007 | `worktree recovery 移到 platform/Corpus 对应 port adapter` |

当前 seam：`wiring/adapters/{verification_command,worktree}.rs`，以及 approval 路径上的 `ManagedWorktreeCleaner`。

**要求修正：** 在计划里选定唯一 adapter，不要留给实施临场决定。建议：

- 受约束 `cargo`/`git`/verification command → `platform`（OS 进程/可执行文件定位；现 `verification/mod.rs:50-51` 用 `which::which`）
- worktree create/clean/recovery → `corpus`（受治理工作区/patch 执行体）或 `platform`（纯文件系统/进程），**只留一个**
- `adapters-agent-backend` 明确不拥有 worktree
- `adapters-sqlite` 只存 worktree/command 的 durable receipt，不执行副作用

选定后同步改 §3.4、M2.3、M7.4，三处不得再出现“或”。

---

## 2. Non-blocking（建议一并改，不阻止下一轮 APPROVE）

### N1. §3.2 目标树不完整

§3.2 `host/` 只有 `core.rs`、`user_daemon.rs`、`exec.rs`、`unix_server.rs`。

§3.3 / M8 还指定了 `readiness`、`doctor`、`goal_scheduler`、`cli::extension`。

以 M8 为权威，把 §3.2 补全，避免实施时按 §3.2 把这些文件塞进 composition。

### N2. M5 缺 GenerationFence 静态门禁

M5 正文要求 cutover 后 `GenerationFence` 只由 `runtime::SettlementEngine` 使用，wiring/adapter 不维护第二份判定。验证命令没有对应 `rg`。

建议补：

```bash
! rg -n 'GenerationFence' crates/aletheon crates/adapters crates/application/src --glob '*.rs'
rg -n 'GenerationFence' crates/runtime/src --glob '*.rs'
```

（若 adapter 测试夹具必须提到该类型，按 `#[cfg(test)]` 分界，与 §4.2 一致。）

### N3. `adapters-inference` 内部分界

core_rpc 进入 `adapters-inference` 可接受：`CoreRpcClient` 实现 cognit `InferencePort`。crate 内必须：

- `core_rpc/` 与 `anthropic` / `openai` / `ollama` 分模块
- Unix peer-policy 不得进入 HTTP provider
- host 只保留 socket 路径、配置、server 生命周期

### N4. §0.4 过期编号

「P6 Turn/Agent 核心生命周期范围」应改为 **M5/M6**。

---

## 3. 已闭合、不要回退的修订

修订稿已正确吸收这些点。补 R1/R2 时不要改回去：

1. §4.1：当前只允许 `application -> {contracts, runtime, 窄 kernel API}`；`mnemosyne/dasein/cognit/metacog` 因经 `platform` 回边而禁止；每个改 manifest 的 packet 用完整 `cargo metadata` 图，不用 `--no-deps`。
2. A4 图是角色图，不是 domain 白名单。
3. M6：`application::turn` 拥有 provider-neutral 用例顺序；禁止整包搬 `TurnPipeline`；`daemon_react` 拆 cognit 核心 + adapters；`notification_sender` 不得进入 application 类型。
4. Goal：`GoalArtifactStore` port + platform FS adapter；`GoalWorker` 拆 `advance_once` / host scheduler / gateway progress / platform quota；合并已有 `application/src/goal_*.rs`；不新建 `application::admin`。
5. `adapters-runtime` 已改名为 `adapters-agent-backend`。
6. Google SQLite 唯一 owner 为 `adapters-google`；Corpus Google 只做工具执行。
7. §9：传递依赖、authority census、effect census、composition 禁状态分支、`runtime::orchestration` 先审计再删/并、source gate 与 behavior 分栏。
8. A9 / M6 / M10：runtime 唯一 mint canonical ID；contracts 转换为显式可失败边界。
9. M10 已覆盖 principal fail-closed、fence、cancel/deadline、artifact hash、sensitivity、transient grant、robot target。

---

## 4. 给修订者的最小补丁

只改计划，不写 Rust：

1. §3.3 增加 Machine core runtime 行（见 R1 表）。
2. M8 增加 `core_runtime.rs` 去向，并写明 `:137-139` fail-closed 必须保留。
3. 在 §3.4、M2.3、M7.4 为 worktree 和 verification command 各选一个 adapter，删掉所有“或”。
4. 顺手改 N1–N4。

补完后再走计划 §10。R1/R2 闭合即可把设计标为 **APPROVE**。那仍只表示设计可实施；真正改代码还要 §0.4 的另行授权。
