# Phase E 仿真门禁确认回复

> **用途：** 回复 Phase E 的 5 个前置问题，并给出 `move_base_timed` 是否可以开始实现的明确结论。

**验证环境：**

- Docker 容器：`kuavo_container_4fcbbae6`
- 启动命令：`roslaunch humanoid_controllers load_kuavo_mujoco_sim.launch`
- ROS：Noetic
- 仿真：MuJoCo
- 当前控制器：`mpc`
- 验证日期：2026-07-23

**对应计划：**

- 5 个门禁问题：`docs/plans/2026-07-23-kuavo-noetic-joint-simulation-implementation-plan.md:368-380`
- 首轮运动参数：同文件 `:390-417`
- stop 稳定性要求：同文件 `:330-341`
- watchdog 要求：同文件 `:343-366`

---

## 一、结论摘要

5 个问题均已通过源码检查和运行中 MuJoCo 探测得到明确答案，不再存在“未知接口”。

| 门禁 | 结论 | 是否明确 | 对 Phase E 的影响 |
|---|---|---:|---|
| 1. 谁拥有 gait/control mode | 当前由 `mpc` controller 执行；H12 FSM 管理输入状态，gait 由 MPC gait scheduler 执行 | 是 | 可以实现 |
| 2. 两个 `/cmd_vel` publisher 如何仲裁 | 当前仿真中 Joy 空闲，Quest 没有输入设备；真机 H12/VR 由既有状态机互斥 | 是 | 当前仿真无冲突 |
| 3. 输入超时是否自动归零 | 当前 MPC 仿真不可依赖自动归零；停止发送 2 秒后仍有运动 | 是，答案为否 | 必须由 Bridge 显式 stop/watchdog |
| 4. 如何观察 stance→walk→stance | 主 topic 为 `/humanoid_mpc_gait_time_name`，service 用于初始化/回查，odom 用于验证真实停止 | 是 | 可以实现确定性验证 |
| 5. 最小安全速度和时间 | `linear_x=0.05 m/s`、`duration=500 ms` 已在仿真触发可观测 walk | 是 | 作为首轮测试上限 |

**决策：**

> Phase E 的接口发现门禁已经解除，可以实现 `move_base_timed`。但在生产代码真正接通 watchdog、统一 stop 和状态稳定验证之前，不得把 Phase E 标记为完成。

---

## 二、门禁 1：gait/control mode 所有权

原问题把三个概念合并在了一起，实际应拆分为：

1. **输入控制权：** 谁产生运动目标；
2. **gait：** 当前采用 `stance/walk/trot` 中哪种步态；
3. **controller：** 当前由哪套底层控制算法执行。

### 2.1 当前 controller

运行命令：

```bash
rosservice call /humanoid_controller/get_controller_list
```

实测：

```text
controller_names:
  - mpc
current_controller: "mpc"
```

当前 MuJoCo launch 实际只加载了 `mpc`。第一阶段目标控制器范围确定为：

```text
mpc       当前仿真已验证
amp_hand  代码支持目标；加载进当前仿真并验证后再开放 capability
```

其他 controller 本阶段不支持。

### 2.2 gait 范围

第一阶段只支持：

```text
stance
walk
trot
```

明确不支持：

```text
climb_stair
其他自定义 gait
```

### 2.3 H12/VR 输入状态机

H12 状态机包含 `vr_remote_control` 状态：

`/root/kuavo_ws/src/humanoid-control/h12pro_controller_node/robot_state/robot_state_machine.py:14-31,34-77`

普通摇杆只有在非 VR、非头控、非导航状态才会转发：

```python
if current_state != "vr_remote_control" \
        and not self.head_control_mode \
        and not self.is_navigation_mode:
    self._handle_joystick_input(msg)
```

源码：

`/root/kuavo_ws/src/humanoid-control/h12pro_controller_node/src/h12pro_node/ocs2_h12pro_node.py:854-904`

紧急停止在普通摇杆裁决之前处理：

`ocs2_h12pro_node.py:698-715`

因此真机控制关系是：

```text
H12/G12 channel
       │
       ▼
H12 input FSM
       ├─ emergency stop：始终优先
       ├─ normal state：允许 Joy 运动输入
       └─ vr_remote_control：屏蔽 Joy 普通运动输入
                               │
                               ▼
                           Quest/VR 接管
```

### 2.4 H12 monitor 的职责

`ocs2_h12pro_monitor` 管理自动/手动 H12 节点树的生命周期和所有权，不负责 gait 算法：

`/root/kuavo_ws/src/humanoid-control/h12pro_controller_node/scripts/monitor_ocs2_h12pro.sh:67-123`

它使用 `/start_way` 和 `/h12_yield_done` 避免服务侧与手动侧重复启动 `/joy_node`。

---

## 三、门禁 2：`/cmd_vel` publisher 仲裁

ROS graph 中注册了：

```text
/humanoid_joy_control_auto_gait_with_vel
/humanoid_quest_control_with_arm
```

但“publisher 注册存在”不等于“正在产生活跃命令”。

### 3.1 当前仿真实际状态

Quest 输入 topic：

```text
/quest_joystick_data
Publishers: None
```

因此当前没有 Quest 设备，也没有 Quest 输入流。

Joy 输入来自：

```text
/joystickSimulator
```

空闲时连续监测 `/cmd_vel`：

```text
no new messages
```

Joy 源码也明确在长期空闲时停止发布零速，以免抢占外部 `/cmd_vel`：

`/root/kuavo_ws/src/humanoid-control/humanoid_interface_ros/src/newTargetPublisher/HumanoidAutoGaitJoyCommandNodeWithVel.cpp:951-979`

所以当前仿真实际为：

```text
Joy节点：注册但空闲
Quest节点：注册但无设备输入
Bridge：可以成为唯一活跃非零命令源
```

### 3.2 真机仲裁

真机 H12 与 Quest 的互斥由 H12 FSM 处理，不需要 Bridge 再造一套设备状态机。

Bridge 仍需保证：

- 同一时刻只有一个 Bridge operation；
- observation 必须新鲜；
- 不在已知 VR/导航/动作执行状态中抢占控制；
- `safe_stop` 永远允许；
- 不以 ROS publisher endpoint 数量作为“活跃命令流”的唯一判断。

当前 `CommandArbiter` 只有 endpoint allowlist 逻辑：

`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/command_arbiter.py:9-38`

它不能区分“已注册 publisher”和“正在发送控制流”，因此不得把发现两个 publisher 本身作为 Phase E 阻塞条件。

---

## 四、门禁 3：控制器输入超时是否自动归零

### 4.1 实测方法

在 MuJoCo 中执行：

```text
发布频率：20 Hz
linear.x：0.05 m/s
持续时间：0.5 s
随后停止发布：2.0 s
最后显式发布：10 帧 zero Twist
```

### 4.2 实测结果

停止发布后：

```text
0.25 s：gait=walk
0.50 s：gait=walk
1.00 s：gait=walk
1.50 s：gait=walk
2.00 s：gait=walk
```

2 秒时 odometry 仍出现明显速度：

```text
linear.y  ≈ -0.147 m/s
angular.z ≈ -0.275 rad/s
```

显式发布零速度后恢复：

```text
gait: stance
linear.x: 接近 0
linear.y: 约 0.000007 m/s
angular.z: 约 -0.000015 rad/s
```

### 4.3 结论

> 当前 MPC MuJoCo 仿真不能依赖 `/cmd_vel` 输入超时自动归零。

因此 `move_base_timed` 的所有退出路径必须显式 stop：

```text
正常完成
cancel
deadline
lease expiry
RPC stream 中断
ROS state stale
内部异常
```

统一进入：

```text
连续 zero Twist
    → gait 回到 stance
    → odometry 连续稳定窗口
    → 才允许 settlement
```

---

## 五、门禁 4：stance→walk→stance 的可观测判据

### 5.1 权威变化 topic

```text
/humanoid_mpc_gait_time_name
```

类型：

```text
kuavo_msgs/gaitTimeName

float32 start_time
string gait_name
```

仿真实测事件：

```text
start_time: 910.06
gait_name: "walk"

start_time: 910.62
gait_name: "stance"
```

因此可以直接观察：

```text
stance → walk → stance
```

该 topic 由 gait receiver/reference manager 发布，现有代码也使用它判断 MPC gait：

`/root/kuavo_ws/src/humanoid-control/humanoid_plan_arm_trajectory/script/arm_trajectory_bezier_process.py:188-203,407-408`

### 5.2 启动初始化和异常回查

`/humanoid_mpc_gait_time_name` 是变化事件，不保证稳定状态下周期发布。Bridge 启动时应调用：

```text
/humanoid_get_current_gait_name
```

随后由 topic 更新缓存。

### 5.3 辅助信号

```text
/humanoid/GaitReceiver/is_walking
```

类型：

```text
std_msgs/Float64
```

它可作为辅助证据，但实测收到 `stance` gait event 后仍会短暂保持 `1.0`，不能单独作为最终状态权威。

### 5.4 推荐判据

进入 walk/trot：

```text
最新 gait event == 目标 gait
AND gait observation 新鲜
AND odometry 出现超过噪声阈值的运动
```

停止成功：

```text
最新 gait == stance
AND sqrt(vx² + vy²) < 0.01 m/s
AND abs(wz) < 0.02 rad/s
AND 上述条件连续保持 500 ms
AND odometry age < 500 ms
```

Bridge 应增加 gait cache：

```text
启动：service 初始化当前 gait
运行：gait_time_name topic 更新
验证：gait cache + odometry stable window
```

---

## 六、门禁 5：最小安全速度和持续时间

已验证首轮参数：

```json
{
  "linear_x": 0.05,
  "linear_y": 0.0,
  "angular_z": 0.0,
  "duration_ms": 500
}
```

结果：

- 能触发 `stance → walk`；
- 能产生可观测 odometry 变化；
- 显式 zero Twist 后能恢复 `stance`；
- 不需要为了“看出效果”提高速度或延长时间。

该参数应作为首轮 live test 的固定上限。

注意：`cmd_vel.linear.x` 是机器人 body frame 前进方向；odometry 位移是 world frame。结果验证必须按起始姿态将 world displacement 转换到 body frame，不能简单要求 world `Δx > 0`。

---

## 七、Phase E 实现范围

### 7.1 支持矩阵

| controller | gait | 当前状态 |
|---|---|---|
| `mpc` | `stance` | 支持并已验证 |
| `mpc` | `walk` | 支持并已验证 |
| `mpc` | `trot` | 目标支持；需增加对应 live case |
| `amp_hand` | `stance/walk/trot` | 目标支持；当前 launch 尚未加载，验证后开放 |
| 任意 | `climb_stair` | 明确不支持 |

Bridge 的运行时 capability 只能发布已经在当前 launch 中验证的组合。

### 7.2 `move_base_timed` 实现要求

实现：

- `linear_x/linear_y/angular_z/duration_ms` 有限数和范围校验；
- 固定 20 Hz 发布；
- 同时只允许一个 active movement operation；
- 进入 `walk` 后验证 gait event；
- 位移按起始姿态转换到 body frame；
- 正常、取消、超时和异常全部在 `finally` 中 stop；
- zero Twist 后等待 `stance + 500 ms stable odometry`；
- 未发生可观测位移返回 mismatch/failed；
- stop 验证失败时绝不能返回 succeeded。

首轮测试固定：

```text
controller: mpc
gait: walk
linear_x: 0.05 m/s
linear_y: 0
angular_z: 0
duration: 500 ms
```

---

## 八、Phase D 完成度复核

虽然单元测试和提交记录表明 Phase D 类已创建，但生产接线仍需复核。

当前代码搜索结果：

- `CommandArbiter` 只发现类定义；
- `OperationWatchdog` 只发现类定义；
- ROS provider 未发现两者的构造和调用；
- `_publish_zero_velocity()` 发完零帧后直接报告 succeeded；
- `safe_stop()` 未等待 gait/odom 稳定。

代码锚点：

- Arbiter：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/command_arbiter.py:9-38`
- Watchdog：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/watchdog.py:20-72`
- stop skill：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/provider.py:212-240`
- safe stop：同文件 `:248-256`
- gRPC Unix deadline 输入：`../aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/grpc_service.py:237-243`

Watchdog 使用 monotonic clock，而 gRPC 字段为 Unix time。接线时必须先转换剩余时长：

```text
remaining_ms = deadline_unix_ms - current_unix_ms
deadline_monotonic_ms = current_monotonic_ms + max(0, remaining_ms)
```

不得直接拿 Unix timestamp 与 monotonic timestamp 比较。

---

## 九、给执行者的最终回复

```text
5 个 Phase E 前置问题已经在当前 MuJoCo 仿真中确认。

1. 当前 controller 是 MPC；H12 FSM 管理输入状态，MPC gait scheduler 执行 gait。
2. 当前仿真 Joy 空闲，Quest 无输入设备，不存在活跃 /cmd_vel 冲突；
   真机 H12/Quest 的互斥由既有 vr_remote_control 状态机处理。
3. MPC 仿真停止接收命令后不会可靠自动归零，Bridge 必须在所有退出路径显式发送零速。
4. /humanoid_mpc_gait_time_name 可观察 walk/stance 变化；
   启动时用 /humanoid_get_current_gait_name 初始化，最终停止还需 odometry 稳定窗口。
5. 0.05 m/s、500 ms 已验证能产生可观测运动，是首轮测试固定上限。

因此可以开始实现 Phase E move_base_timed，但必须先把 Phase D 的 Arbiter、
Watchdog、统一 stop、deadline 时钟转换和 gait/odom 稳定验证真正接入生产链路。
完成这些接线及 live test 后，才能把 Phase D/E 标记为完成并进入 Phase F。
```

