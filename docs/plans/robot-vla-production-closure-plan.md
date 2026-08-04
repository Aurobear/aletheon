# Robot / VLA 生产闭环计划

> 基线：**见 `Aletheon_Unified_Execution_Plan_2026-08-04.md` §0（唯一声明处）**，本计划不自行声明 SHA
> 性质：基于现有 RobotHarness、Embodiment gRPC 和独立 Kuavo bridge 的剩余实施计划
> 状态：`revised-for-owner-approval`；统一计划 B0/B1 与 owner approval 完成后方可调度
> **执行权威**：robot 轨道（R0–R8）独立并行推进，与统一主链在 `Aletheon_Unified_Execution_Plan_2026-08-04.md` 的 X13 汇合；R 轨道的调度表是统一计划 §3.4，本计划只保留设计契约与 R0–R8 的域内关闭判定。
> **DO-NOT-SCHEDULE**：本文件 §6「PR 切分」表**仅为历史记录，禁止作为调度输入**（依赖与状态上限以统一计划 §3.4 为准）；§6.1 阶段交付矩阵与 §6.2 完成模板仍然有效，且 §6.2 的模板已被统一计划 §8.5 采纳为全轨道通用模板。
> **X13 前置**：X13 不含实机，因此 R6 达到 `code_complete` 即满足 X13 前置，不要求 R6 `accepted`；R7 要求 `accepted`（见统一计划 §3.4）。
> 已知事实：`aletheon-kuavo-bridge` 已在本地独立项目实现；本计划不重复开发 bridge。
> 第一验收目标：Aletheon 接收自然语言任务，调用 Policy/VLA，经过受控技能执行 Kuavo
> MuJoCo，持续观察并确定性验证结果，生成可追踪 `EpisodeReport`。

> 当前交付约束：本轮实现者负责 Aletheon 仓库内的代码、契约和确定性测试；用户已明确
> 暂不要求物理实机验证。MuJoCo/Bridge/真实 Policy 和安装态全链路仍保留为 R8 生产关闭条件，
> 在没有真实运行证据时不得把本计划整体标记完成。

## 0. 当前结论

最新 `dev` 已经不处于“只有简单配置”的阶段。以下生产主链已经存在：

```text
TurnRequest
 -> HarnessKind::Robot bootstrap
 -> RobotCognitiveSession
 -> RobotHarness
 -> PolicyProviderPort / GrpcPolicyProvider
 -> SkillProposal validation
 -> EmbodimentExecutionPort
 -> Kernel admission + Broker
 -> GrpcEmbodimentProvider
 -> local Kuavo bridge
 -> ROS / MuJoCo
 -> WorldStatePump
 -> DeterministicOutcomeVerifier
 -> SQLite EpisodeSink
 -> EpisodeReport
 -> matched+settled promotion to Mnemosyne
```

本计划不再新建另一个 robot runtime crate，也不把 WBC/MPC/IK、电机闭环或 ROS 节点搬进
Aletheon。Aletheon 是任务、策略治理、权限、执行编排、验证和证据平面；bridge 与机器人栈
是设备接入和实时控制平面。

### 0.1 本轮交付层级

| 层级 | 本轮是否要求 | 可宣称的结论 |
|---|---|---|
| 单元/契约测试 | 是 | 类型、状态机和边界行为满足契约 |
| In-process robot session | 是 | Aletheon 内部 Turn→EpisodeReport 链路成立 |
| Direct live Policy/Bridge diagnostic | 否 | 只有真实执行后才能声明外部协议可用 |
| MuJoCo 场景 | 否 | 只有真实执行后才能声明仿真闭环通过 |
| Installed `/usr/bin/aletheon` E2E | R8 要求，本轮可暂缓 | 没有证据时计划状态为 `code_complete`，不是 `accepted` |
| 实机 | 否，另受 R6 gate 约束 | 仿真通过也不得声明 real-ready |

### 0.2 执行状态与纪律

1. 阶段状态使用 `not_started`、`in_progress`、`code_complete`、`externally_blocked`、`accepted`；
2. 每阶段开始前重新读取所列代码符号，路径/行为与本文不一致时先更新事实定位；
3. 不复制 bridge 源码到本仓库，不引入 ROS/MuJoCo 类型到 Fabric/Cognit；
4. 不为固定自然语言验收句、Kuavo 名称或固定 checkout path 增加生产分支；
5. Policy/VLA 只能提出 semantic skill，不能获得执行端口或输出底层电机命令；
6. 所有 Cargo 命令通过 `bash scripts/cargo-agent.sh ...`，优先最窄 package/target；
7. 每个 PR 输出变更文件、契约、验证证据、外部依赖、回滚方式和未完成项；
8. 外部服务缺失时保留 typed failure 和 gated test，不得用 Stub 成功冒充生产通过。

---

## 1. 当前代码事实

### 1.1 已实现

| 能力 | 当前 owner/文件 | 状态 |
|---|---|---|
| Embodiment domain contract | `fabric::types::embodiment` | 已实现 |
| Expected outcome / verification | `fabric::types::expected_outcome`、`outcome_verification` | 已实现 |
| Episode report | `fabric::types::episode_report` | 已实现 |
| gRPC bridge protocol | `crates/hardware/proto/.../gateway.proto` | 已实现 |
| gRPC provider | `hardware::grpc::provider` | 已实现 |
| Provider registry/Broker | `hardware` | 已实现 |
| Kernel admission/authority | `executive::application::embodiment_authority` | 已实现 |
| Robot state machine | `cognit::harness::robot` | 已实现 |
| Proposal validation | `cognit::harness::robot::proposal_validator` | 已实现 |
| Production Robot bootstrap | `executive::host::daemon::bootstrap::robot` | 已实现 |
| Policy gRPC client | `cognit::adapters::policy::grpc_provider` | 已实现 |
| World state pump | `executive::application::world_state` | 已实现 |
| Stable-window verifier | `deterministic_outcome_verifier` | 已实现 |
| Durable attempts | `sqlite_episode_sink` | 已实现 |
| Turn output report | `RobotCognitiveSession` | 已实现 |
| Session event spine progress | `embodiment_progress` + bootstrap binding | 已实现 |
| Mnemosyne promotion trigger | `robot_episode_promotion` | 已实现 |
| Real bridge execute E2E | `executive/tests/robot_bridge_execute_e2e.rs` | 已实现、外部 gated |

### 1.2 生产路径的关键行为

1. `build_robot_session_factory` 先从 embodiment provider 调 `list_skills(device)`；空技能表
   fail closed，得到的 descriptor 成为 Policy 和 validator 的 allowlist。
2. `HarnessKind::Robot` 需要真实 `ALETHEON_POLICY_ENDPOINT`；缺失或无法连接时不回退到
   `StubRobotPolicy`。
3. `Observe` 没有 fresh snapshot 时直接失败，不把空观测交给 Policy。
4. Policy 只产生 `SkillProposal`，不执行动作；proposal 必须经过 allowlist/schema/device/
   expected-outcome 校验。
5. Execute 经过 Executive/Kernel/Hardware 主链，不允许 Policy 绕过 authority 直连 bridge。
6. Verify 使用执行后的 observation 和 continuous stable window。
7. durable attempt/verification 是 report 的权威来源；artifact 只引用，不内嵌 rosbag/图像。
8. 只有 `EpisodeReport::can_promote()` 的 matched+settled episode 才进入长期记忆。

### 1.3 剩余缺口

1. Policy endpoint 仍由 `ALETHEON_POLICY_ENDPOINT` 提供，没有进入 typed TOML config；
2. `GrpcPolicyProvider` 支持 frame URI/label/confidence，但 RobotHarness 当前向 Policy 传 `&[]`；
3. `DefaultPlanPort::plan/replan` 仍 fail closed，失败后的受限重规划未配置；
4. `RobotHarnessConfig::default()` 在生产 composition 中缺少 per-device/per-scene 配置入口；
5. bridge 与 Policy capability/protocol compatibility 主要在连接后暴露，启动前诊断不足；
6. real bridge E2E 证明 execute/observe，但完整 natural-language + real Policy + bridge 测试仍缺；
7. 实机相对仿真的硬安全 capability、watchdog、ownership 和部署 gate 尚未形成统一证据；
8. episode artifact 的长期保存、清理和 report 可访问性需要完整策略。

---

## 2. 目标边界

### 2.1 Aletheon 负责

- 接收自然语言目标并创建 robot turn；
- 解析有效 provider/device/policy 配置；
- 获取 fresh world state 和 perception references；
- 调用 Policy/VLA 生成语义技能 proposal；
- 校验 proposal、执行权限和风险；
- 调用受控 embodiment skill；
- 观察执行进度和执行后状态；
- 用确定性 predicate 验证，不让 LLM 主观裁决；
- 持久化 attempts、receipts、evidence refs 和 report；
- 失败时有限 retry/replan/safe-stop；
- 对 matched+settled episode 做受控记忆 promotion。

### 2.2 Bridge / 机器人栈负责

- ROS/MuJoCo/真机 SDK 接入；
- skill 到机器人控制接口的翻译；
- 实时状态和 sensor/frame 引用；
- joint/velocity/torque/current 等硬限制；
- watchdog、heartbeat、emergency stop；
- control-mode ownership；
- 实时 WBC/MPC/IK/RL/电机闭环；
- 设备侧 operation identity 和 evidence 生成。

### 2.3 Policy/VLA 负责

- 根据 goal、world snapshots、frame refs 和 allowed skills 提 proposal；
- 返回 confidence、expected outcome 和 provenance；
- 不发送 CAN/EtherCAT、电机目标或未注册 arbitrary trajectory；
- 不直连 bridge 执行动作。

---

## 3. 目标配置

把 Policy 和 RobotHarness 参数加入 Executive typed config。建议形态：

```toml
[integrations.embodiment]
kind = "grpc"
device_id = "kuavo-mujoco-01"
endpoint = "http://127.0.0.1:50051"
connect_timeout_ms = 5000
request_timeout_ms = 10000

[integrations.robot]
scene_version = "kuavo-mujoco-v1"
bridge_protocol_digest = "sha256:..."
world_poll_interval_ms = 250
max_observation_age_ms = 500
max_attempts = 3
max_replans = 1
safe_stop_on_failure = true

[integrations.robot.policy]
kind = "grpc"
endpoint = "http://127.0.0.1:50052"
protocol_version = "v1"
connect_timeout_ms = 5000
request_timeout_ms = 30000

[integrations.robot.perception]
enabled = true
max_frames = 4
max_frame_age_ms = 500
allowed_schemes = ["file", "shm", "http"]
```

secret/token 不放 checked-in TOML；允许对应环境变量覆盖。环境变量只作为配置 source，最终必须
进入 resolved typed config 和 host-owned runtime facts。

### 3.1 外部依赖与仓库边界

| 能力 | Aletheon 本仓库负责 | Bridge/Policy 外部项目负责 | 缺失时行为 |
|---|---|---|---|
| Device/skill discovery | typed client、descriptor 校验、runtime facts | 返回设备、skill/schema、协议身份 | Robot bootstrap fail closed |
| Policy proposal | request/response contract、allowlist/schema 校验 | VLA 推理、模型 provenance、协议兼容 | typed provider failure，不回退 Stub |
| Perception | FrameRef 选择、freshness/预算/policy | frame 产生、URI 生命周期、sensor identity | required perception 缺失则 fail closed |
| Execute/observe | authority、operation identity、attempt/report | ROS/SDK 翻译、实时执行、sequence evidence | attempt failed，按策略 safe stop |
| Hard safety | capability gate、approval、上层 safe-stop request | watchdog、limits、ownership、emergency stop | real device 禁止启动 |
| Artifact | manifest、digest、retention 引用和 tombstone | rosbag/frame/log 实体及可访问 URI | 报告保留“不可用/已过期”事实 |

外部协议需要变化时，Aletheon PR 必须记录所需 proto 字段、最低 bridge/policy 版本和兼容失败行为；
外部仓库尚未发布不阻止纯 Aletheon 契约代码合并，但该阶段最多为 `externally_blocked`。

---

## 4. 分阶段实施

## Phase R0：基线冻结与证据清单（P0）

### 修改范围

- 本文档；
- `docs/testing/robot-runtime.md`（若当前不存在，先确认是否应扩展现有机器人测试文档，禁止创建重复入口）；
- 不修改生产行为。

### 工作项

1. 记录 bridge repository/version、proto digest、Kuavo scene/version、ROS/MuJoCo 启动方法；
2. 记录当前 Aletheon commit、配置、device ID 和 bridge endpoint；
3. 将测试分为：
   - deterministic unit/contract；
   - in-process robot session；
   - direct live bridge diagnostic；
   - installed full runtime acceptance；
4. 禁止把 `#[ignore]` live test 的存在写成当次已经通过；
5. 建立测试 artifact 目录和清理策略。
6. 记录本轮“无物理实机验证”的范围，历史运行记录只能作为背景，不作为本次 acceptance evidence。

### 完成条件

- 每条“已完成”能力都链接具体类型、测试或运行证据；
- bridge 本身不再列为 Aletheon 未实现项；
- diagnostic 与 installed acceptance 明确区分。

### 实施结果复核（2026-08-05）

> **当前判定**：`docs/testing/robot-runtime.md` 仍未入库，并缺 bridge commit/version、proto digest 与 scene version。按统一计划 §3.4，R0 保持 `in_progress`；B0 入库且补齐这些字段后才能转为 `accepted`。

```text
STATUS: in_progress
PHASE: R0（基线冻结与证据清单；不修改生产行为）
SCOPE:
  - docs/testing/robot-runtime.md
  - 本计划 R0 段
CONTRACT:
  - 无生产代码变化
VALIDATION:
  - 测试分层与“#[ignore] live test 不等于当次通过”纪律已写入
  - 已完成能力链接到 fabric/cognit/executive 类型或 gated 测试
  - 尚缺：bridge commit/version、proto digest、Kuavo scene/version
RUNTIME EVIDENCE:
  - 无本轮新运行证据；历史仿真记录仅作背景
ROLLBACK:
  - 无生产路径影响
BLOCKER:
  - 外部版本事实尚未采集；B0 尚未入库证据文件
OPEN ITEMS:
  - 补齐 R0 版本事实后转 accepted
  - R1-R7；R8 安装态真实 Policy+Bridge+MuJoCo
```

## Phase R1：Robot/Policy typed config（P0）

### 修改范围

- `crates/executive/src/composition/config/*`
- `crates/executive/src/host/daemon/bootstrap/robot.rs`
- `config/aletheon.kuavo-mujoco.example.toml`
- config schema/snapshot/layered contract tests。

### 工作项

1. 增加 `RobotIntegrationConfig`、`RobotPolicyConfig`、`RobotPerceptionConfig`；
2. 将 `ALETHEON_POLICY_ENDPOINT` 降为 override source，不在 bootstrap 直接读取环境变量；
3. 所有 duration 使用 checked conversion，并设置上下界；
4. 非 loopback 明文 gRPC 继续拒绝，或要求显式 reviewed insecure override；
5. scene version、Aletheon version、bridge protocol digest 来自 resolved config/build facts；
6. `RobotHarnessConfig::default()` 不再作为生产唯一来源；
7. debug/config inspect 能显示生效 endpoint（隐藏 secret）和 device/scene。

### 验收

- 缺 Policy config 时 `HarnessKind::Robot` fail closed；
- 配置层级覆盖测试通过；
- 非法 timeout、空 device、空 protocol version 被拒绝；
-普通 Linear harness 不要求 robot config；
-旧 embodiment-only config 的兼容策略被明确测试或明确迁移失败信息。

## Phase R2：启动 capability/compatibility gate（P0）

### 修改范围

- `cognit::adapters::policy::grpc_provider`
- `hardware::grpc::provider`
- `executive::host::daemon::bootstrap::robot`
- Fabric 中已有 descriptor/provenance 类型，只有确实缺字段时才扩展。

### 工作项

1. Policy 启动执行 Health + GetCapabilities；
2. Bridge/provider 启动获取 skill descriptors、observation schemas 和协议版本；
3. 检查：
   - device 是否存在；
   - allowlist 是否非空；
   - skill input schema 是否合法；
   - Policy protocol 是否兼容；
   - bridge protocol digest 是否匹配允许集合；
   - required observation schema 是否可用；
4. capability snapshot 写入 runtime facts/启动日志，不依赖模型声称；
5. 连接失败、协议不兼容和无技能分别给出 typed error。

### 完成条件

- bridge 可连但 device 不存在时不能启动 RobotHarness；
- Policy healthy 但协议不兼容时不能启动；
- skill descriptor 变化能进入 episode provenance；
-启动 gate 不执行任何机器人动作。

## Phase R3：Perception / FrameRef 数据链（P0）

### 当前问题

`PolicyProviderPort::propose` 已接收 `visual: &[PerceptionObservation]`，但
`RobotHarness::step(Plan)` 传空 slice。需要让 Cognit 消费抽象 perception port，而不是依赖
ROS、camera SDK 或 bridge 具体实现。

### 设计

新增或复用端口：

```rust
#[async_trait]
pub trait RobotPerceptionPort: Send + Sync {
    async fn latest(
        &self,
        device: &DeviceId,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PerceptionObservation>, String>;
}
```

Owner：抽象 contract 在 Cognit/Fabric 合理边界，bridge/world-state adapter 在 Executive/
Hardware；Cognit 不依赖 Hardware。

### 工作项

1. 从 embodiment observation 中提取/映射 `FrameRef` 和 perception metadata；
2. 每帧保留 source、sequence、captured/received time、confidence、content digest；
3. Plan 前按 max age、device 和 sequence 过滤；
4. URI scheme 和 workspace 访问受 policy 约束；
5. 不把 base64 大图复制进 session events、SQLite episode 或 prompt；
6. 把选中的 frame refs 传给 Policy，并记录在 `SkillProposal.provenance/frame_refs`；
7. 没有视觉但 skill 不要求视觉时允许继续；required perception 缺失时 fail closed。

### 测试

- fresh/stale frame；
- device/schema 隔离；
- URI scheme 拒绝；
- frame 数量和字节预算；
- RobotHarness 确实把非空 visual 传给 recording policy；
- report 只包含 refs/digest，不包含原图。

### 完成条件

- VLA 能收到与 world snapshot 同一设备、时间可解释的 frame refs；
- stale frame 不进入 proposal；
- perception port 故障不会绕过 required-perception gate。

## Phase R4：真实 VLA Policy Gateway 接入（P0）

### 协议边界

Policy 只能返回：

```rust
SkillProposal {
    skill,
    device,
    parameters,
    expected_outcome,
    confidence,
    frame_refs,
    provenance,
}
```

### 工作项

1. 用真实 gateway 替换测试 Stub；
2. proposal request 包含 goal、fresh snapshots、visual refs、allowed skill IDs 和 protocol version；
3. provenance 至少包含 provider、model、version、digest；
4. validator 增加/确认：
   - device 完全匹配；
   - skill 在 descriptor allowlist；
   - parameters 满足 JSON schema；
   - expected outcome path/schema 可验证；
   - timeout/stable window 在系统上限内；
   - confidence 有限且在合法范围；
5. 空 proposal、多个非法 proposal、gateway timeout 全部进入明确失败状态；
6. Policy 不获得 EmbodimentExecutionPort，架构上不能直接 execute。

### 完成条件

- 一个自然语言 goal 能通过真实 Policy 产生合法 semantic skill；
-恶意/错误 proposal 无法越过 validator；
-记录的 model/protocol 来自响应和连接事实，不来自自然语言输出；
- Policy timeout 会触发受控失败/安全策略，不会重复无限请求。

## Phase R5：受控 replan/recovery（P1）

### 当前问题

`DefaultPlanPort` 仍明确返回 `robot planner not configured`。这保证 fail closed，但完整失败恢复
尚未完成。

### 设计

不让 LLM 直接重写任意动作；replan request 必须是有限状态：

```rust
pub struct ReplanContext {
    pub goal: String,
    pub device: DeviceId,
    pub latest_snapshot: WorldSnapshot,
    pub failure_class: RobotFailureClass,
    pub completed_attempts: Vec<AttemptSummary>,
    pub allowed_skills: Vec<SkillDescriptor>,
    pub retries_remaining: u32,
    pub replans_remaining: u32,
}
```

### 工作项

1. 把字符串 failure reason 收敛为 typed failure class；
2. Policy gateway 增加 Replan 或复用 Propose 加 typed prior-attempt context；
3. 每次 replan 输出仍走 validate/authorize/execute/verify；
4. 同一 proposal/参数/相同世界状态重复失败时触发 loop detector；
5. 超出 retry/replan budget 后 safe stop + settle failed；
6. cancel、provider disconnect、verification timeout 分别定义 recovery；
7. unsafe predicate 或 hard safety event 禁止 replan，直接 safe stop。

### 完成条件

- retry 和 replan 分开计量；
-不会在不变状态下无限重复同一动作；
- safe stop 失败被记录为独立严重错误，不能覆盖原始失败；
- EpisodeReport 包含所有 durable attempts。

## Phase R6：实机安全 capability（P1）

### 目标

仿真成功不自动授权真机。设备 descriptor 必须声明 execution environment 和 safety capability：

```text
simulation | hil | real
watchdog
heartbeat
emergency_stop
joint_limits
velocity_limits
torque_or_current_limits
control_ownership
safe_stop
```

### 工作项

1. bridge/provider capability 增加环境和 safety manifest；
2. real device 缺少强制 capability 时 bootstrap fail closed；
3. lease/ownership 丢失立即阻断新动作；
4. heartbeat 超时触发 bridge 本地 watchdog，不能依赖云端 Agent；
5. Aletheon safe stop 是上层请求，设备侧还必须有独立硬停路径；
6. 高风险 skill 要求显式 approval policy，不能由 VLA confidence 代替；
7. sim/HIL/real 使用不同 profile 和 deployment gate。

### 完成条件

- 修改配置不能把 simulation device 冒充 real-ready；
-断网/daemon crash 时 bridge watchdog 有独立测试证据；
-越界 proposal 在 bridge/驱动硬限制和 Aletheon validator 两层都被拒绝；
- emergency stop 不经过 LLM。

## Phase R7：Episode artifact 和报告闭环（P1）

### 修改范围

- `fabric::types::episode_report`
- `executive::adapters::episode::sqlite_episode_sink`
- robot audit/promotion；
- artifact store/retention 的现有抽象。

### 工作项

1. 定义 artifact manifest：URI、digest、media type、size、producer、time range；
2. rosbag、日志、plot、frame 只作为引用；
3. report 中记录 before/after/verified sequence；
4. report settlement 与 TurnStop 映射保持单一权威；
5. attempt append、verification update、episode close 的 crash recovery 测试；
6. promotion 只消费 immutable settled report；
7. failed episode 保留证据但不自动变成成功经验；
8. retention 删除 artifact 时保留 tombstone/digest，报告明确“证据已过保留期”。

### 完成条件

- 多次 attempt 报告顺序和 operation ID 正确；
- SQLite 重启后能重建相同 report；
-大 artifact 不进入事件总线和数据库 JSON；
- report 可追踪到 model、bridge protocol、scene、skill descriptor 和验证事实。

## Phase R8：完整安装态 E2E（P0 closure）

### 场景

自然语言任务：

```text
让 kuavo-mujoco-01 进入站立状态并保持稳定 3 秒；如果无法满足稳定条件，执行安全停止并生成报告。
```

### 必须覆盖的真实链路

```text
/usr/bin/aletheon client
 -> official user socket
 -> installed daemon
 -> RobotCognitiveSession
 -> real Policy gateway
 -> Kernel admission
 -> Hardware Broker/gRPC provider
 -> local bridge
 -> ROS/MuJoCo
 -> observation/stable-window verification
 -> durable episode/report
```

### 证据

- release binary、`/usr/bin/aletheon` 与动态枚举到的所有运行中 Aletheon daemon executable SHA-256 一致（至少 machine core、user daemon，以及运行中的 Memory Agent）；
- systemd restart counter 前后稳定；
-生效配置和 endpoint identity；
- Policy model/version/protocol；
- bridge version/protocol digest；
- skill descriptor digest；
- before/after/verified sequences；
-每次 attempt 的 operation ID；
- verification predicate、stable window 和结果；
- safe-stop 负向场景；
-最终 `EpisodeReport` 和 SQLite 重启后读取结果。

直接运行 `robot_bridge_execute_e2e --ignored` 只能作为 diagnostic，不能替代本阶段。

---

## 5. 测试矩阵

| 层级 | 测试 | 外部依赖 | 目的 |
|---|---|---|---|
| Fabric contract | proposal/outcome/report serde + invariant | 无 | ABI 正确性 |
| Cognit unit | state transitions/proposal validation | 无 | 状态机边界 |
| Executive unit | world pump/verifier/episode sink | 无 | adapter 正确性 |
| In-process session | `robot_session_e2e` | 无 | Turn 到 report |
| Bridge diagnostic | `robot_bridge_execute_e2e --ignored` | bridge+MuJoCo | execute/observe 主链 |
| Policy diagnostic | gRPC policy provider live test | Policy | proposal 协议 |
| Full diagnostic | dev binary + Policy + bridge | 全部 | 开发态排障 |
| Installed acceptance | `/usr/bin/aletheon` official socket | 全部 | 生产关闭证据 |

负向测试必须包括：

- 无 fresh observation；
- stale/错 device frame；
-空 allowlist；
-非法 skill/参数/schema；
- Policy timeout/协议不兼容；
- bridge disconnect；
- execute 成功但 expected outcome 不匹配；
- stable window 中途跌落；
- episode write/update/close 失败；
- cancellation；
- safe stop 失败；
- retry/replan budget 耗尽。

---

## 6. 历史 PR 切分（DO-NOT-SCHEDULE）

> **本表仅为历史记录，禁止作为调度输入。** R 轨道的调度表是
> `Aletheon_Unified_Execution_Plan_2026-08-04.md` §3.4（含每阶段状态上限与 X13 前置）。
> 本节保留原文只为追溯当初的风险/前置判断。下面的 §6.1 与 §6.2 **仍然有效**。

| PR | 内容 | 风险 | 前置 |
|---|---|---|---|
| R0 | 基线/证据清单与测试分层 | 低 | 无 |
| R1 | Robot/Policy typed config | 中 | R0 |
| R2 | startup capability gate | 中 | R1 |
| R3 | perception port + fresh frame selection | 中高 | R1 |
| R4 | real VLA policy compatibility/validation | 高 | R2-R3 |
| R5 | typed failure + bounded replan | 高 | R4 |
| R6 | safety capability manifest | 高 | R2，可并行 |
| R7 | artifact retention/report recovery | 中 | R1，可并行 |
| R8 | full installed E2E and evidence | 高，外部系统 | R4-R7 |

每个 PR 必须保持 Cognit 不依赖 Executive/Hardware；不能为了快速接 Kuavo 把 ROS 类型放入 Fabric
或 Cognit。

### 6.1 阶段交付矩阵

| 阶段 | 本轮最小验证 | 关键负向断言 | 阶段结束状态上限 |
|---|---|---|---|
| R0 | 文档路径、类型和测试入口核对 | 不把历史 live test 当本次通过 | `accepted` |
| R1 | Executive config/layered contract 定向测试 | 缺 config、空 device、非法 timeout、明文远端 | `accepted` |
| R2 | Cognit/Hardware/Executive capability contract | 不存在 device、空 allowlist、协议/schema 不兼容 | 无外部服务时 `code_complete` |
| R3 | RobotHarness/perception adapter 定向测试 | stale、错 device、URI/数量/字节超限 | `accepted` |
| R4 | Policy provider + proposal validator 定向测试 | timeout、恶意 skill、schema/device/confidence 错误 | 无真实 gateway 时 `code_complete` |
| R5 | Robot state/replan/session 定向测试 | loop、预算耗尽、unsafe event、safe-stop 失败 | `accepted` |
| R6 | safety manifest/bootstrap contract | real device 缺 watchdog/ownership/e-stop | 无真实设备证据时 `code_complete` |
| R7 | Episode sink/report recovery 定向测试 | crash recovery、artifact 过期、failed promotion | `accepted` |
| R8 | 安装态真实 Policy+Bridge+MuJoCo E2E | stale、disconnect、verification fail、safe stop | 只有真实证据才 `accepted` |

所有测试先定位现有最窄 target，并用 `bash scripts/cargo-agent.sh` 执行。只有集成验证负责人可以运行
workspace-wide 检查；不得并发运行 `executive` 或 workspace build。

### 6.2 每个 PR 的固定完成模板

```text
STATUS: code_complete | externally_blocked | accepted
BASE: origin/dev commit
PHASE: R0-R8
SCOPE: 修改文件和明确未修改的外部仓库
CONTRACT: config/proto/public type/state transition 的变化
VALIDATION: 完整命令、退出码和关键负向断言
RUNTIME EVIDENCE: in-process/direct live/installed，明确属于哪一层
SAFETY: authority、freshness、watchdog/safe-stop 影响
ROLLBACK: 独立回滚 commit、兼容/迁移说明
OPEN ITEMS: 外部 Bridge/Policy/MuJoCo/物理实机 gate 分开记录，不得写成当前已完成
```

---

## 7. 明确不做

1. 不在 Aletheon 内实现 WBC、MPC、IK、RL policy 或电机驱动闭环。
2. 不允许 LLM/VLA 直接输出 CAN FD、EtherCAT 或未经注册的关节命令。
3. 不新建与现有 `EmbodimentExecutionPort` 并行的第二套机器人执行 ABI。
4. 不让自然语言回答成为任务成功的权威；权威是 durable attempt + verification + report。
5. 不用缓存代替 fresh robot observation。
6. 不因仿真通过就自动放开实机。
7. 不在 Aletheon 仓库复制本地 bridge 源码；通过版本化协议和独立仓库维护。

---

## 8. 项目负责人的评审点

你不需要逐行实现，但每阶段应亲自确认：

1. **控制边界：** VLA 输出的是 semantic skill，不是电机命令；
2. **实时边界：** WBC/MPC/驱动/watchdog 留在机器人实时栈；
3. **安全边界：** Aletheon policy 和 bridge 硬限制是两层，不互相替代；
4. **验证边界：** 成功由数值 predicate + stable window 判断；
5. **证据边界：** report 能追到 frame、skill、operation、bridge、模型和 observation sequence；
6. **部署边界：** dev test、bridge diagnostic 和 installed acceptance 不能混用。

---

## 9. 最终关闭标准

本计划完成必须同时满足：

1. typed config 覆盖 embodiment、Policy、perception 和 RobotHarness budget；
2. 启动时验证 device、skills、Policy/bridge protocol 和 required schemas；
3. RobotHarness 向真实 VLA 传入 fresh world snapshots 和非空 frame refs；
4. proposal 经过 allowlist/schema/device/risk/expected-outcome/authority；
5. bridge 执行动作后通过 continuous stable-window verifier；
6. 失败能 bounded retry/replan，并在预算耗尽时 safe stop；
7. 全部 attempt、verification 和 artifact refs 可从 durable sink 重建；
8. matched+settled report 才能 promotion；
9. MuJoCo 全链路通过安装态 `/usr/bin/aletheon`、official socket 和真实 Policy/bridge 验收；
10. 实机启用前另行满足 safety capability、watchdog、ownership 和 emergency-stop gate。

在用户当前“暂不做物理实机验证”的约束下，本轮应执行 R0-R7，并尽可能完成 R8 所需的
安装态测试脚手架、gated 场景和证据采集代码。物理实机缺失不自动阻塞 R8；没有本次真实 Policy/Bridge/MuJoCo 运行结果时，最终
汇报必须写“代码关闭，生产验收待执行”，不得删除本计划或标记整体完成。
