# Wave 2：能力底座（Workspace Tools V2 + Corpus 依赖倒置 + Runtime API/Broker）Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Wave 1 收敛后的唯一主链上，把编码 Agent 的能力底座补齐——统一/加固 Workspace Tools（cwd/分页/结构化结果/hash/artifact），倒置 Corpus 的跨界依赖（不再依赖 cognit/mnemosyne），并引入通用 Runtime API + Broker（删除 Pi 字符串特判）。

**Architecture:** 三条并列但可分别落地的线：(A) Workspace Tools V2——read/ls/find/grep/edit/write/apply_patch 统一语义、optimistic concurrency、长输出进 Artifact Store。(B) Corpus 依赖倒置——MCP config 归 provider、credential 归 Credential Port、Exec Server 只依赖 sandbox/platform contract。(C) 新 crate `runtime-api` + `runtime-broker`——Manifest/WorkOrder/LaunchSpec/RuntimeEvent/RuntimeReceipt，Broker 按 capability/health/policy 选 Runtime；Goal/Executive 不再 import Pi 具体类型。

**Tech Stack:** Rust（corpus / executive / 新建 runtime-api / runtime-broker / exec-server crates）。

**环境说明:** cargo 可用；构建/测试走 `bash scripts/cargo-agent.sh test -p <crate> <filter>`，不要用裸 cargo。

**依赖:** Wave 1（唯一 TurnEngine + ResolvedTurnProfile + agent 工具可达）。Phase B 的"Dasein/Executive Command → Host Capability"依赖 Host **H1**（见 host-platform 计划）——本 Wave 与 Host H0/H1 并行，Command 迁移部分以 H1 就绪为前置。

**granularity 说明:** Workspace Tools（Phase A）与 Corpus 倒置（Phase B）有现成代码锚点，写到文件级；Runtime API/Broker（Phase C）引入新 crate，写到接口/类型 + 文件目标 + contract test 级别。

---

## 当前状态（已代码验证）

Workspace tools 缺口：
- `crates/corpus/src/tools/tools/file_read.rs` — 无 sha256/total_lines；`ToolResultMeta`（`crates/fabric/src/types/tool.rs:148-155`）无相应槽位。
- `crates/corpus/src/tools/tools/file_write.rs:98` — 无条件覆盖，无 expected hash。
- `crates/corpus/src/tools/tools/apply_patch.rs` — 无 expected hash / checkpoint / 事务。
- `crates/corpus/src/tools/tools/glob.rs:73` — 硬编码 1000，无 cursor。
- 长输出 8 KB 截断（`crates/cognit/src/harness/linear/tool_output.rs:4`）——需 Artifact Store 承接。

Corpus 反向依赖：
- `crates/corpus/src/tools/mcp/config.rs:54` — `pub use cognit::config::{McpServerConfig, ...}`。
- `crates/corpus/src/tools/mcp/auth.rs:7` — `use mnemosyne::credential::EmbeddingCredentialGrant`。
- `crates/exec-server/Cargo.toml:15` — `corpus = { path = "../corpus" }`（整个 corpus）。

Runtime 耦合：
- Goal 特判 `crates/executive/src/impl/goal/attempt_coordinator.rs:231`（`== PI_CODER_RUNTIME_ID`）。
- AgentControlService 字符串判断 `crates/executive/src/service/agent_control/mod.rs:740`（`.contains("pi")`）。
- 高层 `agent` 工具锁定 NativeCognit（`crates/executive/src/impl/daemon/bootstrap/runtime.rs:171`）。

---

## Phase A：Workspace Tools V2

### Task A1：ToolResultMeta 增加结构化字段

**Files:**
- Modify: `crates/fabric/src/types/tool.rs:148-155`（`ToolResultMeta`）

- [ ] 增加可选字段（向后兼容，`#[serde(default)]`）：`content_sha256: Option<String>`、`total_lines: Option<u64>`、`artifact_ref: Option<String>`、`truncated_bytes: Option<u64>`。
- [ ] 构建 fabric；确认现有构造点用 `..Default::default()` 不破坏。
- [ ] Commit。

### Task A2：file_read 返回 hash + total_lines

**Files:** `crates/corpus/src/tools/tools/file_read.rs`

- [ ] TDD：测试断言 read 结果 meta 含 `content_sha256` 与 `total_lines`。
- [ ] 实现：读文件时计算 sha256（`sha2` crate）与总行数，填入 meta；响应体也可附结构化头（path/start_line/end_line/total_lines/sha256）。
- [ ] `bash scripts/cargo-agent.sh test -p corpus file_read` → PASS。Commit。

### Task A3：file_write / apply_patch optimistic concurrency

**Files:** `crates/corpus/src/tools/tools/file_write.rs`、`crates/corpus/src/tools/tools/apply_patch.rs`

- [ ] input schema 增加 `expected_sha256: Option<String>`；写前读现文件 hash，不一致返回 `StaleWorkspaceView` 错误（要求 Agent 重读），而非静默覆盖。
- [ ] `apply_patch`：patch 前建 checkpoint（备份原文件到临时/artifact），patch 后产出 authoritative diff artifact；支持多文件事务或明确部分成功语义。
- [ ] TDD：stale hash 冲突用例；apply_patch checkpoint 回滚用例。
- [ ] 测试 + Commit（两文件可分两 PR）。

### Task A4：glob cursor + 稳定排序

**Files:** `crates/corpus/src/tools/tools/glob.rs`

- [ ] input schema 增加 `cursor`/`limit`；`max_results` 由参数控制（去硬编码 1000）；统一排除 `.git`/`target`/`node_modules`；稳定排序；返回 continuation cursor + truncated 状态。
- [ ] TDD：>limit 时返回 cursor，二次调用续取。
- [ ] 测试 + Commit。

### Task A5：Artifact Store 承接长输出

**Files:** 新增 `crates/corpus/src/tools/artifact/`（store + `artifact_read` 工具）；接入 `crates/cognit/src/harness/linear/tool_output.rs`

- [ ] 定义 Artifact Store（内容寻址，落 data_dir）；工具产大输出时写 artifact，模型上下文只留结构化摘要 + `ArtifactRef`。
- [ ] 新增 `artifact_read` 工具（分页读取）。
- [ ] `tool_output.rs` 截断路径改为：超阈值 → 存 artifact + 返回 ref，而非纯 head/tail 丢弃。
- [ ] TDD：大输出往返（写 artifact → artifact_read 分页）。
- [ ] 测试 + Commit。

**Phase A 验收:** daemon cwd≠workspace 时正确；大仓库搜索遵守全局上限并可分页；stale edit 不覆盖用户修改；workspace escape 被拒；编译错误/大 diff/测试失败经 artifact 完整可读。

---

## Phase B：Corpus 依赖倒置

### Task B1：MCP config provider-owned

**Files:** `crates/corpus/src/tools/mcp/config.rs:54`、config 定义迁移点

- [ ] 把 `McpServerConfig/McpTransportConfig/McpTrustLevel` 的**定义**从 `cognit::config` 移到 corpus 的 MCP provider 模块（provider-owned）；Executive 解析后投影成 provider config 传入。
- [ ] 删除 `corpus/mcp/config.rs:54` 的 `pub use cognit::config::...`。
- [ ] 构建 corpus + cognit + executive；调整调用点。
- [ ] Commit。

### Task B2：Credential Port（凭据脱离 Memory）

**Files:** `crates/corpus/src/tools/mcp/auth.rs:7`、新增 Credential Port trait

- [ ] 定义 `CredentialPort`（Security/Credential 归属），MCP auth 依赖该 port 而非 `mnemosyne::credential::EmbeddingCredentialGrant`。
- [ ] Executive 在 composition 处把凭据来源注入 port。
- [ ] 删除 `corpus/mcp/auth.rs:7` 对 mnemosyne 的直接 import。
- [ ] 构建 + Commit。

### Task B3：Exec Server 最小依赖

**Files:** `crates/exec-server/Cargo.toml:15`、`crates/exec-server/src/filesystem.rs:276`

- [ ] 把 exec-server 用到的 corpus 功能（如 `structured_patch`）抽到 sandbox/platform contract 或独立小 crate；exec-server 改依赖该 contract，而非整个 corpus。
- [ ] 删除 `exec-server/Cargo.toml` 对 corpus 的 path 依赖。
- [ ] 构建 exec-server + Commit。

### Task B4：架构门禁收紧

**Files:** `scripts/architecture-check.sh:575-583`、`architecture-status.toml`

- [ ] 从 `architecture-check.sh` 的 `reviewed` 集合移除已消除的 `("corpus","cognit")`、`("corpus","mnemosyne")`、`("exec-server","corpus")`（三项完成后逐一移除）。
- [ ] 更新 `architecture-status.toml` 对应 `reviewed_dependency` 条目为已解决。
- [ ] 运行门禁确认 `corpus→cognit/mnemosyne = 0`、`exec-server→corpus = 0`。Commit。

**Phase B 验收:** `corpus` 不再依赖 `cognit`/`mnemosyne`；exec-server 不依赖整个 corpus；架构门禁 reviewed 例外相应减少。

---

## Phase C：Runtime API + Broker

### Task C1：新建 runtime-api crate

**Files:** 新增 `crates/runtime-api/`（`manifest.rs`/`work_order.rs`/`lifecycle.rs`/`events.rs`/`receipt.rs`/`transport.rs`）

- [ ] 定义（参考 audit 文档 §8）：`RuntimeManifest`（id/aliases/capabilities/interaction_modes/transports/workspace_mode/resumability/tool_governance）、`WorkOrder`（objective/task_kind/acceptance_criteria/context/workspace/required_capabilities/verification）、`RuntimeLaunchSpec`、`CapabilityRuntime` trait（prepare/start/send/snapshot/checkpoint/cancel/settle）、`RuntimeMessage`、`RuntimeEvent`（标准事件流）、`RuntimeReceipt`（status/usage/evidence/artifacts/workspace_delta/commands/tests/diagnostics/verification）、`CompletionStatus`。
- [ ] `runtime-api` 不依赖任何具体 Runtime（依赖规则：runtime-api ← runtime-pi/native/codex；Executive ← runtime-api + runtime-broker）。
- [ ] 构建 + 单元测试（类型 round-trip / manifest 序列化）。Commit。

### Task C2：新建 runtime-broker crate

**Files:** 新增 `crates/runtime-broker/`（`registry.rs`/`selector.rs`/`health.rs`/`policy.rs`）

- [ ] `RuntimeSelector`（Auto / Named(alias) / RequiredCapabilities）；Broker 负责 alias 解析、capability 匹配、health 检查、workspace mode 匹配、policy、fallback、admission。
- [ ] 身份分层：用户别名 `pi` / 稳定 RuntimeId `pi/coding` / 实现版本 / 运行实例。
- [ ] 构建 + 测试（选择器 + health fallback）。Commit。

### Task C3：Native Cognit 与 Pi 实现 contract + 删除特判

**Files:**
- 新增 `crates/runtime-native-cognit/`（包裹现 NativeCognitRuntime）
- Modify: `crates/executive/src/impl/goal/attempt_coordinator.rs:231`（删 `PI_CODER_RUNTIME_ID` 特判）
- Modify: `crates/executive/src/service/agent_control/mod.rs:740`（删 `.contains("pi")`，改由 Manifest 的 workspace/persistence capability 声明驱动）
- Modify: `crates/executive/src/impl/daemon/bootstrap/runtime.rs:171`（高层 delegate 经 Broker 选 Runtime，不锁定 NativeCognit）

- [ ] Native Cognit 与（现有）Pi runtimes 各实现 `CapabilityRuntime` + 提供 `RuntimeManifest`。
- [ ] 用 Manifest 的 `workspace_mode`/`resumability` 取代 `.contains("pi")` 的存储/worktree 分支判断。
- [ ] Goal 路径改用 `WorkOrder` + Broker，不再解析 `PiAttemptRequest`/判断固定 runtime id。
- [ ] 一套 Runtime contract tests（所有 adapter 共用）：manifest 完整、start 只发一次 Started、tool event 有序、cancellation 终止进程树、terminal event 唯一、receipt 可验证、workspace policy 不可绕过。
- [ ] 构建 + contract tests。Commit（分多 PR：先 native adapter，再删 Goal 特判，再删字符串判断）。

**Phase C 验收:** Executive/Goal 不 import Pi 具体类型；可通过 alias/capability 选 Runtime；Native/Pi 产生同一标准事件与 Receipt；Runtime 不健康时 Broker 能拒绝或 fallback。

---

## 自审对照（roadmap Wave 2）

| roadmap 项 | 覆盖 |
|---|---|
| Workspace Tools V2（audit PR2） | Phase A |
| Corpus 收敛 + 依赖倒置（arch A2） | Phase B |
| Runtime API + Broker（audit PR3 / arch A4a） | Phase C |
| 删除 Pi 字符串特判 | Task C3 |
| corpus→cognit/mnemosyne = 0 | Task B1/B2/B4 |

---

## 建议 PR 切分

- PR-W2-A1..A5：Workspace Tools（meta → read → write/patch → glob → artifact），逐工具独立 PR。
- PR-W2-B1..B4：MCP config → Credential Port → Exec Server → 门禁收紧。
- PR-W2-C1..C3：runtime-api → runtime-broker → adapters + 删特判。

Phase A/B/C 相互独立，可交错；但每个 PR 单独减少兼容面或增加可验证能力。

## 下一步

Wave 2 完成后，Wave 3 把 Pi 做成默认 Coding Runtime（基于本 Wave 的 runtime-api/broker）并接入 CodingCompletionVerifier。
