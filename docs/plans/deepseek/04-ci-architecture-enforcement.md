# CI 架构执行机制 — 代码级分析

## 概述

分析 `scripts/architecture-check.sh`、`.github/workflows/ci.yml` 和 allowlist 配置的架构执行机制。

**结论：CI 执行比架构文档描述的更完善。485 行多层 fitness gate + 20+ 删除门 + 7 类扫描规则 + 依赖图强制执行 + 基线回归防护。已完成的改进（K02/X02/Clock 统一）已被 CI 永久锁定。**

---

## 1. `scripts/architecture-check.sh` — 485 行多层 Fitness Gate

**文件:** `scripts/architecture-check.sh`（485 行）

四层执行层级：

### Level 1: Deletion Gates（硬失败 `exit 1`，20+ 门）

**Q01 — 应用层发现:** `AppConfig`/`load_layered`/`ALETHEON__` 不得出现在 cognit/corpus/mnemosyne/dasein/agora 源文件中

**Q02 — Bin/Interact 隔离:** `interact` 不得依赖 `kernel`/`corpus`；`bin` 不得依赖任何领域 crate

**F01 — 领域 facade:** 10 个特定 executive 源文件不得引用 `mnemosyne::FactStore`、`corpus::HookRegistry`、`ToolRunnerWithGuard`、`MorphogenesisPipeline` 等

**K02/X02 — Kernel 权威:** `SystemClock` 不得在 cognit/dasein/agora 生产代码中出现；`ServicePorts`/`ProcessTable`/`OperationTable` 不得在 executive/src 中出现；`executive/src/impl/kernel` 目录不得存在；Kernel 不得引用任何应用领域 crate

**G03/G06/G07/G08/G09/G10 — Agent 控制:** 仅特定模块可实现 `AgentControlPort`、调用 `register_process_mailbox`、绕过审查的 memory promotion

**M04/M05 — Memory 边界:** legacy `RecallInjector`/`inject_into_prompt`/`mnemosyne::backends` 不得出现；`crates/mnemosyne/src/impl/pipeline` 不得存在

### Level 2: Scan-based Allowlist Rules（7 类）

| 规则 | 扫描目标 | 排除 |
|------|---------|------|
| `direct_tool` | `tool.execute(` 在 corpus/executive/bin | `runner.rs`/`executor.rs` |
| `legacy_event` | `use fabric::envelope`/`Envelope::` | — |
| `concrete_clock` | `SystemClock::new(` 在 dasein/agora/cognit/mnemosyne/metacog/interact | 仅生产代码 |
| `core_systems_field` | `.runtime.`/`.domain.`/`.infra.`/`.orchestration.`/`.memory.` 在 executive/bin | — |
| `duplicate_kernel` | `executive::impl::kernel` | — |
| `raw_process` | 直接 `tokio::process::Command` 在 dasein/executive | 仅生产代码 |
| `executive_store_import` | `mnemosyne::.*Store`/`Database` 或 `corpus::.*Registry`/`Runner` 在 executive | bootstrap + exec_session.rs |

结果与 `config/architecture-allowlist.txt` 对比。

### Level 3: Dependency Graph Enforcement

`cargo metadata` 对比 `config/architecture-dependencies.txt`。新的依赖边 → 失败。

### Level 4: Path Inventory Enforcement

关键符号位置（`TurnService`、`TurnPipeline`、`ExecTurnServices`、`CapabilityInvoker`）对比 `config/architecture-path-inventory.txt`。

### Baseline Regression Prevention

CI 中设置 `ARCH_BASE_REF: origin/${{ github.base_ref || 'dev' }}` 后，allowlist/dependency/path-inventory baseline 文件**只能删除条目，不能增加**。

---

## 2. CI 集成

**文件:** `.github/workflows/ci.yml:13-27`

```yaml
architecture:
  name: architecture fitness
  # CI 中第一个运行的 job
  steps:
    - run: bash tests/architecture_check.sh        # Smoke test：故意违规确认被捕获
    - run: bash tests/architecture_path_inventory.sh # 关键符号存在性
    - name: Reject architecture drift
      env:
        ARCH_BASE_REF: origin/${{ github.base_ref || 'dev' }}
      run: bash scripts/architecture-check.sh        # 真正的门
```

---

## 3. 当前违规扫描结果

### `#[allow(deprecated)]` — 仅 7 处

5 处 fabric IPC 层（`communication_bus.rs:1`、`unix_socket.rs:8`、`lib.rs:19`、`manager.rs:6`、`transport_adapter.rs:8`）+ 2 处 agora（`ops/mod.rs:336`、`workspace/mod.rs:411`，已标记 `#[deprecated]` 的旧 API）

### `use fabric::kernel` 泄漏 — 6 处，全部 executive

全部是 `debug_bus` 类型（`PerfCounter`、`DebugEvent`、`DebugBusHook`）。1/6 在 `#[cfg(test)]` 块内。

### `SystemClock` 在领域 crate

| Crate | 数量 | 状态 |
|-------|------|------|
| cognit | 0 | 清洁 ✅ |
| dasein | 0 | 清洁 ✅ |
| agora | 0 | 清洁 ✅ |
| metacog | 0 | 清洁 ✅ |
| mnemosyne | 0 | 清洁 ✅ |
| corpus | 9 | 不在 enforcement scope |

Corpus 中的 9 处：`drivers/display/clipboard_x11.rs:22`、`drivers/platform/android.rs:33`、`drivers/platform/boot.rs:675`、`tools/subagent/worktree.rs:4,58`、`acix/experience.rs:219`、`acix/tools.rs:944`、`acix/task.rs:715,732,822`

### `ServicePorts` — 生产代码零引用 ✅

### 直接 `Tool::execute` — 2 处在 dasein

`dasein/src/impl/security/runner.rs:227,238`（L0 直接执行 + output guardrail retry）。`direct_tool` 扫描仅覆盖 corpus/executive/bin，dasein 不在范围内。

### `executive/src/impl/kernel/` — 不存在 ✅

---

## 4. 盲区

| 盲区 | 详情 |
|------|------|
| corpus 残留 `SystemClock` | 9 处不在 K02 scope 中 |
| dasein 直接 `Tool::execute` | 2 处不在 `direct_tool` scope 中 |
| 跨 crate trait 一致性 | 无自动验证 |

---

## 总结

1. **485 行多层 fitness gate**，不是简单 lint 脚本
2. **20+ deletion gates** — 二进制门，零妥协
3. **7 个扫描规则** 对比 allowlist，自动检测回归
4. **依赖图 + 路径 inventory** 强制执行
5. **Baseline 回归防护** — allowlist 只能收缩
6. **已在 CI 运行**（第一个 job），不是未来目标

**已完成的改进已被永久锁定。** 盲区可通过扩展 enforcement scope 关闭。
