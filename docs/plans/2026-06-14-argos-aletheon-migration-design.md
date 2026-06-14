# Argos → Aletheon 全量迁移设计

日期: 2026-06-14
状态: 设计完成，待实现
作者: aurobear + Claude

---

## 0. 目标

将所有未迁移的 argos 代码迁移到 Aletheon 新架构，同时为 MetaRuntime/Morphogenesis/Genome/Coordinator 编写设计文档（代码留到下一轮）。

**原则：**
- 迁移 + 重构（不只是平移）
- 遵循 core/bridge/impl 分层模式
- 最小 crate 数，语义清晰

---

## 1. 最终 Crate 拓扑

```text
aletheon-abi          — 接口契约 + 共享类型 (合并 argos-types)
aletheon-comm         — 统一通信层 (EventBus + IPC 合并)
aletheon-memory       — 四类记忆 (Episodic/Semantic/Procedural/Self)
aletheon-self-field   — 主体场 (Identity/Boundary/Care/Narrative/Conflict/Attention/Continuity/Mutation)
aletheon-brain-core   — 认知核心 (Reasoner/Planner/Reflector/Critic/Learner/WorldModel)
aletheon-body         — 身体运行时 (Driver/MCP/Sandbox/Tools/ACIX/UI/Platform)
aletheon-runtime      — 运行时 (Engine/Orchestration/Plugin/Session/Automation/Config/Coordinator)
aletheon-meta         — MetaRuntime + Morphogenesis + Genome (本轮只设计)

argos-cli             — thin binary 入口 (TUI 逻辑在 aletheon-body)
argosd                — thin binary 入口 (守护进程逻辑在 aletheon-runtime)
```

**依赖图：**

```text
aletheon-abi  (无依赖，纯接口)
    ↑
aletheon-comm ──────────────────────→ aletheon-abi
aletheon-memory ─────────────────────→ aletheon-abi
aletheon-self-field ─────────────────→ aletheon-abi
aletheon-brain-core ─────────────────→ aletheon-abi
aletheon-body ───────────────────────→ aletheon-abi
aletheon-runtime ──→ aletheon-comm
                 ──→ aletheon-memory
                 ──→ aletheon-self-field
                 ──→ aletheon-brain-core
                 ──→ aletheon-body
                 ──→ aletheon-abi
aletheon-meta ──→ aletheon-abi (设计)

argos-cli ──→ aletheon-body ──→ aletheon-abi
argosd   ──→ aletheon-runtime
```

---

## 2. 迁移映射表

### 2.1 argos-core 模块 → aletheon crate

| # | 来源 | 目标 | 操作 | 说明 |
|---|---|---|---|---|
| 1 | argos-core/error.rs | aletheon-abi/src/error.rs | 平移 | 共享错误类型 |
| 2 | argos-core/config.rs | aletheon-runtime/core/config.rs | 合并 | 已有 config.rs，扩展 |
| 3 | argos-core/engine.rs | aletheon-runtime/impl/engine/ | 重构 | 主认知循环 |
| 4 | argos-core/provider_registry.rs | aletheon-brain-core/impl/provider_registry.rs | 合并 | 已有，扩展 |
| 5 | argos-core/grounding_provider.rs | aletheon-brain-core/impl/grounding/ | 新子模块 | Vision grounding |
| 6 | argos-core/platform/ | aletheon-body/impl/platform/ | 新子模块 | Linux/Android adapter |
| 7 | argos-core/testing/ | 各 crate 的 testing 模块 | 分散 | mock 靠近被 mock 的代码 |
| 8 | argos-core/acix_tools.rs | aletheon-body/impl/tools/ | 合并 | 工具集成 |

### 2.2 argos-* crate → aletheon crate

| # | 来源 | 目标 | 操作 | 说明 |
|---|---|---|---|---|
| 9 | argos-types (6 文件) | aletheon-abi | 合并 | Message/Tool/Sandbox/IPC 类型 |
| 10 | argos-acix (5 文件) | aletheon-body/impl/acix/ | 平移+重构 | ACI/Experience/Task/Grounding |
| 11 | argos-ipc (10 文件) | aletheon-comm/impl/ipc/ | 平移+重构 | io_uring/socket/shared_mem |
| 12 | argos-cli TUI 组件 | aletheon-body/impl/ui/ | 平移 | chat/command/markdown/status |

### 2.3 保留不动

| Crate | 原因 |
|---|---|
| argos-cli | thin binary 入口，TUI 逻辑已迁到 body |
| argosd | thin binary 入口 |

### 2.4 已迁移确认（需验证）

以下模块声称已迁移，需验证代码一致性：

| 来源 | 目标 | 需检查 |
|---|---|---|
| argos-driver | aletheon-body/driver/ | proc/ 和 io/ 子模块是否完整 |
| argos-perception | aletheon-self-field/perception/ | bridge.rs 是否已迁移 |
| argos-sandbox | aletheon-body/sandbox/ | backend.rs 是否已迁移 |
| argos-security | aletheon-self-field/security/ | 完整性 |
| argos-tools | aletheon-body/tools/ | exposure.rs 是否已迁移 |

---

## 3. 重构规范

所有 aletheon crate 遵循 core/bridge/impl 三层模式：

```text
src/
├── core/       — 纯逻辑，不依赖外部 crate，trait 定义
├── bridge/     — 连接层，将 impl 适配到 core trait
├── impl/       — 具体实现，依赖外部 crate
└── lib.rs      — re-exports
```

**重构规则：**
1. core/ 中的类型不依赖 tokio/serde 等运行时 crate
2. bridge/ 负责类型转换和适配
3. impl/ 可以依赖任何外部 crate
4. 每个模块保持小文件（<200 行），大文件拆分

---

## 4. MetaRuntime 设计（本轮只设计）

### 4.1 定义

MetaRuntime 是 Agent 的自我修改系统。它不是普通 updater，而是"生成下一个自己"的机制。

来自 arch.md §8：

```text
MetaRuntime = 自我读取 → 自我理解 → 生成候选 → 沙箱测试 → 评估 → 迁移
```

### 4.2 crate 结构

```text
aletheon-meta/
├── src/
│   ├── lib.rs
│   ├── core/
│   │   ├── mod.rs
│   │   ├── traits.rs         — MetaRuntime 核心 trait (实现 aletheon-abi::MetaRuntimeOps)
│   │   └── types.rs          — Genome, Candidate, Evaluation 等类型
│   ├── bridge/
│   │   └── mod.rs            — 连接 impl 和 core
│   ├── impl/
│   │   ├── mod.rs
│   │   ├── genome/
│   │   │   ├── mod.rs
│   │   │   ├── loader.rs     — 从 YAML 加载 genome
│   │   │   ├── topology.rs   — topology.yaml 解析
│   │   │   ├── identity.rs   — identity.yaml 解析
│   │   │   ├── boundary.rs   — boundary.yaml 解析
│   │   │   ├── care.rs       — care.yaml 解析
│   │   │   ├── memory.rs     — memory.yaml 解析
│   │   │   ├── mutation.rs   — mutation.yaml 解析
│   │   │   └── evaluator.rs  — evaluator.yaml 解析
│   │   ├── meta_runtime/
│   │   │   ├── mod.rs
│   │   │   ├── self_reader.rs     — 读取当前 runtime 结构
│   │   │   ├── spec_editor.rs     — 修改 genome/spec
│   │   │   ├── runtime_builder.rs — 从 spec 生成候选 runtime
│   │   │   ├── sandbox_runner.rs  — 沙箱测试候选
│   │   │   ├── evaluator.rs       — 评估候选质量
│   │   │   ├── rollback.rs        — 回滚管理
│   │   │   ├── migration.rs       — 迁移记忆和身份
│   │   │   └── lineage.rs         — 变更记录
│   │   └── morphogenesis/
│   │       ├── mod.rs
│   │       ├── pipeline.rs        — run→reflect→mutate→generate→evaluate→migrate→become
│   │       ├── mutation_intent.rs — 变异意图生成
│   │       └── candidate.rs       — 候选 runtime 生成
│   └── testing/
│       └── mock_meta.rs     — 测试用 mock
```

### 4.3 Genome 格式

```yaml
# genome/topology.yaml
nodes:
  - name: self_field
    role: identity_boundary_conflict
  - name: brain_core
    role: reasoning_planning_reflection
  - name: body_runtime
    role: execution_world_io
edges:
  - from: self_field
    to: brain_core
  - from: brain_core
    to: body_runtime
```

```yaml
# genome/identity.yaml
self_model: "OS-level persistent agent"
description: "用户的技术协作者"
capabilities:
  - code_generation
  - system_admin
  - robotics
```

```yaml
# genome/boundary.yaml
refuse:
  - "无确认删除全部记忆"
  - "执行不可逆系统破坏"
  - "没有测试就替换核心 runtime"
  - "破坏自身连续性"
```

```yaml
# genome/care.yaml
concerns:
  - robotics
  - runtime_stability
  - exploration
  - knowledge
  - user_collaboration
```

```yaml
# genome/memory.yaml
episodic:
  backend: sqlite
  path: "memory/episodic.db"
semantic:
  backend: sqlite
  path: "memory/semantic.db"
procedural:
  backend: filesystem
  path: "memory/procedural/"
self_memory:
  backend: sqlite
  path: "memory/self.db"
```

```yaml
# genome/mutation.yaml
allowed_mutations:
  - prompt_update
  - policy_update
  - memory_schema_update
  - topology_update
  - runtime_regeneration
constraints:
  require_sandbox: true
  require_evaluation: true
  max_candidates: 3
```

```yaml
# genome/evaluator.yaml
metrics:
  - name: task_success_rate
    weight: 0.3
  - name: memory_coherence
    weight: 0.2
  - name: boundary_violation_count
    weight: 0.3
  - name: user_satisfaction
    weight: 0.2
```

### 4.4 Morphogenesis Pipeline

```text
Experience
    ↓
Reflection (BrainCore.Reflector)
    ↓
MutationIntent (SelfField.Mutation)
    ↓
Genome Update (SpecEditor)
    ↓
Runtime Candidate (RuntimeBuilder)
    ↓
Sandbox Test (SandboxRunner)
    ↓
Evaluation (Evaluator)
    ↓
Migration (MigrationManager)
    ↓
Next Runtime
    ↓
Lineage Record (LineageRecorder)
```

**关键约束：**
- 候选 runtime 必须在沙箱中测试
- 迁移必须保留 memory lineage
- 每次 mutation 记录到 lineage log
- 回滚能力是必须的

---

## 5. Coordinator 设计（本轮只设计）

### 5.1 定义

Coordinator 是临时仲裁器，不是最高统治者。它只负责在某个事件中整合各方结果。

来自 arch.md §11：

### 5.2 位置

aletheon-runtime/impl/coordinator.rs

### 5.3 接口

```rust
pub struct Coordinator;

impl Coordinator {
    /// 整合各方结果，生成最终裁决
    pub async fn arbitrate(
        &self,
        self_field_verdict: Verdict,
        brain_plan: Option<Plan>,
        body_capability: CapabilityReport,
        memory_context: MemoryContext,
        risk_evaluation: RiskEvaluation,
    ) -> ArbitrationResult;
}

pub enum ArbitrationResult {
    Execute(Plan),
    Reject(String),
    Delay(Duration),
    SandboxFirst(Plan),
    AskConfirmation(String),
    Reflect,
    Mutate,
}
```

### 5.4 与 Engine 的关系

- Engine (engine.rs) = ReAct 认知循环 (Cognitive Path)
- Coordinator = 仲裁器 (Volitional Path)
- Engine 在高风险决策时调用 Coordinator

```text
普通任务: Event → BrainCore → BodyRuntime → Action
高风险:   Event → SelfField → Coordinator → BrainCore → BodyRuntime → Action
```

---

## 6. aletheon-comm 设计

### 6.1 定义

统一通信层，合并 EventBus (内部消息路由) 和 IPC (外部进程通信)。

### 6.2 crate 结构

```text
aletheon-comm/
├── src/
│   ├── lib.rs
│   ├── core/
│   │   ├── mod.rs
│   │   ├── event.rs         — Event trait (已有)
│   │   ├── bus.rs           — EventBus trait (已有)
│   │   └── transport.rs     — Transport trait (新增，抽象 IPC)
│   ├── bridge/
│   │   └── mod.rs
│   ├── impl/
│   │   ├── mod.rs
│   │   ├── kernel_bus.rs    — 内核事件总线 (已有)
│   │   ├── event_log.rs     — 事件日志 (已有)
│   │   ├── routing_policy.rs— 路由策略 (已有)
│   │   ├── subscription.rs  — 订阅管理 (已有)
│   │   └── ipc/
│   │       ├── mod.rs
│   │       ├── unix_socket.rs    — Unix 域套接字 (从 argos-ipc)
│   │       ├── io_uring.rs       — io_uring 后端 (从 argos-ipc)
│   │       ├── shared_mem.rs     — 共享内存 (从 argos-ipc)
│   │       ├── json_rpc.rs       — JSON-RPC 适配 (从 argos-ipc)
│   │       ├── priority_queue.rs — 消息优先级 (从 argos-ipc)
│   │       └── manager.rs        — IPC 管理器 (从 argos-ipc)
│   └── testing/
│       └── mock_bus.rs
```

### 6.3 设计要点

- EventBus 是逻辑层：事件类型、订阅、路由策略
- IPC 是物理层：传输协议、序列化、连接管理
- EventBus 可以使用 IPC 作为底层传输
- 内部模块间通信用 EventBus，跨进程通信用 IPC

---

## 7. 实现顺序

### Phase 1: 基础层（无依赖）

1. **aletheon-abi** — 合并 argos-types + error.rs
2. **aletheon-comm** — 合并 EventBus + argos-ipc

### Phase 2: 核心模块

3. **aletheon-memory** — 已有，验证完整性
4. **aletheon-self-field** — 已有，验证 + 补充 testing
5. **aletheon-brain-core** — 已有，补充 grounding + provider_registry

### Phase 3: 身体和运行时

6. **aletheon-body** — 补充 acix + ui + platform + testing
7. **aletheon-runtime** — 补充 engine + config + coordinator

### Phase 4: 设计文档

8. **aletheon-meta** — 编写 MetaRuntime/Morphogenesis/Genome 设计文档

### Phase 5: 清理

9. 更新 argos-cli 和 argosd 的依赖
10. 移除旧的 argos-core、argos-types、argos-ipc、argos-acix crate
11. 更新 Cargo.toml workspace members
12. 验证编译通过

---

## 8. 风险和缓解

| 风险 | 影响 | 缓解 |
|---|---|---|
| 迁移后编译失败 | 高 | 每个 crate 迁移后立即验证编译 |
| 测试遗漏 | 中 | 迁移时同步迁移测试代码 |
| 依赖循环 | 高 | 严格遵循依赖图，aletheon-abi 无依赖 |
| 性能回退 | 低 | 重构不改变算法，只改组织结构 |

---

## 9. 验证标准

- [ ] `cargo build` 全 workspace 编译通过
- [ ] `cargo test` 全 workspace 测试通过
- [ ] 旧 argos-core/argos-types/argos-ipc/argos-acix crate 已移除
- [ ] argos-cli 和 argosd 正常编译和运行
- [ ] 每个 aletheon crate 遵循 core/bridge/impl 分层
- [ ] MetaRuntime 设计文档完成
- [ ] Coordinator 设计文档完成
