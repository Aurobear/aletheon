# 生产就绪缺口与路线图覆盖分析（E′/A/B/C/D 的伴随文档）

**Date:** 2026-07-30
**Status:** 覆盖/缺口分析（非实现规格）——用于交代五份 workstream 之外**没有覆盖**的生产能力，以及并行落地的排序风险。
**Scope:** 本文件不新增实现细节；它命名 E′（能力委派）、A（语义记忆）、B（多智能体）、C（元认知）、D（TUI）**共同没触及**的横向生产能力，并给出跨文档协调结论。所有主张锚定 `path:line`。

> 验收状态口径（贯穿全文）：目前只有**评分内核基础部分**有历史生产验收记录；本文件列出的能力与五份 workstream 一样，均属**待实施设计**，未接入生产前不得标称 production-wired。

## 一、五份 workstream 的覆盖边界

它们是五个**纵向能力**，不是横向的运营/安全面：

| Workstream | 覆盖 |
|---|---|
| E′ | 能力委派安全（且仅 tools/workspace/budget 的 per-child 收窄） |
| A | 召回质量（真 embedding + 向量库 + RRF 融合） |
| B | 多智能体编排（Planner/Executor/Reviewer 角色图接线） |
| C | 受治理自演化（genome-only，operator 批准） |
| D | TUI 体验（diff/多窗格/scoped 审批） |

**它们整体没触及"让常驻 Agent OS 真正可运营"的横向面**（下节）。

## 二、真生产还实际缺的能力（按承重程度排序）

| # | 能力 | 现状（已证实） | 严重度 |
|---|---|---|---|
| 1 | **机器级 provider 并发/限流/冷却协调** | ❌ 已证实缺口。找到的 semaphore 只是 agora 广播 + corpus 按路径写锁（`corpus/src/tools/tools/executor.rs:52-71`），**没有全局 LLM 许可池**。架构 §7 自己也点了这条。 | 🔴 极高 |
| 2 | **L2/L3 破坏性动作审批闭环** | 🟡 原语已在（`SocketApprovalGate`/`PendingApproval`、tool `permission_level()` `corpus/src/core/mod.rs:70`、critic 强制"破坏性动作须有 rollback" `cognit/src/core/critic.rs:75-84`），闭环接线 + 验收未完成。**注：D 已吸收其中一大块**——把 `RequireApproval` 从仅 L2+（`corpus/src/security/runner.rs:328-330`）改为每级以 host policy 为权威 + 重启安全路径级 grant；C 补上 genome 修改（`DaseinModification`）的审批 resolve。**仍缺**：`DeleteFile`/`GitPush` 等其余破坏性 `ApprovalCategory` 的带-resume 闭环。 | 🔴 高（安全） |
| 3 | **密钥/PII 在记忆与审计流的脱敏** | 🟡 静态密钥有加密（`corpus/src/security/credential_vault.rs`），但审计明确"内容默认敏感、需另做投影脱敏"（`agora/src/trace/mod.rs:13`），**没有系统性 scrub**；召回内容/工具输出/事件日志未统一脱敏。 | 🟠 高 |
| 4 | **运营可观测性** | 🟡 有 provider/MCP health probe（`cognit/src/adapters/inference/scheduler.rs:407`）、tracing-json，但**没有 metrics 导出端点（prometheus/otel）、没有 daemon health/readiness 端点、无 turn 级 watchdog**。 | 🟠 高 |
| 5 | **统一存储生命周期** | ❌ 迁移是各 crate 各自 ad-hoc（`dasein/src/core/continuity.rs:184` 内联 ALTER TABLE、agora `WORKSPACE_SCHEMA_V1`、`credential_vault` `migrate_to`），无跨库统一 schema 版本/迁移框架、无增长/vacuum/保留策略执行。 | 🟠 中高 |
| 6 | **注入防御扩展到工具输出/外部通道** | 🟡 召回已包成"untrusted reference block"（好），但工具输出、外部 channel 摄入的内容未同等处理；无工具 egress 策略。 | 🟠 中高 |
| 7 | **provider/runtime 完整性** | 🟡 架构 §7 未完成：跨 provider 的流式 tool-schema 转换对齐、Pi RPC diff/artifact receipt、Runtime selector 统一、降级链验证（缺 Ollama）。 | 🟡 中 |
| 8 | **行为回归/评测 harness（真 fixture）** | 🟡 eval-kernel 基座已生产验收，但架构 §7"coding benchmark 尚未以真实 fixture/harness/receipt 落地"，缺 agent 行为回归门。 | 🟡 中 |
| 9 | **可维护性/总线因子** | ❌ 超大文件仍在（`corpus/src/security/runner.rs`~1998、`executive/.../agent_control/mod.rs`~1795、`.../settlement.rs`~1524、`cognit/.../harness/linear/mod.rs`~1582、`mnemosyne/src/service.rs`~1469）；单人项目、SECURITY 联系人。 | 🟡 中 |

> 正面修正（已证实）：结算级 crash-recovery **是**实现且测试过的——`agent_settlement_receipts` 表 + `idempotency_key` + `SettlementEvidence::IdempotentReplay` + `INSERT OR IGNORE` + `recover_settlement_resources`（`executive/.../agent_control/settlement.rs:128-243`、`mod.rs:402`）。"重启不重复已结算副作用"在 agent 结算层落地，不是缺口。（C 指出的 rollback 仅内存是另一处，见 §三。）

## 三、五份文档"内部"的覆盖漏洞

- **E′**：显式排除了**跨兄弟聚合预留**（正好是 #1）和 **per-agent MCP 注册**——MCP 工具目前全局注册，B 一放开多 agent，per-agent 工具作用域就是个洞。
- **B**：角色**只支持串行执行**（无并行角色）；取消只在 `wait` 边界（无 child 级 cancel handle，O5）；**工作流中途崩溃的恢复未定义**。
- **C**：候选沙箱（旧实现）**工作区级、忽略候选本身**；rollback **仅内存**（非重启安全）；`migrate` 是 cosmetic。（C 的修订稿已把这三点列为"必须一并解决"，不是遗留——见 C 稿 §3.3.1/§3.5。）
- **A**：大规模 backfill 未压测；rerank/查询扩展是（可接受的）非目标。
- **D**：多客户端/远程、无障碍、协议版本化未覆盖。

## 四、跨文档实现排序风险（落地前必须协调）

**B、D、C 都改 executive 的审批 / turn / daemon-bootstrap 这一片，高度重叠。并行实现会撞车。** 具体重叠：

| 共享面 | B | C | D |
|---|---|---|---|
| `executive/.../turn_pipeline.rs` | `drive_role_graph`，从 `run`(:337) 调用 | — | 在 `approval_request` 上发 scope/artifact 字段（:976-999） |
| 审批子系统 | — | `approval_service.rs` resolve 路径处理 `DaseinModification` | `admin_service.rs` transient + 新 `approval/session_grant.rs` |
| daemon bootstrap | `bootstrap/services.rs`（RoleWorkflowFactory） | `bootstrap/request.rs`（genome path） | `bootstrap/{turn_runtime,approval_gate,protocol,server}.rs` |
| daemon handler/rpc | — | — | `handler/rpc.rs`、`rpc_admin.rs` |

**结论/建议（交代）：**
1. **E′ 先行、独立落地**（Wave 0），它只改 `agent_control/{mod,live_runs,execution}.rs` + fabric 类型，与 B/C/D 的审批/turn 面基本不撞。
2. **B、C、D 触及 executive 审批/turn 的部分，必须在动手前做一次显式的文件级归属/顺序切分**——按"谁先落、谁改哪段"分配，或先做一次 `turn_pipeline.rs` 的职责拆分（它已是 ~大文件，B/D 都往里加）。不切分就并行 = 合并冲突 + 语义打架。
3. 推荐顺序：**E′ →（把 B/C/D 对审批-turn 的改动合成一个"executive 审批/turn 收敛波"，内部按上表分段）→ 其余（A 独立、可任意穿插）**。

**另一处依赖（交代）：C 的评测语料来源。** C 的 `GenomeCandidateSandbox` 需要一个"versioned evaluation corpus"（`C 稿 §3.3.1`），但该语料本身**未被指定/编写**。最可能的来源是已有的**评分内核 fixtures**；必须先明确语料的归属与作者，否则候选感知沙箱"再安全也只与它的语料一样可信"。建议写进 C 的 open decisions。

## 五、建议纳入路线图的新增 workstream

现有五块之外，下面两项是**让 B 能安全落地的真正前提**，比 C 更该先做：

- **F1 — 机器级 provider 并发/冷却（= #1）**：B 的硬前提。多智能体扇出而没有共享 LLM 许可池 + 冷却，主 agent 与子 agent 会一起撞 429、互相拖垮；E′ 又明确把"跨兄弟聚合预留"排除在外。**建议提到 Wave 0/1，与 E′ 并列**。
- **F2 — 破坏性动作审批闭环补全（= #2 剩余部分）**：D + C 已覆盖工具审批 + genome 审批；补上 `DeleteFile`/`GitPush`/`SendMail` 等其余 `ApprovalCategory` 的带-resume 闭环 + 验收。安全底线。

其余 #3–#9（脱敏 / 可观测性 / 存储生命周期 / 注入防御 / provider-runtime 完整性 / 评测门 / 可维护性）属横向加固，可作为独立 wave 依次排。

## 六、排序汇总（含新增项）

```
Wave 0:  E′  ∥  F1(provider 并发)          ← F1 是 B 的硬前提，与 E′ 并列
Wave 1:  [executive 审批/turn 收敛波: B + C(plumbing) + D，按 §四 上表分段]  ∥  A
Wave 2:  A 完成 → C 的 apply 解锁（依赖 A/B + 候选沙箱 + durable genome store）
Wave 3+: F2(审批闭环补全) 及 #3–#9 横向加固依次排
```

（此为覆盖分析建议序；与各 workstream 稿内的 Wave 标注一致：E′=Wave0，B=Wave1，A=Wave2，C=Wave3。）
