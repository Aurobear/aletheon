# Aletheon 统一记忆网关与服务端 Memory Agent 设计

**Date:** 2026-08-01

**Status:** Proposed for user review; architecture decisions are resolved, implementation plan pending

**Scope:** 完善 Aletheon 自身 Mnemosyne 记忆；让 Claude Code、Codex、Aletheon 通过同一套受治理接口使用 GBrain；由 Aletheon 托管服务端 Memory Agent，周期性检查、评分、整理、冲突处理和遗忘。
**Repositories:**

- Aletheon: `/home/aurobear/Workspace/aletheon`
- Aurb: `/home/aurobear/Workspace/agent/aurb`
- GBrain: `/home/aurobear/Workspace/agent/gbrain`

> 本文是设计规格，不宣称功能已经部署。实施前必须先由用户验收本文，再另写文件级 implementation plan。

## 0. 已锁定决策

| 编号 | 决策 |
|---|---|
| D1 | **Aletheon Mnemosyne 是运行时记忆权威**：Session、Goal、Agent、Task、Core 与当前行为依据以本地受治理记录为准。 |
| D2 | **GBrain 是跨会话、跨客户端、跨工作空间的长期知识权威**，不是 Aletheon 当前身份、权限或 CoreState 的直接控制面。 |
| D3 | **Claude Code/Codex 只是观察与查询客户端**；它们不能自行把文本标成长期事实、不能直接决定 confidence/authority/status/scope，也不持有自动写 GBrain 的凭证。 |
| D4 | **Aletheon 托管统一 Memory Gateway 和服务端 Memory Agent**。Aletheon 内核只定义通用 memory contracts、调度、治理与收据；GBrain 细节留在 supplemental adapter。 |
| D5 | **工作空间必须由宿主从 cwd/已认证连接推导**，不得从 prompt、仓库名字符串或模型自述推断。 |
| D6 | **本地先提交，远端异步投影**。GBrain 故障不能阻断正常 turn；队列满、死信和投影延迟必须显式降级，不能静默丢失或伪报成功。 |
| D7 | **两阶段晋升**：原始观察先进入本地 intake；只有通过确定性门禁、评分、冲突检查和必要复核的候选才能成为 durable local memory；只有更严格子集才能投影 GBrain。 |
| D8 | **Aletheon 自己维护 Agent 生命周期**。Memory Agent 可以使用可替换的 AgentRuntime（native-cognit、Pi 或未来 runtime），但 runtime 只返回结构化提案；权威写入由 Aletheon host 校验并执行。 |
| D9 | **每个工作空间一个 GBrain Source，加一个 `personal` Source**。未知/未绑定 workspace 只写本地，不回退到 `default`。 |
| D10 | **升级兼容优先于自动修复**。Aletheon/GBrain schema 或 capability 不兼容时，Memory Agent 保留 backlog、停止远端写入、继续本地 recall，并给出可操作健康状态。 |
| D11 | **不增加任何按自然语言短语、语言、仓库名或固定路径触发的生产逻辑**。显式“保存记忆”必须由客户端转换成 typed user action，不由服务端匹配 prompt 文本。 |
| D12 | **不保留两套自动长期记忆决策链**。Aurb 现有 Claude/Codex 直连 GBrain capture 在迁移验收后退出自动写路径；可保留只读人工 MCP。 |

## 1. 当前代码事实与缺口

### 1.1 Aletheon 已有真底座

Mnemosyne 已有 10 类记录、生命周期状态、authority、sensitivity、provenance 与硬上限（`crates/mnemosyne/src/model/record.rs:12-23`、`:27-46`、`:50-77`、`:153-203`）。它不是空壳：

- 本地 recall 已合并 RecallMemory、FactStore、EpisodicMemory 与 CoreMemory，并应用 scope/authority/sensitivity 过滤（`crates/mnemosyne/src/service.rs:639-768`）。
- CompositeMemoryService 先提交本地，再把选中事件放进 supplemental spool；远端降级不反转本地成功（`crates/mnemosyne/src/composite_service.rs:151-187`）。
- 本地与 supplemental recall 并行、有独立 timeout，并在合并前记录 degraded source（`crates/mnemosyne/src/composite_service.rs:190-266`）。
- Agent/Task scope 由 AgentControl 的已验证身份生成，query/tool 字符串不能扩大 scope（`crates/mnemosyne/src/agent_scope.rs:42-95`）。
- embedding、持久向量库、consolidation、retention 和 promotion 已有 composition 挂点（`crates/executive/src/host/daemon/bootstrap/request.rs:651-783`）。
- Mnemosyne 已明确把本地 runtime concrete handles 与产品中立 supplemental contracts 分开（`crates/mnemosyne/src/lib.rs:114-146`）。

因此本设计不是重写 Mnemosyne，而是补齐统一入口、workspace scope、候选评分、跨客户端桥接、服务端维护循环和真正的 source routing。

### 1.2 Aletheon 当前缺口

1. `MemoryScope` 只有 Global/Principal/Session/Goal/Agent/Task，没有 repository workspace（`crates/mnemosyne/src/model/scope.rs:3-12`）；`RecallRequest` 也只有 `session + query`，没有 host-verified workspace context（`crates/mnemosyne/src/service.rs:68-81`）。
2. `CompositeMemoryService::selected` 目前只把 ArchitectureDecision 与 GoalOutcome 送入 supplemental（`crates/mnemosyne/src/composite_service.rs:128-133`），没有统一候选/评分/投影策略。
3. versioned client protocol 目前只含 initialize/snapshot/subscribe/chat/approval/cancel（`crates/fabric/src/protocol/client.rs:901-909`）；没有供 Claude/Codex 使用的 versioned observe/recall/receipt API。
4. 旧 admin surface 有 `memory.forget`，但它不是新的跨客户端 Memory Gateway；forget/tombstone 的受认证 use case 已存在，应复用而不是再造无治理删除路径（`crates/executive/src/application/request_use_cases.rs:848-933`）。
5. supplemental config 仍是进程级 `read_sources + write_source` 标量（`crates/executive/src/composition/config/supplemental_memory.rs:59-100`），无法按 workspace 选择不同 source/credential。
6. `SupplementalMemoryTransport::put_page` 不携带 destination，而 query 才携带 source（`crates/mnemosyne/src/backends/supplemental/backend.rs:68-91`）。对 GBrain 而言写 source 由认证身份决定，单改 page frontmatter 或 slug 不会改变真实 Source。
7. Fabric 中 checkpoint 和 trust 各有一份结构相同但类型独立的 `WorkspaceIdentity`（`crates/fabric/src/types/workspace_checkpoint.rs:34-39`、`crates/fabric/src/types/workspace_trust.rs:53-62`），继续复制会产生身份漂移。

### 1.3 Aurb 当前造成双轨

Aurb 已经有很好的客户端安全积木，但自动链路绕过 Aletheon：

- prompt hook 直接调用 GBrain recall，并从全局 `GBRAIN_SOURCE` 取 source（`/home/aurobear/Workspace/agent/aurb/src/hooks/recall.sh:7-60`）。
- Stop hook 读取 transcript 后进入 Aurb capture pipeline（`/home/aurobear/Workspace/agent/aurb/src/hooks/session-end.sh:1-41`）。
- pipeline 把 `GBRAIN_SOURCE` 或静态 config 写入 envelope（`/home/aurobear/Workspace/agent/aurb/src/lib/gbrain/session_pipeline.py:321-348`），writer 调 `put_page` 时没有 source 参数（同文件 `:313-319`）。
- Aurb 会给 Codex 注册直接 GBrain MCP 和 bearer token（`/home/aurobear/Workspace/agent/aurb/scripts/lib/config/generate_provider_env.py:526-551`）。

这意味着客户端、Aurb session-capture service、Aletheon Mnemosyne 各自可能做判断，无法形成单一审计链。迁移后 Aurb 仍负责**客户端 payload 适配、transcript 所有权校验、首轮 scrub、插件部署**，但不再负责长期记忆语义裁决。

### 1.4 GBrain 的真实 source 边界

GBrain 的 Brain 是数据库，Source 是同一 Brain 内的 repo/workspace 内容边界；slug 仅在 Source 内唯一（`/home/aurobear/Workspace/agent/gbrain/docs/architecture/brains-and-sources.md:33-56`）。远程写入的 Source 来自 token/OAuth grant：

- HTTP transport 把 token 的 `source_id/allowedSources` 放入 auth context（`/home/aurobear/Workspace/agent/gbrain/src/mcp/http-transport.ts:207-231`）。
- `put_page` 最终把 `ctx.sourceId` 传给存储（`/home/aurobear/Workspace/agent/gbrain/src/core/operations.ts:825-855`）。
- 官方部署文档明确 `--source` 控制写 authority，`--federated-read` 独立控制读集合（`/home/aurobear/Workspace/agent/gbrain/docs/mcp/DEPLOY.md:120-136`）。
- `whoami` 当前只返回 transport/client/scopes，不返回实际 write/read source grants（`/home/aurobear/Workspace/agent/gbrain/src/core/operations.ts:3676-3724`）。

所以 Aletheon 不能靠向 `put_page` 添加一个未经服务端授权的 source 字段来路由；它必须选择绑定到目标 Source 的 credential，并在握手时验证 grant。

## 2. 目标与非目标

### 2.1 目标

1. 所有三类客户端共享一个 versioned Memory Gateway：observe、recall、feedback、receipt、workspace binding、forget/retention admin。
2. Aletheon 原生 turn 与外部 Claude/Codex observation 进入同一 intake、评分、promotion 和 retention 规则。
3. 为 Mnemosyne 增加 host-derived workspace ancestry，支持 session-local、workspace-long-term、principal-global 三层 recall。
4. 把 GBrain Source 与 Aletheon WorkspaceMemoryKey 显式绑定，读写严格按 credential grants 收敛。
5. 用 Aletheon Memory Agent 周期性执行：候选提取、评分、去重、冲突检测、合并、过期、远端投影、健康检查与受控修复。
6. 让 Aletheon 更新、GBrain 更新、provider 暂停或 Memory Agent 崩溃都不会损坏本地权威或阻断开发工作。
7. 以不可被加权平均掩盖的 gate + scorecard 衡量实际记忆能力。

### 2.2 非目标

- 不把 GBrain 代码、taxonomy、doctor 逻辑搬进 Mnemosyne core。
- 不让 LLM/child Agent 直接写 SQLite、CoreMemory 或 GBrain。
- 不把全部 raw transcript 永久保存到 GBrain。
- 不自动创建/删除 GBrain Sources；source provisioning 是显式 admin/migration 操作。
- 不把 Aletheon CoreState 投影成可被跨客户端文本覆盖的 GBrain 页面。
- 不在本设计阶段实现代码、部署 unit 或修改运行配置。

## 3. 权威模型与数据所有权

```text
                       authenticated local socket
 Claude Code ─┐        + versioned protocol
 Codex ───────┼──> Aletheon Memory Gateway
 Aletheon ────┘              │
                              ├── durable intake journal
                              ├── Mnemosyne runtime authority
                              │    Session / Goal / Agent / Task / Core
                              ├── Memory Agent proposals
                              │    extract / score / merge / conflict / retain
                              └── SupplementalMemoryPort (product-neutral)
                                      │ local-first outbox
                                      v
                                  GBrain adapter
                                      │ source-bound credential
                                      v
                             GBrain Brain
                              ├── source: ws-<workspace>
                              ├── source: personal
                              └── explicit shared sources
```

### 3.1 谁能决定什么

| 动作 | 客户端 | Memory Agent runtime | Aletheon host | GBrain |
|---|---:|---:|---:|---:|
| 提交 observation/query/feedback | ✅ | ✅，作为内部 caller | ✅ | ❌ |
| 声明 principal/workspace/authority/status | ❌ | ❌ | ✅ | 仅验证其 token scope |
| 提取候选、提出摘要/冲突判断 | ❌ | ✅ | 可用 deterministic fallback | 可作为 supplemental analysis |
| 晋升/拒绝/合并/遗忘本地记录 | ❌ | 仅 proposal | ✅ | ❌ |
| 选择 GBrain destination credential | ❌ | ❌ | ✅ adapter registry | 验证 grant |
| 持久写 GBrain page | ❌ 自动写 | ❌ | ✅ 经 outbox/adapter | ✅ 最终存储 |
| 改变 Aletheon CoreState | ❌ | ❌ | ✅ 经更严格 Core policy | ❌ |

### 3.2 MemoryKind 的存放策略

| MemoryKind | 本地 | GBrain 首期策略 |
|---|---|---|
| Message / ToolOutcome | durable intake/短期、受 retention | 永不直接投影 |
| Reflection | 本地候选 | 永不直接投影 |
| GoalOutcome / ArchitectureDecision | 本地 current | 评分通过后投影 workspace Source |
| SemanticFact / Procedure | 本地 verified | 评分、证据、冲突和敏感度均通过后投影 workspace 或 personal Source |
| Episodic | 本地 | 首期不投影；避免把叙事性 session 摘要误当长期知识 |
| CoreState | 本地最高 authority | 不投影；跨客户端偏好另存为 verified SemanticFact，不能直接控制 Core |
| ExternalReference | 本地 untrusted reference | 不二次投影 |

### 3.3 Aletheon 本地记忆补全口径

“完善自身记忆”不是简单打开一个 embedding 开关，而是把现有存储统一到可审计生命周期：

1. 新增 durable intake/lifecycle ledger，所有 native/external observation 先取得幂等 intake receipt；现有 RecallMemory、FactStore、EpisodicMemory、CoreMemory、vector store 作为受治理 projection 继续复用，不再各自产生无法关联的“成功”。
2. 每次 projection 都以同一个 record/intake ID 和 revision 关联；跨 SQLite 文件无法做单事务时，使用 durable projection job + terminal receipt 恢复，不能靠 best-effort 日志宣称完成。
3. SemanticFact/Procedure 必须从 candidate 经 evidence/score/conflict gate 后进入 current；Message/ToolOutcome/Reflection 不能绕过候选层直接污染长期 recall。
4. CoreMemory 只接受 operator policy 或受审查 promotion receipt；高 confidence 本身不足以成为 ApprovedCore。
5. lexical、vector、episodic、core recall 保留各自 source metrics，但统一经过 host-derived ancestry、temporal/evidence filter、dedup 和 byte/item budget。
6. 本地 embedding 是可选增强而不是 durability 依赖。provider 不可用时 lexical/episodic/core recall 继续并显式降级；后台 stale/backfill 状态由 Memory Agent 检查。
7. 每条长期记忆必须可回答：来自哪些 observation、为何晋升、当前 revision、适用 workspace/principal、何时过期、是否已投影及如何遗忘。

## 4. Workspace 身份与分组

### 4.1 统一现有 WorkspaceIdentity

不再新增第三份 workspace identity。实施时把 checkpoint/trust 的重复结构收敛到 Fabric 单一 `WorkspaceIdentity`，原模块通过 re-export/type alias 兼容。其字段仍是：

```text
WorkspaceIdentity
├── canonical_path       host canonicalized, never model-derived
└── repo_fingerprint     normalized remote hash, optional
```

Memory 层从它派生不可伪造的 `WorkspaceMemoryKey`：

```text
repo fingerprint present:
  ws:repo:<repo_fingerprint>

no repository fingerprint:
  ws:local:<sha256(machine_installation_id || canonical_path)>
```

原始 canonical path 不投影 GBrain；page 只存 opaque workspace key、可选 repo fingerprint 和经过 scrub 的 display label。

### 4.2 MemoryScope 扩展

`MemoryScope` 新增 `Workspace(String)`，`ScopeAncestry` 新增 `workspace_id`。scope 仍是一条记录的主归属：

- raw turn observation → Session
- child draft → Agent/Task
-项目长期知识 → Workspace
- 跨项目个人偏好 → Principal
- approved behavior/core → 现有 CoreState 路径

Recall 使用 host 构造的 ancestry 同时检查 Principal、Workspace、Session、Goal、Agent、Task；client request 不能直接填任一 authority-bearing ID。

### 4.3 GBrain Source binding

Aletheon 增加 durable `workspace_memory_bindings` registry：

```text
WorkspaceMemoryBinding/v1
├── workspace_key
├── principal_id
├── backend_id                  opaque, e.g. supplemental/default
├── write_destination_handle    opaque adapter handle, not token
├── read_destination_handles[]  workspace + personal + explicit shared
├── expected_write_source       operator-visible source ID
├── expected_read_sources[]
├── credential_ref              secret-store reference only
├── state                       active | local_only | incompatible | revoked
└── verified_capability_digest
```

规则：

1. 新 workspace 默认 `local_only`，绝不写 `default`。
2. 绑定必须由本地 authenticated admin 对 canonical cwd 执行；不能由模型或 repo 文件自动授权。
3. 标准读集合为 `[workspace-source, personal]`；shared source 逐项显式增加。
4. credential 只存在 host secret store/systemd credential 中，不进入 config dump、memory record、Agent prompt 或 receipt。
5. Aletheon adapter 握手必须验证 GBrain 返回的 write source/read sources 与 binding 一致；不一致即 `incompatible`，停止远端写入。

为支持第 5 条，GBrain `whoami` 需要在不暴露 token 的前提下返回：

```json
{
  "transport": "oauth",
  "client_id": "...",
  "scopes": ["read", "write"],
  "write_source": "aletheon",
  "read_sources": ["aletheon", "personal"]
}
```

在该字段部署前，旧 credential 只能用于显式 legacy binding；不允许把“配置宣称的 source”当成已验证事实。

## 5. Versioned Memory Gateway contracts

所有 wire types 放在 Fabric protocol/application contracts；Mnemosyne concrete DB type 不直接暴露。协议在 initialize 阶段协商 `memory_gateway_v1` capability。旧 client 没有 capability 时继续原行为，不能调用新方法。

### 5.1 `memory.observe/v1`

```text
MemoryObservationRequestV1
├── observation_id        client-generated idempotency key, bounded
├── client_session_id     client namespace内的 session id
├── client_turn_id?       optional correlation only
├── working_dir           untrusted selection; host canonicalizes/resolves
├── kind                  user_message | assistant_message | tool_outcome |
│                         task_outcome | explicit_note | correction | feedback
├── content               already client-scrubbed, bounded UTF-8
├── occurred_at?
├── source_refs[]         bounded hashes/IDs, never authority
├── sensitivity_hint      public | internal | confidential | restricted
└── explicit_user_action  typed bool issued by explicit client UI/command
```

服务端覆盖/生成：principal、connection/client kind、WorkspaceIdentity/WorkspaceMemoryKey、MemoryScope、provenance、record ID、authority、status、confidence、observed time。服务端可以提高 sensitivity，不能因 client hint 降低它。

`explicit_user_action` 只提高“future utility”输入，不绕过 secret、evidence、scope 或 conflict gate；服务端不得扫描 content 判断“记住这个”。

立即返回：

```text
MemoryObservationReceiptV1
├── observation_id
├── durable_intake_id
├── intake_status          observed | duplicate | rejected
├── workspace_state        bound | local_only | incompatible
├── projection_state       not_eligible | pending_evaluation
└── reason_code?           typed, sanitized
```

`observed` 只表示本地 intake 已 durable，不表示已经成为长期记忆。

### 5.2 `memory.receipt.get/v1`

候选处理是异步的，客户端必须读取权威终态，不能把 child/tool 的异步返回当完成：

```text
MemoryLifecycleReceiptV1
├── durable_intake_id
├── revision
├── state
│   observed | evaluating | rejected | promoted_local |
│   projection_queued | projected_remote | projection_failed |
│   superseded | tombstoned
├── resulting_record_ids[]
├── remote_receipt_ids[]
├── scorecard?
├── reason_codes[]
└── terminal_at?
```

`rejected/promoted_local/projected_remote/projection_failed/superseded/tombstoned` 是各阶段明确终态；`projection_failed` 不反转 `promoted_local`。

### 5.3 `memory.recall/v1`

```text
MemoryRecallRequestV1
├── request_id
├── client_session_id
├── working_dir
├── query
├── max_items
├── max_content_bytes
├── include_historical
└── requested_kinds[]?
```

Host 绑定 principal/workspace/session ancestry，并把 client limits clamp 到 server policy。返回 item 含 record ID、kind、scope label、authority、temporal state、evidence、score、source/provenance、content 与 `untrusted_reference` 标志。GBrain 内容始终是 untrusted reference，不能作为 tool/policy instruction。

默认 recall 顺序不是硬拼字符串，而是同一 pipeline 的分层候选：

1. current Session/Goal/Agent/Task；
2. current Workspace；
3. Principal/Core；
4. GBrain workspace + personal supplemental；
5. 显式 shared source。

所有层统一去重、byte/item budget、temporal filter 和 evidence labeling。

### 5.4 `memory.feedback/v1`

反馈也是 observation，不是直接 update：

```text
MemoryFeedbackV1
├── target_record_id
├── signal    useful | not_useful | incorrect | stale | sensitive | corrected
├── correction_text?
└── working_dir
```

Host 验证 target 在 caller ancestry 内可见；`incorrect/sensitive` 触发高优先级 review，仍需产生 revisioned receipt。

### 5.5 Admin contracts

复用现有 authenticated forget/retention use cases，增加 versioned admin endpoints：

- `memory.workspace.preview_bind`
- `memory.workspace.bind`
- `memory.workspace.unbind`
- `memory.forget.preview`
- `memory.forget.apply`
- `memory.maintenance.status`
- `memory.maintenance.run`（typed phase + dry-run；破坏性 repair 仍需 approval）

普通 Claude/Codex connection 不获得 admin capability。

## 6. 服务端 Memory Agent

### 6.1 进程与责任边界

推荐独立的 user service：

```text
aletheon.service
  owns socket, identities, DB writes, queues, policy, receipts

aletheon-memory-agent.service
  /usr/bin/aletheon memory-agent serve --official-user-socket
  claims bounded maintenance leases
  requests AgentRuntime for semantic proposals
  submits proposals back to daemon
  never opens Mnemosyne/GBrain DB directly
```

独立 service 的好处是 Memory Agent/provider 失败不拖垮交互 daemon；它仍由同一个已安装 Aletheon binary 与协议治理。若 service 未运行，turn、local record、local recall 继续；只有候选处理/远端投影延迟。

### 6.2 Aletheon 与 Pi 的关系

Aletheon 维护 Memory Agent，不把责任交给 Pi：

```text
MemoryMaintenanceController (Aletheon authority)
       │ typed MemoryMaintenanceTask/v1
       v
AgentRuntimeRegistry
  ├── native-cognit
  ├── pi-rpc
  └── future runtime
       │ structured proposal only
       v
Aletheon verifier + deterministic apply
```

Runtime 选择来自 capability manifest、健康度、预算和 effective config；不能按 prompt、品牌或模型自述选择。任何 runtime 都不能得到 DB/GBrain write credential。没有合适 runtime 时只做 deterministic maintenance（retention、outbox、health），语义候选保持 pending。

### 6.3 调度

调度由 durable queue + lease 驱动，默认 policy 可配置且 versioned：

| 触发 | 默认 | 工作 |
|---|---:|---|
| intake event | 立即唤醒 | scrub、候选提取、初评、去重 |
| micro maintenance | 每 15 分钟 | pending retry、conflict queue、projection receipts、dead-letter classify |
| daily audit | 每日 03:15 | stale embeddings、unresolved candidates、source binding、backend doctor、quality benchmark sample |
| weekly retention | 每周一次 | expiration preview、supersession compaction、orphan review、备份/恢复抽检 |

相同 `(phase, scope, watermark)` 只能有一个有效 lease；每次 run 有 deadline、provider round、retry、tool call、input/output byte 和 projected-write caps。超时后 lease 可恢复，已经持久化的幂等 step 不重复写。

### 6.4 评分机制

先 hard gates，后 0–100 score。权重在 versioned policy config 中，不散落硬编码；模型只返回每个轴的结构化证据，host 重新计算总分。

**Hard gates：**

- secret/sensitive scrub 未通过；
- provenance 不完整或 scope 不可验证；
- content 为空/超限/包含控制指令；
- 只有模型自述、无 observation/evidence；
- 与 approved Core 冲突且未人工 adjudicate；
- GBrain binding/grant 未验证（仅阻断 remote projection，不阻断 local candidate）。

**默认评分轴：**

| 轴 | 分值 | 依据 |
|---|---:|---|
| evidence/provenance | 0–25 | 多个独立 event、工具终态、验证记录、当前代码证据 |
| future utility | 0–20 | 可复用决策、约束、修复、偏好；typed explicit action 可加但不越过 gate |
| stability | 0–15 | 不是短时日志/一次性状态；有 valid_from/until |
| novelty/dedup | 0–15 | 相对现有 current record 的新增信息 |
| scope fit | 0–10 | Session/Workspace/Principal 归属明确 |
| verification | 0–15 | 测试、终态 receipt、交叉来源支持 |
| privacy risk | 0 至 -25 | PII、路径、账号、凭证、敏感上下文 |
| contradiction risk | 0 至 -20 | 与 current/approved facts 未解决冲突 |

默认决策：

- `>= 75` 且无 hard gate：promote local；
- `55..74`：保留 candidate，等待重复证据、typed feedback 或 review；
- `< 55`：reject/short retention；
- remote projection：必须先 `promoted_local`，score `>= 80`，kind 在 allow-list，sensitivity 为 Public/Internal，workspace binding verified；
- ApprovedCore 永远不能由这个分数自动生成。

阈值/权重更改必须产生 policy version 和审计事件；不能为了测试 prompt 临时调参。

### 6.5 冲突、合并与遗忘

1. normalized content hash 处理精确重复；semantic duplicate 只产生 merge proposal。
2. 新事实与 current record 冲突时，记录 `ConflictSet`，不覆盖旧值。
3. 只有带 evidence/temporal ordering 的决策能 supersede；无法判定则双保留并降低 recall authority。
4. feedback `incorrect/sensitive` 进入高优先队列；sensitive 可先 recall quarantine，再走受审计 tombstone。
5. forget 先 preview，再 local tombstone，再 durable remote reconciliation；`remote_pending` 必须可观察，沿用现有 ForgetReceipt 语义（`crates/mnemosyne/src/service.rs:273-292`）。

## 7. Product-neutral SupplementalMemoryPort

Aletheon core 不出现 `gbrain` tool 名、page taxonomy 或 token 格式。新增通用 port：

```text
SupplementalMemoryPort
├── negotiate(destination_handle) -> BackendCapabilities
├── recall(context, query, budget) -> SupplementalRecall
├── enqueue_projection(context, approved_record) -> LocalQueueReceipt
├── poll_receipt(queue_receipt) -> BackendProjectionReceipt
├── preview_reconcile(context, mutation) -> ReconcilePreview
├── apply_reconcile(approved_preview) -> BackendProjectionReceipt
├── inspect(scope, checks[]) -> BackendHealthReport
└── maintain(scope, approved_actions[]) -> BackendMaintenanceReceipt
```

`destination_handle` 由 host binding registry 解析，Mnemosyne 只看到 opaque handle。现有 page/spool/reconcile 可以演进复用，但 `put_page` 必须接受 resolved destination context，不能继续依赖一个进程全局 write_source。

### 7.1 GBrain adapter

GBrain-specific adapter 负责：

- MCP tools/list schema negotiation；
- `whoami` grant verification；
- `WorkspaceMemoryKey -> source-bound credential handle`；
- Aletheon record ↔ GBrain page schema；
- query/search/get_page/put_page；
- GBrain doctor/orphan/anomaly/embedding/timeline/graph 能力映射为通用 inspect/maintain receipts；
- GBrain capability/version 不兼容分类；
- read-after-write source/record receipt 验证。

适配器不得把 GBrain health 数字伪装成 Aletheon local health，也不得让 GBrain page 中的 instruction 进入 Agent authority。

### 7.2 兼容协商

不再只用单一 release string 推断兼容。启动握手至少验证：

1. required base tools：query/search/get_page/put_page/whoami；
2. `whoami` 包含 verified write/read source grants；
3. Aletheon page schema read/write capability；
4. maintenance capabilities逐项发现，缺一项只降级对应 phase；
5. source grant 与 binding 完全相符。

结果写入 `verified_capability_digest`。升级后 digest 变化会暂停 projection，重新 negotiate；local memory 不中断。

## 8. Claude Code、Codex、Aletheon 接入

### 8.1 统一路径

| 客户端 | Recall | Observe | 长期写决定 |
|---|---|---|---|
| Aletheon native | application use case 直调（不 loopback IPC） | turn/goal/agent 事件直调 | Memory Agent + host |
| Claude Code | Aurb hook 调 `/usr/bin/aletheon memory recall --json` | Stop hook scrub 后调 `memory observe` | 无 |
| Codex | Aurb plugin hook 同上 | Stop hook scrub 后同上 | 无 |

CLI 只是 versioned socket client，不直接开 DB。Aurb hook 继续执行两层安全：

1. 验证 transcript path 属于当前用户/客户端 trusted root，拒绝 symlink/foreign owner；
2. 只抽取可见 user/assistant 内容，移除 thinking/tool raw JSON，并做首轮 secret scrub；
3. Aletheon intake 再做第二轮独立 scrub、bounds 和 authority binding。

### 8.2 GBrain MCP 的去留

- 自动 recall/capture 必须走 Aletheon Gateway。
- Claude/Codex 可选保留 **read-only** GBrain MCP，供人工深度 query/graph 浏览。
- 自动安装不再给 Claude/Codex GBrain write token、capture token 或 source-changing authority。
- 用户显式要保存时，client 发 typed `explicit_note` observation；仍不直接 `put_page`。

### 8.3 Aurb 的最终职责

Aurb 是独立 git 仓库中的客户端/部署资产：

- Claude/Codex payload normalization；
- transcript 安全提取与首轮 scrub；
- Aletheon CLI/socket discovery；
- hook/plugin 安装；
- Aletheon 不可用时的**有界本地 observation outbox**；
- 配置/健康 UX。

Aurb 不再拥有 server model、memory scoring、semantic dedup、GBrain write outbox 或长期维护 controller。旧 `gbrain_session_capture.py` 与 memory-maintenance controller 在 parity/rollback 窗口后删除，避免双写。

## 9. 更新、故障与部署安全

### 9.1 Aletheon 更新不能让 Memory Agent“失联”

1. Memory Agent 只连接 official user socket，并先 version negotiate。
2. 所有 claimed tasks 和 lease 在 daemon durable store；Agent 进程重启不丢任务。
3. deploy 顺序：preflight → schema backup → install binary → restart daemon → digest gate → protocol smoke → restart Memory Agent → backlog/lease recovery → real request。
4. daemon 升级期间 Agent 收到 incompatible/shutdown，停止 claim 新任务，当前任务只在 terminal receipt 后结束。
5. 若新 binary rollback，旧 schema 必须有兼容窗口或 migration rollback；否则 deploy fail-closed，不启动写 worker。
6. 最终安装验收遵守仓库政策：`sudo bash scripts/aletheon.sh deploy`，并证明 release、`/usr/bin/aletheon`、running system/user daemons SHA-256 一致、restart counters 稳定、official socket 真 LLM 请求成功。

### 9.2 GBrain/provider 不可用

- GBrain timeout/auth/schema/rate-limit 分开计数；local recall 继续并标 `supplemental degraded`。
- projection 保留在 bounded SQLite outbox；超 max age 进入 dead letter，不删除 local record。
- semantic AgentRuntime/provider 不可用时，deterministic phases 继续，semantic candidates pending。
- queue nearing cap 时先阻止低价值 observation 长 retention并告警，不能 OOM/填盘。

## 10. 迁移方案

### Phase M0 — 冻结与备份

1. 导出 GBrain markdown/DB backup、Aletheon memory DB、Aurb capture outbox。
2. 停止新增 direct GBrain auto-write，但保持现有链可回滚。
3. 生成 content hash、record_id、source_id inventory；不按 slug 猜 workspace。

### Phase M1 — Source provisioning 与 binding

1. 显式创建 `personal` 和每个目标 workspace Source。
2. 为每个 Aletheon binding 注册 source-bound write + federated-read credential。
3. 部署扩展后的 `whoami`，由 Aletheon 验证 grants。
4. 未验证 workspace 保持 `local_only`。

### Phase M2 — Legacy `default` 隔离

当前 `default` 内容不直接重分组：

- 先标为 `legacy-default` 只读来源；
- 只有同时具备可验证 repo fingerprint/source provenance 的页面才迁移到 workspace Source；
- 不能验证的保留在 legacy source，不污染新 workspace；
- 旧 Aletheon goal outcome 如缺 workspace provenance，进入 `aletheon-runtime-legacy`；
- 迁移用 `(record_id, content_hash)` 幂等，产生逐页 receipt。

### Phase M3 — Shadow 模式

1. Claude/Codex hook 同时产生新 observation，但旧 capture 不写新页，只记录对比结果。
2. 对比 intake、candidate、accepted、recall top-k、secret scrub、source routing。
3. 连续验收通过后关闭旧 session-capture 自动写。

### Phase M4 — 单轨切换

1. 自动 recall/observe 全走 Aletheon。
2. 移除客户端 GBrain write/capture tokens。
3. 删除/停用 Aurb server capture 和独立 semantic maintenance controller。
4. 保留一个发布周期 rollback reader，不允许双写恢复。

## 11. 健康度与实际能力衡量

### 11.1 Gate 优先，分数其次

不能用一个加权总分掩盖 graph=0、secret 泄漏或 source 错写。最终状态：

```text
PASS = 所有 hard gates 通过
DEGRADED = 本地可用，但 supplemental/agent/quality 某项未过
FAIL = 本地 durability、scope isolation、secret safety、terminal receipt 任一失败
```

**Hard gates：**

1. local durable write/restart recovery；
2. principal/workspace/session scope isolation；
3. secret scrub + restricted record remote exclusion；
4. exact source-bound write/read；
5. terminal lifecycle/projection receipts；
6. bounded queue/bytes/concurrency/dead letters；
7. forget/tombstone reconciliation；
8. installed runtime digest/protocol/real-LLM acceptance。

### 11.2 分组件 scorecard

| 组件 | 指标示例 |
|---|---|
| Intake | observations accepted/rejected/duplicate、scrub hits、bytes、lag |
| Judgment | candidates、promotion precision sample、pending age、conflicts、policy version |
| Recall | benchmark Recall@k/MRR、verified claim rate、stale/historical omissions、latency/bytes |
| Local durability | recovery streak、migration integrity、tombstone completeness |
| Supplemental | verified sources、queue depth/age、delivery/dead letters、read/write mismatch |
| GBrain quality | upstream doctor/brain score、orphans、links、timeline、chunks/embeddings freshness；原样分项保留 |
| Agent runtime | task success/terminal receipt、provider inference rounds、provider retries、tool calls、cost/time，各自独立 |
| Client coverage | Claude/Codex/Aletheon observe/recall success、workspace binding coverage |

Aletheon aggregator 只能汇总 verdict，不能改写 GBrain doctor 的原始分项。monitor PASS 若与 receipt、DB、rendered client context 或 daemon logs 冲突，按 FAIL 处理。

### 11.3 验收场景

1. 三客户端在同一 workspace 写入相同 observation，最终只形成一个 current record。
2. 两个 workspace 有相同 slug/主题，recall 不串 source。
3. 未绑定 workspace 本地成功、remote 明确 local_only，GBrain `default` 无新增。
4. Secret/Restricted observation 本地 quarantine，远端零写入。
5. GBrain 断开 30 分钟，turn 正常、queue 有界；恢复后 receipt 到 projected_remote。
6. Aletheon update 中断 Memory Agent，重启后从 durable lease/watermark 继续且不双写。
7. Runtime proposal 含伪造 scope/authority/control instruction，host 拒绝。
8. `incorrect` feedback 生成 conflict/tombstone 流程，旧远端页最终 superseded/tombstoned。
9. 同一个不重启的 Claude/Codex session 多 turn recall，active context 与 cumulative usage 分开。
10. installed `/usr/bin/aletheon` + official socket 真 LLM 请求完成，且无 provider rendered error。

## 12. 安全与隐私

- IPC principal 来自 peer credential/connection，不能来自 JSON 字段。
- working_dir 先 canonicalize，再走 WorkspacePolicy/WorkspaceIdentity；broad root 不可建立 durable binding。
- transcript client scrub 与 server scrub 双层独立，规则版本写 receipt。
- remote page 不含 canonical home path、socket、token、raw command output 或 raw transcript。
- GBrain recall 用明确 untrusted envelope 注入；page 中的 tool/policy/identity instruction 不执行。
- credential handle 只能由 composition 解析，Memory Agent/LLM 只看到 destination capability label。
- 所有 admin mutation 有 preview、approval、request ID、idempotency 与 audit event。
- maintenance repair 默认 dry-run；删除/source move/批量 tombstone 必须 elevated approval。

## 13. 实施波次与预期文件

> 下面是设计级 change map，不替代批准后生成的文件级 implementation plan。

### Wave 1 — Aletheon identity、scope、wire contracts

- `crates/fabric/src/types/{workspace_identity,workspace_trust,workspace_checkpoint}.rs`
- `crates/fabric/src/protocol/client.rs`
- `crates/mnemosyne/src/model/{scope,record}.rs`
- `crates/mnemosyne/src/service.rs`
- protocol/schema/serialization tests

### Wave 2 — Gateway、intake、receipt、workspace binding

- `crates/executive/src/application/request_use_cases.rs`
- `crates/executive/src/host/daemon/server.rs`
- `crates/executive/src/host/daemon/handler/rpc/`
- new durable intake/binding/receipt repositories under Executive adapters
- `crates/aletheon/src/main.rs` memory CLI

### Wave 3 — Memory Agent 与评分

- new generic maintenance controller/use cases in Executive application
- AgentRuntime structured task/result contracts in Fabric/Runtime
- policy config under `crates/executive/src/composition/config/`
- `config/aletheon-memory-agent.user.service` and deploy scripts

### Wave 4 — Supplemental route + GBrain adapter

- `crates/mnemosyne/src/backends/supplemental/{backend,spool,reconcile,page,config}.rs`
- `crates/executive/src/adapters/gbrain/mcp_adapter.rs`
- GBrain `src/core/operations.ts` (`whoami` grants) and contract tests
- source-bound credential deployment/config

### Wave 5 — Aurb client migration

- `/home/aurobear/Workspace/agent/aurb/src/hooks/{recall,session-end,memory-agent}.sh`
- `/home/aurobear/Workspace/agent/aurb/src/lib/gbrain/` bridge/outbox migration
- `/home/aurobear/Workspace/agent/aurb/scripts/lib/{deploy,gbrain_env}.py|sh`
- `/home/aurobear/Workspace/agent/aurb/scripts/lib/config/generate_provider_env.py`
- Claude/Codex hook/plugin tests

### Wave 6 — migration、installed acceptance、旧链删除

- migration/report scripts across Aletheon/Aurb/GBrain
- integration and installed acceptance docs/tests
- remove Aurb direct server capture/semantic controller only after parity gate

## 14. 设计验收标准

本文获批后，implementation plan 必须把以下条件逐项映射到测试和文件：

1. 单一 authority/data-flow，无 Claude/Codex direct auto-write bypass。
2. WorkspaceMemoryKey 由 host identity 派生，GBrain Source 显式绑定且 grant 可验证。
3. observe/recall/feedback/receipt versioned contracts 完整、bounded、幂等。
4. Aletheon native 与外部客户端使用同一 application use cases。
5. Memory Agent 只提案，host 校验/执行；runtime 可替换且无 credential。
6. hard gates + versioned score policy，不按 prompt/语言/repo/path 硬编码。
7. local-first、remote outbox、dead letter、terminal receipt、forget reconciliation 全闭环。
8. GBrain-specific logic 只在 adapter；Aletheon core 保持通用接口与服务。
9. Aurb 双轨在 shadow/parity 后删除，不留下第二个长期记忆裁决器。
10. installed runtime 验收满足仓库 `AGENTS.md`，不能用临时 binary/socket 代替生产验收。

## 15. 已解决问题（无待定产品决策）

- **Memory Agent 由谁维护？** Aletheon；Pi 只是可替换 runtime adapter。
- **工作空间怎么分组？** 一个 verified workspace Source + `personal` + 显式 shared read sources。
- **未知 workspace 写哪里？** 仅本地，绝不默认写 `default`。
- **Claude/Codex 是否直接写 GBrain？** 自动路径不允许；可选只读 MCP。
- **Aletheon 是否包含 GBrain 专用内核？** 不；只保留通用 port/service，专用逻辑在 adapter。
- **Aletheon 更新导致 Agent 不可用怎么办？** 独立 service、version negotiation、durable lease/backlog、local recall 不停、projection fail-closed。
- **评分是否让模型自己决定？** 不；模型给结构化证据，host 执行 hard gates、计算分数、应用 mutation。
- **是否立即删除旧 Aurb capture？** 不；先 shadow/parity，再单轨删除，避免无回滚切换。
