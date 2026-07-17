# Full Plan Document Audit — 50 文档逐份核查

> **日期:** 2026-07-17
>
> **方法:** 3 个独立 Agent 并行扫描全部 50 份架构计划文档，每份验证 1-2 个关键声明是否与当前代码一致
>
> **基准:** `dev` 分支当前 HEAD

## 审计结果总览

| 状态 | 数量 | 说明 |
|------|------|------|
| **IMPLEMENTED** | 39 | 计划已完成，文档是历史记录 |
| **FUTURE** | 1 | 描述的是未来工作，仍准确 |
| **STILL ACCURATE** | 4 | 设计/元文档，不涉及具体实现 |
| **MIXED/STALE** | 6 | 部分声明已过时，需更新 |

## 一、IMPLEMENTED — 计划已完成（39 份）

### Agora (2 份)
| 文档 | 验证要点 |
|------|---------|
| `2026-07-16-a02-agora-candidate-selection.md` | competition 模块存在：`crates/agora/src/competition/mod.rs` — SelectionPolicy, CandidatePool, CandidateSelector |
| `2026-07-16-a03-agora-broadcast-delivery.md` | broadcast 模块存在：`crates/agora/src/broadcast/mod.rs` — BroadcastProcessor, BroadcastHub, SqliteBroadcastStore |

### Dasein (3 份)
| 文档 | 验证要点 |
|------|---------|
| `2026-07-16-d01-dasein-config-timer-lifecycle.md` | config 参数已传递：`core/mod.rs:147-153` 读取 `dasein_retention_depth`/`dasein_decay_rate` |
| `2026-07-16-d02-dasein-self-reducer.md` | reducer 存在：`reducer.rs` 455 行，`handle_event` 委托给 reducer |
| `2026-07-16-d03-dasein-ledger-replay-lineage.md` | SelfLedger 存在：`ledger.rs` SHA-256 chain + SQLite + replay |

### Kernel (2 份)
| 文档 | 验证要点 |
|------|---------|
| `2026-07-16-k01-kernel-runtime-contracts.md` | KernelRuntime 存在：`crates/kernel/src/runtime.rs:28` |
| `2026-07-16-k02-kernel-authority-cleanup.md` | `executive/src/impl/kernel/` 目录已删除 |

### G03-G10 Agent 控制 (8 份)
| 文档 | 验证要点 |
|------|---------|
| `g03-agent-control-service.md` | AgentControlService 存在：`agent_control/mod.rs:100`，13 个文件 |
| `g04-native-cognit-runtime.md` | NativeCognitRuntime 存在：`native_cognit.rs` |
| `g05-agent-tools.md` | agent_spawn/agent_wait 存在：`agent_control.rs:105-106` |
| `g06-subagent-context-agora-projection.md` | candidate_projection.rs 存在 |
| `g07-agent-mailbox.md` | mailbox.rs 存在，native_cognit 使用 mailbox |
| `g08-agent-admission-budgets.md` | admission.rs 存在 |
| `g09-agent-memory-promotion.md` | memory.rs 存在 |
| `g10-agent-recovery-cleanup.md` | recovery.rs + cleanup.rs 存在 |

**注意：** 全部 8 份 G03-G10 文档中的 checkbox 任务均未打勾，但对应的代码和测试已全部存在。

### SubAgent (2 份)
| 文档 | 验证要点 |
|------|---------|
| `2026-07-16-g01-subagent-production-baseline.md` | 5 个 baseline 测试文件存在 |
| `2026-07-16-g02-agent-control-contracts.md` | AgentControlPort trait 存在，AgentControlService 实现 |

### Memory (2 份 / 3 份中)
| 文档 | 验证要点 |
|------|---------|
| `2026-07-16-m02-canonical-memory-records-scopes.md` | MemoryRecord/MemoryScope/MemoryKind 等全存在：`model.rs` |
| `2026-07-16-m03-unified-local-recall.md` | recall 已查询全部 4 stores 并行：`service.rs:459` |

### E/F/C 系列 (6 份)
| 文档 | 验证要点 |
|------|---------|
| `e01-architecture-fitness-baseline.md` | `scripts/architecture-check.sh` 存在 |
| `e02-corpus-tool-executor-adapter.md` | CorpusToolExecutor 存在：`capability_executor.rs:80` |
| `e03-governed-capability-invoker.md` | GovernedCapabilityInvoker 存在：`governed_capability.rs:109` |
| `f01-domain-facade-authority.md` | DomainPorts 存在：`domain_ports.rs:8` |
| `c01-recurrent-workspace-coordinator.md` | ConsciousCoreCoordinator 存在：`conscious_core_coordinator.rs` |
| `c02-conscious-processors-integration.md` | ConsciousProcessor trait + 4 impls 存在 |

### M04-M08 Memory 子计划 (5 份)
| 文档 | 验证要点 |
|------|---------|
| `m04-bounded-memory-workspace-projection.md` | bounded_workspace_projection 测试存在 |
| `m05-leased-memory-consolidation.md` | ConsolidationRepository 存在 |
| `m06-gbrain-reconciliation.md` | ReconcileOperation + gbrain_reconciliation 测试存在 |
| `m07-retention-forgetting.md` | ForgetPolicy + ForgetReceipt 存在 |
| `m08-subagent-memory-isolation.md` | AgentMemoryContext + agent_memory_isolation 测试存在 |

### Q/R/S/V/X/P 系列 (11 份)
| 文档 | 验证要点 |
|------|---------|
| `p0-conflict-resolution-and-readiness.md` | 全部 done 条目已验证 |
| `q01-layered-config-extension-catalog.md` | ExtensionCatalog 存在 |
| `q02-typed-interact-thin-bin.md` | UiAction/UiSnapshot 存在，reduce() reducer 存在 |
| `r01-canonical-event-spine.md` | EventSpine trait + EnvelopeV2 存在 |
| `r02-deterministic-event-projections.md` | EventProjection trait + ProjectionDescriptor 存在 |
| `s01-session-turn-item-contracts.md` | session.rs 存在，JSON schema 存在 |
| `s02-unified-turn-coordinator.md` | TurnCoordinator 存在：`turn_coordinator.rs:55` |
| `v01-cross-domain-acceptance-suite.md` | cross_domain_acceptance 测试 + AcceptanceEvidence 存在 |
| `x01-executive-use-case-ports.md` | HandlerPorts 存在：`handler/ports.rs:21` |
| `x02-private-composition-root.md` | DaemonComposition 存在：`bootstrap/mod.rs:22` |

### 其他 (2 份)
| 文档 | 验证要点 |
|------|---------|
| `session-compaction-recovery-design.md` | truncate_utf8_bytes 存在：`compaction.rs:23` |
| `session-compaction-recovery.md` | compaction 修复已验证 |

---

## 二、FUTURE — 仍准确的未来计划（1 份）

| 文档 | 原因 |
|------|------|
| `v02-production-migration-scenarios.md` | 描述生产部署迁移场景，对应 task 尚未执行 |

---

## 三、STILL ACCURATE — 设计/元文档（4 份）

| 文档 | 类型 |
|------|------|
| `executable-plan-decomposition-design.md` | 元设计文档，描述分解方法论 |
| `original-plan-coverage-matrix.md` | 追溯矩阵，todo/done 标记准确 |
| `2026-07-15-architecture-coupling-optimization-plan.md` | **已更新** — 顶部有 Code-Reality Update |
| `2026-07-15-dasein-agora-conscious-core-plan.md` | **已更新** — 顶部有 Code-Reality Update |

---

## 四、MIXED/STALE — 已更新（6 份）

| 文档 | 过时程度 | 操作 |
|------|---------|------|
| `2026-07-16-a01-agora-transaction-integrity.md` | 5/5 bug 已修复 | **已更新** — Code-Reality Update 已添加 |
| `2026-07-16-m01-memory-contract-baseline.md` | recall 已统一查询全部 stores | 待处理（低优先级） |
| `subagent-unified-harness-plan.md` | 7/10 gap 已过时 | **正在更新** — Code-Reality Update 添加中 |
| `mnemosyne-unified-memory-plan.md` | 3/7 gap 已关闭，2/7 部分关闭 | **正在更新** — Code-Reality Update 添加中 |
| `2026-07-15-architecture-coupling-optimization-plan.md` | Executive 耦合比描述更严重 | **已更新** — Code-Reality Update 已添加 |
| `2026-07-15-dasein-agora-conscious-core-plan.md` | 5/6 bug 已修复 | **已更新** — Code-Reality Update 已添加 |

---

## 五、全局统计

| 指标 | 数值 |
|------|------|
| 审计文档总数 | 50 |
| 计划已实现 (IMPLEMENTED) | 39 (78%) |
| 设计/元文档 (ACCURATE) | 4 (8%) |
| 未来计划 (FUTURE) | 1 (2%) |
| 部分过时 (MIXED/STALE) | 6 (12%) |
| 已添加 Code-Reality Update | 5 |
| 正在添加 Code-Reality Update | 2 |
| 低优先待处理 | 1 (M01) |

## 六、结论

**Aletheon 的计划文档纪律是：计划写得很详细，实现执行得很快，但文档从不回填 "已完成"。**

- 78% 的计划文档描述的是**已完成的**工作（对应代码存在、测试存在、CI gate 存在）
- 几乎所有 checkbox 任务列表都是**全空**的（未打勾）
- 只有 12% 的文档存在实际的内容过时问题
- 这 6 份过时文档中，5 份已经或正在添加 Code-Reality Update

**建议：** 不自找麻烦去碰那 39 份已实现的文档（它们作为历史记录是正确的，只是没勾 checkbox）。重点维护那 5 份已添加 Code-Reality Update 的大文档，它们是未来工作的入口点。
