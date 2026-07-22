# Aletheon–Kuavo MuJoCo 通用具身网关设计

> 状态：设计已确认，等待文件级实施计划
>
> 范围：P2 ROS Noetic / MuJoCo 桥接，不包含 P3 RobotHarness、视觉、VLA 或真机部署

## 1. 目标与事实基线

本设计落实机器人认知路线中的 P2：通过独立私有 Bridge 将 Aletheon P1 的
`EmbodimentExecutionPort` 接到 Kuavo ROS Noetic MuJoCo，同时保持 Aletheon 核心无 ROS、
Kuavo 和仿真器类型。

需求锚点：

- 独立 `aletheon-kuavo-bridge`、五个领域方法及本地失联保护：
  `docs/plans/Aletheon_Robot_Cognitive_Inference_Architecture_and_Coupling_Plan.md:535-563`；
- P2 要求白名单接口、状态限频去重、断连及 lease fail-safe：同文件 `:715-723`；
- Bridge 推荐目录边界：同文件 `:822-831`；
- Aletheon 当前领域端口包含 observe/get_state/list_skills/execute/cancel/safe_stop：
  `crates/fabric/src/types/embodiment.rs:109-127`。

Kuavo 当前事实：

- MuJoCo 标准入口是
  `roslaunch humanoid_controllers load_kuavo_mujoco_sim.launch`，并同时启动控制器、MPC、WBC
  与仿真器（`/home/aurobear/Workspace/kuavo-ros-control/readme.md:118-127`）；
- `/cmd_vel` 使用 `geometry_msgs/Twist`，非零值进入 walk、全零值回到 stance
  （`/home/aurobear/Workspace/kuavo-ros-control/docs/运动控制API.md:133-158`）；
- 轮臂环境存在 `/enable_control` 软急停语义，但其适用范围仅是轮臂 WBC
  （`/home/aurobear/Workspace/kuavo-ros-control/docs/轮臂相关接口文档说明.md:163-188`）。

最后一项不能直接作为标准双足 MuJoCo 的通用安全保证。实现前必须通过 ROS 接口发现确认目标
launch 暴露的控制与状态接口；缺失时 fail closed，不凭文档名称猜测。

## 2. 架构裁决

采用独立 Python `rospy` 服务和 localhost gRPC：

```text
Aletheon
  fabric domain DTOs
        |
  hardware::GrpcEmbodimentProvider
        |
        |  aletheon.embodiment.gateway.v1 (protobuf/gRPC)
        v
aletheon-kuavo-bridge
  generic gateway server
        |
  KuavoNoeticMujocoProvider
        |
        |  allow-listed ROS topic/service/action mappings
        v
Kuavo ROS Noetic -> MPC/WBC/controllers -> MuJoCo
```

新项目位于：

```text
/home/aurobear/Workspace/aletheon-kuavo-bridge
```

Bridge 可以读取 Kuavo ROS 接口，但不修改 `kuavo-ros-control` 核心代码。Aletheon 持有目标、
授权、lease、operation identity、deadline 和 settlement；Bridge 持有 ROS 映射、执行反馈、
状态聚合、取消和本地 fail-safe；Kuavo 保持高频控制与机器人安全权威。

## 3. 通用外部通信边界

Wire contract 命名为 `aletheon.embodiment.gateway.v1`，不能出现 Kuavo、ROS 或 MuJoCo 类型。
它是可复用的外部具身网关，不是厂商专用控制协议。

```proto
service EmbodimentGateway {
  rpc GetCapabilities(GetCapabilitiesRequest) returns (GetCapabilitiesResponse);
  rpc Snapshot(SnapshotRequest) returns (SnapshotResponse);
  rpc ListSkills(ListSkillsRequest) returns (ListSkillsResponse);
  rpc ExecuteSkill(ExecuteSkillRequest) returns (stream ExecuteSkillEvent);
  rpc Cancel(CancelRequest) returns (CancelResponse);
  rpc SafeStop(SafeStopRequest) returns (SafeStopResponse);
  rpc Health(HealthRequest) returns (HealthResponse);
}
```

约束：

1. protobuf 是 wire contract，Fabric DTO 是 domain contract；只在 Hardware adapter 显式转换；
2. endpoint、消息上限、deadline、并发与退避来自 typed config，不在业务代码硬编码；
3. 请求携带 request ID、operation ID、trace ID、device ID、protocol version 和 deadline；
4. 错误使用稳定 code/category/retryable 字段，禁止 Display 字符串分类；
5. 默认监听 `127.0.0.1`，非 localhost 部署必须启用 mTLS；
6. 日志不记录凭据、完整高频状态或无限参数 payload；
7. 健康状态区分 `ready`、`degraded`、`unavailable`；
8. capability handshake 明确协议版本、Provider、设备、技能和限制。

## 4. Bridge 内部接口与目录

```text
aletheon-kuavo-bridge/
├── pyproject.toml
├── proto/aletheon/embodiment/gateway/v1/gateway.proto
├── src/aletheon_kuavo_bridge/
│   ├── server.py
│   ├── config.py
│   ├── lifecycle.py
│   ├── operation_registry.py
│   ├── observation.py
│   ├── safety.py
│   ├── generated/
│   └── providers/
│       ├── base.py
│       └── kuavo_noetic/
│           ├── provider.py
│           ├── discovery.py
│           ├── state.py
│           ├── mappings.py
│           └── skills/
├── config/bridge.example.yaml
├── config/skills/kuavo_mujoco.yaml
├── launch/bridge.launch
├── scripts/bootstrap.sh
├── scripts/check.sh
├── scripts/run-mujoco-e2e.sh
├── tests/{unit,contract,integration,fault_injection}/
└── docs/{architecture,operations}.md
```

内部 Provider contract：

```python
class EmbodimentProvider(Protocol):
    async def snapshot(self, device_id): ...
    async def list_skills(self, device_id): ...
    async def execute_skill(self, request, events): ...
    async def cancel(self, operation_id): ...
    async def safe_stop(self, device_id, reason): ...
    async def health(self): ...
```

首个实现为 `KuavoNoeticMujocoProvider`。后续真机、Gazebo、Isaac Sim 或 ROS2 Provider 不改变
gRPC contract 与 Aletheon Cognit/Executive/Corpus。

## 5. 首批技能

### 5.1 `kuavo.stance`

请求稳定站立；成功必须由状态窗口确认，不能仅以 ROS publish/service 返回作为成功。

### 5.2 `kuavo.move_base_timed`

输入 `linear_x`、`linear_y`、`angular_z`、`duration_ms`。Bridge 校验速度和持续时间上限，
内部有限时长发布 `/cmd_vel`，并在完成、取消、deadline、lease expiry 或失联时发布零速度。
它是有界语义技能，不暴露任意 topic publish。

初始限制建议由配置声明：线速度不超过 0.25 m/s、角速度不超过 0.5 rad/s、持续时间不超过
3000 ms。实现必须允许部署配置进一步收紧，但不得超过编译/代码的硬安全上限。

### 5.3 `kuavo.execute_arm_action`

只执行 manifest 注册的命名动作。首个候选集合为 `wave`、`reset_arm`、`ready`，但这些名称必须
在目标 MuJoCo launch 中通过实际 service/action discovery 和一次手工仿真验证后才能进入默认
manifest；未验证的动作不注册，不能以 success sentinel 代替实现。

### 5.4 `kuavo.stop`

正常停止当前运动并回到稳定状态。它不同于 operation `Cancel`，也不同于故障优先级的设备级
`SafeStop`。

明确禁止：任意 topic/service、原始关节力矩、CAN/EtherCAT 写入、无限 `/cmd_vel`、动态控制器
上传、任意关节轨迹，以及绕过 MPC/WBC 的模型高频控制。

## 6. Observation 与 Snapshot

Bridge 从 ROS 高频状态构造有界快照，默认最多 5 Hz，对相同序列/内容去重。快照至少包含：

- device ID、simulation 标志、控制模式与 controller readiness；
- base pose、orientation 与低频 velocity；
- joint count/summary（完整数组不默认进入认知上下文）；
- active operation IDs、faults、source time、received time、sequence、staleness；
- 可选 artifact/evidence 引用，不内嵌 rosbag 或图像大对象。

状态源超过配置 freshness 阈值时标记 stale；关键状态 stale 时拒绝运动技能，并且不能把旧状态
作为成功证据。

## 7. Operation 状态机

```text
Received -> Validated -> Accepted -> Executing -> Succeeded
                         |              |  |  |
                         |              |  |  +-> TimedOut
                         |              |  +----> Failed
                         |              +-------> Cancelling -> Cancelled
                         +-----------------------> Rejected
```

规则：

- terminal 只能写入一次；
- 相同 operation ID + 相同 payload 返回已有状态/结果；
- 相同 operation ID + 不同 payload 拒绝；
- 已结束 operation 的 cancel 幂等成功；
- 每设备首版只允许一个运动技能；Snapshot、Health、ListSkills 可并行；
- Cancel/SafeStop 可抢占，SafeStop 不等待普通 operation 锁；
- progress 默认不超过 10 Hz，队列有界，慢消费者可合并中间进度但不能丢 terminal；
- gRPC stream 断开本身不决定取消，Bridge 按 deadline/lease 处理。

## 8. 本地 Fail-safe

SafeStop 顺序：

1. 阻止新的运动技能；
2. 标记并终止活动 operation；
3. 取消可取消 action；
4. 有界重复发布零速度；
5. 请求进入已验证的稳定控制模式；
6. 用新鲜状态确认；不能确认则保持 unavailable；
7. 记录 SafetyEvent 与 evidence。

Aletheon–Bridge 失联时，技能最多运行到最近 deadline/lease 边界；Bridge–ROS Master 失联时拒绝
新执行、终止活动 operation 并尝试本地停止。ROS 恢复后必须重新发现接口和同步状态，不恢复旧
operation。

SafeStop 不关机、不重启电脑，且不能依赖 Aletheon 的下一次 RPC 才生效。

## 9. 验证矩阵

### 9.1 Bridge 单元测试

- protobuf round-trip 与版本拒绝；
- typed config、manifest 和安全上限；
- operation 合法/非法迁移、幂等和 exactly-once terminal；
- deadline、progress 背压、snapshot 限频/去重/staleness；
- 日志与错误响应不泄漏 credential。

### 9.2 Fake ROS contract/fault tests

- 七个通用 RPC；
- accepted/progress/result stream；
- cancel、SafeStop 抢占、Provider exception；
- ROS 断连/恢复、lease expiry、状态陈旧；
- gRPC 客户端断连和重复 operation；
- 任意 topic/service 请求不可表达。

### 9.3 Kuavo MuJoCo 集成测试

以标准 launch 启动仿真后验证：接口发现、Health ready、Snapshot sequence、技能白名单、stance、
有界移动及归零、已验证手臂动作、cancel、lease fail-safe、ROS 断连拒绝、恢复后重新发现、幂等
SafeStop。

### 9.4 Aletheon E2E

仅将 P1 `SimulatedEmbodiment` 替换为 `GrpcEmbodimentProvider`：

```text
Corpus robot tool
 -> Executive EmbodimentService
 -> Kernel admission/lease
 -> GrpcEmbodimentProvider
 -> EmbodimentGateway
 -> KuavoNoeticMujocoProvider
 -> ROS/MuJoCo
 -> progress/result/snapshot
 -> settlement
```

所有 Aletheon Cargo 命令继续通过 `bash scripts/cargo-agent.sh ...`，使用最窄 package/target；只有
最终验证 owner 运行 workspace 范围检查。

## 10. 完成定义与非目标

P2 完成要求：

1. 同一 P1 Provider contract 在 gRPC Provider 上成立；
2. Cognit、Executive、Corpus 不出现 Kuavo/ROS 类型；
3. 模型只能调用注册技能；
4. operation/deadline/cancel/SafeStop 全链路可追踪；
5. 断连与 lease expiry 在 MuJoCo 集成测试中触发本地 fail-safe；
6. Provider 返回成功而新鲜状态没有变化时保留 mismatch evidence；
7. 替换 Provider 不修改 Cognit/Executive/Corpus；
8. 不修改 `kuavo-ros-control` 核心代码。

非目标：P3 RobotHarness/OutcomeVerifier、视觉/VLA、任意 ROS gateway、真机安全认证、多设备并行、
关节级或力矩级控制。

## 11. 实施分段

后续文件级计划应拆为可独立验收的阶段：

1. **G1 Wire contract**：proto、生成与协议 contract tests；
2. **G2 Bridge core**：config、operation、observation、safety、Fake Provider；
3. **G3 Kuavo MuJoCo adapter**：ROS discovery、snapshot、stance/timed move/stop；
4. **G4 Aletheon provider**：Hardware gRPC adapter、typed config、P1 contract reuse；
5. **G5 Named arm action**：只在实际接口验证后启用；
6. **G6 Fault/E2E**：断连、lease、cancel、SafeStop 和完整 settlement；
7. **G7 Operations**：一键 bootstrap/check/run、systemd/ROS launch 与故障说明。

每阶段先在本地验证，Bridge 与 Aletheon 分别提交和评审；不得用未接线 placeholder、固定 success
返回或 mock-only 验收宣称 P2 完成。
