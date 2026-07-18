# 统一可执行索引（Grok exec × DeepSeek plans）

> **给执行者（deepseek）的主控文档。** 本目录把两套计划合并为一条可执行序列：
> - **Grok exec specs**（`G1–G8` / `S1` / `C1`，本目录 `../exec/*.md`）——已完成 **fabric 契约层**（纯类型 + 测试，已提交到分支 `auro/feat/conscious-r2-r3-production`）。
> - **DeepSeek plans**（`../../deepseek/*.md`）——OUTSTANDING 的工程计划（多含 Executive/corpus 集成层）。
>
> 合并原则：**Grok 交付了地基（fabric 纯类型），DeepSeek 交付上层接线（consumer/wiring/protocol）。** 大多数重叠是「build-on」，仅一处 **命名冲突**（S1 sandbox）必须先协调。

## 0. 执行前必读（每个任务开始前）

1. **仓库有活跃并发提交** —— 分支 HEAD 持续移动。开工前 `git log --oneline -5` 确认基线。
2. **所有 `path:line` 锚点可能已漂移**（DeepSeek 计划基线是更早的 commit；本索引已标注已知漂移）。**动手前对每个引用符号重新 `grep`/LSP 定位并更新。** 这是硬性要求——本轮合并中已发现多处漂移（如 tool-exec 的 `runner.rs:369→401`）。
3. **隔离实现，cherry-pick 回主分支**：按 grok 已验证的工作流——`git worktree add` 基于当前 HEAD 建隔离副本，实现 + `cargo test -p <crate>` 全绿后提交，再 cherry-pick 回 `auro/feat/conscious-r2-r3-production`。避免污染活跃 WIP。
4. **feature flag 门控**：所有高风险机制默认关，关闭态等价当前行为（见各 spec §7）。
5. **提交纪律**：commit message 不含模型名；每个可独立编译的切片一个提交。
6. **测试**：`cargo test -p <crate>`（单 crate）；`cargo check --workspace`（合并前全量）；`cargo clippy`/`cargo fmt`。

## 1. 已完成（DONE）——不要重做

### 1.1 Grok fabric 契约层（已提交，10 个 commit）

| Spec | fabric 模块 | 提交 | 状态 |
|---|---|---|---|
| G1 | `fabric::types::workspace_trust` | `df90b775` | 纯类型 + `decide()` + 11 测试 |
| G2 | `fabric::types::tool_stream` + `TurnEventV1::ToolProgress` | `55e18eea` | 流式契约 + 7 测试 |
| G3 | `fabric::types::prompt_queue` | `d3023c04` | `evaluate_edit/cancel` + 8 测试 |
| G4 | `fabric::types::workspace_checkpoint` | `4c8c439a` | 事务 restore 类型 + 6 测试 |
| G5 | `fabric::types::lifecycle` | `6b5cb929` | 声明式 effect + `validate_effects` + 7 测试 |
| G6 | `fabric::types::agent_settlement` | `cddad001` | 结算 + `can_reparent` + 7 测试 |
| G7 | `mnemosyne::credential` | `299f9d68` | `approved_for` fail-closed + 8 测试 |
| G8 | `interact::acp` | `423962ea` | `map_client_event_to_acp` + 4 测试 |
| S1 | `fabric::types::sandbox`（追加 profile 层） | `13e27987` | `merge_project_additive` + 3 测试 |
| C1 | `fabric::include::compaction`（追加 guardrails） | `001f4632` | `is_degenerate_summary`/`safe_tail_cut` + 7 测试 |

> **这些类型当前 UNWIRED**（仅 fabric 内自测；`grep` 确认无外部 consumer）。它们是下面各集成流的地基。

### 1.2 DeepSeek 已完成（审计 `../../deepseek/09-full-plan-audit.md`）

- **Multi-user runtime M0–M2** 已实现（`../../deepseek/2026-07-17-multi-user-runtime-m0-m2.md`：principal/workspace 契约、per-user runtime、任意 cwd + add-dir、systemd core/user 单元）。
- **39/50（78%）** 广义架构计划已实现（Agora、Dasein、Kernel 机制层、Memory、Agent 控制、CI fitness gate 等）。
- **Kernel** 已是干净机制层（crate/依赖无环）；MCP client 已工作（3 传输 + bearer/OAuth）；Conscious-core Task1–5 已提交、R4（文档修正）已完成。
- **Quick-wins**（`../../deepseek/2026-07-17-capability-hardening-roadmap.md §6`）已完成：`max_iterations` 默认已 50（`config/agent.rs:42`）、Clock 注入已存在（`kernel/runtime.rs:107 with_clock`）——**勿重做**。

### 1.3 DeepSeek 母路线图（本索引的对照基准）

`../../deepseek/2026-07-17-capability-hardening-roadmap.md` 是 DeepSeek capability-hardening 项目的**自有母索引**。其 §4 优先级为：
**Phase 0 Capability Activation → Phase 1 Testing Infrastructure（回归保护，硬前置）→ Phase 2 并行三轨（Tool Exec ∥ MCP ∥ Structured Editing）。**
本统一索引的 §4 序列已并入该顺序，并叠加 Grok fabric 地基与 kernel/conscious 流。

### 1.4 消费层进展（2026-07-17，本轮）

在 fabric 契约层之上开始接线消费层。已提交到 `auro/feat/conscious-r2-r3-production`：

| 提交 | 内容 | 层 | 测试 |
|---|---|---|---|
| `46a40489` | 修复架构门禁：`LC_ALL=C`（locale 排序不一致导致门禁空转/误报，一直未真正把关） | ci | 门禁跑通 28/34/4 |
| `627decd1` | **S1** `resolve_profile` + `ResolvedSandboxPolicy`：profile→可执行 policy，credential 恒 deny | fabric | 6 新（sandbox 共 12） |
| `daaf4d00` | **C1** `maybe_compact_v2` + `AdvancedCompressor` 实现：degenerate/short/sampler-error → 缓冲不变（fail-safe） | fabric + mnemosyne | 4 新（compressor 共 13） |
| `8a2a1681` | **grok_hardening flag 面**：`AppConfig.grok_hardening`，10 个 flag 全默认关，`deny_unknown_fields` | executive config | 4 新 + schema 重生 |
| `9bf0e502` | **C1 T11** harness 接线：`HarnessConfig.compaction_v2` 门控 4 个 loop 压缩点（关=旧行为逐字节等价） | cognit | 2 新路由测试 |
| `4c49c9eb` | **C1 尾巴** executive 接线：`grok_hardening.compaction_v2` → `HarnessConfig`，跨主 turn（`ExecutiveConfig.compaction_v2` + `harness_config_from_executive`）与子 Agent（`NativeCognitRuntimeResources` + `native_cognit::harness_config`）两路，`RequestHandler::new` 加参、两入口传参 | executive | check 干净 / 4 flag 测试 / clippy 干净 |
| `72690c68` | **S1 T8** `sandbox_glob::expand_deny_globs`：无依赖 深度受限遍历 + `**`/`*`/`?` 匹配；三上限（entries/depth/matches）均 fail-closed → `GlobOverflow`；非 glob/缺失 root 跳过，symlink 目录不下降 | fabric | 9 新 |
| `50b17e7f` | **S1 T9** `SandboxConfig.policy: Option<ResolvedSandboxPolicy>`（`#[serde(skip)]`，per-exec 派生不持久）；11 处构造点全 `policy: None`（等价旧行为） | fabric + corpus + dasein + executive | check 干净 |
| `d5edc0cb` | **S1 T10** bubblewrap backend 消费 `policy.deny_exact`：文件→`--ro-bind /dev/null`，目录→空 `--tmpfs`；排在 mount plan 之后（后 mount 覆盖）；net 恒 `--unshare-net`（restrict_network 只收紧）；None=严格 no-op。**首个真正施加的 S1 enforcement** | corpus | 2 新 + 3 回归 |
| `b95e2ff8` | **S1 T11** `SandboxExecutor::run` 网络一致性：`restrict_network` × backend 缺 `network_isolation`——Require 时 fail-closed（同 noop 守卫姿态），否则告警降级不静默放网；None 跳过（等价旧行为） | fabric | 4 新 |
| `26936382` | **S1 T12a** `SandboxProfiles.default_profile` + `JsonSchema` 派生；`AppConfig.sandbox_profiles` 从 daemon 可信 config 加载 | fabric + executive | fabric 25 全走 + executive check 干净 |
| `b2b065dd` | **S1 T13** `ToolRunnerWithGuard.with_sandbox_profiles` builder，bash_exec 执行前 `resolve_profile` 填 `policy`；daemon+user 两路接线 `sandbox_profiles`；None=旧行为 | corpus + executive | corpus 10 全走 + executive check 干净 |
| `ac7fe197` | **Structured editing P1.1–P1.4**：结构化 patch 类型、文本/JSON parser、路径校验与四类文件操作执行器 | corpus | structured_patch 初始单测 |
| `59e3d83b` | **Structured editing Phase 1 收尾**：`apply_patch` 双格式接入、Unicode/尾空白归一化、±3 后有界全文匹配、Add 防覆盖、hunk 失败进度 | corpus | apply_patch 11 + structured_patch 40 全走 |
| `69c163a3` | **Testing infrastructure 兼容收尾**：mock response 生命周期、runtime fixture 新配置、并发 policy 生命周期、直接 event-spine 单调性验证 | executive tests | native runtime 9 + coordinator 22 全走 |
| `ba1f05cb` | **G1 T12–T16**：Executive trust resolver/ports、principal+workspace 内存 upsert、已知 repo 执行配置的有界只读稳定 digest 发现 | executive | workspace_trust 4 全走 |
| `458f37ab` | **G1 T17**：principal+workspace receipt 持久 JSON store，串行原子替换；缺失/损坏状态 fail-closed | executive | workspace_trust 6 全走（含 reopen/corruption） |
| `cf12ba0e` | **G1 T18**：启用态 trust decision 经 canonical bus 发布版本化事件，含 principal/workspace/scope/granting client | executive | scoped decision event 1 全走 |
| `b894d455` | **G1 T19–T20**：daemon chat 入口装配持久 resolver；headless 按 canonical workspace 评估，flag 关严格旁路；普通文件访问与 repo executable config 授权解耦 | executive | workspace_trust 8 全走 + executive check |
| `4f1cf222` | **G1 T21**：清理 Executive 严格 Clippy 依赖链遗留 lint；格式化触及文件 | corpus + executive | `clippy -p executive --lib -- -D warnings` 通过；相关单测 110 全走 |
| `bd97d4c5` | **G2 T7–T11**：有界 progress bridge，UTF-8 text 合并、洪水采样、terminal 精确透传、drop/cancel fail-closed | executive | tool_stream_bridge 5 全走 + strict clippy |
| `a1fe8b0f` | **G2 T4–T5/T12**：补齐 legacy Tool/Invoker streaming adapter；flag 后 governed bridge 与 canonical turn sender 装配 | fabric + executive | adapter 1 + governed path 4 + bridge 5 |
| `ee03f78e` | **G2 T13**：stream sink 贯通 Kernel admit/settle、Corpus activation/guard 与 Executive hook 包装；副作用只执行一次 | kernel + corpus + executive | guarded streaming override 1 + strict clippy |
| `9cdce29f` | **G2 T15**：bubblewrap/process/noop 子进程运行期间逐行流式 stdout/stderr，保留完整 terminal capture | fabric + corpus | process timing 1 + bash multi-progress/one-terminal 1 |
| `6bc1f27e` | **G2 T14/T16**：ToolProgress 投影到 TUI/CLI/ACP；未知附加 client event 安全忽略 | fabric + executive + interact | projection/compat/TUI/ACP 7 |
| `b22db914` + `8d1d8b63` | **G2 observability/T8**：按 tool 记录 dropped/no-terminal；text 按 4 KiB 或 100 ms 先到者 flush | fabric + executive | bridge 6 + strict clippy |
| `8218c1a1` | **G2 T17**：workspace canonical rustfmt 收口 | workspace | `fmt --all -- --check` 通过 |

**状态**：G1、S1 端到端完成。`grok_hardening.sandbox_profiles = true` + 配置中声明 profiles → `resolve_profile` → `SandboxConfig.policy` → bubblewrap 施加 deny + network；关=逐字节旧行为。C1+S1 两条从契约贯通到激活。

**进行中**：G1 已完成；G2 T1–T18 实现与定向验证已完成，尚待属性测试及协议违规日志收口；S1 T14（事件）与 G3–G8 consumer 收尾仍按 §4 序列推进。

## 2. 合并映射：Grok fabric × DeepSeek OUTSTANDING

| DeepSeek 流 | Grok fabric 关系 | 结论 | 合并文档 |
|---|---|---|---|
| tool-execution-hardening | S1（sandbox profile）、G2（streaming） | **1 处命名冲突 + build-on** | [D1](D1-tool-execution-integration.md) |
| multi-user M3–M5 | G1（trust）、G3（queue）、G5（lifecycle） | **build-on，零冲突** | [D2](D2-multi-user-m3-m5.md) |
| mcp-integration | G7（endpoint 凭证）；G8 无关 | **G7 build-on** | [D3](D3-mcp-integration.md) |
| testing-infrastructure-hardening | G8/S1/C1/G2 测试面 | **build-on（纯增量测试，不改 fabric 类型）** | 见 §4（硬前置） |
| conscious-core R1–R3+metrics | C1 邻接但**无关** | 独立执行 | 见 §4 |
| structured-code-editing | 无重叠 | 独立执行（Phase 1 自包含） | 见 §4 |
| capability-activation | 无重叠（前提需校正） | 独立执行 | 见 §4 |
| kernel-separation K1–K5 | 无重叠 | 独立执行 | 见 §4 |
| platform（A 线 OS 多平台 / B 线硬件） | 无重叠 | 范围外，见 §5 | — |

## 3. ⚠ 必须先解决的冲突：S1 sandbox 命名

- Grok **S1** 已在 `fabric::types::sandbox` 定义 `SandboxProfileConfig`（**可信源 DTO**：`extends/restrict_network/read_only/read_write/deny` + `SandboxProfiles::merge_project_additive` 反 hollowing）。
- DeepSeek tool-exec 计划 §3.1.1 想在**同一 crate 同名** `SandboxProfileConfig` 定义一个**运行时 per-tool** 结构（`read_roots/write_roots/deny_paths/network_enabled/env_vars/timeout_override/max_output_bytes`）——形状不同，会冲突。
- **裁定（合并方案）**：保留 S1 的 `SandboxProfileConfig` 作**可信源**；DeepSeek 的运行时结构改用 Grok S1 spec §4.1 已命名的 **`ResolvedSandboxPolicy`**（`read_only_roots/read_write_roots/deny_exact/deny_globs/restrict_network`，S1 已定义但**未实现**）；用 S1 spec §4.2 的 **`resolve_profile(name, workspace, profiles) -> ResolvedSandboxPolicy`** 作桥。详见 [D1](D1-tool-execution-integration.md)。

## 4. 执行序列（依赖排序）

> 每项标注：**源文档**（deepseek 执行者要读的原始详单）+ **合并/前置**（本目录的 grok fabric 或 D-bridge）+ **可独立性**。

### 阶段 P0 —— 独立、高价值、低风险，可立即并行

1. **structured-code-editing Phase 1**（P1.1–P1.13）— **DONE**（`ac7fe197` + `59e3d83b`）
   源：`../../deepseek/2026-07-17-structured-code-editing-plan.md`。
   自包含于 `crates/corpus`，无跨 crate 依赖，与 grok 无重叠。新增 `corpus/src/tools/tools/structured_patch.rs`。**可直接执行。**

2. **capability-activation Phase 1–2**
   源：`../../deepseek/2026-07-17-capability-activation-and-agent-profiles-plan.md`。
   ⚠ **前提校正（必读）**：`agents/*.md` **无 frontmatter**，loader（`agent_loader/mod.rs:60-66`）要求 frontmatter → `.md` 当前授权 0 工具；真实授权是 `agents/code-agent.toml`（仅 3 工具）。**先决策**：(A) 给 `.md` 补 frontmatter 或 (B) 扩展 `.toml`，再谈激活更多工具。`max_iterations` quick-win 已失效（默认已 50）。

3. **kernel-separation K1–K2**
   源：`../../deepseek/2026-07-17-kernel-k1-k2-fabric-traits-detailed-plan.md`（母计划 [`...kernel-application-layer-separation-plan.md`](../../deepseek/2026-07-17-kernel-application-layer-separation-plan.md)）。
   在 `fabric` 新增 `BudgetController`/`LeaseManager` trait，kernel `InMemory*` 实现之，getter 返回 `Arc<dyn Trait>`。编译期收敛，测试守护。与 grok 无重叠。

### 阶段 P0.5 —— 回归保护（硬前置，DeepSeek 母路线图 Phase 1）

> 母路线图明言「所有后续硬化工作都需要回归保护」。**在启动 P1 能力轨之前先建立测试地基。**

3b. **testing-infrastructure-hardening Phase 0–4**（TestAletheonBuilder + mock LLM/sandbox + 关键路径集成测试）
   源：`../../deepseek/2026-07-17-testing-infrastructure-hardening-plan.md`。
   自包含（仅 `tests/` + 一处生产改动 `KernelRuntime::with_clock`）。Phase 0 建 `tests/support/{mock_llm_provider,mock_sandbox,test_aletheon_builder}.rs`；Phase 1–4 补 TurnCoordinator / EventSourcedStore / daemon_turn+react / canonical_store 集成测试（当前这些路径**零直接测试**）。
   **build-on grok**：`MockSandbox` 实现 `SandboxBackend`（`fabric/src/types/sandbox.rs:77`，S1 所在文件）；TUI/JSON-RPC snapshot + fuzz 覆盖 G8；daemon_react 流测试触及 G2。纯增量测试，不改任何 fabric 类型。
   Phase 5–9（snapshot/fuzz/criterion/chaos/#[ignore] 文化）可在能力轨并行推进后补。

### 阶段 P1 —— 消费 grok fabric 地基（build-on，回归保护就位后）

4. **tool-execution-hardening Phase 1**（通用沙箱包裹）→ 见 [D1](D1-tool-execution-integration.md)
   **先做 §3 命名协调**，再实现 `ResolvedSandboxPolicy` + `resolve_profile`（消费 S1 的 `SandboxProfileConfig`），把 `SandboxConfig` 接上 policy，让 bubblewrap/process backend 施加 deny/roots/network，用 `resolve_strategy` 取代 `if tool_name=="bash_exec"` 门（**重新定位**该门，漂移到 `runner.rs:~401`）。

5. **multi-user M3**（显式 thread + 单客户端协议）→ 见 [D2](D2-multi-user-m3-m5.md)
   消费 G3（`prompt_queue::evaluate_edit/cancel` 作 interrupt 前置 `(thread_id,turn_id,operation_id)`）、G1（`WorkspaceIdentity` 复用 design §6.2/§7.2）。移除全局 default-session 切换（`legacy_session_service.rs:338-360`）。

6. **mcp-integration Phase 1**（统一 + 硬化）→ 见 [D3](D3-mcp-integration.md)
   统一两处 `McpServerConfig`（`cognit/src/config/mod.rs:664` + `corpus/tools/mcp/config.rs:23`）；**验证并修正** §3.7 trust 映射反转（`wrapper.rs` `Untrusted=>L2`，疑应 L1——**先验证再改**）；采纳 G7 `approved_for` 给 bearer/OAuth 加 endpoint-scoping。

### 阶段 P2 —— 深化（依赖 P1）

7. **tool-execution-hardening Phase 2–3**（exec-server 进程隔离 + 逃逸检测/网络策略）→ [D1](D1-tool-execution-integration.md)；Phase 2 的 `process/read` 流是 G2 `ToolEventSink` 的天然生产者。
8. **multi-user M4–M5**（durable recovery + diagnostics）→ [D2](D2-multi-user-m3-m5.md)；M4 消费 C1 compaction lineage + G5 lifecycle。
9. **mcp-integration Phase 2–4**（resources/notifications、elicitation、HTTP/OAuth polish）→ [D3](D3-mcp-integration.md)。
10. **structured-code-editing Phase 2–3**（streaming + model-awareness）；Phase 2 复用 G2 `TurnEventV1` 流。
11. **capability-activation Phase 3**（RPC + 父子 tier 强制）。
12. **kernel-separation K3–K5**（应用层改走 facade + CI 封边 + 聚合句柄收口）。
    源：[K3–K4](../../deepseek/2026-07-17-kernel-k3-k4-facade-and-ci-detailed-plan.md) · [K5](../../deepseek/2026-07-17-kernel-k5-aggregate-handle-detailed-plan.md)（K5 可选，单独评审）。

### 阶段 P3 —— 谨慎、改变真实行为

13. **conscious-core R1 → R2 → R3+metrics**（总览见 [engineering-plan](../../deepseek/2026-07-17-conscious-core-engineering-plan.md)，但**按批次详单取粒度**）：
    - R1：`../../deepseek/2026-07-17-conscious-core-r1-care-decision-detailed-plan.md`
    - R2：`../../deepseek/2026-07-17-conscious-core-r2-one-field-detailed-plan.md`
    - R3+度量：`../../deepseek/2026-07-17-conscious-core-r3-arbitration-and-metrics-detailed-plan.md`
    R3 改 `select_action` 显著性与 `GovernedCapabilityInvoker::invoke` 软否决/改序——**只收紧不放宽**。C1 compaction 是邻接但独立的 context 轴，勿混。R4（文档修正）已完成。

### 阶段 P4 —— 独立大工程（范围外提示）

14. **grok 剩余独立流**：G4（checkpoint/rewind Executive 集成）、G6（subagent 结算状态机）——见各自 exec spec，DeepSeek 未覆盖，按 grok spec 独立推进。
15. **platform-driver-hardware**：从零 effector/fieldbus/RT 栈，与本合并无交集，见 §5。

## 5. 明确范围外（列全，供执行者知情，但不在本合并序列内）

- **Platform B 线（实际硬件控制，从零）** —— 与 grok 机制无交集，独立评审：
  - [platform-driver-hardware-control（母）](../../deepseek/2026-07-17-platform-driver-hardware-control-plan.md)
  - [B0 契约+仿真](../../deepseek/2026-07-17-platform-b0-contract-and-sim-detailed-plan.md) · [B1-B2 bus+RT 回路](../../deepseek/2026-07-17-platform-b1-b2-bus-and-rt-loop-detailed-plan.md) · [B3 fieldbus failsafe](../../deepseek/2026-07-17-platform-b3-fieldbus-failsafe-detailed-plan.md)
- **Platform A 线（OS 多平台适配）** —— 桌面 HAL 可移植性，与 grok/能力硬化正交：
  - [A 线 OS 多平台](../../deepseek/2026-07-17-platform-a-os-multiplatform-detailed-plan.md)
- **conscious-core Phase F（continuous field）** —— 独立后续项目：[Phase F 详单](../../deepseek/2026-07-17-conscious-core-phase-f-continuous-field-detailed-plan.md)
- **审计报告 `01`–`09`** —— 已完成的代码级验证快照，历史记录（`../../deepseek/09-full-plan-audit.md` 等）。
- **已实现的 39 份计划**：历史记录，勿回填 checkbox（审计 09 §6 建议）。

## 6. 合并文档清单（本目录新增）

| 文档 | 内容 |
|---|---|
| `00-EXECUTION-INDEX.md` | 本文——主控索引 |
| [`D1-tool-execution-integration.md`](D1-tool-execution-integration.md) | tool-exec × S1/G2：命名协调 + 消费层任务 |
| [`D2-multi-user-m3-m5.md`](D2-multi-user-m3-m5.md) | M3–M5 × G1/G3/G5：Executive 接线 |
| [`D3-mcp-integration.md`](D3-mcp-integration.md) | mcp Phase 1–4 × G7 endpoint-scoping |

Grok fabric spec（`G1..G8`/`S1`/`C1`）与 DeepSeek 原始计划（`../../deepseek/*`）仍是各任务的详细来源；本目录负责**合并、去重、排序、协调冲突**。
