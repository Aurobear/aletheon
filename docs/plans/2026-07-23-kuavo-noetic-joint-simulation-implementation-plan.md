# Kuavo ROS Noetic 联合仿真实现计划

> **交给 DeepSeek 执行：** 严格按阶段和复选框推进；每个阶段先写失败测试，再实现，再运行指定验证。不得把 ROS、Kuavo、MuJoCo 的类型或名称引入 Aletheon 通用领域协议。

**目标：** 打通 `Aletheon → 通用 gRPC embodiment contract → 私有 Bridge → ROS Noetic → MuJoCo`，先完成只读观测，再完成受控的 `stop` 和短时速度技能，最后完成故障安全验证。

**架构：** Aletheon 只依赖通用 `EmbodimentExecutionPort` 和 gRPC contract；所有 ROS topic、message、控制器状态和厂商语义都封装在相邻私有仓库 `../aletheon-kuavo-bridge`。Bridge 使用单写者命令仲裁、状态新鲜度门禁、deadline/lease watchdog 和零速度安全停止。

**技术栈：** Rust、Tokio、Tonic、Python 3.8、`grpc.aio`、ROS Noetic `rospy`、`nav_msgs/Odometry`、`geometry_msgs/Twist`、pytest。

---

## 0. 需求锚点与不可破坏约束

本计划落实以下原始要求：

- ROS Noetic 适配必须位于独立私有 sidecar：`docs/plans/Aletheon_Robot_Cognitive_Inference_Architecture_and_Coupling_Plan.md:533-543`。
- Bridge 负责私有接口映射、消息转换、取消、降采样、断连检测和本地安全策略：同文件 `:545-553`。
- 第一版领域接口为 `snapshot/list_skills/execute_skill/cancel/safe_stop`：同文件 `:555-561`。
- ROS 类型与公司私有 action 名不得进入 Aletheon core：同文件 `:563`。
- P2 要求 observation 限频、聚合、去重，以及断连和 lease expiry fail-safe：同文件 `:715-723`。
- `OperationId`、progress/cancel/timeout、崩溃安全停止和 provider 可替换性必须验证：同文件 `:778-787`。

硬约束：

1. 不修改 `crates/fabric` 的协议来容纳 ROS topic 或 ROS message。
2. 不在 Aletheon 仓库增加 `rosrust`、ROS message generation、catkin 或厂商 SDK。
3. 不把 `/cmd_vel`、`/odom` 等名字放入 protobuf；它们只允许出现在 Bridge 配置与 ROS provider。
4. 第一阶段禁止运动，仅允许读取状态。
5. `/cmd_vel` 当前已有两个 publisher；未完成单写者检查前禁止 Bridge 发布非零速度。
6. `safe_stop` 必须可在 gRPC 请求取消、deadline 超时、lease 到期和 ROS master 断连时调用。
7. 不以 RPC 返回的 `"succeeded"` 作为唯一成功证据；运动结果必须由新鲜 odometry 验证。

## 1. 已验证现状

```text
ROS/MuJoCo
  /ground_truth/state       nav_msgs/Odometry     ~878 Hz
  /odom                     nav_msgs/Odometry     ~1007 Hz
  /humanoid_mpc_observation ocs2_msgs/...         ~423 Hz
  /cmd_vel                  geometry_msgs/Twist   已有两个 publisher

aletheon-kuavo-bridge
  gRPC service              已实现
  FakeEmbodimentProvider    已实现并被 server.py 写死使用
  Kuavo Noetic provider     目录存在但实现为空

Aletheon
  GrpcEmbodimentProvider    已实现
  六个 robot tools          已注册
  有效 runtime provider     被 Default::default() 覆盖为内置 simulator
  RobotHarness production factory 尚未接入；不阻塞 P2 工具级联调
```

关键代码锚点：

- Bridge 写死 Fake provider：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/server.py:16-38`。
- Bridge 通用 provider protocol：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/base.py:75-99`。
- Aletheon provider 枚举：`crates/executive/src/composition/config/integrations.rs:113-158`。
- Aletheon gRPC provider 构造：`crates/executive/src/host/daemon/bootstrap/embodiment.rs:25-78`。
- runtime 错误覆盖配置：`crates/executive/src/composition/user_runtime/mod.rs:94-98`、`crates/executive/src/core/runtime_core.rs:122-126`。
- robot tools 注册：`crates/executive/src/host/daemon/bootstrap/request.rs:735-749`。
- RobotHarness 尚未接入通用工厂：`crates/cognit/src/harness/mod.rs:67-82`。

## 2. 最终数据流

```text
robot.observe / robot.execute_skill
        │ generic Fabric DTO
        ▼
Executive EmbodimentService
        │ generic gRPC proto
        ▼
Bridge EmbodimentGatewayService
        │ EmbodimentProvider protocol
        ▼
KuavoNoeticMujocoProvider
        ├── StateCache  ◄── /odom, /ground_truth/state
        ├── CommandArbiter ──► /cmd_vel
        ├── OperationRegistry
        └── SafetyWatchdog ──► zero Twist
```

频率边界：

- ROS 原始输入：约 400–1000 Hz；
- Provider 内存缓存：每个 callback 覆盖最新样本，不排队保存原始流；
- `snapshot()`：按请求生成，Bridge 配置目标 5 Hz；
- progress：最多 10 Hz；
- `/cmd_vel`：技能执行期间固定 20 Hz，结束后连续发布 5 帧零速度；
- Aletheon 不接收高频原始 telemetry。

## 3. 阶段 A：固化 Bridge provider 选择机制

### Task A1：增加 provider 配置

**文件：**

- 修改：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/config.py`
- 修改：`../aletheon-kuavo-bridge/config/bridge.example.yaml`
- 测试：`../aletheon-kuavo-bridge/tests/unit/test_config.py`

- [ ] 在 `BridgeConfig` 增加 `provider_kind: str = "fake"`。
- [ ] 只接受 `fake` 和 `kuavo_noetic_mujoco`；其他值启动即失败。
- [ ] 增加 ROS 映射配置，但只存在于 Bridge：

```yaml
provider_kind: kuavo_noetic_mujoco
ros:
  master_uri: http://127.0.0.1:11311
  node_name: aletheon_kuavo_bridge
  odom_topic: /odom
  ground_truth_topic: /ground_truth/state
  command_topic: /cmd_vel
  command_hz: 20
  zero_stop_frames: 5
  publisher_exclusion:
    - /humanoid_joy_control_auto_gait_with_vel
    - /humanoid_quest_control_with_arm
```

- [ ] 校验 topic 必须是以 `/` 开头的绝对名，`command_hz` 范围为 10–50，`zero_stop_frames` 范围为 3–20。
- [ ] 测试未知 provider、相对 topic、越界频率均 fail closed。

运行：

```bash
cd /home/aurobear/Workspace/aletheon-kuavo-bridge
.venv/bin/pytest tests/unit/test_config.py -q
```

预期：全部通过。

### Task A2：增加 provider factory，删除 server 对 Fake 的硬编码

**文件：**

- 新建：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/factory.py`
- 修改：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/server.py:13-40`
- 新建测试：`../aletheon-kuavo-bridge/tests/unit/test_provider_factory.py`

- [ ] `build_provider(config)` 在 `fake` 时返回 `FakeEmbodimentProvider`。
- [ ] `kuavo_noetic_mujoco` 使用延迟 import，避免非 ROS 开发环境导入 `rospy` 即失败。
- [ ] server 只调用 factory，不 import 具体 ROS provider。
- [ ] 测试 fake 选择成功、未知 kind 失败、ROS 模块缺失时错误包含可操作信息。

提交建议：

```text
feat(bridge): select embodiment provider from validated config

The bridge server always constructed the fake provider, so a configured ROS
deployment could never reach simulation state.

- add fail-closed provider kind validation
- centralize provider construction behind a factory
- keep ROS imports lazy for non-ROS contract tests
```

## 4. 阶段 B：实现只读 ROS Provider

### Task B1：实现线程安全最新状态缓存

**文件：**

- 新建：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/state_cache.py`
- 新建测试：`../aletheon-kuavo-bridge/tests/unit/test_ros_state_cache.py`

- [ ] 定义 `OdomSample`：sequence、source_time_ms、received_monotonic_ms、frame_id、position、orientation、linear_velocity、angular_velocity。
- [ ] 使用 `threading.Lock`；ROS callback 只做常数时间字段复制。
- [ ] 拒绝 source timestamp 倒退的样本。
- [ ] `snapshot(now_ms, freshness_ms)` 返回深拷贝，并计算 `stale`。
- [ ] 不保存无限历史，不在 callback 内执行 gRPC/asyncio 操作。

测试必须覆盖：

- sequence 单调递增；
- 时间倒退样本不覆盖新样本；
- 超过 500 ms 标记 stale；
- 并发读写不产生半更新 payload。

### Task B2：实现 ROS lifecycle 和观测转换

**文件：**

- 新建：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/provider.py`
- 修改：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/__init__.py`
- 新建测试：`../aletheon-kuavo-bridge/tests/contract/test_kuavo_snapshot.py`

- [ ] `KuavoNoeticMujocoProvider` 实现 `EmbodimentProvider` 全部方法。
- [ ] 构造时初始化独立 ROS node，并订阅配置中的 odom 和 ground-truth topic。
- [ ] `snapshot(device_id)` 输出两个通用 observation：
  - `schema="base_pose"`：位置和 quaternion；
  - `schema="base_twist"`：linear/angular velocity。
- [ ] `source` 使用 provider/device 逻辑身份，不暴露 topic 名。
- [ ] `source_time_ms` 来自 ROS header；`received_time_ms` 来自接收时钟。
- [ ] `confidence`：fresh 为 `1.0`，stale 为 `0.0`。
- [ ] `health()` 至少报告 `ros_master`、`odom_stream`、`ground_truth_stream` 三个 component。
- [ ] ROS master 不可达返回 `unavailable`；主 master 正常但状态 stale 返回 `degraded`。
- [ ] 本阶段的 `execute_skill` 除 `stop` 外全部拒绝，错误不得伪装成功。

不要读取 `/humanoid_mpc_observation` 作为第一版世界状态。该消息是控制器内部高维状态，既不稳定也不适合进入通用协议。

### Task B3：ROS 容器内只读 smoke test

**文件：**

- 新建：`../aletheon-kuavo-bridge/scripts/ros-readonly-smoke.sh`
- 新建：`../aletheon-kuavo-bridge/tests/integration/test_ros_readonly_live.py`

运行环境必须 source：

```bash
source /opt/ros/noetic/setup.bash
source /root/kuavo_ws/devel/setup.bash
export ROS_MASTER_URI=http://127.0.0.1:11311
```

验收：

1. 5 秒内 `health=ready`；
2. 连续 20 次 snapshot 的 sequence 单调；
3. observation age 小于 500 ms；
4. Bridge 进程退出前后 `/cmd_vel` publisher 数不增加；
5. 不发布任何非零命令。

提交建议：

```text
feat(bridge): expose normalized ROS odometry snapshots

The gRPC bridge had no implementation reading the running ROS simulation.

- cache odometry callbacks without retaining raw telemetry
- normalize pose and twist behind the provider protocol
- fail closed on stale state or ROS master loss
```

## 5. 阶段 C：打通 Aletheon gRPC 配置

### Task C1：修正 provider 配置透传

**文件：**

- 修改：`crates/executive/src/composition/user_runtime/mod.rs:90-99`
- 修改：`crates/executive/src/core/runtime_core.rs:118-127`
- 测试：`crates/executive/tests/embodiment_provider_config.rs`
- 新建测试：`crates/executive/tests/runtime_embodiment_selection.rs`

- [ ] 从有效 `AppConfig.integrations.embodiment` 取得 provider。
- [ ] 配置缺失时才使用 `EmbodimentProviderConfig::default()`。
- [ ] 删除两个无条件 `embodiment_provider: Default::default()`。
- [ ] 测试配置 `kind = grpc` 后，生成的 daemon request 保留 endpoint、device_id 和 timeout。
- [ ] 测试未配置时仍选择内置 simulator，保持向后兼容。

Rust 命令必须遵守仓库约束：

```bash
cd /home/aurobear/Workspace/aletheon
bash scripts/cargo-agent.sh test -p executive --test runtime_embodiment_selection
bash scripts/cargo-agent.sh test -p executive --test embodiment_provider_config
```

禁止直接运行 `cargo`。

### Task C2：增加联合仿真配置样例

**文件：**

- 修改：`config/aletheon.example.toml`
- 新建：`config/aletheon.kuavo-mujoco.example.toml`

样例只使用通用字段：

```toml
[integrations.embodiment]
kind = "grpc"
device_id = "kuavo-mujoco-01"
endpoint = "http://127.0.0.1:50051"
connect_timeout_ms = 5000
request_timeout_ms = 10000
```

注意：具体 ROS topic 不得出现在这个文件。

### Task C3：只读端到端测试

启动顺序：

```text
1. ROS/MuJoCo roslaunch
2. KuavoNoetic Bridge :50051
3. Aletheon daemon with gRPC embodiment config
4. robot.list_skills
5. robot.observe
6. robot.get_state
```

验收：

- Aletheon 启动日志确认选择 gRPC，而非 simulator；
- `robot.observe(device="kuavo-mujoco-01")` 返回实时 base pose/twist；
- sequence/source timestamp 变化；
- 停止 ROS master 后 500 ms 内 observation stale，命令接口 fail closed；
- 重启 ROS 后可恢复，无需重启 Aletheon。

## 6. 阶段 D：安全停止与命令所有权

### Task D1：实现命令 publisher 单写者门禁

**文件：**

- 新建：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/command_arbiter.py`
- 新建测试：`../aletheon-kuavo-bridge/tests/fault_injection/test_command_ownership.py`

- [ ] 通过 ROS master system state 查询 `/cmd_vel` publishers。
- [ ] 只要发现不在 `publisher_exclusion` 预期清单之外的活动控制 publisher，就拒绝非零命令。
- [ ] Bridge 不尝试杀死其他 ROS node；仅 fail closed 并报告冲突。
- [ ] zero Twist 始终允许发布，用于安全停止。
- [ ] 非零命令要求：health ready、状态 fresh、lease 未过期、deadline 未过期、没有活跃 operation。

当前已观察到 `/cmd_vel` 有：

- `/humanoid_joy_control_auto_gait_with_vel`
- `/humanoid_quest_control_with_arm`

在明确这些节点是否持续发布以及如何进入外部控制模式之前，`move_base_timed` 必须保持禁用。

### Task D2：实现 stop

**文件：**

- 新建：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/skills/stop.py`
- 修改：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/provider.py`
- 新建测试：`../aletheon-kuavo-bridge/tests/integration/test_stop_live.py`

- [ ] `safe_stop` 和 `kuavo.stop` 共用同一个幂等实现。
- [ ] 以 20 Hz 连续发布至少 5 帧全零 Twist。
- [ ] 发布后等待 fresh odometry，要求平移速度和角速度连续稳定窗口低于阈值。
- [ ] 超时则返回 failed，而不是 succeeded。
- [ ] stop 不把位姿重置为零；FakeProvider 当前重置位置的行为不得复制到 ROS provider。

### Task D3：实现 lease/deadline watchdog

**文件：**

- 新建：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/watchdog.py`
- 修改：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/grpc_service.py`
- 新建测试：`../aletheon-kuavo-bridge/tests/fault_injection/test_watchdog.py`

- [ ] 每个 operation 注册 deadline 和 lease expiry。
- [ ] watchdog 使用 monotonic clock，不使用可回拨 wall clock 做超时判断。
- [ ] cancel、RPC stream 中断、deadline、lease expiry 均进入同一 `safe_stop` 路径。
- [ ] `safe_stop` 完成前 operation 不能 settlement 为成功。
- [ ] 重复 cancel/stop 幂等。

验收故障矩阵：

| 故障 | 预期 |
|---|---|
| gRPC 客户端取消 | 立即零速度，result=cancelled |
| deadline 到期 | 立即零速度，result=timed_out |
| lease 到期 | 本地 watchdog 零速度 |
| ROS master 断连 | provider unavailable；若 publisher 仍可用则尽力零速度 |
| odometry stale | 禁止新非零命令；活跃技能停止 |
| Bridge SIGKILL | 外部控制器必须依靠命令超时归零；若 ROS 控制器无此能力，不得开放运动技能 |

## 7. 阶段 E：短时速度技能（满足门禁后才实现）

### Task E1：确认 `/cmd_vel` 控制契约

执行前必须给出证据：

1. 哪个节点拥有 gait/control mode；
2. 外部 `/cmd_vel` 与现有两个 publisher 如何仲裁；
3. 输入超时后控制器是否自动归零；
4. stance → walking → stance 的可观测判据；
5. MuJoCo 环境中最小安全速度和持续时间。

如果任一项未知，停止在只读 + stop，不实现非零运动。

### Task E2：实现 `move_base_timed`

**文件：**

- 新建：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/skills/move_base_timed.py`
- 修改：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/provider.py`
- 新建测试：`../aletheon-kuavo-bridge/tests/integration/test_move_base_timed_live.py`

参数：

```json
{
  "linear_x": 0.05,
  "linear_y": 0.0,
  "angular_z": 0.0,
  "duration_ms": 500
}
```

实现规则：

- 对每个字段做有限数检查，拒绝 NaN/Inf；
- 同时应用编译时硬上限、部署上限、skill manifest 上限，取三者最小值；
- 发布频率 20 Hz；
- progress 最多 10 Hz；
- duration 结束、cancel 或异常时都在 `finally` 中调用 stop；
- 成功要求 odometry 位移方向与命令一致且超过噪声阈值，并最终回到速度稳定窗口；
- 未发生可观测位移时返回 mismatch/failed。

首轮 live test 只允许：

- `linear_x=0.05 m/s`
- `duration_ms=500`
- 仿真环境
- 人工观察 RViz/MuJoCo
- 测试前后自动 stop

## 8. 阶段 F：完整联合验收

### Task F1：Bridge 自动化验证

```bash
cd /home/aurobear/Workspace/aletheon-kuavo-bridge
.venv/bin/pytest tests/unit tests/contract tests/fault_injection -q
.venv/bin/ruff check src tests
.venv/bin/mypy src
```

ROS live tests必须串行运行，不与其他运动测试并发：

```bash
.venv/bin/pytest tests/integration/test_ros_readonly_live.py -q
.venv/bin/pytest tests/integration/test_stop_live.py -q
.venv/bin/pytest tests/integration/test_move_base_timed_live.py -q
```

### Task F2：Aletheon 窄范围验证

```bash
cd /home/aurobear/Workspace/aletheon
bash scripts/cargo-agent.sh test -p hardware --test grpc_provider
bash scripts/cargo-agent.sh test -p hardware --test grpc_contract
bash scripts/cargo-agent.sh test -p executive --test embodiment_provider_config
bash scripts/cargo-agent.sh test -p executive --test runtime_embodiment_selection
bash scripts/cargo-agent.sh fmt --all -- --check
```

只有 integration/verification owner 才能运行 workspace-wide check。

### Task F3：端到端验收记录

必须保存以下证据：

- ROS node/topic/service inventory；
- Bridge capabilities 和 health 响应；
- 20 次 observation 的 sequence、timestamp、age；
- OperationId 从 Aletheon request 到 Bridge result 的一致性；
- cancel/deadline/lease/ROS disconnect 的结果；
- 每次运动前后的 odometry；
- stop 后的稳定速度窗口；
- Aletheon 和 Bridge 日志中不得出现原始密钥或高频 telemetry dump。

## 9. 明确不在本计划范围

以下项目单独规划，不能混入本次 P2 联调：

1. RobotHarness production `CognitiveSessionFactory`；
2. P3 世界模型、OutcomeVerifier 与 EmbodiedEpisode 完整接线；
3. 手臂 action；
4. `/humanoid_mpc_observation` 高维状态协议；
5. 真机/HIL/TLS production gate；
6. ROS2、VLA、RL provider。

P2 完成后，Linear Harness 已可通过六个通用 robot tools 操作仿真；RobotHarness 是后续认知闭环增强，不应成为 ROS provider 联调的前置条件。

## 10. DeepSeek 执行停止条件

遇到下列任一情况立即停止并报告证据，不得猜接口：

- `/cmd_vel` 存在无法解释的并发 publisher；
- 找不到控制器的输入超时归零机制；
- odometry 超过 500 ms 不更新；
- gRPC proto 与两仓库生成代码 hash 不一致；
- 修改需求跨入 `fabric` ROS 类型或公司私有接口；
- live test 需要超过 `0.05 m/s` 或 `500 ms` 才能“看出效果”；
- safe stop 无法由独立 watchdog 触发。

## 11. 推荐提交顺序

1. `feat(bridge): select embodiment provider from validated config`
2. `feat(bridge): expose normalized ROS odometry snapshots`
3. `fix(executive): preserve configured embodiment provider`
4. `feat(bridge): enforce command ownership and safe stop`
5. `feat(bridge): stop active skills on lease expiry`
6. `feat(bridge): execute bounded timed base motion`
7. `test(simulation): verify Aletheon Kuavo fault-safe loop`

每个非平凡提交必须包含 conventional subject、空行、问题/方案说明和具体变更列表；提交前检查 staged diff，不得把两个仓库的修改误放进同一个提交。
