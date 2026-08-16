# Aletheon 架构完善清单(Codex Handoff)

状态:`ACTIVE — 待 Codex 执行`
日期:2026-08-15
分支:`fix/tui-live-agent-inspector`
基线:HEAD `8a8138613162f2495ca626f8dda0bfcb33ec4ce0`(未提交工作区 1777 路径)
前置:迁移收尾已完成(七 crate 删除、329 测试套件全过、sudo deploy 验收通过、真实 LLM 请求通过)

> 本文档是**问题清单 + 执行顺序**,不是新架构设计。所有 locator 已在本会话核实。
> 一次只做一项,验证后停下等人审。禁止 `git add -A`。

## 当前健康基线(已核实,2026-08-15)

- 编译:`cargo check --workspace` 通过
- 测试:cognit 439 / runtime 224 / 全 workspace 329 套件零失败
- 安装态:daemon active、NRestarts=0、`/usr/bin/aletheon` 官方 socket 真实 LLM 返回 `ALETHEON_DEPLOYMENT_OK`
- 依赖方向:无循环;runtime 只依赖 contracts;无 crate 依赖 aletheon

## 问题总览

| # | 严重度 | 问题 | 位置 |
|---|---|---|---|
| P0 | 🔴 | 核心 writer 锁 poison 无恢复(守护进程崩溃风险) | `crates/runtime/src/agent_writer.rs`、`turn_writer.rs` |
| P1 | 🔴 | aletheon crate 膨胀:81,872 行占全仓 24%,wiring/application 3 万行是 executive 机械搬入 | `crates/aletheon/src/wiring/application/`(69 文件 29,709 行) |
| P2 | 🟡 | 无外部消费者的 pub 模块(潜在死代码) | `conscious`、`conscious_action`、`conscious_core_coordinator`、`daemon_turn`(详见下) |
| P3 | 🟡 | 工作区 1777 个未提交路径,迁移成果从未提交 | 全仓 |

---

## P0:核心 writer 锁 poison 无恢复

### 事实(已核实)

`crates/runtime/src/agent_writer.rs` 生产代码 88 处 `.unwrap()` + 53 处 `.lock().unwrap()` + 1 处 `.write().unwrap()`;`turn_writer.rs` 13 处 unwrap。**全仓 `unwrap_or_else(|e| e.into_inner())` / PoisonError 恢复:0 处**。

### 风险

- `agent_writer`/`turn_writer` 是每次 Agent/Turn 生命周期都经过的生产权威 writer
- 任一内部 `std::sync::Mutex` 中毒(panic 发生在持锁期间)→ 后续所有 `lock().unwrap()` 立即 panic → **整个 daemon 崩溃**
- systemd 虽会重启,但运行中的 session/turn 全部丢失,且可能进入崩溃循环

### 执行要求

1. **`crates/runtime/src/agent_writer.rs`**:将 `lock().unwrap()` 改为显式 poison 处理:
   - 读锁场景:`.lock().unwrap_or_else(|poisoned| poisoned.into_inner())`(读取不受毒影响,继续服务)
   - 写锁场景:同样 recover,但需确认写入的临时状态在 panic 前未提交(若 panic 发生在提交前,recover 安全)
2. **`crates/runtime/src/turn_writer.rs`**:同上
3. 保持行为不变:recover 只是**避免 panic**,不改变生命周期语义
4. 不要把所有 unwrap 一刀切替换——只处理**锁 unwrap**;对 `VecDeque::pop_front().unwrap()` 等逻辑不变量,保留(或改为 `expect("...")` 带明确消息)
5. 加回归测试:构造 poisoned Mutex,验证 writer 不 panic 且能继续服务

### 验证

```bash
bash scripts/cargo-agent.sh check -p runtime
bash scripts/cargo-agent.sh test -p runtime --lib
bash scripts/cargo-agent.sh test --workspace 2>&1 | grep -E "test result: FAILED" | head
git diff --check
```

---

## P1:aletheon crate 膨胀(wiring/application 3 万行)

### 事实(已核实)

```
crates/aletheon/src/wiring/application/  69 文件  29,709 行
crates/aletheon/src/wiring/adapters/     33 文件  12,673 行
aletheon 全 crate                       236 文件  81,872 行  ← 全仓 335,392 行的 24%
```

这些代码在 Packet 3 从 `executive::application/adapters` **整体机械搬入**,只是"换了个家"。按权威拆分(goal 5,658 行、agent_control 5,721 行、turn_pipeline 2,524 行、turn_coordinator 1,382 行、admin_service 1,189 行等)仍应回到真正的 owner。

### 目标(最终态)

```
aletheon = 纯 composition root(组装 + config + extensions)
runtime = Agent/Session/Turn 语义权威
application = use-case facade
adapters-sqlite = 持久化实现
kernel = admission/capability/process
```

### 执行要求(按权威逐刀,每刀独立验证)

按此顺序(依赖最少的先迁):

1. **`session_service`(10 行,已是 re-export)→ 确认指向 `runtime::session_service`**,若 wiring 内还有实体实现,删除
2. **`governed_capability`(15 行,已是 re-export)→ 确认指向 `kernel::capability::governed`**
3. **`harness_factory`(re-export)→ 确认指向 `cognit::harness::factory`**
4. **`memory_gateway`(re-export)→ 确认指向 `mnemosyne::memory_gateway`**
5. **`cognitive_role_workflow`(1,750 行)→ `agora`**(依赖 agora::cognitive_workspace,零 executive 依赖,已核实)
6. **`capability_benchmark`(594 行)→ `application`**(依赖 contracts+runtime+rusqlite,application 已具备)
7. **`evaluation`(978 行)→ 迁 `application` 或按 surface-ledger 拆**(依赖 kernel/metacog/runtime)
8. **`goal`(5,658 行)→ 拆**:ObjectiveStore 等 SQLite 部分 → adapters-sqlite;协调器 → aletheon 或 application
9. **`agent_control`(5,721 行)→ 拆**:runtime 已有 agent_supervisor/agent_writer;host 适配留在 aletheon
10. **`turn_pipeline`/`turn_coordinator`(3,906 行)→ runtime**(Turn 语义权威)

> 注意:第 5-10 项每刀都会遇到"类型分裂"风险(闭包内互引)。**策略**:整闭包一起迁(如 evaluation+post_turn_projection+verification 是一组),不要单文件搬。每刀后 `cargo check --workspace` + 相关测试必须过。

### 验证(每刀)

```bash
bash scripts/cargo-agent.sh check --workspace
bash scripts/cargo-agent.sh test -p <affected-crate>
git diff --check
```

### 完成标准

- `crates/aletheon/src/wiring/application/` 缩减到仅剩"组装适配"(目标 < 5,000 行)
- `rg 'wiring::application::' crates/aletheon/src` 仅剩 aletheon 内部组装点

---

## P2:无外部消费者的 pub 模块(潜在死代码)

### 事实(已核实,2026-08-15)

`crates/aletheon/src/wiring/application/mod.rs` 声明 30 个 `pub mod`,以下模块**在 aletheon 内无 `wiring::application::<m>` 形式的外部引用**:

| 模块 | 外部引用 | 说明 |
|---|---|---|
| `conscious` | 0 | 可能被 conscious_workspace 等闭包内引用,需人工确认 |
| `conscious_action` | 0 | 同上 |
| `conscious_core_coordinator` | 0 | 同上 |
| `daemon_turn` | 0 | 但 `daemon_react`/`turn_pipeline` 引用 `daemon_turn::execute` 等,需确认是否经 mod.rs 间接引用 |

> ⚠️ 之前误报过 `dasein_workspace_adapter`(实际在 `composition/dasein_workspace.rs` 且被 request.rs 使用)和 `executive_runtime` 命名残留(已清零)。**Codex 必须先核实再删,禁止凭文件名判断**。

### 执行要求

1. 对每个 0 引用模块:用 `rg` 全仓搜索模块内**符号**(如 `ConsciousActionBridge`、`ConsciousCoreCoordinator`),确认无任何使用
2. 确认模块间是否互引(mod.rs 内部 `super::` 引用)
3. 真死代码 → 删除文件 + mod.rs 声明
4. 活代码但命名误导 → 重命名(如 `daemon_turn` 若仍生产使用,保留但更新文档注释)

### 验证

```bash
bash scripts/cargo-agent.sh check -p aletheon
bash scripts/cargo-agent.sh test -p aletheon --lib
git diff --check
```

---

## P3:工作区 1777 个未提交路径

### 事实

- HEAD `8a813861`(2026-08-10),整个迁移(七 crate 删除 + gateway 收敛 + config 拆解 + 测试迁移)从未提交
- 545 modified + 443 untracked + 786 deleted

### 执行要求

**按你的指示执行,不要自行提交**。若获准提交,建议分阶段:

1. `docs/plans/2026-08-15-codex-closeout.md` 更新(已完成)
2. 架构清单(module-boundaries/architecture-check/census TSV/README)更新(已完成)
3. 核心迁移(七 crate 删除、gateway 收敛、config 拆解)
4. 测试迁移(146 个测试文件)
5. 修复(harness 取消、max_iterations、exec CLI)

每阶段一个 commit,禁止 `git add -A`。

---

## 执行顺序建议

```
1. P0(锁 poison)→ 小而明确,先做
2. P2(死代码核实清理)→ 独立,低风险
3. P1(按权威拆 wiring/application)→ 大工程,逐刀,每刀人审
4. P3(提交)→ 最后,由用户决定
```

## 给 Codex 的第一句话

```text
只读并执行 docs/plans/2026-08-15-architecture-hardening.md 的 P0,完成后停下等人审。

禁止:git add -A、删除任何文件前必须 rg 核实、改动超出本项范围。
P0 完成标准:runtime agent_writer/turn_writer 的锁 unwrap 全部有 poison 恢复,
runtime 测试全过,行为不变。
```
