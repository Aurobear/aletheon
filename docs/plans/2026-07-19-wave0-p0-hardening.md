# Wave 0：P0 能力止血 + 架构冻结 Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地 `docs/plans/2026-07-19-aletheon-execution-roadmap-design.md` 的 Wave 0——修复三项隔离的 P0 能力缺陷（max_iterations 死配置、file_search cwd、搜索全局 limit）并建立架构冻结门禁，且不改动主链结构。

**Architecture:** 三项能力修复各自局限在单文件/单函数，走 TDD；架构冻结新增一份 `architecture-status.toml` 账本 + 强化 `scripts/architecture-check.sh`（cargo 探测 fail-fast、冻结 fabric 根级重导出数量）。

**Tech Stack:** Rust（executive / corpus crates）、Bash + Python（architecture-check.sh）、TOML。

**环境说明:** 当前环境无 `cargo`。所有 `cargo test` / `cargo build` 步骤必须在完整 Rust 环境执行；本计划在每个验收步骤显式标注。

---

## 范围与不做

**本计划覆盖（PR-01 的隔离子集 + PR-02）:**
- Task 1 — max_iterations 死配置（`0 = unlimited` 语义可达）
- Task 2 — `file_search` cwd + 全局 limit
- Task 3 — `grep` ripgrep 路径全局 limit
- Task 4 — `architecture-status.toml` 架构账本
- Task 5 — `architecture-check.sh` cargo 探测 + fabric 根重导出冻结门禁

**明确不在本计划（结构性，需专属计划，见文末 §附录）:**
- roadmap PR-01 #2「ResolvedTurnProfile」——跨多个 turn 消费者穿线，与 Wave 1 重叠。
- roadmap PR-01 #3「agent 工具可达性」——`AgentControlService` 依赖 profiles，无法简单前移，需拆分服务构造，与 Wave 1 重叠。

---

## Task 1: max_iterations 死配置修复

`0 = unlimited` 当前不可达：Profile 加载器 `runtime.rs:65-69` 用 `.min(config.max_iterations).max(1)` 把 0 压成 1，且 `ExecutiveConfig.max_iterations` 在 `request.rs:502` 恒为默认 50。修复分两处：引入 `combine_limits`（0 表示无限制）+ 把 `config.agent.max_iterations` 接进 `ExecutiveConfig`。

**Files:**
- Modify: `crates/executive/src/impl/daemon/bootstrap/runtime.rs:65-69`
- Modify: `crates/executive/src/impl/daemon/bootstrap/request.rs:502-509`
- Test: `crates/executive/src/impl/daemon/bootstrap/runtime.rs`（`#[cfg(test)] mod` 内新增单测）

- [ ] **Step 1: 写失败测试**（追加到 `runtime.rs` 的测试模块；若无则在文件末尾新增）

```rust
#[cfg(test)]
mod combine_limits_tests {
    use super::combine_limits;

    #[test]
    fn zero_profile_zero_global_is_unlimited() {
        assert_eq!(combine_limits(0, 0), 0);
    }

    #[test]
    fn zero_profile_uses_global_cap() {
        assert_eq!(combine_limits(0, 50), 50);
    }

    #[test]
    fn zero_global_keeps_profile() {
        assert_eq!(combine_limits(20, 0), 20);
    }

    #[test]
    fn both_nonzero_takes_min() {
        assert_eq!(combine_limits(20, 50), 20);
        assert_eq!(combine_limits(80, 50), 50);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run（完整 Rust 环境）: `cargo test -p executive combine_limits_tests`
Expected: FAIL —— `cannot find function combine_limits in this scope`

- [ ] **Step 3: 实现 `combine_limits` 并替换 clamp**

在 `runtime.rs` 中，紧邻 `load_agent_profiles`（含 `:65-69` 的函数）之前新增：

```rust
/// Combine a profile-level and global iteration limit where `0` means
/// "unlimited". The old `.min(global).max(1)` collapsed `0` (unlimited)
/// into `1`; this preserves unlimited semantics on both sides.
fn combine_limits(profile: usize, global: usize) -> usize {
    match (profile, global) {
        (0, 0) => 0,
        (0, global) => global,
        (profile, 0) => profile,
        (profile, global) => profile.min(global),
    }
}
```

把 `:65-69` 的：

```rust
        let max_iterations = overrides
            .and_then(|ov| ov.max_iterations)
            .unwrap_or(role.max_iterations)
            .min(config.max_iterations)
            .max(1);
```

替换为：

```rust
        let profile_limit = overrides
            .and_then(|ov| ov.max_iterations)
            .unwrap_or(role.max_iterations);
        let max_iterations = combine_limits(profile_limit, config.max_iterations);
```

- [ ] **Step 4: 把 `config.agent.max_iterations` 接进 ExecutiveConfig**

`request.rs:502-509`，把：

```rust
        let runtime_config = ExecutiveConfig {
            session_id: session_id.clone(),
            context_window_tokens: context_window,
            conscious_arbitration_mode: config.conscious_arbitration_mode,
            compaction_v2: grok_hardening.compaction_v2,
            streaming_tools: grok_hardening.streaming_tools,
            ..Default::default()
        };
```

改为（新增一行 `max_iterations`；`config: &DaemonConfig` 已在 `core/config/mod.rs:69` 暴露 `pub agent: AgentConfig`）：

```rust
        let runtime_config = ExecutiveConfig {
            session_id: session_id.clone(),
            context_window_tokens: context_window,
            conscious_arbitration_mode: config.conscious_arbitration_mode,
            compaction_v2: grok_hardening.compaction_v2,
            streaming_tools: grok_hardening.streaming_tools,
            // Wave 0: honor configured agent iteration cap (0 = unlimited)
            // instead of the hardcoded Default (50).
            max_iterations: config.agent.max_iterations,
            ..Default::default()
        };
```

- [ ] **Step 5: 运行测试确认通过**

Run（完整 Rust 环境）: `cargo test -p executive combine_limits_tests`
Expected: PASS（4 tests）

- [ ] **Step 6: 编译校验**

Run（完整 Rust 环境）: `cargo build -p executive`
Expected: 成功；无 `unused` 警告（`combine_limits` 已被调用）

- [ ] **Step 7: Commit**

```bash
git add crates/executive/src/impl/daemon/bootstrap/runtime.rs \
        crates/executive/src/impl/daemon/bootstrap/request.rs
git commit -m "fix(executive): make max_iterations=0 unlimited semantics reachable

combine_limits treats 0 as unlimited on both profile and global sides;
wire config.agent.max_iterations into ExecutiveConfig instead of hardcoded 50.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: file_search cwd + 全局 limit

`file_search.rs` 三个子进程助手（`try_ripgrep`/`try_grep`/`try_find_grep`）均缺 `.current_dir()`，相对路径落到 daemon cwd；且 ripgrep 路径的 `--max-count` 仅限每文件、收集时无 `.take()`。对照已正确的 `grep.rs`。

**Files:**
- Modify: `crates/corpus/src/tools/tools/file_search.rs`
- Test: 同文件 `#[cfg(test)] mod tests`

- [ ] **Step 1: 写失败测试（相对路径 cwd 回归）**

追加到 `file_search.rs` 测试模块：

```rust
    #[tokio::test]
    async fn test_file_search_relative_path_uses_working_dir() {
        // Regression: with path="." the search must resolve against
        // ctx.working_dir, not the daemon process cwd.
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("marker.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn unique_marker_zzz() {{}}").unwrap();

        let tool = FileSearchTool;
        let input = json!({ "query": "unique_marker_zzz", "path": "." });

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(!result.is_error, "got: {}", result.content);
        assert!(
            result.content.contains("unique_marker_zzz"),
            "relative search did not use working_dir: {}",
            result.content
        );
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run（完整 Rust 环境）: `cargo test -p corpus file_search::tests::test_file_search_relative_path_uses_working_dir`
Expected: FAIL —— 断言 `unique_marker_zzz` 未命中（搜索落到 daemon cwd）

- [ ] **Step 3: 给三个助手加 working_dir 参数并 current_dir**

`try_ripgrep` 签名与调用（`:125-146`）：签名末尾新增 `working_dir: &std::path::Path,`，并在 `let output = cmd.output()` 之前加 `cmd.current_dir(working_dir);`。对 `try_grep`（`:191-208`）、`try_find_grep`（`:251` 起，`find` 的 `cmd`）做同样处理：各自签名加 `working_dir: &std::path::Path,`，在 `.output().await` 前对该 `Command` 调用 `.current_dir(working_dir)`。

`execute()` 内三个调用点（`:92-108`）改为传入 `&ctx.working_dir`：

```rust
        // Strategy 1: Try ripgrep
        if let Some(result) =
            try_ripgrep(&query, &path, include.as_deref(), max_results, &*ctx.clock, &ctx.working_dir).await
        {
            return result;
        }

        // Strategy 2: Fallback to grep -r
        if let Some(result) =
            try_grep(&query, &path, include.as_deref(), max_results, &*ctx.clock, &ctx.working_dir).await
        {
            return result;
        }

        // Strategy 3: Fallback to find + grep
        if let Some(result) =
            try_find_grep(&query, &path, include.as_deref(), max_results, &*ctx.clock, &ctx.working_dir).await
        {
            return result;
        }
```

- [ ] **Step 4: 修 ripgrep 路径的全局 limit**

`try_ripgrep` 内 `:154-155`：

```rust
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let truncated = lines.len() >= max_results;
```

改为（与 `grep.rs` fallback 一致的全局 take 语义）：

```rust
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().take(max_results).collect();
    let truncated = stdout.lines().count() > max_results;
```

- [ ] **Step 5: 运行测试确认通过**

Run（完整 Rust 环境）: `cargo test -p corpus file_search`
Expected: PASS（含新回归测试与原有 4 个测试）

- [ ] **Step 6: Commit**

```bash
git add crates/corpus/src/tools/tools/file_search.rs
git commit -m "fix(corpus): file_search honors working_dir and global result limit

Add .current_dir(&ctx.working_dir) to all three subprocess strategies so
relative paths resolve against the workspace; apply .take(max_results) on
the ripgrep path so the cap is global, not per-file.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: grep ripgrep 路径全局 limit

`grep.rs` 已有 `.current_dir(working_dir)`，但 ripgrep 路径 `:147-148` 收集全部行、`--max-count` 仅每文件限制。fallback grep 路径已正确用 `.take()`。

**Files:**
- Modify: `crates/corpus/src/tools/tools/grep.rs:147-148`
- Test: 同文件 `#[cfg(test)] mod tests`

- [ ] **Step 1: 写失败测试（多文件命中总量受控）**

追加到 `grep.rs` 测试模块：

```rust
    #[tokio::test]
    async fn test_grep_global_limit_across_files() {
        let tmp = tempfile::tempdir().unwrap();
        // 6 files, each with one match; global max_results = 3.
        for i in 0..6 {
            let p = tmp.path().join(format!("f{i}.txt"));
            std::fs::write(&p, "needle here\n").unwrap();
        }

        let tool = GrepTool;
        let input = json!({
            "pattern": "needle",
            "path": tmp.path().to_str().unwrap(),
            "max_results": 3
        });

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(!result.is_error, "got: {}", result.content);
        let match_lines = result
            .content
            .lines()
            .filter(|l| l.contains("needle"))
            .count();
        assert!(
            match_lines <= 3,
            "global limit not enforced: {} match lines",
            match_lines
        );
        assert!(result.metadata.truncated, "expected truncated=true");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run（完整 Rust 环境）: `cargo test -p corpus grep::tests::test_grep_global_limit_across_files`
Expected: FAIL —— match_lines == 6（rg 每文件 1 条，未做全局 take）

- [ ] **Step 3: 修 ripgrep 全局 limit**

`grep.rs:147-148`：

```rust
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let truncated = lines.len() >= max_results;
```

改为：

```rust
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().take(max_results).collect();
    let truncated = stdout.lines().count() > max_results;
```

- [ ] **Step 4: 运行测试确认通过**

Run（完整 Rust 环境）: `cargo test -p corpus grep`
Expected: PASS（含新测试与原有 4 个测试）

- [ ] **Step 5: Commit**

```bash
git add crates/corpus/src/tools/tools/grep.rs
git commit -m "fix(corpus): grep ripgrep path enforces a global result limit

--max-count only caps per file; apply .take(max_results) on collected
lines so the returned total honors the requested cap.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: architecture-status.toml 架构账本

建立可机器读取的架构状态账本，记录关键接口的 owner / 生产调用者 / authority / 兼容删除期限。Wave 0 先固化现状（含已知的 reviewed 例外与占位实现），为后续 Wave 提供收敛基线。

**Files:**
- Create: `architecture-status.toml`（仓库根）

- [ ] **Step 1: 写账本**

```toml
# Aletheon 架构状态账本（Wave 0 基线，2026-07-19）
# 由 scripts/architecture-check.sh 校验。每个条目声明 owner / 生产调用者 /
# authority / 兼容删除期限。roadmap: docs/plans/2026-07-19-aletheon-execution-roadmap-design.md

schema_version = 1

[[turn_entry]]
name = "TurnPipeline"
role = "daemon 编排器"
production_caller = "DaemonTurnOrchestrator"
status = "production"

[[turn_entry]]
name = "TurnService"
role = "CLI 生产入口"
production_caller = "ExecSessionBuilder"
status = "compatibility_facade"
converge_into = "TurnEngine"
target_wave = 1

[[turn_entry]]
name = "TurnCoordinator"
role = "operation/session/cancel 控制外壳"
production_caller = "TurnService"
status = "production"

[[turn_entry]]
name = "AgentControlService"
role = "child agent 运行"
production_caller = "bootstrap/services.rs"
status = "production"

# 有名无实的抽象（无生产调用者）——标 experimental，后续 Wave 接入或删除。
[[phantom_abstraction]]
name = "fabric::RuntimeOps"
evidence = "crates/fabric/src/include/runtime.rs:13 —— 零 impl"
status = "experimental"
decision = "delete_or_wire"
target_wave = 2

[[phantom_abstraction]]
name = "AletheonExecutive::step"
evidence = "crates/executive/src/core/orchestrator.rs:135 —— 占位递增，非生产路径"
status = "experimental"
decision = "delete_or_wire"
target_wave = 2

[[phantom_abstraction]]
name = "CognitCore"
evidence = "真实 turn 走 LinearCognitiveSession + ReActLoop"
status = "experimental"
decision = "delete_or_wire"
target_wave = 2

# 已 review 的跨界依赖——Wave 0 冻结（不新增），Wave 2 收敛到 0。
[[reviewed_dependency]]
from = "corpus"
to = "cognit"
reason = "MCP config 复用 cognit::config"
target_removal_wave = 2

[[reviewed_dependency]]
from = "corpus"
to = "mnemosyne"
reason = "MCP credential 复用 mnemosyne::credential"
target_removal_wave = 2

[[reviewed_dependency]]
from = "exec-server"
to = "corpus"
reason = "隔离 exec server 依赖整个 corpus"
target_removal_wave = 2

# 冻结基线：fabric 根级 pub use 数量（防止继续膨胀）。
[freeze]
fabric_root_reexports_max = 132
```

- [ ] **Step 2: 校验 fabric 根重导出基线数值**

Run: `grep -c '^pub use' crates/fabric/src/lib.rs`
Expected: 输出一个整数 N（实测 132）。把 `fabric_root_reexports_max` 设为该实测值（此值即冻结上限）。

- [ ] **Step 3: Commit**

```bash
git add architecture-status.toml
git commit -m "docs(arch): add architecture-status.toml Wave 0 baseline ledger

Records turn entry points, phantom abstractions, reviewed cross-crate deps
and a fabric root re-export freeze baseline for architecture-check gating.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: architecture-check.sh cargo 探测 + fabric 重导出冻结门禁

`architecture-check.sh:571` 直接调 `cargo metadata`，无 cargo 时诊断不清。新增 cargo 探测 fail-fast，并新增"fabric 根重导出数量不得超过账本冻结值"的门禁。

**Files:**
- Modify: `scripts/architecture-check.sh:570` 附近（依赖检查块）
- Modify: `scripts/architecture-check.sh`（末尾新增 fabric 重导出冻结检查）

- [ ] **Step 1: cargo 探测 fail-fast**

把 `:570` 的：

```bash
if [[ ${ARCH_SKIP_DEPENDENCIES:-0} != 1 ]]; then
  cargo metadata --no-deps --format-version 1 | python3 -c '
```

改为（缺 cargo 时给出清晰诊断，而非让 pipe 静默失败）：

```bash
if [[ ${ARCH_SKIP_DEPENDENCIES:-0} != 1 ]]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "architecture-check: cargo not found; dependency graph gate cannot run." >&2
    echo "  Install Rust/cargo, or set ARCH_SKIP_DEPENDENCIES=1 to explicitly skip." >&2
    exit 1
  fi
  cargo metadata --no-deps --format-version 1 | python3 -c '
```

- [ ] **Step 2: 新增 fabric 根重导出冻结门禁**

在依赖检查块之后（`:591` 的 `fi` 之后、迁移清单块之前）新增：

```bash
# Freeze: fabric root-level re-exports must not grow beyond the ledgered
# baseline (architecture-status.toml [freeze].fabric_root_reexports_max).
fabric_reexports_now=$(grep -c '^pub use' crates/fabric/src/lib.rs || echo 0)
fabric_reexports_max=$(grep -E '^\s*fabric_root_reexports_max\s*=' architecture-status.toml \
  | grep -oE '[0-9]+' | head -1)
if [[ -z "$fabric_reexports_max" ]]; then
  echo "architecture-check: architecture-status.toml missing fabric_root_reexports_max" >&2
  exit 1
fi
if (( fabric_reexports_now > fabric_reexports_max )); then
  echo "architecture-check: fabric root re-exports grew from ${fabric_reexports_max} to ${fabric_reexports_now}" >&2
  echo "  New root-level 'pub use' in crates/fabric/src/lib.rs are frozen (Wave 0)." >&2
  echo "  Import from a submodule, or lower the baseline as re-exports are removed." >&2
  exit 1
fi
```

- [ ] **Step 3: 运行门禁（无 cargo 分支）**

Run: `ARCH_SKIP_DEPENDENCIES=1 bash scripts/architecture-check.sh; echo "exit=$?"`
Expected: fabric 冻结检查通过（`fabric_reexports_now == max`）；因 `ARCH_SKIP_DEPENDENCIES=1` 跳过依赖图；脚本其余原有检查照常。若脚本因其他既有原因失败，记录但本 Task 只对新增两段负责。

- [ ] **Step 4: 运行门禁（缺 cargo fail-fast 分支验证）**

Run: `PATH=/usr/bin bash -c 'command -v cargo || echo no-cargo'`（确认可模拟无 cargo）；随后在无 cargo 环境运行 `bash scripts/architecture-check.sh; echo "exit=$?"`
Expected: 打印 `cargo not found; dependency graph gate cannot run` 并 `exit=1`（不再静默 pipe 失败）

- [ ] **Step 5: Commit**

```bash
git add scripts/architecture-check.sh
git commit -m "chore(arch): fail-fast on missing cargo and freeze fabric re-exports

architecture-check now emits a clear diagnostic when cargo is absent instead
of a silent pipe failure, and gates fabric root re-export growth against the
architecture-status.toml baseline.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## 自审对照

| roadmap Wave 0 项 | 覆盖任务 |
|---|---|
| max_iterations 死配置 | Task 1 |
| file_search cwd | Task 2 |
| 搜索全局 limit（file_search + grep） | Task 2 + Task 3 |
| architecture-status.toml | Task 4 |
| architecture-check cargo 探测 | Task 5 Step 1 |
| 禁止新增 fabric 根重导出 | Task 5 Step 2 |
| Profile 快照（ResolvedTurnProfile） | 附录（专属计划） |
| agent 工具可达性 | 附录（专属计划） |

---

## 附录：拆出的结构性任务（不在本计划）

以下两项虽列在 roadmap Wave 0 的"能力止血"，但经代码核对属于**结构性改动**，与 Wave 1 主链收敛重叠，需各自专属计划：

1. **ResolvedTurnProfile**（roadmap PR-01 #2）
   - `ActiveAgentProfileSnapshot`（`turn_runtime_ports.rs:59`）仅含 `profile_name` + `allowed_tools`；扩成携带 `system_prompt`/`model_policy`/`budget`/`verifier` 需同步改造 `snapshot()`（`turn_runtime.rs:435`）、`constrain_profile_capabilities`（`:450`）以及消费 `model_policy` 的 `execute.rs:175` 等多个 turn 消费者。
   - 建议并入 Wave 1「唯一 TurnEngine」的 profile 解析统一改造，避免在双轨主链上各改一遍。

2. **agent 工具可达性**（roadmap PR-01 #3）
   - `register_agent_tools`（`runtime.rs:153`）在 `services.rs:152` 于 profile 编译后调用，且依赖 `AgentControlService`（本身需要 profiles）。无法简单前移——需把"稳定 agent 控制 definitions 注册"与"依赖 profile 的高层 delegate 工具注册"拆成两阶段。
   - 建议与 Wave 1 一并设计（高层 `delegate_code/review/research` 接口亦在此引入）。

---

## 下一步

Wave 0 完成后，进入 Wave 1「唯一 TurnEngine」专属 spec → plan 循环；上述两项结构性任务并入 Wave 1 设计。
