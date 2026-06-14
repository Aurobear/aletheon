# Aletheon v4 统一目录结构设计

日期: 2026-06-14
状态: 已批准

---

## 1. 背景

### 1.1 问题

v3 迁移后存在以下结构问题：
- **散落文件**: runtime 5 个、self-field 8 个、brain-core 3 个 loose files 在根目录
- **bridge 位置不一致**: 有的在根目录，有的在子模块内
- **God file**: runtime/engine/mod.rs 1,369 行
- **命名冲突**: body/driver/sandbox/ vs body/sandbox/
- **结构不一致**: 有的扁平，有的混合

### 1.2 目标

- 所有 crate 统一采用 `core/` + `bridge/` + `impl/` 三层模式
- 删除旧 Engine god file
- 消除散落文件
- 统一命名规范

---

## 2. 统一目录结构模式

```
crate/src/
├── lib.rs              ← 模块声明 + 公共 re-exports
├── core/               ← 核心逻辑（本层的"大脑"）
│   ├── mod.rs
│   └── ...
├── bridge/             ← 与其他层的桥接
│   ├── mod.rs
│   └── ...
└── impl/               ← 具体实现（子系统）
    ├── mod.rs
    └── ...
```

---

## 3. 各层具体结构

### 3.1 Body（身体层）

```
aletheon-body/src/
├── lib.rs
├── core/
│   ├── mod.rs          ← Body trait 实现
│   └── conversions.rs  ← 类型转换
├── bridge/
│   └── mod.rs          ← 与 ABI/EventBus 的桥接
└── impl/
    ├── mod.rs
    ├── tools/          ← 工具执行（23 files）
    ├── sandbox/        ← 沙箱（11 files）
    ├── driver/         ← 硬件驱动（19 files）
    └── mcp/            ← MCP 客户端（6 files）
```

变更：
- 删除 `body_runtime.rs`（旧 bridge）→ 移入 `core/mod.rs`
- 删除 `conversions.rs` 根目录文件 → 移入 `core/conversions.rs`
- 重命名 `driver/sandbox/` → `driver/sandbox_driver/` 消除命名冲突
- 删除 `driver/io/mod.rs` 和 `driver/proc/mod.rs`（TODO stubs）
- 内联 `sandbox/backend.rs` 和 `tools/exposure.rs`（re-export stubs）

### 3.2 BrainCore（认知层）

```
aletheon-brain-core/src/
├── lib.rs
├── core/
│   ├── mod.rs          ← BrainCore trait 实现
│   ├── reasoner.rs
│   ├── planner.rs
│   ├── reflector.rs
│   ├── critic.rs
│   ├── learner.rs
│   └── world_model.rs
├── bridge/
│   ├── mod.rs
│   ├── llm.rs          ← LLM 桥接（原 llm_bridge.rs）
│   ├── inference.rs    ← 推理桥接（原 inference_bridge.rs）
│   └── learning.rs     ← 学习桥接（原 learning_bridge.rs）
└── impl/
    ├── mod.rs
    ├── llm/            ← LLM 提供者
    ├── inference/      ← 推理路由
    ├── learning/       ← 学习系统
    └── provider_registry.rs
```

变更：
- 移动 `brain_core.rs` → `core/mod.rs`
- 移动 `reasoner.rs`, `planner.rs`, `reflector.rs`, `critic.rs`, `learner.rs`, `world_model.rs` → `core/`
- 移动 `*_bridge.rs` → `bridge/`
- 移动 `llm/`, `inference/`, `learning/`, `provider_registry.rs` → `impl/`

### 3.3 SelfField（主体场）

```
aletheon-self-field/src/
├── lib.rs
├── core/
│   ├── mod.rs          ← SelfField trait 实现
│   ├── identity.rs
│   ├── boundary.rs
│   ├── care.rs
│   ├── narrative.rs
│   ├── conflict.rs
│   ├── attention.rs
│   ├── continuity.rs
│   └── mutation.rs
├── bridge/
│   ├── mod.rs
│   ├── hook.rs         ← Hook 桥接（原 hook_bridge.rs）
│   ├── policy.rs       ← 策略桥接（原 policy_bridge.rs）
│   ├── loop_detector.rs ← 循环检测桥接（原 loop_bridge.rs）
│   └── perception.rs   ← 感知桥接（原 perception/bridge.rs）
└── impl/
    ├── mod.rs
    ├── hook/           ← Hook 系统
    ├── resilience/     ← 韧性保护
    ├── security/       ← 安全策略
    └── perception/     ← 感知系统
```

变更：
- 移动 `self_field.rs` → `core/mod.rs`
- 移动 `identity.rs`, `boundary.rs`, `care.rs`, `narrative.rs`, `conflict.rs`, `attention.rs`, `continuity.rs`, `mutation.rs` → `core/`
- 移动 `*_bridge.rs` → `bridge/`
- 移动 `perception/bridge.rs` → `bridge/perception.rs`
- 移动 `hook/`, `resilience/`, `security/`, `perception/` → `impl/`

### 3.4 Runtime（编排层）

```
aletheon-runtime/src/
├── lib.rs
├── core/
│   ├── mod.rs          ← Runtime trait 实现
│   ├── orchestrator.rs ← 顶层编排器（原 aletheon_runtime.rs）
│   ├── behavior_paths.rs
│   ├── react_loop.rs
│   └── config.rs
├── bridge/
│   └── mod.rs          ← 与其他层的桥接
└── impl/
    ├── mod.rs
    ├── agent/          ← Agent 生命周期（原 agent_runtime.rs）
    ├── orchestration/  ← 多 Agent 编排
    ├── automation/     ← 自动化调度
    ├── session/        ← 会话管理
    └── plugin/         ← 插件系统
```

变更：
- 删除 `engine/mod.rs`（1,369 行 God file）
- 移动 `aletheon_runtime.rs` → `core/orchestrator.rs`
- 移动 `behavior_paths.rs`, `react_loop.rs`, `config.rs` → `core/`
- 移动 `agent_runtime.rs` → `impl/agent/mod.rs`
- 移动 `orchestration/`, `automation/`, `session/`, `plugin/` → `impl/`

### 3.5 ABI（保持不变）

```
aletheon-abi/src/
├── lib.rs
├── body.rs
├── brain.rs
├── capability.rs
├── context.rs
├── event.rs
├── event_bus.rs
├── genome.rs
├── memory.rs
├── meta.rs
├── runtime.rs
├── self_field.rs
└── subsystem.rs
```

纯类型 crate，扁平结构合理。

### 3.6 EventBus（保持不变）

```
aletheon-event-bus/src/
├── lib.rs
├── event_log.rs
├── kernel_event_bus.rs
├── routing_policy.rs
└── subscription.rs
```

小 crate，扁平结构合理。

### 3.7 Memory（保持不变）

```
aletheon-memory/src/
├── lib.rs
├── episodic.rs
├── procedural.rs
├── router.rs
├── schema.rs
├── self_memory.rs
└── semantic.rs
```

小 crate，扁平结构合理。

---

## 4. 命名规范

### 4.1 文件命名
- 模块目录: `snake_case/`（如 `tools/`, `sandbox/`, `llm/`）
- 文件名: `snake_case.rs`（如 `behavior_paths.rs`, `provider_registry.rs`）
- Bridge 文件: 不加 `_bridge` 后缀，直接用功能名（如 `bridge/llm.rs` 不是 `bridge/llm_bridge.rs`）

### 4.2 模块命名
- 核心模块: `core/`
- 桥接模块: `bridge/`
- 实现模块: `impl/`
- 子系统: 用功能名（如 `tools/`, `sandbox/`, `llm/`）

### 4.3 消除命名冲突
- `body/driver/sandbox/` → `body/driver/sandbox_driver/`
- `body/sandbox/` 保持不变（bubblewrap 沙箱）

---

## 5. 实施步骤

### Phase 1: Body 重构
1. 创建 `core/`, `bridge/`, `impl/` 目录
2. 移动文件到新位置
3. 更新 `lib.rs` 模块声明
4. 更新内部 import 路径
5. 删除旧文件
6. 运行测试

### Phase 2: BrainCore 重构
同上

### Phase 3: SelfField 重构
同上

### Phase 4: Runtime 重构
1. 删除 `engine/mod.rs`
2. 创建 `core/`, `bridge/`, `impl/` 目录
3. 移动文件到新位置
4. 更新 import 路径
5. 运行测试

---

## 6. 验证

- 每个 Phase 运行 `cargo test --workspace`
- 确保所有测试通过
- 确保没有编译错误
- 确保没有 orphaned 文件
