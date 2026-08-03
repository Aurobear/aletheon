# Aletheon 机器人通用接口与仿真闭环接入计划

> 目标场景：Aletheon 接收自然语言任务，调用公司机器人仿真，观察状态，执行受控动作，
> 自动验证结果并形成可追踪报告  
> 基线：`dev` / `8f69268c`（2026-08-02）  
> 原则：复用现有 Fabric/Executive/Cognit/Corpus/Hardware 边界，不增加第三套 Robot 状态、
> Planning、Verification 或 Receipt 权威

## 0. 结论

当前仓库不是从零开始。以下骨架已经存在：

- `fabric::types::embodiment` 的稳定设备/skill/observation/result contract；
- `hardware::EmbodimentProvider`、Broker、lease 验证、simulator、gRPC provider；
- Executive 的 Kernel admission、exclusive lease、capability settlement；
- Corpus 的六个 `robot_*` 模型工具；
- `agents/robot-agent.md`；
- daemon bootstrap 中的 simulator/gRPC provider 选择与工具注册；
- `RobotHarness` 状态机、policy proposal port、expected outcome 和 verification 类型；
- Kuavo MuJoCo 的 Aletheon 示例配置。

当前缺失的是把这些部分收成一条生产主链。**差距清单已按 2026-08-03 两个外部仓库
（`~/Workspace/aletheon-kuavo-bridge`、`~/Workspace/kuavo-ros-control`）的现状修订：**

| # | 差距 | 现状（2026-08-03 核对） |
|---|---|---|
| 1 | Kuavo bridge 不在本仓库 | ⚠️ 仍在独立仓库，但已存在且可复现（`aletheon-kuavo-bridge`，`scripts/bootstrap.sh`），并已通过 live MuJoCo Phase E 验收 |
| 2 | gRPC 协议兼容未落地 | ✅ **已关闭**：`crates/hardware/proto/.../gateway.proto` 与 bridge 逐字节一致，且 `crates/hardware/tests/grpc_contract.rs` 用 SHA-256 锁死防漂移；`runtime_embodiment_selection.rs` 证明配置接线完整 |
| 3 | `RobotHarness` 无生产 composition | 🔴 开放：`RobotHarness::new` 目前只在测试里被调用，生产代码无 composition root |
| 4 | gRPC Policy Provider 是 Stub | 🔴 开放：`crates/cognit/src/adapters/policy/grpc_provider.rs` **只有 `StubPolicyProvider`**（`propose`@65 返回硬编码 stance proposal），无真实 gRPC client；Executive 未接线任何 policy provider（`GrpcPolicyConfig` + 非 loopback 强制 TLS 已有） |
| 5 | expected outcome 硬编码 `mode == stance` | 🔴 开放：`crates/cognit/src/harness/robot/mod.rs:202,216,244`（Plan 丢弃 proposal outcome，Execute/Verify 各自硬编码） |
| 6 | operation ID 用 `"op"` 占位 | 🔴 开放：`crates/cognit/src/harness/robot/mod.rs:215`；修复简单，因为 `SkillResult.operation_id: OperationId`（`fabric/src/types/embodiment.rs:84`）已存在 |
| 7 | world-state ingest 无生产管道 | 🟡 核心已实现：`PollingWorldState` + `observation_to_snapshot` + `WorldStatePump`（`crates/executive/src/application/world_state.rs`，含单调/过期/低置信处理），接线到 daemon 在 PR4 composition |
| 8 | progress 用 `NoopEmbodimentProgress` | 🔴 开放：`crates/executive/src/host/daemon/bootstrap/request.rs:805` |
| 9 | report/artifact/rosbag 非端到端完成条件 | 🔴 开放 |

**因此本计划不新建另一个 `robot-runtime` crate，而是补齐现有 owner。** 剩余工作的主战场在
Aletheon 侧（PR1–6、9、10 + simulator 升级），bridge 不是瓶颈。

### 0.1 为什么这 6 项仍然开放，以及如何关闭

这些差距不是"没发现"，而是**代码尚未实现**——bridge 的进度只能关闭"协议侧"差距，关不掉
"harness 侧"差距。每项的当前证据、关闭 PR、以及从 🔴 翻成 ✅ 的判定如下：

| 开放差距 | 当前代码证据（为何还开放） | 关闭 PR | 关闭判定（✅ 当且仅当） |
|---|---|---|---|
| 3 无生产 composition | `RobotHarness::new` 生产代码 0 调用；`robot_harness_factory.rs` 只测 `HarnessKind::Robot` 枚举可选性 | PR4 + PR6 | daemon 启动即构造 `RobotHarness`；`HarnessKind::Robot` 配置失败 fail closed，不回落 Linear |
| 5 硬编码 `mode==stance` | `mod.rs:216-225`（Execute `append_attempt`）、`mod.rs:244-252`（Verify）仍是 `Equals("mode","stance")`；Plan 态 `mod.rs:147-190` 丢弃 proposal outcome | PR1 | 注入带非 stance outcome 的 `SkillProposal`，fake verifier 断言收到的是 proposal 的 outcome |
| 6 `"op"` 占位 | `mod.rs:215` 仍是字面量 `"op"` | PR2 | 已创建 operation 的 attempt 保存 host/provider 返回的 typed `OperationId`；创建前失败保持 `None` 并使用独立 attempt ID |
| 7 无 world-state 管道 | 无 pump 模块；RobotHarness 只在 Observe 态 `world_state.latest()` 拉一次快照 | PR3 ✅ | `PollingWorldState`/`WorldStatePump` 实现：单调 sequence 由 ingest 强制；过期/低置信标记 `stale` 且不进入稳定窗口（verifier 侧拒绝） |
| 8 `NoopEmbodimentProgress` | `request.rs:805` 仍把 `NoopEmbodimentProgress` 注入 `build_embodiment_port` | PR6 | session log 出现带真实 operation_id 的 `SkillProgress` 事件 |
| 9 无报告/artifact | 无 report builder；`EpisodeSink::close_episode` 只写 `"completed"` 字符串 | PR5 + PR10 | 端到端报告能打开 `EvidenceRef` 引用的 artifact |

> **10 个 PR 的完整归属**：上表只覆盖 6 个开放项。剩余 PR 关闭另外 3 项——#2（已关闭）→ PR7
> 维护项、#4（开放）→ PR9、#1（单独仓库）→ PR8。即：PR1–6 关 #5/#6/#7/#3/#8，#9 跨 PR5+PR10，
> PR7/8/9 关 #2/#1/#4，10 个 PR 全部有归属。

---

## 1. 目标与非目标

### 1.1 第一阶段目标

用户输入：

```text
让 kuavo-mujoco-01 进入站立状态，保持 3 秒，检查是否稳定并生成报告。
```

系统必须完成：

```text
自然语言目标
 -> 选择 robot-agent / robot goal flow
 -> 获取设备和 skill manifest
 -> 获取执行前 observation
 -> 生成结构化 SkillProposal + ExpectedOutcome
 -> Kernel admission + exclusive lease
 -> gRPC bridge 调用 ROS/MuJoCo
 -> 持续接收 progress / observation
 -> 确定性验证稳定窗口
 -> settlement
 -> 保存 episode、receipt、关键指标和 artifact 引用
 -> 返回基于证据的报告
```

#### 第一阶段入口与选择规则

`robot-agent / robot goal flow` 不是由模型根据自然语言自由切换的隐式模式。第一阶段采用以下确定性
选择规则：

1. Executive 从 typed request 中解析显式 `device_id`，并确认 effective configuration 已启用
   embodiment provider；
2. host 从 provider 返回的设备与 skill manifest 中确认目标设备和候选 skill；
3. 只有设备存在、provider healthy、manifest 有效且 RobotHarness 所需端口全部完成 composition 时，
   才能进入 Robot Goal Flow；
4. Robot Goal Flow 使用 `robot-agent` profile，但 profile 不能自行扩大 capability、设备或 skill scope；
5. 任一条件不满足都 fail closed，返回结构化拒绝原因；不得回落 linear harness 后继续宣称机器人任务
   已执行。

模型、LLM planner 或 VLA 只能在 host 给出的设备、skill 和参数 schema 范围内产生 proposal，不能把
prompt 中出现但 manifest 不存在的设备或 skill 当成运行时事实。

#### 执行前门禁

进入 `Kernel admission` 前必须同时满足：

- provider health 检查成功，设备存在且处于第一阶段允许的 MuJoCo simulation environment；
- manifest 的 provider/device/skill/schema version 已记录，目标 skill 属于当前 allowlist；
- `SkillProposal` 通过 schema、设备、skill、参数、scope 和 `ExpectedOutcome` 校验；
- before observation 的 device/schema 匹配、未过 `freshness_ms`，并带可比较的单调 sequence；
- deadline、取消句柄、safe-stop 策略、principal、permit scope 和 exclusive lease scope 已确定；
- episode 已创建，proposal digest、manifest digest 和 before observation 引用可持久化。

manifest 在 admission 后、执行前发生版本变化时必须重新规划和重新 admission；不得沿用旧 proposal。
上述任一门禁失败时，不得调用 bridge 的动作 RPC。

#### Observation 与确定性验证合同

动作执行后的 observation 必须满足：

- `after.sequence > before.sequence`，且 observation 时间不早于 operation 开始时间；
- device、schema 和 simulation instance 与 before observation 一致；
- 样本未过 `freshness_ms`，丢序、重复、过期或其他实例的样本不能计入稳定窗口；
- verifier 在连续 `stable_window_ms` 内对 `ExpectedOutcome` 求值，第一条 E2E 的窗口为 3 秒；
- `fall_detected`、emergency stop、provider disconnect 等 unsafe predicate 优先于成功 predicate；
- LLM/VLA 文本、bridge RPC success 和单次 `SkillResult` 都不能替代 observation-based verification。

#### 成功与失败终态

```text
verification == Matched
 -> capability settlement success
 -> goal/episode settlement success
 -> persist report + receipts + EvidenceRefs

Unsafe / timeout / stale observation / bridge unavailable / cancellation
 -> request cancel（仍可通信时）
 -> safe_stop（策略要求且执行边界可用时）
 -> 等待权威 terminal receipt 或记录 terminal state unknown
 -> capability/goal/episode settlement failure
 -> persist failure report + available evidence
```

提交动作不等于动作完成。调用方必须观察 authoritative terminal snapshot、terminal event 或 durable
receipt 后才能报告终态。若 cancel/safe-stop 的终态不可确认，报告必须写 `unknown`，不得渲染为成功。

#### EpisodeReport 是报告权威

最终自然语言回答只是结构化 `EpisodeReport` 的投影，不是新的事实来源。报告至少包含：

- goal、device、skill、simulation instance 和版本；
- proposal、`ExpectedOutcome`、proposal/manifest digest；
- principal、permit、lease、attempt/invocation ID，以及存在时的 authoritative operation ID；
- before/after observation sequence、验证窗口和参与判定的样本摘要；
- progress、`SkillResult`、`VerificationReport` 与 settlement；
- cancel/safe-stop 结果和失败分类（如适用）；
- episode/receipt/audit/artifact 的 `EvidenceRef`，以及 Aletheon/bridge/protocol 版本。

rosbag、日志和图表等大对象由 artifact store 持有，EpisodeReport 只保存引用、摘要和 digest。只有报告
已持久化且引用可打开，任务才可对用户显示“已完成”。

#### 第一阶段环境边界

第一阶段只允许显式标识的 Kuavo MuJoCo simulation instance。HIL 和实机必须使用不同的 typed
environment/mode、配置门禁和验收阶段；不得根据名称、地址或模型输出把 simulation 自动提升为 HIL
或 real robot。配置宣称 simulation 但 provider/observation 无法证明实例身份时，启动或任务 admission
必须失败。

### 1.2 暂不包含

- Aletheon 内部实现 WBC、MPC、IK 或电机环；
- LLM/VLA 直接输出 CAN FD/EtherCAT 帧；
- 未经仿真/HIL gate 直接接实机；
- 自主生成任意关节轨迹并直接下发；
- 用 LLM 主观判断替代数值 verifier；
- 同时支持所有机器人、仿真器和 VLA。

---

## 2. 当前实际调用链

当前 daemon 已执行：

```text
bootstrap/request.rs
 -> build_embodiment_port(...)
 -> Simulator or GrpcEmbodimentProvider
 -> Broker
 -> EmbodimentService
 -> register_robot_tools(...)
 -> robot_observe / robot_get_state / robot_list_skills
    / robot_execute_skill / robot_cancel / robot_safe_stop
```

自然语言可通过默认 linear harness 和 `robot-agent` 产生工具调用。动作路径是：

```text
robot_execute_skill
 -> EmbodimentExecutionPort::execute_skill
 -> Executive EmbodimentService
 -> CapabilityRequest(name = hardware.command)
 -> Kernel CapabilityInvoker admit
 -> ExecutionPermit + Lease
 -> EmbodimentCapabilityExecutor
 -> Hardware Broker validates projected authority
 -> EmbodimentProvider::execute_skill
 -> SkillResult
 -> capability settlement
```

这条链已经具备较好的权限边界。不要绕过它直接从 Corpus Tool 调 gRPC/ROS。

---

## 3. 目标架构

```text
User / Automation
       |
       v
Executive Goal/Turn authority
       |
       v
Cognit RobotHarness
  Observe -> Plan -> Authorize -> Execute -> Verify
       |          proposal only          |
       |                                 v
       |                         deterministic verifier
       v
PolicyProviderPort
  LLM planner / VLA / rule policy
       |
       v
SkillProposal + ExpectedOutcome
       |
       v
Corpus robot capability tools / Executive EmbodimentService
       |
       v
Kernel admission + exclusive device lease
       |
       v
Hardware Broker + EmbodimentProvider
       |
       v
External Kuavo Bridge
       |
       v
ROS Noetic + MuJoCo / HIL / Real Robot Runtime
       |
       v
Observations + Progress + EvidenceRefs
       |
       v
Executive settlement + Episode report + Mnemosyne promotion
```

### 3.1 三个端口必须分开

#### Policy Provider

输入语言目标和观测，输出候选 `SkillProposal`。它没有执行方法。

```rust
trait PolicyProviderPort {
    async fn propose(...) -> Result<Vec<SkillProposal>, String>;
}
```

实现可以是 LLM、VLA 或规则策略。

#### Embodiment Provider

把稳定 Aletheon contract 映射到 ROS/MuJoCo/机器人 SDK。它不理解 Prompt 或 Goal。

```rust
trait EmbodimentProvider {
    async fn observe(...);
    async fn list_skills(...);
    async fn execute_skill(...);
    async fn cancel(...);
    async fn safe_stop(...);
}
```

#### Robot Runtime

公司已有控制系统，拥有实时状态估计、轨迹、MPC/WBC/RL、安全监督和硬件接口。它运行在
Aletheon 进程之外，Aletheon 只通过 bridge 调用语义 skill。

---

## 4. 通用 Contract 如何完善

### 4.1 保留现有类型

继续使用：

- `DeviceId`；
- `SkillId`；
- `SkillDescriptor`；
- `SkillRequest`；
- `SkillProgress`；
- `SkillResult`；
- `EmbodiedObservation`；
- `EvidenceRef`。

不要在 Fabric 引入 ROS message、URDF、MuJoCo 或 Kuavo 专有类型。

### 4.2 SkillDescriptor 需要的增量

现有字段已经有 schema、risk、timeout、preconditions 和 success criteria。下一步建议增加版本
和环境约束，但应通过向后兼容版本演进完成：

```rust
pub struct SkillDescriptorV2 {
    pub skill: SkillId,
    pub schema_version: u16,
    pub provider_version: String,
    pub parameters_schema: Value,
    pub expected_outcome_schema: Value,
    pub supported_namespaces: Vec<ExecutionNamespace>,
    pub risk: RiskClass,
    pub timeout_ms: u64,
    pub cancellable: bool,
    pub preconditions: Vec<TypedPrecondition>,
    pub limits_digest: String,
}
```

优先实现版本和 digest；不要一开始加入过度复杂的通用运动学类型。

### 4.3 Observation schema

Kuavo 第一批标准 observation：

| schema | 关键 payload |
|---|---|
| `robot.state/v1` | mode、control_state、faults、estop |
| `robot.base/v1` | pose、twist、height、orientation |
| `robot.joints/v1` | position、velocity、effort、tracking_error |
| `robot.stability/v1` | contact、zmp/support、fall_detected |
| `simulation.state/v1` | sim_time、paused、scene、reset_generation |
| `task.progress/v1` | phase、fraction、controller status |

每条 observation 必须有：

- 单调 sequence；
- source_time 和 received_at；
- valid_until；
- frame_ref；
- source/provider identity；
- evidence refs。

### 4.4 SkillResult 不能单独代表成功

`SkillResult::Succeeded` 只表示 provider 完成命令。Goal 成功还需要：

```text
terminal SkillResult
+ post-action observation
+ ExpectedOutcome matched
+ stable window satisfied
+ evidence persisted
+ Executive settlement succeeded
```

---

## 5. Kuavo MuJoCo Bridge 计划

### 5.1 仓库与部署位置

桥接器可保持独立仓库/进程，例如现有配置注释中的 `aletheon-kuavo-bridge`，因为它依赖公司
ROS Noetic workspace 和 Python/ROS 生态。Aletheon 仓库只保留：

- proto/contract 兼容测试；
- 示例配置；
- bridge manifest 和版本要求；
-端到端 fixture；
-部署与验收说明。

### 5.2 Bridge 内部结构

```text
gRPC server
  -> Contract validator
  -> Device/skill registry
  -> ROS adapter
       -> topic subscribers
       -> service/action clients
       -> rosbag/artifact recorder
  -> Safety supervisor
  -> Progress/observation publisher
```

### 5.3 第一批 skill（按 bridge 真实 manifest）

**已核对** `aletheon-kuavo-bridge/config/skills/kuavo_mujoco.yaml`，bridge 实际暴露以下三个 skill
（与计划初稿的命名不同，以真实 manifest 为准）：

| skill_id | risk | timeout | cancellable | success criteria |
|---|---|---|---|---|
| `kuavo.stance` | low | 10s | false | 稳定站立窗口 + fresh observation 确认 |
| `kuavo.move_base_timed` | medium | 5s | true | 有界 base 运动（≤0.2 m/s、≤0.3 rad/s、≤2s），结束后回到 stance |
| `kuavo.stop` | low | 5s | false | 发布零速度，状态确认回到稳定站立 |

RobotHarness 当前硬编码的 `kuavo.stance`（`cognit/src/harness/robot/mod.rs:202,220`）恰好与
bridge 真实 skill 一致，无需改名。

不要第一版开放：

- 任意 shell；
- 任意 ROS topic publish；
- 任意 joint torque；
- 任意控制器参数写入；
- 原始 CAN/EtherCAT 帧。

### 5.4 ROS 映射原则

Bridge 私有代码负责把 skill 映射到具体 topic/service/action。Aletheon 只看语义 contract。
已核对的真实映射（`kuavo-ros-control` 文档可查）：

```text
kuavo.stance
 -> validate mode and parameters
 -> 切换/确认 gait（bridge 读 /humanoid_mpc_gait_time_name）
 -> stream progress
 -> collect post-state（Odometry + gait 缓存）
 -> return terminal result + evidence refs
kuavo.move_base_timed
 -> 有界 Twist 发布（≤0.2 m/s, ≤0.3 rad/s, ≤2s）
 -> 结束发布零速度 -> state 确认回到 stance
kuavo.stop
 -> 发布零速度 / 统一停止 -> state 确认稳定站立
```

ROS 名称、msg 类型和机器人版本放在 bridge manifest/config，不进入 Fabric。

---

## 6. RobotHarness 产品化计划

### 6.1 修复当前硬编码

当前 `RobotHarness` 在 Execute/Verify 中构造固定的 `mode == stance` ExpectedOutcome，并使用
`"op"` 作为 episode operation ID。这必须先修复：

- `SkillProposal.expected_outcome` 随 proposal 保存到 harness state；
- Execute 使用真实 host-issued operation ID；
- EpisodeSink 接收真实 attempt/operation 关联；
- Verify 使用 proposal 的 expected outcome；
- Retry/Replan 保留每次 attempt 历史；
- unsafe/unknown 按配置进入 SafeStop 或失败，不静默成功。

建议状态增加：

```rust
pub latest_proposal: Option<SkillProposal>,
pub expected_outcome: Option<ExpectedOutcome>,
pub operation_id: Option<OperationId>,
pub before_sequence: Option<u64>,
pub evidence: Vec<EvidenceRef>,
```

### 6.2 Production composition

在 Executive composition root 构造，而不是 Cognit 自己寻找实现：

```text
EmbodimentExecutionPort
+ EmbodimentWorldState
+ DeterministicOutcomeVerifier
+ PolicyProviderPort
+ EpisodeSink
+ allowed SkillDescriptor snapshot
 -> RobotHarness
```

`HarnessKind::Robot` 配置选择失败时必须 fail closed，不能回退 Linear 后仍显示 robot active。

### 6.3 两条入口的关系

短期保留两条入口，但定义清楚：

1. `robot-agent` + linear harness：用于人工明确指定 skill 的诊断/运维；
2. RobotHarness：用于自然语言 Goal、自动 plan/verify/retry 的受控任务。

两条入口最终必须共用同一 `EmbodimentExecutionPort`、Kernel admission 和 receipt，不允许出现
第二条直接 provider 调用路径。

---

## 7. 自动验证设计

### 7.1 第一版确定性 verifier

使用现有 `ExpectedOutcome`：

```rust
ExpectedOutcome {
    predicate: All([
        Equals("mode", "stance"),
        Equals("fall_detected", false),
        Range("base.height_m", 0.75, 0.95),
        Range("joint_error.max_rad", 0.0, 0.08),
    ]),
    freshness_ms: 200,
    stable_window_ms: 3_000,
    timeout_ms: 10_000,
}
```

具体阈值必须来自 Kuavo 版本化配置/HIL evidence，不能写死在通用 Fabric 或由模型自由生成。

### 7.2 验证过程

```text
capture before snapshot
 -> execute skill
 -> wait for sequence > before_sequence
 -> reject stale/missing observation
 -> evaluate predicate
 -> require stable window
 -> classify mismatch
 -> emit VerificationReport
```

### 7.3 mismatch 分类

| 情况 | Decision |
|---|---|
| 短暂未稳定且预算充足 | RetryableMismatch |
| 参数/skill 不适合当前状态 | ReplannableMismatch |
| 跌倒、急停、越界、失联 | Unsafe |
| observation 缺失或过期 | Unknown |
| 所有条件满足稳定窗口 | Matched |

---

## 8. 可追踪报告与 Episode

### 8.1 完成报告必须包含

```text
Goal ID / Episode ID
Device ID + serial/model
Simulation scene/version
Aletheon commit and runtime digest
Bridge version/protocol digest
Robot software/URDF/controller version
Policy/VLA provenance
Selected skill and bounded parameters
Admission permit and lease reference
Before/after observation sequence
Every attempt and retry/replan reason
Verification report
rosbag/log/plot artifact refs
Final settlement status
```

### 8.2 存储边界

- Executive：Goal/Attempt/terminal settlement authority；
- Session/Event spine：交互和 capability evidence；
- Episode store：机器人任务 attempt/verification 记录；
- Artifact store：rosbag、日志、图表、截图等大文件；
- Mnemosyne：从已验证 episode 提炼的长期经验；
- Agora：当前未完成机器人任务的工作状态。

不要把完整 rosbag 放进模型 Context 或 Mnemosyne 文本记录，只保存 artifact 引用和摘要。

---

## 9. VLA 接入路线

### Phase V0：无 VLA

先使用明确 skill 和规则/LLM planner，走通仿真闭环。这能验证所有非 VLA 基础设施。

### Phase V1：VLA 只选择语义 skill

VLA 输入语言、图像和状态，输出：

```text
SkillProposal {
  skill,
  bounded parameters,
  expected outcome,
  confidence,
  provenance
}
```

proposal validator 必须校验：

- device 一致；
- skill 已注册；
- JSON schema；
- confidence 范围；
- frame/observation 新鲜度；
- expected outcome 合法；
-风险和时限不超过 descriptor。

### Phase V2：VLA 输出 waypoint/end-effector target

只有在 Robot Runtime 提供独立安全校验器后支持。输出仍转换为注册 skill，不暴露原始执行端口。

### Phase V3：trajectory proposal

要求：

-仿真验证；
- joint/velocity/acceleration/effort limits；
- collision/stability 检查；
- trajectory digest；
- local watchdog；
- HIL evidence；
-人工 arming。

不规划 VLA 直接输出电机控制帧。

---

## 10. 分阶段实施任务

> 状态标记（2026-08-03）：✅ 已完成 ｜ 🟡 大部分完成 ｜ 🔴 待办

### Phase 0：冻结 contract 与真实状态审计 —— ✅ 基本完成

交付：

- 本计划作为基线；
- 列出真实 Kuavo ROS skill、topic/service/action 和验证信号（bridge 的
  `config/skills/kuavo_mujoco.yaml` + `kuavo-ros-control/docs/readme_topics.md`）；
- 确定 bridge 仓库、owner、协议版本（`aletheon-kuavo-bridge`，proto 与 Aletheon 逐字节一致）；
- 记录仿真启动命令、镜像、URDF 和控制器版本（bridge `scripts/bootstrap.sh`）。

完成条件：每个 skill 都有输入 schema、precondition、timeout、cancel、safe-stop 和 success
criteria；没有“后面再决定”的自由文本关键字段。

### Phase 1：Simulator 主链验收 —— 🔴 待办

修改现有 deterministic simulator（`crates/hardware/src/simulator.rs`，`SimulatedEmbodiment`），
使其不仅返回 terminal result，还能生成 before/after observation 并满足稳定验证。

测试：

- 自然语言选择 robot-agent；
- list -> observe -> execute -> observe；
- lease 缺失、过期和冲突 fail closed；
- stale observation 不成功；
- disconnect/timeout 触发 safe-stop；
- terminal receipt 可重放且不重复执行。

### Phase 2：Kuavo MuJoCo Bridge MVP —— 🟡 bridge 侧已过验收，Aletheon 侧 E2E 待办

bridge 侧已完成：handshake/health、list skills、observe/get state、stance、execute progress、
cancel/safe-stop，并已通过 live MuJoCo Phase E 验收。Aletheon 侧契约测试
（`crates/hardware/tests/grpc_contract.rs`）已锁死 proto。

剩余：跨仓 E2E（对应 PR8）——Aletheon 的 `GrpcEmbodimentProvider` 连真实 bridge 跑
`kuavo.stance` 稳定验证。

### Phase 3：RobotHarness production wiring —— 🔴 主战场（PR1–6）

- 修复 expected outcome/operation 占位（PR1/PR2）；
- 接入 world-state observation pump（PR3）；
- 接入 deterministic verifier（PR4）；
- 接入 durable EpisodeSink（PR5）；
- 把 `NoopEmbodimentProgress` 替换为 canonical event projection（PR6）；
- 增加显式 RobotHarness 配置与 fail-closed startup（PR4 composition）。

完成条件：自然语言 Goal 不依赖模型手动再次调用 observe，也能由 harness 自动验证和结算。

### Phase 4：报告与记忆 —— 🔴 待办（PR10）

- 保存 episode report；
- 关联 rosbag/log/plot；
- 报告展示每次 attempt；
- 只有 Matched 且 settled 的 episode 才能进入 Mnemosyne promotion；
- 失败 episode 保留故障证据，但不提炼为“成功经验”。

### Phase 5：真实 VLA Policy Provider —— 🔴 待办（PR9）

- 实现真实 gRPC `PolicyProviderPort` client（当前 `grpc_provider.rs` 只有 `StubPolicyProvider`）；
- gRPC policy protocol 契约测试；
- 模型/权重/version/digest provenance；
- 视觉 frame refs；
- proposal schema 和 budget；
- 离线 fixture 回放。

### Phase 6：Simulation -> HIL -> Real

严格升级：

```text
deterministic simulator
 -> Kuavo MuJoCo
 -> recorded observation replay
 -> HIL
 -> low-speed real robot with local operator
```

Production gate 必须继续 fail closed，不能连接失败后静默切 simulator。

---

## 11. 具体实施计划（PR1–PR10 完整设计）

> 每个 PR 一个边界；文件/行号基于 2026-08-03 代码，实施前重新 grep 确认。
> 构建统一走 `just` / `scripts/cargo-agent.sh`，不用裸 `cargo`。
> 关键代码事实（已核对）：
> - `SkillProposal.expected_outcome` 是**必填字段**（`fabric/src/types/skill_proposal.rs:35`），
>   且 `SkillProposal::validate()` 已校验（depth/NaN/confidence/frame_refs/provenance digest）。
> - `WorldStatePort`（`fabric/src/types/world_state.rs:25-38`）是**只读**：`latest()` +
>   `observe_until()`，无写入方法。
> - `ExpectedOutcome`/`OutcomePredicate`（`fabric/src/types/expected_outcome.rs`）只有 `validate()`，
>   **没有 predicate 求值函数**——PR4 必须新写。
> - `SkillResult.operation_id: OperationId`（`fabric/src/types/embodiment.rs:84`）已存在，
>   `OperationId` 有 UUID 解析器（`embodiment.rs:164-167`）。
> - `HarnessKind::Robot` 在 `build_harness` 必然返回 `Err(RequiresExecutivePorts)`
>   （`cognit/src/harness/mod.rs:88`）；`production_cognitive_session_factory`
>   （`executive/src/application/harness_factory.rs:109-124`）无论 `harness_kind` 都建 Linear session，
>   只打日志。
> - policy 侧 `crates/cognit/src/adapters/policy/grpc_provider.rs` **只有 `StubPolicyProvider`**，
>   没有真实 gRPC client；Executive 未接线任何 policy provider。
> - `FabricEventSink::emit`（`executive/src/application/turn_engine.rs:127-128`）目前是 no-op。

### PR1 `robot-harness-carries-proposal-outcome`

**目标**：RobotHarness 使用 proposal 携带的 `expected_outcome`，删除 Execute/Verify 两处硬编码。

**现状与证据**：
- Plan 态 `mod.rs:147-190`：`validate_proposal(...).is_ok()` 通过后只构造 `SkillRequest{skill, device, parameters}`，
  丢弃 `proposal.expected_outcome`。
- Execute 态 `mod.rs:210-231`：`append_attempt` 硬编码 `Equals("mode","stance")`（`mod.rs:216-225`）。
- Verify 态 `mod.rs:241-289`：`verifier.verify` 硬编码同款（`mod.rs:244-252`）。

**设计**：
1. `crates/cognit/src/harness/robot/state.rs`：`RobotHarnessState` 增加
   `latest_expected_outcome: Option<ExpectedOutcome>`；`init` 置 `None`。
   `RobotHarnessConfig` 增加 `default_expected_outcome: Option<ExpectedOutcome>`（`Default = None`）。
2. `crates/cognit/src/harness/robot/mod.rs`：
   - **Plan 态**：`validate_proposal(proposal, ...).is_ok()` 分支内，先
     `harness_state.latest_expected_outcome = Some(proposal.expected_outcome.clone())`，再构造 `SkillRequest`。
   - 新增私有 helper（供 Execute/Verify 共用）：
     ```rust
     fn resolve_expected(&self, s: &RobotHarnessState) -> Result<ExpectedOutcome, String> {
         s.latest_expected_outcome
             .clone()
             .or(self.config.default_expected_outcome.clone())
             .ok_or_else(|| "no expected outcome available".into())
     }
     ```
   - **Execute 态**：`append_attempt` 的 expected 参数改用 `resolve_expected(&harness_state)`；
     `Err` 时进入 `RobotState::Failed`（fail closed，不静默硬编码）。
   - **Verify 态**：`verifier.verify(&expected, ...)` 同样用 `resolve_expected`。
3. 移除硬编码的 `mode==stance` 构造，并删除 `mod.rs:202-205` 的 fallback `SkillRequest`。Execute
   没有经过校验的 `latest_skill_request` 时直接进入 `RobotState::Failed`；不得凭默认
   `kuavo.stance` 绕过 Plan、manifest 和 proposal 校验。PR2 只负责 operation identity，不负责修复
   request fallback。

**改动文件**：`crates/cognit/src/harness/robot/state.rs`、`crates/cognit/src/harness/robot/mod.rs`。

**测试**：新增 unit test——fake `PolicyProviderPort` 返回 `Equals("mode","stance2")` 的 proposal；
fake `OutcomeVerifierPort` 记录收到的 expected；断言 Verify 收到 `stance2` 而非 `stance`。同时更新
`crates/executive/tests/robot_policy_path.rs` 现有断言（其 fake policy 已带 stance outcome，行为应不变）。

**关闭判定**：注入非 stance outcome 时 fake verifier 断言收到 proposal 的值；没有有效 proposal 时
executor 调用次数为 0。

**依赖**：无。

---

### PR2 `robot-harness-real-operation-identity`

**目标**：`"op"` 字面量换成 host 签发的真实 `OperationId`。

**现状与证据**：Execute 态 `mod.rs:215` 传 `"op"` 给 `append_attempt`。`SkillResult.operation_id:
OperationId` 已存在（`embodiment.rs:84`），无需改契约。

**设计**：
1. `crates/cognit/src/harness/robot/mod.rs` **Execute 态**：
   - `Ok(result)` 分支：`let op_id = result.operation_id.clone();`
     把 host/provider 返回的 typed `OperationId` 传给 episode sink；不得先降级为无类型字符串再由
     报告层猜测恢复。
   - `Err(e)` 分支：记录 attempt/invocation failure，但 `operation_id = None`。执行端没有返回权威
     operation identity 时不得本地生成一个 ID 冒充已创建的机器人 operation。
2. `crates/cognit/src/harness/robot/state.rs`：`RobotHarnessState` 增加
   `latest_operation_id: Option<OperationId>`；Execute 成功时 `= Some(result.operation_id.clone())`，
   失败且无 terminal receipt 时保持 `None`。
3. 把 `EpisodeSink::append_attempt` 的 operation 参数改为 `Option<&OperationId>`，并增加独立、必填的
   typed `attempt_id` 或 `invocation_id`。attempt identity 表示“发起过一次尝试”，operation identity
   只表示 host/provider 已确认创建的操作，两者不得复用。
4. 如果错误携带 authoritative terminal receipt 或 durable receipt，其中的 operation ID 可以写入；
   仅有字符串错误时不能推断 operation 已存在。

**测试**：fake executor 返回 `SkillResult{ operation_id: OperationId::new(), .. }`，断言 EpisodeSink
收到同一个 typed ID；executor 在创建 operation 前失败时断言 `operation_id == None` 且 attempt ID
存在；携带 terminal receipt 的失败保留 receipt 中的 operation ID；断言 `"op"` 和本地伪造 ID
不再出现。

**关闭判定**：成功或有权威 receipt 的 attempt 使用 host/provider 返回的 typed operation ID；未创建
operation 的失败 attempt 明确为 `None`，但仍有独立 typed attempt/invocation identity。

**依赖**：无（可与 PR1 同批）。

---

### PR3 `embodiment-observation-to-world-state-pump` —— ✅ 核心已实现

**目标**：持续 observation → 可查询的 `WorldStatePort`，供 Verify 做 `observe_until` 等待新快照。

**现状与证据**：`WorldStatePort` 只读（`world_state.rs:25-38`）。RobotHarness 只在 Observe 态调一次
`world_state.latest()`，Execute 后无新快照来源。**pump 不能"写入"只读端口**——正确设计是实现一个
轮询型 `WorldStatePort` 实现。

**设计**（新建 `crates/executive/src/application/embodiment_world_state.rs`）：
1. 结构：
   ```rust
   pub struct PollingWorldState {
       // 每 device 一个 watch 通道，存最新 WorldSnapshot
       latest: Arc<dashmap::DashMap<DeviceId, WorldSnapshot>>,  // 或 tokio::sync::watch 每 device
       poll_interval: Duration,
       executor: Arc<dyn fabric::EmbodimentExecutionPort>,  // 用 fabric 端口（带 SkillDispatchError）
   }
   ```
2. `PollingWorldState::spawn(executor, interval) -> Arc<Self>`：后台 task 周期调
   `executor.observe(device)`（device 集合来自 config/registry），对每个 observation：
   - 丢弃 `stale == true` 或 `observed_at` 超过 `valid_until` 的样本；
   - 丢弃 `sequence <= 已存 sequence` 的样本（单调约束）；
   - 更新缓存。
3. 实现 `WorldStatePort`：
   - `latest(device)` → 返回缓存最新快照。
   - `observe_until(device, after_sequence, deadline)` → 轮询缓存直到 `sequence > after_sequence`，
     或到 deadline 返回 `None`。这正是 `DeterministicOutcomeVerifier` 需要的等待语义。
4. 依赖注入：pump 在 Executive bootstrap 构造，作为 `Arc<dyn WorldStatePort>` 传给 RobotHarness
   composition（PR4 落点）。不使用 cognit 侧的 `EmbodiedExecutionPort`（那是 String error 的教学端口）。

**测试**：unit test——pump 连续采样 sequence 递增；插入 stale/过期样本被丢弃；`observe_until`
在超限后返回、未超限前阻塞到 deadline 返回 None。

**关闭判定**：连续采样递增 sequence；stale 丢弃。

**依赖**：PR1/PR2。

---

### PR4 `deterministic-outcome-verifier-wiring` + 生产 composition

**目标**：确定性 `OutcomeVerifierPort` 实现 + `HarnessKind::Robot` 的生产构造与 fail-closed 选择。

**现状与证据**：
- `OutcomeVerifierPort`（`mod.rs:27-35`）**无任何实现**。
- predicate 求值函数**不存在**（`expected_outcome.rs` 只有 `validate()`）。
- fabric 的 `Verifier`（`fabric/src/policy/verifier.rs`）是 LLM 最终文本验证器，**与机器人无关**，不复用。
- `HarnessKind::Robot` 必然 `Err(RequiresExecutivePorts)`（`harness/mod.rs:88`）；
  `production_cognitive_session_factory` 不分支（`harness_factory.rs:109-124`）。

**设计**：
1. **Predicate 求值器**（fabric，新模块 `crates/fabric/src/types/outcome_evaluation.rs`）：
   ```rust
   pub struct EvaluationInput<'a> {
       pub payload: &'a serde_json::Value,
       pub observed_at: MonoTime,
       pub now: MonoTime,
       pub sequence: u64,
   }
   pub enum EvaluationResult { Match, Mismatch { reason: String }, Stale, Unknown { reason: String } }

   pub fn evaluate(predicate: &OutcomePredicate, input: &EvaluationInput) -> EvaluationResult;
   ```
   - dot-path 遍历 `payload`（`a.b.c`），实现 Equals / NotEquals / Range / Change / All / Any；
     数值路径自动做 numeric 比较；`All`/`Any` 递归且受 `MAX_PREDICATE_DEPTH=8` 约束。
   - `Change` 需要 `before` payload 对比——签名里带 `before: Option<&Value>`。
   - 校验 `now - observed_at <= freshness_ms`，否则 `Stale`。
2. **DeterministicOutcomeVerifier**（executive，`crates/executive/src/application/deterministic_outcome_verifier.rs`）：
   ```rust
   pub struct DeterministicOutcomeVerifier {
       world: Arc<dyn WorldStatePort>,
       clock: Arc<dyn MonotonicClock>,
       unsafe_predicates: Vec<OutcomePredicate>,  // 命中即 Unsafe（fall_detected/estop 等，领域输入配置）
   }
   #[async_trait]
   impl OutcomeVerifierPort for DeterministicOutcomeVerifier {
       async fn verify(&self, expected, before, after, attempt) -> VerificationReport {
           // 1) 等待 after_sequence 后的新快照：world.observe_until(device, after.seq, deadline)
           //    拿不到 -> Unknown
           // 2) 先测 unsafe_predicates，任一命中 -> Unsafe
           // 3) 测 expected.predicate；连续 stable_window_ms 内 Match -> Matched
           // 4) 超时 -> Retryable（attempt 低）或 Replannable
           // 填 VerificationReport{ decision, evaluated_sequence, observed_paths, reasons, evidence }
       }
   }
   ```
   这是 PR4 的核心逻辑（stable window 判定参照 `ExpectedOutcome.stable_window_ms / freshness_ms / timeout_ms`）。
3. **Composition（harness_factory.rs + request.rs）**：
   - `production_cognitive_session_factory` 增加 `HarnessKind::Robot` 分支：不返回
     `LinearCognitiveSessionFactory`，而是返回一个会构造 RobotHarness 的 factory。
   - 具体：新增 `crates/executive/src/application/robot_harness_composition.rs`，提供
     `build_robot_harness(&ExecutiveConfig, embodiment_port, world, verifier, episodes, policy) -> Result<RobotHarness>`。
     需要的端口全部在 request.rs bootstrap 构造（embodiment_port 已有；world/pump 见 PR3；
     verifier 见本 PR；episodes 见 PR5；policy 见 PR9）。
   - **CognitiveSession 适配**：`RobotHarness` 是 `step()` 状态机，不是 `cognit::harness::CognitiveSession`。
     新建 adapter `RobotCognitiveSession`（`crates/cognit/src/harness/robot/session.rs`）实现
     `CognitiveSession`，内部驱动 `init` → 循环 `step` 直到 `is_terminal()`。
   - **composition 与 activation 分离**：本 PR 构造可注入 policy port 的 factory；只有 PR3 world、
     PR5 episode、PR6 progress 和 PR9 production policy 均已接入且健康时，配置才能把
     `HarnessKind::Robot` 标记为 active。单元测试可注入 fake，但 fake/stub 不能满足安装态 activation。
   - **fail closed**：`HarnessKind::Robot` 但所需端口未配置或 policy 仅为 stub → 构造返回
     `Err`，daemon 启动失败；不得回落 Linear 后仍宣称 robot active（对齐
     `production_embodiment_gate.rs::never_downgrade_to_simulator` 精神）。

**测试**：
- predicate 求值器 unit：Equals/Range/All/嵌套/Change/深度超限报错/数值比较。
- verifier 集成：simulator 产生 before/after，stable window 满足 → Matched；`fall_detected` → Unsafe；
  超时 → RetryableMismatch；observation 缺失 → Unknown。
- composition：`config.harness_kind = Robot` + 端口齐全 → 构造成功；端口缺失 → 启动失败。
- 扩展 `crates/executive/tests/hardware_simulation.rs`。

**关闭判定**：daemon 启动即构造 RobotHarness；Robot 配置失败 fail closed。

**依赖**：PR3、PR5、PR6、PR9。predicate evaluator 和 verifier 本身可以提前实现，但 production
activation 必须等待这些端口全部完成。

---

### PR5 `durable-robot-episode-sink`

**目标**：SQLite `EpisodeSink`，attempt/verification 持久化，重启可重放不重复执行。

**现状与证据**：`EpisodeSink`（`mod.rs:57-70`）无实现。SQLite 模式参照
`crates/executive/src/adapters/agent_control/sqlite_repository.rs`（rusqlite + `Arc<Mutex<Connection>>`
+ `include_str!` 迁移 + sha2 digest）。

**设计**（新建 `crates/executive/src/adapters/episode/`）：
1. `sqlite_episode_sink.rs`：
   ```rust
   pub struct SqliteEpisodeSink { connection: Arc<Mutex<rusqlite::Connection>> }
   impl SqliteEpisodeSink {
       pub fn open(path: impl AsRef<Path>) -> Result<Self, ...>;
   }
   ```
2. migration `001_episodes.sql`：
   ```sql
   CREATE TABLE episodes (
     episode_id       TEXT NOT NULL,
     attempt_id       TEXT NOT NULL,
     attempt          INTEGER NOT NULL,
     operation_id     TEXT,
     request_digest   TEXT NOT NULL,
     status           TEXT NOT NULL,          -- running | settled | failed
     expected_json    TEXT NOT NULL,
     before_json      TEXT,
     after_json       TEXT,
     result_json      TEXT,
     verification_json TEXT,
     created_at_ms    INTEGER NOT NULL,
     settled_at_ms    INTEGER,
     PRIMARY KEY (episode_id, attempt_id)
   );
   CREATE UNIQUE INDEX episodes_digest ON episodes(episode_id, request_digest);
   CREATE UNIQUE INDEX episodes_operation
     ON episodes(operation_id) WHERE operation_id IS NOT NULL;
   ```
3. 实现 `EpisodeSink`：
   - `append_attempt`：`request_digest = sha256(episode_id | attempt_id | canonical request | expected_json)`；
     operation ID 不参与请求幂等键，因为执行前它可能尚不存在；`INSERT OR IGNORE` 保证同一 durable
     attempt 重放不重复写。
   - operation 创建后以 compare-and-set 方式把 authoritative `operation_id` 关联到原 attempt；冲突
     或同一 operation 关联不同 attempt 时 fail closed，不覆盖旧记录。
   - `close_episode(episode_id, outcome)`：把该 episode 全部 attempt 的 `status` 置为 settled/failed，
     写 `settled_at_ms`。

**测试**：重启恢复——同 attempt/request digest 的 settled attempt 不重复写；执行前失败允许
`operation_id = NULL`；operation 创建后的 compare-and-set 可恢复；未结算 attempt 状态可查询。

**关闭判定**：E2E 报告能读到每次 attempt 的持久化记录。

**依赖**：PR2（真实 operation_id 关联）。

---

### PR6 `canonical-embodiment-progress-events`

**目标**：`NoopEmbodimentProgress` → 把 `SkillProgress` 发布到 session/event 投影。

**现状与证据**：`EmbodimentProgressPort::record(SkillProgress)`（`embodiment_progress.rs:9-11`）；
`NoopEmbodimentProgress`（`embodiment_progress.rs:82-87`）在 `request.rs:805` 注入 `build_embodiment_port`。
`FabricEventSink::emit` 目前 no-op（`turn_engine.rs:127-128`）。`BoundedProgressSink` 已有
（限频 + 注入 operation_id，`embodiment_progress.rs:24-54`）。

**设计**：
1. `crates/executive/src/application/embodiment_progress.rs` 增加 `EventEmbodimentProgress`：
   ```rust
   pub struct EventEmbodimentProgress {
       sink: Arc<dyn fabric::TurnEventSink>,
   }
   impl EmbodimentProgressPort for EventEmbodimentProgress {
       async fn record(&self, progress: SkillProgress) {
           self.sink.emit(fabric::TurnEvent::EmbodimentProgress { ... }).await;
       }
   }
   ```
2. `fabric::TurnEvent` 增加 `EmbodimentProgress { operation_id, skill, fraction, note, at }` 变体；
   session/event 投影消费该事件 → UI/log 可见。
3. `turn_engine.rs:127-128`：`FabricEventSink::emit` 从 no-op 改为把事件写入 session event spine
   （确认 daemon 的真实事件出口并接入；若 `FabricEventSink` 就是最终 sink，则在它内部接上投影）。
4. `request.rs:805`：把 `NoopEmbodimentProgress` 换成
   `Arc::new(EventEmbodimentProgress { sink: session_event_sink })`。
   生产配置显式选择；无 sink 时 fail closed，不允许静默 Noop。

**测试**：daemon 级测试——execute 过程中 session log 出现带 operation_id 的
`EmbodimentProgress` 事件。

**关闭判定**：session log 出现带真实 operation_id 的进度事件。

**依赖**：PR2。

---

### PR7 `kuavo-bridge-contract-fixtures`（维护 + 跨仓 fixture）

**现状**：**主体已完成**——proto 逐字节一致（两边各 209 行），`crates/hardware/tests/grpc_contract.rs`
用 SHA-256 锁死（`EXPECTED_PROTO_HASH=4a205a75...`），bridge 侧 `tests/contract/` 有 fake server 契约测试。

**设计**：
1. 新增 `crates/hardware/tests/grpc_cross_repo.rs`（`#[ignore]`，需 bridge 环境）：
   - 拉起 bridge `tests/contract/test_grpc_service.py` 的 gateway（或真实 bridge）；
   - `GrpcEmbodimentProvider::connect` → GetCapabilities 握手 → ListSkills → Snapshot；
   - 断言 provider_id、device_ids、skill 清单与 bridge manifest 一致。
2. `grpc_contract.rs` 注释固化维护约定：bridge proto 变更 → 同步 Aletheon 拷贝 → 更新 hash。

**测试**：`just test` 中 `grpc_contract` 保持绿；跨仓 fixture 手动跑通。

**依赖**：无。

---

### PR8 `kuavo-mujoco-stance-e2e`

**目标**：Aletheon daemon → `GrpcEmbodimentProvider` → 真实 bridge → MuJoCo → `kuavo.stance` → 稳定验证。

**现状与证据**：接线已存在（`runtime_embodiment_selection.rs` 证明 `EmbodimentProviderConfig::Grpc`
配置可达 daemon）；`ProductionStartupGate` fail-closed（`production_embodiment_gate.rs`）；
bridge 已过 live MuJoCo Phase E 验收；simulator 目前是 mobile_robot（非 biped）。

**设计**：
1. **Simulator 升级（Phase 1）**：`crates/hardware/src/simulator.rs` 的 `SimulatedEmbodiment`
   增加 biped stance 场景——observation payload 含 `mode`、`base.height_m`、
   `joint_error.max_rad`、`fall_detected`，execute `kuavo.stance` 后进入稳定站立窗口
   （sequence 递增 + 持续 stable_window），使无 bridge 时 CI 也能跑完整 harness 主链。
2. 新增 gated 集成测试 `crates/executive/tests/kuavo_stance_e2e.rs`（`#[ignore]`）：
   配 `EmbodimentProviderConfig::Grpc { endpoint: "http://127.0.0.1:50051", device_id: "kuavo-mujoco-01" }`，
   组装生产 composition → 驱动 RobotHarness（Plan `kuavo.stance` → Execute → Verify stable 3s）→
   断言 §12.1 前 9 项以及 provisional evidence bundle；第 10 项报告引用由 PR10 关闭。
3. `scripts/kuavo-e2e.sh`：bootstrap bridge → 起 daemon → 跑测试 → 清理 + rosbag 收集。

**测试**：除最终报告展示外的主链观察全部通过，并产出 PR10 可消费的 provisional episode/evidence
bundle；provider/bridge 不可用、仅 mock 成功、observation 过期、lease 冲突、fall_detected、
provisional evidence 缺失任一 → 失败。§12.1 的“report 能打开证据引用”只在 PR10 后关闭。

**依赖**：PR1–7、PR9（harness、跨仓契约和 production policy 正确后才运行真实 bridge E2E）。

---

### PR9 `policy-provider-grpc-protocol`

**目标**：真实 gRPC `PolicyProviderPort` client，替换 `StubPolicyProvider`；Executive 接线。

**现状与证据**：`grpc_provider.rs` **只有 `StubPolicyProvider`**（`grpc_provider.rs:53-105`，`propose`
返回硬编码 stance proposal），**无真实 client**；Executive 未接线任何 policy provider。
`PolicyGateway` proto 已定义（`crates/cognit/proto/aletheon/policy/gateway/v1/policy.proto`）：
GetCapabilities / Propose / Health；`ProposeRequest` 带 `frame_uris/labels/summary/confidence` +
`allowed_skill_ids`。

**设计**：
1. `crates/cognit/src/adapters/policy/grpc_provider.rs` 新增真实 client：
   ```rust
   pub struct GrpcPolicyProvider {
       client: policy_gateway_client::PolicyGatewayClient<tonic::transport::Channel>,
       config: GrpcPolicyConfig,
   }
   ```
   - 复用 `GrpcPolicyConfig` + `validate_policy_endpoint`（非 loopback 强制 TLS）。
   - `propose`：`snapshots` → `frame_summary`（最新 payload 摘要）、`visual` → `frame_uris/labels`、
     `confidence` 取均值、`allowed_skills` → `allowed_skill_ids`；调 `Propose`；
     `SkillProposalWire` → `SkillProposal`（`expected_outcome` 的 Struct 反序列化为 `ExpectedOutcome`）。
   - `health` → `Health` RPC。
2. 保留 `StubPolicyProvider` 仅用于 unit/integration test 和显式隔离的 simulator fixture。安装态
   RobotHarness、Kuavo MuJoCo、HIL 和实机配置没有有效 production policy endpoint 时必须启动或
   admission 失败；不得以 Stub + warning 降级，也不得把 Stub proposal 写成真实 policy provenance。
3. **Executive 接线**：request.rs 构造 policy provider（读 config），作为 PR4 composition 的一个端口；
   `robot_policy_path.rs` 的"policy 不能直接调执行端口"保证继续成立。

**测试**：fake `PolicyGateway` server 契约测试（对齐 `grpc_contract.rs` 模式，可加 hash 锁 proto）；
propose 往返 conversion 测试（frame 摘要、expected_outcome 序列化）；`StubPolicyProvider` 保留测试。

**关闭判定**：契约测试双向通过；fake VLA 返回的 `SkillProposal` 能被 `proposal_validator` 接受/拒绝；
安装态 Robot 配置缺少 production policy 时 fail closed，且运行证据不出现 `stub-policy`。

**依赖**：无。client 和契约可独立推进；Executive 注入由 PR4 composition 消费。

---

### PR10 `robot-episode-report-and-artifacts`

**目标**：episode 报告 + artifact 引用（rosbag/log/plot），作为端到端完成条件。

**现状与证据**：无 report builder；`EpisodeSink::close_episode(episode_id, "completed"/"failed")`
只写字符串；`EvidenceRef{kind, uri}`（`fabric/src/types/embodiment.rs`）；`VerificationReport` 有
`observed_paths / reasons / evidence`。

**设计**：
1. `crates/executive/src/application/episode_report.rs`：
   ```rust
   pub struct EpisodeReport {
       pub episode_id: String,
       pub goal: String,
       pub device: DeviceId,
       pub sim_scene_version: String,
       pub aletheon_commit: String,
       pub bridge_protocol_digest: String,
       pub attempts: Vec<AttemptRecord>,      // attempt_id/optional op_id/expected/result/verification/retry_reason
       pub permit_ref: Option<PermitRef>,     // 从 capability settlement 拿
       pub lease_ref: Option<LeaseRef>,
       pub before_sequence: Option<u64>,
       pub after_sequence: Option<u64>,
       pub settlement: SettlementStatus,
       pub artifacts: Vec<EvidenceRef>,       // rosbag/log/plot 引用 + 摘要
   }
   pub fn build_report(...) -> EpisodeReport;
   ```
   对齐 §8.1 完成报告清单；`AttemptRecord.retry_reason` 来自 VerificationReport.reasons。
2. artifact store：`EvidenceRef` → 大文件存储（独立目录/对象存储），报告只放 `uri` + 摘要，
   不放 rosbag 进 Mnemosyne 正文。
3. **Mnemosyne promotion 门禁**：只有 `decision == Matched` 且 `settled` 的 episode 才允许提升；
   失败 episode 保留证据但标记 `not_promoted`。

**测试**：E2E 后 report 可打开 evidence 引用；promotion 门禁单测（Matched→promote，Failed→not）。

**关闭判定**：端到端报告能打开 `EvidenceRef` 引用的 artifact。

**依赖**：PR5、PR8。

---

**实施顺序建议**：PR1+2（状态与身份地基）→ PR3+5+6 与 PR9 并行（补齐 production ports）→
PR4（verifier + 统一 composition/activation）→ PR7（跨仓契约）→ PR8（真实 bridge 主链 E2E，产出
provisional evidence）→ PR10（报告、artifact、promotion）→ 重跑 §12.1 完整安装态验收。

**约束**：除非依赖门禁证明现有模块无法表达，否则不新增一级 crate。

---

## 12. 测试与验收矩阵

| 测试层 | 必须证明 |
|---|---|
| Fabric serde/validation | 版本、schema、ID、ExpectedOutcome 可稳定往返 |
| Hardware contract | permit/lease/scope/deadline/replay fail closed |
| Corpus tool | 参数校验、权限级别、错误投影 |
| Bridge fixture | Aletheon 与 Kuavo bridge 协议兼容 |
| RobotHarness | 状态转换、retry/replan/unsafe/safe-stop |
| Simulation E2E | 真实 MuJoCo 状态变化而非 mock 返回成功 |
| Restart recovery | settled 动作不重复；未结算动作进入恢复/停止 |
| Installed runtime | 官方 binary/socket、稳定 systemd、真实模型任务 |

### 12.1 第一条端到端验收场景

输入：

```text
让 kuavo-mujoco-01 进入站立状态，保持 3 秒，验证没有跌倒且关节跟踪误差在限制内，生成报告。
```

必须观察：

1. host 选择已注册设备和 skill；
2. 执行前 observation 非 stale；
3. Kernel 发放 permit 和 exclusive lease；
4. bridge 调用真实 ROS/MuJoCo；
5. operation progress 可见；
6. after sequence 大于 before sequence；
7. stability 条件连续保持 3 秒；
8. verification 为 Matched；
9. capability 与 Goal settlement 成功；
10. report 能打开证据引用。

以下任一情况必须失败：

- provider/bridge unavailable；
-仅 mock 返回 success；
- after observation 缺失或过期；
- lease 冲突；
- operation timeout；
- fall_detected；
-报告缺少真实 evidence；
- monitor PASS 与 session/log/仿真状态矛盾。

---

## 13. 项目负责人需要做的领域输入

这部分不能完全交给 Codex 猜测，需要项目负责人和公司控制栈提供：

1. 第一台目标机器人和版本；
2. MuJoCo 启动方式与容器环境；
3. stance/reset/stop 的真实 ROS 接口；
4. 控制模式切换前置条件；
5. 必须监控的 joint/base/contact/fault 信号；
6. 稳定、跟踪误差和跌倒的验收阈值；
7. timeout/cancel 后正确的安全动作；
8. rosbag 录制范围；
9. 仿真、HIL、实机的不同限制；
10. 可公开与不可公开的公司协议边界。

Codex 可以实现 adapter、测试和报告，但不能从通用知识推断公司机器人的安全阈值。

---

## 14. 最终完成定义

该能力完成不是“代码里存在 RobotHarness/VLA trait/gRPC provider”，而是：

> 在官方安装态 Aletheon 中输入自然语言机器人目标，系统通过唯一受治理主链控制真实 Kuavo
> MuJoCo 仿真，使用新的时序观测完成确定性验证，保存可追踪 episode/receipt/artifact，并在
> provider、观测、lease 或验证失败时 fail closed 且执行正确的取消或安全停止。
