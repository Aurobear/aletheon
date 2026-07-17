# D1 合并可执行 Spec：Tool Execution Hardening × Grok S1/G2

> 合并 DeepSeek `../../deepseek/2026-07-17-tool-execution-hardening-plan.md` 与 Grok `S1-sandbox.md`（sandbox profile）+ `G2-streaming-tools.md`（streaming）。
> 执行前按 `00-EXECUTION-INDEX.md §0` 重新核对锚点（本流锚点已知漂移）。

## 1. 一句话

DeepSeek 要「让所有工具走沙箱 + 进程隔离 + 逃逸/网络策略」；Grok 已交付两块地基（S1 sandbox profile 源类型、G2 流式工具契约）。本文把两者拼成一条执行线：**先协调 S1 命名冲突 → 实现 profile 解析消费层 → 用 G2 流承载工具进度**。

## 2. 现状锚点（合并，需重新核对）

| 事实 | 锚点（DeepSeek 计划所述，标注漂移） |
|---|---|
| 仅 `bash_exec` 走沙箱 | `crates/corpus/src/tools/tools/runner.rs:~401`（计划写 369，**已漂移**，重新定位 `if tool_name == "bash_exec"`） |
| 未沙箱的 `tool.execute()` | `runner.rs:~449`（计划写 414，**已漂移**） |
| `SandboxConfig` 仅 workspace+env | `crates/fabric/src/types/sandbox.rs:25`（Grok S1 已在此文件**追加** profile 类型） |
| corpus `SandboxProfile` 存在但**无消费者** | `crates/corpus/src/security/sandbox/profile.rs:10`（read_roots/write_roots/deny_paths/network_enabled） |
| **Grok S1 已提交**：`SandboxProfileConfig`（源 DTO）+ `SandboxProfiles::merge_project_additive` + `ProfileName` | `fabric::types::sandbox`（commit `13e27987`），`grep` 确认 fabric 外无消费者 |
| **Grok G2 已提交**：`ToolExecutionEvent`/`ToolEventSink`/`tool_event_channel` + `TurnEventV1::ToolProgress` | `fabric::types::tool_stream`（commit `55e18eea`），emit 点仅 `ipc/stream.rs:206` |

## 3. ⚠ 第一步：协调 S1 命名冲突（阻塞后续，先做）

DeepSeek §3.1.1 想新建 `fabric` 的 `SandboxProfileConfig`（运行时字段）——**与 Grok S1 已提交的同名源 DTO 冲突**。裁定：

```text
可信源（已实现，S1 commit 13e27987）
  SandboxProfileConfig { extends, restrict_network, read_only, read_write, deny }
  SandboxProfiles { profiles }  + merge_project_additive（反 hollowing）
        │  resolve_profile(name, workspace, profiles)   ← 待实现（S1 spec §4.2）
        ▼
运行时解析结果（待实现，用 S1 spec §4.1 已命名的类型，勿新造同名）
  ResolvedSandboxPolicy { read_only_roots, read_write_roots, deny_exact, deny_globs, restrict_network }
        │  塞进 ▼
  SandboxConfig { workspace, environment, policy: Option<ResolvedSandboxPolicy> }  ← 扩展字段
```

**任务 D1-T0（命名协调）**：
- 不在 fabric 新建 DeepSeek 版 `SandboxProfileConfig`；改为实现 S1 spec §4.1 的 `ResolvedSandboxPolicy`。
- DeepSeek 计划里出现 `SandboxProfileConfig`（运行时语义）之处，一律读作 `ResolvedSandboxPolicy`。
- DeepSeek 的 `ToolExecutionStrategy`（`Sandboxed/InProcess/NetworkProxied/ExecServerRequired`）保留，与 S1 正交。

## 4. Phase 1 —— 通用沙箱包裹（消费 S1）

对应 DeepSeek Phase 1（~400 loc，1 PR）。

**任务序列**：
- **D1-T1**：实现 `ResolvedSandboxPolicy`（fabric，S1 spec §4.1）+ `resolve_profile(name, workspace, profiles) -> Result<ResolvedSandboxPolicy, ProfileResolveError>`（S1 spec §4.2，消费已提交的 `SandboxProfileConfig`/`SandboxProfiles`）。**测试**：内建 profile（workspace/read-only/strict）解析正确；credential 路径恒并入 `deny_exact`（S1 spec §6 T7）。
- **D1-T2**：`fabric::SandboxConfig` 追加 `policy: Option<ResolvedSandboxPolicy>`（默认 `None` = 等价当前）。`cargo check -p fabric`；确认现有构造点默认 None。
- **D1-T3**：`sandbox_glob.rs` glob 展开（S1 spec §4.1 上限常量，超限 `Err(GlobOverflow)` fail-closed）。测试含超限。
- **D1-T4**：新建 `crates/corpus/src/security/strategy.rs::resolve_strategy(tool_name, permission_level) -> ToolExecutionStrategy`（DeepSeek §3.1）。read-only 工具→`InProcess`；file_write/apply_patch/ebpf_compile/kernel_build/module_build/module_load/script_tool→`Sandboxed`。
- **D1-T5**：`SandboxExecutor::run_with_tool`（消费 `policy`）；bubblewrap/process backend 施加 `deny_exact`（bind-over）+ read/write roots + `restrict_network`（S1 spec §6 T10/T11）。
- **D1-T6**：`runner.rs:~401` 用 `resolve_strategy` 取代 `if tool_name=="bash_exec"` 门（**先重新定位该行**）。flag `grok_hardening.sandbox_profiles` 关时走旧路径（等价回归）。
- **D1-T7**：daemon profile 配置装配（全局 + 项目附加，用已提交的 `merge_project_additive`；配置来自可信 daemon 源，**非** repo `.grok/`）。

**验收（DeepSeek Ph1 + S1）**：
- audit log 中 file_write/apply_patch 带 `sandbox_backend`；
- canary：workspace 外 file_write 被沙箱阻止（不只 `validate_mutation_path`）；
- deny glob 阻止匹配路径；daemon profile 不被 repo 覆盖（`merge_project_additive` 已测）；
- flag 关闭 → 行为等价当前。

## 5. Phase 2 —— exec-server 进程隔离（生产 G2 流）

对应 DeepSeek Phase 2（新 crate `crates/exec-server/`，~1200 loc，2 PR）。

- **D1-T8**：`crates/exec-server/`（main/protocol/process/filesystem/sandbox），JSON-RPC over stdin/stdout + shared-secret handshake；`process/start|read|write|signal|terminate`、`fs/*`。
- **D1-T9**：daemon 在 `daemon/server.rs` spawn；`ExecServerClient` 在 `channel/daemon_adapter.rs`；feature-gate `--exec-server`，回退 Phase 1。
- **D1-T10（G2 接线）**：`process/read` 的流式输出 → 驱动 G2 `ToolEventSink`（`ToolContext`/`run_with_tool` 注入 sink）；`ToolExecutionEvent::Progress` → `TurnEventV1::ToolProgress` 在 daemon/turn 边界桥接（G2 spec §5 桥接任务 + `CapturePolicy::Streaming{max_total_bytes}` 作生产者）。**测试**：长命令产出多 progress + 一 terminal；progress 洪水不丢 terminal（G2 已测背压）。

**验收**：handshake 拒错误 secret；SIGTERM→500ms→SIGKILL；1MB/stream 有界；`fs/*` 施加 profile；symlink 先解析再检查；128 handle 无死锁；崩溃重连；两路径 bash_exec 输出一致；**progress 经 G2 流可观测**。

## 6. Phase 3 —— 逃逸检测 + 网络策略

对应 DeepSeek Phase 3（~500 loc，1 PR）。

- **D1-T11**：`escape_detector.rs::ShellEscalationDetector`（WarnOnly/Block）：heredoc/exec/eval/subshell/reverse-shell。
- **D1-T12**：fabric `NetworkPolicy`（allow/deny hosts、protocols、ports）+ `allows_url`；接入 web_fetch/web_search + bash_exec（需 approval）。

## 7. 依赖与顺序

```
D1-T0（命名协调，阻塞全部）
  └─ Phase 1: T1→T2→T3→T4→T5→T6→T7  （消费 S1，flag 后）
       └─ Phase 2: T8→T9→T10          （生产 G2 流）
            └─ Phase 3: T11, T12
```

## 8. 与 grok spec 的分工

- **S1 spec**（`S1-sandbox.md`）：源类型 + `resolve_profile` 设计 + 内建 profile 语义 —— 本文 Phase 1 落地其 consumer 层。
- **G2 spec**（`G2-streaming-tools.md`）：流式契约 + 桥接 + 背压 —— 本文 Phase 2 落地其生产者/桥接。
- 两者的 fabric 类型**已提交**；本文只做「接线」，不重定义类型（除 `ResolvedSandboxPolicy`/`SandboxConfig.policy`/glob，均 S1 spec 已设计未实现的部分）。
