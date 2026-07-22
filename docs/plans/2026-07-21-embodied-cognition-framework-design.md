# Aletheon 具身认知框架实施设计（P0/P1 首切 spec）

> 类型：实施设计（伴生 spec）
> 伴生文档：`docs/plans/Aletheon_Robot_Cognitive_Inference_Architecture_and_Coupling_Plan.md`（审计 + 目标架构裁决）
> 代码基线：`dev`（核查时最新提交 `4000720`；原审计基线 `114039e`）
> 颗粒度：全景框架 + P0/P1 文件级步骤 + 全景缺失清单
> 设计日期：2026-07-21

## 1. 关系声明与代码基线

本文档不重复伴生审计文档的论证与裁决，只做三件事：

1. 把目标架构**锚定在当前真实代码**上（每个断言带 `path:line`）；
2. 把 **P0（认知执行入口收敛）** 与 **P1（hardware 生产化纵切）** 写成可直接进 `plans` 的文件级步骤；
3. 给出**全景缺失清单**，标注归属 P 阶段与严重度。

伴生审计文档中的七条最终裁决（§14）在此视为已批准前提，不再复述。

> 代码事实为本设计撰写时（基线 `4000720`）的核查结果。实施每个阶段前，应在最新 `dev` 上重跑一次调用方搜索确认锚点未漂移（见 §7）。

## 2. 全景框架（锚定真实代码）

### 2.1 核心结论

伴生文档 §8 期望的依赖纪律**当前已大体成立**，因此本轮工作性质是「防退化 + 补窄模块」，不是「推倒重建」：

- `fabric` 是被所有域依赖的地基契约层；
- 仅 `aletheon`（CLI 入口）依赖 `executive`；
- 没有任何领域 crate 反向依赖 `executive`。

### 2.2 工作区结构与本轮改动落点

```
fabric        契约 / 共享类型 / 通信（地基，被所有域依赖）
  ├─ cognit       harness / ReAct / WorldModel(v0)   ▶ P3 加 RobotHarness
  ├─ hardware     device/command/lease/safety/sim     ▶ P1 加 observation/skill/registry/broker
  ├─ kernel       process/operation/permit/lease      ▷ 保持纯治理，不加机器人专用语义
  ├─ agora        共享认知工作区（黑板）              ▷ 只接归一化候选
  ├─ mnemosyne    记忆（情节/语义/程序）              ▶ P3 加 EmbodiedEpisode
  ├─ metacog      自更新 / 元认知                     ▶ P3 加 OutcomeVerifier
  ├─ dasein       自我约束 / 反思                     ▷ 不引入 Robot/ROS 类型
  └─ corpus       工具执行体                          ▶ P1 加 6 个窄机器人工具
executive        编排 / composition root（仅被 aletheon 依赖）  ▶ P1 加 EmbodimentServices
aletheon         CLI 入口（daemon / exec / TUI）
[私有仓库] aletheon-kuavo-bridge   workspace 外，只见版本化协议  ▶ P2
```

图例：`▶` 本轮或后续阶段新增；`▷` 保持现状 + 防退化约束。

### 2.3 各域职责边界（含关键锚点）

| 域 | 职责一句话 | 关键锚点 | 边界约束 |
|---|---|---|---|
| `fabric` | 跨域 DTO / ID / schema / 事件 envelope | `crates/fabric/src/types/workspace.rs:44`（`WorkspaceObservation`） | 不拥有 provider / 业务实现 / ROS 类型 |
| `cognit` | 推理、规划、harness | `crates/cognit/src/harness/mod.rs:39`（`HarnessKind`）；`crates/cognit/src/core/world_model.rs:28`（`WorldModel`） | 依赖抽象端口，不依赖 ROS |
| `kernel` | Process/Operation/Permit/Lease 治理权威 | — | 不知道 Robot/ROS/Skill/Joint |
| `hardware` | 设备命令 / 许可 / 租约 / fail-safe / 模拟器 | `crates/hardware/src/provider.rs:14`（`DeviceProvider::apply`）；`crates/hardware/src/device.rs:7-11`（自造 ID/时间类型） | 不做意图理解 / 授权决策 / ROS mapping |
| `executive` | 编排 + composition root | `crates/executive/src/service/turn_pipeline.rs:43-67`（god object）；`crates/executive/Cargo.toml:48`（hardware 仅 dev-dep） | 唯一 composition root，可依赖各域 |
| `corpus` | 受治理的工具执行 | — | 不调 ROS、不自管 lease |

## 3. 已存在的可复用 seam（本设计的增量价值）

伴生审计已指出「缺什么」；本节回答「哪些扩展点已经现成、可直接接线」，避免 P0/P1 误做重建。

| Seam | 位置 | 现状 | P0/P1 用法 |
|---|---|---|---|
| `CognitiveSessionFactory::create()` | `crates/executive/src/service/harness_factory.rs:11` | async 工厂 trait，已注入 TurnPipeline | P3 在此按 `harness_kind` 造 RobotHarness；P0 仅接通 Linear 选择并证明未知/Robot 配置 fail closed；P3 才新增 `Robot` variant |
| `LinearCognitiveSessionFactory` | `crates/executive/src/service/harness_factory.rs:42` | 唯一实现，bootstrap 硬编码构造 | P0 保持行为不变 |
| `HarnessKind` | `crates/cognit/src/harness/mod.rs:39-44` | 只有 `Linear`；`build_harness` 在 `:51` | P3 加 `Robot` variant + 分支 |
| `ExecutiveConfig.harness_kind` | `crates/executive/src/core/config/agent.rs:46` | 字段存在，但 bootstrap 用 `..Default::default()` **运行时未消费** | P0 接线：bootstrap 读真实值并透传 |
| `TurnPipelineResources` 注入结构 | 注入点 `crates/executive/src/impl/daemon/bootstrap/services.rs:376-398`；字段 `crates/executive/src/service/turn_pipeline.rs:56-57` | god object 的构造入口 | P1 只新增**一个**窄端口字段 `Arc<dyn EmbodimentExecutionPort>` |
| `TurnEngine` / `SessionTurnEngine` | `crates/executive/src/service/turn_engine.rs` | 已有合同雏形；exec 用它，daemon 走 `DaemonTurnOrchestrator → TurnPipeline`（`launcher.rs:56-88`） | P0 strangler 收敛的委托目标 |
| `DeviceProvider::apply()` | `crates/hardware/src/provider.rs:14` | 同步窄接口 | P1 新增 async `EmbodimentProvider` **并存**，不动旧 trait |
| `RuntimeCapability::{DeviceObserve,DeviceCommand}` | `crates/runtime/src/manifest.rs:14-15` | 枚举在 sub-agent runtime manifest 中 | P4 迁移到 hardware manifest；本轮不动 |

## 4. P0 spec —— 认知执行入口收敛（strangler）

**目标：** daemon 与 exec 产出同构生命周期，且 factory 真正消费 `harness_kind`——**不重写 `TurnPipeline`（≈64KB / 15+ 字段）**。

**策略：** 缘化适配（strangler）。引入/确认共享 `TurnEngine` 合同，让两条入口都委托给它；`TurnPipeline` 内部实现保持不动，只从多个入口收敛为单一入口语义。

### 4.1 步骤

1. **确认 `TurnEngine` 合同边界**
   - 文件：`crates/executive/src/service/turn_engine.rs`
   - 明确 `TurnEngineRequest` / `TurnEngineContext` / `TurnEngineEventSink` 覆盖 daemon 与 exec 双方所需的 operation / cancel / event / settlement 语义。
   - 产出：合同即 daemon/exec 唯一生产 turn 入口的书面定义（不新增机器人专用入口）。

2. **daemon 改为薄适配器委托**
   - 文件：`crates/executive/src/host/launcher.rs`（`run_daemon`，`:56-88`）及 daemon orchestrator。
   - `DaemonTurnOrchestrator` 从「直接驱动 `TurnPipeline`」改为「构造 `TurnEngineRequest` 并委托 `TurnEngine`」；`TurnPipeline` 成为 `TurnEngine` 背后的执行实现，不再是入口。
   - 约束：本步不改 `TurnPipeline` 字段结构，只改调用方向。

3. **exec 对齐同一委托**
   - 文件：`crates/executive/src/service/turn_engine.rs`（`SessionTurnEngine`，`:105-123`）。
   - 确认 exec 路径与 daemon 走同一 `TurnEngine::run` 语义。

4. **factory 接线 `harness_kind`**
   - 文件：`crates/executive/src/impl/daemon/bootstrap/request.rs:482-492`（当前 `..Default::default()`，`harness_kind` 未消费）。
   - 改为从加载配置读取 `harness_kind` 透传给 `harness_factory`（`:494`）。
   - 文件：`crates/executive/src/service/harness_factory.rs` —— 工厂按 `harness_kind` 分支；本阶段仅保证 `Linear` 行为不变，P0 不新增 `Robot` variant；当前反序列化对 `robot` 明确失败，P3 实现真实 RobotHarness 时再一次性加入 variant 与工厂分支。

### 4.2 完成标准（验收）

- `turn_engine_parity` 测试：同一 turn 输入，在 daemon 与 exec 下产生**同构** lifecycle 事件序列与 settlement。
- daemon 与 exec 均通过 `TurnEngine` 入口；代码搜索 `TurnPipeline::` 的直接外部调用点收敛到 `TurnEngine` 实现内部。
- 配置 `harness_kind = "linear"` 被运行时真实读取（可通过日志/断言验证），行为与改动前一致。

### 4.3 非目标（防止 P0 膨胀）

- 不拆分 `TurnPipeline` 字段、不做 god object 内部重构（那是独立的后续迁移，见 §6）。
- 不实现 `HarnessKind::Robot` 的真实逻辑。
- 不改 EventBus 语义（伴生文档 §8 问题 10 为渐进项）。

## 5. P1 spec —— hardware 生产化纵切（模拟器跑通）

**目标：** 用现有 `SimulatedDevice` 走**完整生产路径**（而非测试手工调用），打通「模型 → Corpus 工具 → Kernel 授权 → Hardware 执行 → Executive 监督/settlement → Agora/Mnemosyne」闭环。

### 5.1 fabric：稳定跨域协议

- 新增 `crates/fabric/src/types/embodiment.rs`：`DeviceId`、`SkillId`、`EmbodiedObservation`、`EvidenceRef`、`SafetyEvent`、`ObservationSchemaVersion`。
- 新增 `crates/fabric/src/types/skill.rs`：`SkillDescriptor`、`SkillRequest`、`SkillProgress`、`SkillResult`。
- 新增 `crates/fabric/src/types/observation.rs`：`EmbodiedObservation` envelope（`schema/schema_version/source/sequence/source_time/received_at/valid_until/confidence/frame_ref/payload/evidence`）。
- 约束：所有 payload 带大小边界、schema version、provenance；**不放** `sensor_msgs::*` / ROS topic 名 / 公司 action 名。现有 `WorkspaceObservation`（`crates/fabric/src/types/workspace.rs:44-50`）可扩展 envelope 字段或引用 `EmbodiedObservation`。

### 5.2 hardware：从测试域升级为生产执行域

- 新增 `crates/hardware/src/observation.rs`：规范化观察、sequence、dedupe、staleness。
- 新增 `crates/hardware/src/skill.rs`：技能描述 / 请求 / 进度 / 结果。
- 新增 `crates/hardware/src/registry.rs`：provider / device / skill 注册。
- 新增 `crates/hardware/src/broker.rs`：校验、路由、sequence、deadline、receipt 归一化。
- 新增 async 领域端口 `EmbodimentProvider`（`snapshot / list_skills / execute_skill(带 progress sink) / cancel / safe_stop`），与旧 `DeviceProvider::apply()`（`provider.rs:14`）**并存**，不破坏现有测试。
- **只统一同语义标识**：P1 将 `DeviceId`/`SkillId` 放到 `fabric`；`hardware::{PrincipalId,OperationId,MonotonicInstant}` 与 Fabric/Kernel 同名类型语义不同，P1 保留并在跨域处使用显式、受测映射。是否统一留给独立后续裁决；hardware 内部 sequence 保留。

### 5.3 executive：新增具身编排服务

- `crates/executive/Cargo.toml:48`：把 `hardware` 从 `[dev-dependencies]` 提升为生产 `[dependencies]`。
- 新增 `crates/executive/src/service/embodiment_service.rs`、`robot_operation.rs`、`skill_supervisor.rs`。
- 新增 `crates/executive/src/impl/daemon/bootstrap/embodiment.rs`：把上述聚合为独立 `EmbodimentServices` 装配，**不**把字段堆进 `TurnPipelineResources` / daemon `request.rs`。
- `TurnPipeline`（`turn_pipeline.rs:56-57` 附近）只新增**一个**窄端口字段：`Arc<dyn EmbodimentExecutionPort>`（必要时再加 `Arc<dyn WorldModelPort>`，但 WorldModel 生产化属 P3）。
- 职责：为技能执行创建 Kernel Operation、请求 admission/permit/lease、驱动 broker/provider、接收 progress、处理 cancel/timeout/disconnect、settlement 并把结果送 Agora/Mnemosyne。

### 5.4 corpus：受治理的窄机器人工具

- 新增 `crates/corpus/src/tools/robot.rs`，暴露 6 个窄工具：`robot.observe`、`robot.get_state`、`robot.list_skills`、`robot.execute_skill`、`robot.cancel`、`robot.safe_stop`。
- 工具实现由 executive 注入的 embodiment 端口支撑；Corpus 不调 ROS、不自管 lease。
- **显式禁止**暴露给模型：`robot.publish_topic`、`robot.call_any_service`、`robot.set_joint`、`robot.raw_bus_write`。

### 5.5 依赖方向约束（回归测试）

- `hardware -> fabric`，不依赖 `executive`/`corpus`；
- Corpus tool adapter 依赖定义在 `fabric` 的跨域窄端口 `EmbodimentExecutionPort`；hardware 保留 provider/broker/registry 等领域合同，executive 负责实现、创建并注入；hardware 永不调用 executive（伴生文档 §8 问题 3）。

### 5.6 完成标准（验收）

- 模型经 Corpus 发起模拟技能 → Kernel 授权 → Hardware 执行 → Executive 监督 + settlement → 结果进 Agora/Mnemosyne。
- 全程同一 `OperationId` 可追踪（发起到 settlement）。
- 支持 progress / cancel / success / failure / timeout。
- lease 过期或 provider 断连触发 fail-safe（即使认知会话崩溃）。
- 替换 provider 实现（模拟器 → 未来 ROS bridge）只改变 composition 注册/配置，不改变 Cognit、Corpus、TurnPipeline 或 `EmbodimentService` 业务逻辑。

## 6. 缺失部分清单（全景，标 P 阶段 + 严重度）

| 缺口 | 归属 | 严重度 | 说明 / 首要措施 |
|---|---|---|---|
| TurnEngine 收敛本身是独立迁移工程 | P0 | 高 | 在 64KB god object 上动刀；strangler 降险，但仍需独立 parity 门禁 |
| async provider 的**取消传播 / 进度背压 / 超时与 lease 竞争**时序契约 | P1 | 高 | 伴生文档仅点到（§8 问题 10）；本设计要求在 `EmbodimentProvider` spec 中显式定义取消传播、progress 背压、timeout 与 lease-expiry 的竞争裁决 |
| OutcomeVerifier 的 **expected-outcome 表达方式** | P3 | 高 | 未定义（几何？谓词？状态差分？）；本轮**只登记为待解**并给候选方向（见 §6.1），不在 P0/P1 展开 |
| `TurnPipeline` god object 内部拆分 | P0 后 | 中 | P0 只收敛入口，不拆字段；拆分是独立后续迁移 |
| `WorldModel` 生产化（BeliefVersion/TTL/frame/decay/WorldDelta） | P3 | 中 | 现为 v0（`world_model.rs:28`），仅自身测试使用 |
| 结构化 `EmbodiedEpisode` 记忆 | P3 | 中 | mnemosyne 现记录通用观察/反思，非机器人动作 episode |
| `HarnessKind::Robot` + 状态机 | P3 | 中 | P0 保持 `robot` 配置 fail closed，P3 一次性新增 variant、工厂与 Observe→Verify→Recover 状态机 |
| 视觉 / camera perception ingress | P4 | 中 | 现 `VisionGroundingProvider` 面向 UI 截图，非机器人 perception |
| VLA / `PolicyProvider` 端口 | P4 | 低 | 不复用 `SubAgentRuntime`；`RuntimeCapability::DeviceObserve/Command`（`manifest.rs:14-15`）迁出 |
| EventBus 从状态权威退化为 transport | 渐进 | 低 | command/cancel/stop 用显式 request-response，不绑单次全仓重构 |
| HIL / 故障注入 / 真机 allowlist 门禁 | P5 | 低 | 默认 simulation namespace，production 显式配置 |

### 6.1 OutcomeVerifier expected-outcome —— 待解项候选方向（不在本轮实施）

登记三个候选表达，供 P3 brainstorm 时裁决，本文档不选定：

- **谓词式**：`expected: [gripper.holding(obj), base.at(zone_a)]`，Verifier 对 WorldDelta 求值布尔。
- **状态差分式**：声明关注实体的期望属性区间，Verifier 比对 `WorldDelta` 数值容差。
- **几何/位姿式**：期望 pose/occupancy，需 frame 与 transform evidence 支撑（依赖 WorldModel 生产化）。

## 7. 验收与验证命令

实施 P0/P1 时使用仓库固定的 Rust 1.88 MSRV，并且所有 Cargo 操作必须通过共享缓存与全局编译锁包装器执行：

```bash
# P1 hardware 单元
bash scripts/cargo-agent.sh test -p hardware
# 现有硬件模拟纵切（P1 参照并扩展为生产路径）
bash scripts/cargo-agent.sh test -p executive --test hardware_simulation
# P0 入口收敛 parity
bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity
# 全量回归（含依赖方向）
bash scripts/cargo-agent.sh test --workspace
```

实施每个阶段前的锚点复核（防止基线漂移）：

```bash
# 确认 hardware 仍仅 dev-dep（P1 起点）
rg -n "hardware" crates/executive/Cargo.toml
# 确认 harness_kind 消费点（P0 起点）
rg -n "harness_kind" crates/executive/src
# 确认 turn 入口分叉现状（P0 起点）
rg -n "DaemonTurnOrchestrator|SessionTurnEngine|TurnEngine" crates/executive/src
```

## 8. 证据索引（本设计新增核查，均带行号）

- `crates/executive/Cargo.toml:48` —— hardware 仅 `[dev-dependencies]`
- `crates/hardware/src/provider.rs:14` —— `DeviceProvider::apply()` 同步窄接口
- `crates/hardware/src/device.rs:7-11` —— 自造 `PrincipalId`/`OperationId`/`MonotonicInstant`
- `crates/cognit/src/harness/mod.rs:39-44,51` —— `HarnessKind` 仅 `Linear`；`build_harness`
- `crates/cognit/src/core/world_model.rs:28` —— `WorldModel` v0
- `crates/executive/src/service/turn_pipeline.rs:43-67,56-57` —— god object；factory 字段
- `crates/executive/src/service/turn_engine.rs:105-123` —— `SessionTurnEngine`
- `crates/executive/src/service/harness_factory.rs:11,42` —— 工厂 trait 与唯一实现
- `crates/executive/src/core/config/agent.rs:46` —— `ExecutiveConfig.harness_kind`
- `crates/executive/src/impl/daemon/bootstrap/request.rs:482-492,494-502` —— bootstrap 未消费 harness_kind / 硬编码 factory
- `crates/executive/src/impl/daemon/bootstrap/services.rs:376-398` —— TurnPipeline 注入点
- `crates/executive/src/host/launcher.rs:56-88` —— run_daemon 路径
- `crates/executive/src/impl/conscious/metacog_processor.rs:36-47` —— 只算置信度/冲突
- `crates/fabric/src/types/workspace.rs:44-50` —— `WorkspaceObservation` 四字段
- `crates/runtime/src/manifest.rs:14-15` —— `RuntimeCapability::DeviceObserve/DeviceCommand`
