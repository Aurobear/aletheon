# 开放问题 (Open Questions)

> 持续更新的开放问题追踪。每个问题标注最新进展、关联设计文档和 next step。

**最后更新:** 2026-06-07 (B1-B5 + design improvements merged)

---

## Status Legend

| 标记 | 含义 |
|------|------|
| 🔴 未开始 | 尚未研究，纯问题定义 |
| 🟡 进行中 | 已有部分分析 or 实验 |
| 🟢 已解决 | 已达成设计共识，在 roadmap 中 |
| ✅ 已实现 | 已在代码中落地 |

---

## 1. Agent 的"自我意识"边界在哪里？

`🟡 进行中`

```
1. Agent 应该有多大的自主权?
2. 什么时候应该停下来问人?
3. 如何避免 Agent "过度自信"?
```

### 当前进展

- **权限分级已实现**（L0-L3，见 `security/security-model.md`）— 定义了"自动执行 / 通知 / 确认 / 禁止"四级，对应自主权边界
- **LoopDetector 已实现**（`security/loop_detector.rs`）— 停滞检测 + 连续失败检测，自动阻断循环，对应"停下来"的触发条件
- **L2 确认流程的设计**在 `security/security-model.md` §2.1 完成，但确认流程的用户界面（TUI 确认对话框）未实现
- **上下文压缩已实现**（`memory/compaction.rs`）— LLM 摘要压缩，消息数超阈值自动触发（`engine.rs:488-498, 885-895`）；但压缩时的"信息优先级"选择（哪些该保留、哪些该丢弃）仍依赖 LLM 判断，无显式策略
- **输出护栏 OutputGuardrail** (`security/output_guardrail.rs`) 已实现 — 对过度自信的模型输出进行验证

### 待研究

| 问题 | 关联文档 | 优先级 |
|------|----------|--------|
| L2 确认流程的 TUI 实现 | `security/security-model.md` | P1 |
| 何时"主动出击" vs 等待指令 | 需新设计文档（ProactiveBehavior） | P1 |
| Agent 对自身行为的反思/修正机制 | `core/cognitive-engine.md` | P2 |

---

## 2. 隐私和安全的平衡

`🟡 进行中`

```
1. Agent 需要看到多少用户数据才能有效工作?
2. 本地处理 vs 云端处理的边界
3. 多设备同步时的数据保护
```

### 当前进展

- **沙箱隔离已实现**（bubblewrap / process / noop）— 工具执行在隔离环境中运行
- **策略引擎已实现**（L0-L3 权限分级）— 控制 Agent 对文件/系统的访问范围
- **混合推理代码已实现但未接入**（`inference/router.rs`）— 本地/云端路由逻辑存在，但 Engine 未使用此模块，直接通过 ProviderRegistry 调用 LLM
- **WritableRoot 路径隔离已实现**（B1，`security/writable-root.md`）— 精细化控制 Agent 可写的文件路径
- **多设备记忆同步**为 ⬜ Planned（`platform/multi-device.md`）

### 待研究

| 问题 | 进展 | 优先级 |
|------|------|--------|
| WritableRoot 路径隔离实现 | ✅ 已实现（B1） | P1 |
| 云端推理时数据不上传的保证措施 | 需文档 | P1 |
| 多设备同步的数据加密 | `platform/multi-device.md` §5 | P2 |
| 用户隐私数据的自动识别和脱敏 | 需新设计文档 | P2 |

---

## 3. 记忆的"遗忘"策略

`🟡 进行中`

```
1. 什么该记住，什么该忘记?
2. 如何避免记忆污染?
3. 跨设备记忆冲突解决
```

### 当前进展

- **三级记忆架构已实现**（`core/memory-system.md`）— Core/Recall/Archival，不同层级有不同的"遗忘"机制
- **上下文压缩已实现**（`memory/compaction.rs`）— 旧消息通过 LLM 摘要压缩，本质是一种"结构化遗忘"（`engine.rs:488-498` 触发逻辑）
- **核心记忆的 self-edit 工具已实现** — Agent 可以通过工具自主管理 Core Memory
- **ContextBudget** (`memory/budget.rs`) 已实现 — Token 预算追踪，超限自动截断
- **缺失：** 压缩时的信息优先级策略（当前完全依赖 LLM 判断）、记忆重要性评分、记忆污染检测

### 待研究

| 问题 | 进展 | 优先级 |
|------|------|--------|
| 什么是重要的、应该保留的记忆？ | 需设计记忆重要性评分 | P1 |
| 跨设备记忆同步的冲突解决 | `platform/multi-device.md` §2 | P2 |
| 记忆污染检测（Agent 写入错误信息） | 需新设计文档 | P2 |
| ArchivalMemory 的自动索引和清理 | `core/memory-system.md` | P2 |

---

## 4. 多 Agent 协作

`🟢 已解决 — 已在 orchestration/ 中实现核心能力`

```
1. 多个 Agent 如何分工?
2. 冲突如何解决?
3. 共享知识的粒度
```

### 当前进展

- **编排引擎已实现**（`orchestration/`）— Selector（LLM 选择）、Handoff（交接）、DiGraph（DAG 工作流）
- **Agent 注册表已实现** — TOML+Markdown 驱动，内置 3 个 Agent（fs/code/net）
- **DelegateTool 已实现** — 委托即工具，主 Agent 可委托子任务
- **子 Agent 独立迭代预算已实现**（`orchestration/budget.rs`）
- **Agent 间冲突检测**为 ⬜ Planned（`platform/agent-awareness.md` §3）
- **Per-Agent LoopDetector 隔离**为 ⬜ Planned（`security/security-model.md` §4.13）

### Corner Cases

| 场景 | 当前状态 | 补充措施 |
|------|----------|----------|
| 子 Agent 陷入循环，影响父 Agent | CircuitBreaker 全局状态（已实现） | 需 per-agent 隔离（⬜ Planned） |
| 多个 Agent 写同一文件 | PathConflictDetector 已实现（parallel gate + RwLock） | — |
| 子 Agent 要求提升权限 | 无权限继承链 | 需 `derive_child_permission()`（⬜ Planned） |

---

## 5. 内核修改的维护成本

`🟢 已解决 — 采用渐进式策略`

```
1. 每次内核升级需要 rebase 吗?
2. 能否 upstream?
3. DKMS 方案是否可行?
```

### 当前进展

- **Phase 1-4 无内核修改** — IPC 使用 Unix socket，感知使用 eBPF+ procfs，无需修改内核
- **Phase 5 内核模块 `agent_ipc.ko`** 为 ❌ Not Started — 但用户态 IPC 全功能已实现
- **DKMS 打包方案**在 `platform/kernel-ipc.md` 中设计完成
- **自动降级机制已实现** — `IpcManager` 自动探测是否需要内核模块（`kernel-ipc.md`）

### 决策

| 决策 | 结论 | 依据 |
|------|------|------|
| 是否修改内核 | 不必要，通过 eBPF 实现内核感知 | 减少维护成本，跨内核版本兼容 |
| 内核模块 vs 用户态 | 优先用户态，内核模块作为可选加速 | Phase 6 已确认实践：用户态方案够用 |
| 是否需要 upstream | 暂不需要 | eBPF programs 已足够，内核模块非核心依赖 |

---

## 6. 性能和资源

`🟡 进行中`

```
1. 本地推理的质量和速度平衡
2. 记忆系统的存储效率
3. 感知层的 CPU/内存开销
4. 内核模块的安全审计
```

### 当前进展

- **混合推理代码已实现但未接入** — `inference/router.rs` + `classifier.rs` 实现了本地/云端路由，但 Engine 直接使用 ProviderRegistry，未经过 InferenceRouter
- **ContextBudget** 已实现 — Token 预算追踪
- **Memory L2 Recall (SQLite)** 已实现 — 高效存储
- **Memory L3 Archival** 为 🔶 Partial — 向量搜索设计完成，存根实现
- **瓶颈检测**（`perception-layer.md`）已实现 — CPU/内存/IO/网络瓶颈检测，含升级建议
- **eBPF 源**已实现（mock /proc 回退）— 零开销无 eBPF 时

### 待优化

| 优化点 | 当前状态 | 预估收益 |
|--------|----------|----------|
| 向量搜索 L3 Archival 连接真正 DB | 🔶 Partial | 语义搜索可用 |
| FUSE 性能优化 | ⬜ Planned | 降低文件系统延迟 |
| IPC io_uring 真正实现 | 🔶 Partial | 消息延迟 <10μs |
| Per-action resource budget | ⬜ Planned | 防止单次操作资源失控 |

---

## 7. Android 碎片化

`🟡 进行中`

```
1. 不同厂商的权限差异
2. 后台保活策略
3. 无 Root 情况下的能力边界
```

### 当前进展

- **Android 平台适配器已实现**（`platform/android.rs`）— stub 版本，通过 `getprop`/`dumpsys` 获取系统信息
- **能力受限时的降级策略已设计** — `PlatformAdapter` trait 的 `capabilities()` 方法返回可用能力集
- **Android 上 bubblewrap 不可用** — 沙箱后端自动降级为 ProcessBackend（`sandbox/backend.rs`）

### 待研究

| 问题 | 进展 | 优先级 |
|------|------|--------|
| 不同厂商 Root 获取方式 | 需实验验证 | P1 |
| 无 Root 时能力边界文档 | 需编写 | P1 |
| 后台保活策略（前台服务 / WorkManager） | 需设计 | P2 |
| Android 权限差异映射表 | 需编写 | P2 |

---

## 8. 已解决的实现问题 (Resolved by B1-B5 + Design Improvements)

以下问题在 B1-B5 实现批次和设计改进 PR 中已落地:

| 问题 | 状态 | 实现位置 | PR |
|------|------|----------|-----|
| MemoryScope 隔离 (Global/Session/Agent) | ✅ 已实现 | `memory/scope.rs` | #105 |
| FUSE 真实挂载 (fuse3 integration) | ✅ 已实现 | `fuse/` | #104 |
| 工具搜索 (BM25 + TF-IDF) | ✅ 已实现 | `tool/search.rs` | #100 |
| 并行执行 (RwLock gate + PathConflictDetector) | ✅ 已实现 | `tool/parallel.rs` | #100 |
| MCP Transports (StreamableHTTP + SSE) | ✅ 已实现 | `mcp/transport.rs` | #103 |
| Split Sandbox (bwrap + fallback chain) | ✅ 已实现 | `sandbox/split.rs` | #105 |
| Container Sandbox | ✅ 已实现 | `sandbox/container.rs` | #104 |
| Integrity Monitor | ✅ 已实现 | `security/integrity.rs` | #104 |
| Automation System | ✅ 已实现 | `automation/` | #105 |
| Memory Pipeline | ✅ 已实现 | `memory/pipeline.rs` | #105 |

---

## Implementation Summary

| # | 问题 | 状态 | 关键进展 |
|---|------|------|----------|
| 1 | Agent 自主权边界 | 🟡 进行中 | L0-L3 + LoopDetector 已实现，上下文压缩已实现，确认 UI 待完成 |
| 2 | 隐私与安全平衡 | 🟡 进行中 | 沙箱+策略引擎+WritableRoot 已实现 |
| 3 | 记忆遗忘策略 | 🟡 进行中 | 三级记忆+上下文压缩+ContextBudget+MemoryScope 已实现，信息优先级策略待设计 |
| 4 | 多 Agent 协作 | 🟢 已解决 | 编排引擎+DelegateTool+PathConflictDetector 已实现 |
| 5 | 内核维护成本 | 🟢 已解决 | 渐进式策略，基本不依赖内核修改 |
| 6 | 性能与资源 | 🟡 进行中 | 混合推理代码存在但未接入引擎，瓶颈检测已实现，FUSE real mount 已完成，L3 Archival 待完善 |
| 7 | Android 碎片化 | 🟡 进行中 | Android adapter 已实现（getprop/dumpsys），无实际测试环境 |
