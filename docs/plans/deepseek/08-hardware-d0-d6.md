# Hardware Control D0–D6

**总入口状态：被阻塞。** 解锁条件：W5-10 通过。D0 解锁后严格串行。

| 阶段 | 状态 | 唯一解锁条件 | 固定验收 |
|---|---|---|---|
| D0 API+Simulator | 被阻塞 | W5-10 通过 | deterministic simulator、lease/deadman/stop 状态机 contract 全绿 |
| D1 Linux read-only | 前置条件未满足，禁止执行 | D0 完成；Linux runner 暴露 udev/sysfs 只读 fixture | 发现、断连、时钟、遥测背压测试全绿；无 actuator write syscall |
| D2 ROS 2 simulation | 前置条件未满足，禁止执行 | D1 完成；固定 ROS 2 Jazzy + Gazebo Harmonic image digest | topic/service/action 映射、仿真急停、重连全绿 |
| D3 Serial/CAN | 前置条件未满足，禁止执行 | D2 完成；登记 USB loopback、SocketCAN/vcan 与非危险 CAN fixture | framing/CRC、bus-off、CAN-FD、过滤与权限全绿 |
| D4 Controlled actuators | 前置条件未满足，禁止执行 | D3 完成；双层实体急停、限位开关、独立 watchdog、隔离测试台验收签字 | lease expiry、deadman、limit、E-stop 均在设备侧和 broker 侧生效 |
| D5 Robot HIL | 前置条件未满足，禁止执行 | D4 完成；机器人、护栏实验区、操作者、旁站员和恢复流程已登记 | kill-9、网络断开、时钟漂移、传感器冻结、指令风暴演练全绿 |
| D6 Multi-device/sites | 前置条件未满足，禁止执行 | D5 连续 100 次 HIL 全绿；两个隔离站点可用 | 跨站身份、租约、断网自治、审计复制与恢复全绿 |

## D0 固定任务

1. 创建 `hardware-api`：DeviceId、DeviceManifest、DeviceState、TypedCommand、Lease、StopLevel、TelemetryEnvelope、Provider/Broker traits。
2. 创建 `hardware-broker`：登记、健康、租约、命令授权、deadman、stop escalation；禁止直接持有设备驱动。
3. 创建 `hardware-sim`：虚拟时钟、确定性传感器/执行器、故障注入、记录回放。
4. 只读接入 Kernel capability：默认只暴露 discover/state/telemetry；command capability 在 D4 前必须不存在。
5. 验证：`bash scripts/cargo-agent.sh test -p hardware-api`、`... -p hardware-broker`、`... -p hardware-sim`、`bash tests/architecture_check.sh`。

## 禁止事项

- D4 前禁止任何真实 actuator write。
- 设备侧安全不得依赖 Agent、daemon 或网络存活。
- 缺少门禁设备时禁止用 mock 结果宣称对应阶段完成。
- 每个阶段验收未全部通过时禁止进入下一阶段。
